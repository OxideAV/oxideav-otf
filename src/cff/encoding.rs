//! CFF Encoding (Adobe TN5176 §12).
//!
//! Maps a single-byte codepoint (0..=255) → glyph id. Used by legacy
//! PostScript pipelines; OpenType-CFF fonts almost always defer real
//! codepoint → GID resolution to the sfnt `cmap` table instead.
//!
//! Predefined encodings (top-DICT operator 16 == 0 or 1):
//! - 0: Standard Encoding (TN5176 Appendix B Section 1)
//! - 1: Expert Encoding (TN5176 Appendix B Section 2)
//!
//! Custom encodings come in two formats:
//! - Format 0 (`.0[]`): array of `(code: u8) → gid` indirection.
//! - Format 1 (`.1[]`): run-length encoded as `(first_code, n_left)*`.
//!
//! Both formats may be followed by a "supplemental" array of
//! additional `(code, sid)` pairs (high bit of the format byte =
//! 0x80). We accept-and-skip these in round 1.
//!
//! Round-1 implementation note: we only really need this to decode
//! single-byte legacy encodings and won't try to faithfully model
//! the predefined Standard / Expert tables here — those are large
//! lookup arrays in TN5176 Appendix B that are mostly exercised by
//! Type 1 fonts not OpenType-CFF. For the predefined encodings we
//! return `None` from `lookup`; callers should route through the
//! sfnt `cmap` table instead.

use crate::cff::charset::Charset;
use crate::cff::strings::{glyph_name_to_codepoint, Strings};
use crate::parser::{read_u16, read_u8};
use crate::Error;

#[derive(Debug, Clone)]
pub(crate) enum Encoding<'a> {
    Standard,
    Expert,
    /// Format 0 — `code[gid]` style indirection. Stores the raw
    /// payload (byte 1 onward) and the explicit n_codes count.
    Format0 {
        codes: &'a [u8],
    },
    /// Format 1 — run-length: `(start_code, n_left)*`.
    #[allow(dead_code)]
    Format1 {
        runs: &'a [u8],
    },
}

impl<'a> Encoding<'a> {
    pub(crate) fn parse(bytes: &'a [u8], top_off: i32) -> Result<Self, Error> {
        match top_off {
            0 => Ok(Self::Standard),
            1 => Ok(Self::Expert),
            n if n < 0 => Err(Error::Cff("negative encoding offset")),
            n => {
                let off = n as usize;
                if off >= bytes.len() {
                    return Err(Error::UnexpectedEof);
                }
                let format_byte = read_u8(bytes, off)?;
                // High bit (0x80) signals supplemental data; we don't
                // honour it but the format nibble in the low 7 bits
                // still applies.
                let format = format_byte & 0x7f;
                let after = off + 1;
                match format {
                    0 => {
                        let n_codes = read_u8(bytes, after)? as usize;
                        let payload = bytes
                            .get(after + 1..after + 1 + n_codes)
                            .ok_or(Error::UnexpectedEof)?;
                        Ok(Self::Format0 { codes: payload })
                    }
                    1 => {
                        let n_ranges = read_u8(bytes, after)? as usize;
                        let runs = bytes
                            .get(after + 1..after + 1 + n_ranges * 2)
                            .ok_or(Error::UnexpectedEof)?;
                        Ok(Self::Format1 { runs })
                    }
                    _ => Err(Error::Cff("unknown Encoding format")),
                }
            }
        }
    }

    /// Resolve a single-byte codepoint to a glyph id. Returns `None`
    /// if the encoding has no mapping for `code` or the predefined
    /// Standard/Expert encodings (which would need their full
    /// lookup tables; route through sfnt `cmap` instead).
    pub(crate) fn lookup(
        &self,
        code: u8,
        charset: &Charset<'_>,
        strings: &Strings<'_>,
    ) -> Option<u16> {
        match self {
            Self::Standard | Self::Expert => {
                // The predefined encodings map code → glyph-name;
                // we then need the inverse name → gid via charset.
                // Because we don't ship the full encoding tables in
                // round 1, this always returns None. The sfnt `cmap`
                // path on the public Font handles real Unicode
                // lookup, so this only impacts pure-PostScript users
                // (which are vanishingly rare for OpenType-CFF).
                let _ = (code, charset, strings);
                None
            }
            Self::Format0 { codes } => {
                // codes[gid - 1] = code. Linear search, n is small.
                for (i, &c) in codes.iter().enumerate() {
                    if c == code {
                        return Some(i as u16 + 1);
                    }
                }
                None
            }
            Self::Format1 { runs } => {
                // Walk runs, mirroring charset format-1.
                let mut gid: u16 = 1;
                let mut off = 0;
                while off + 1 < runs.len() {
                    let first = runs[off];
                    let n_left = runs[off + 1];
                    off += 2;
                    let last = first.saturating_add(n_left);
                    if code >= first && code <= last {
                        return Some(gid + (code - first) as u16);
                    }
                    gid = gid.saturating_add(n_left as u16 + 1);
                }
                None
            }
        }
    }
}

// Suppress unused-import lints for the legacy fallback hooks (kept
// available because round-2 / Standard-encoding work will need them).
#[allow(dead_code)]
fn _unused() {
    let _ = (
        read_u16 as fn(&[u8], usize) -> _,
        glyph_name_to_codepoint as fn(&str) -> _,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cff::charset::Charset;
    use crate::cff::index::Index;

    #[test]
    fn format0_lookup() {
        // n_codes=2, code[1]=65 ('A'), code[2]=66 ('B').
        let mut table = vec![0u8; 4]; // padding
        table.push(0); // format = 0
        table.push(2); // nCodes
        table.push(65);
        table.push(66);

        let enc = Encoding::parse(&table, 4).unwrap();
        let charset = Charset::IsoAdobe;
        let custom = Index::parse(&[0u8, 0], 0).unwrap();
        let strings = Strings::new(custom);
        assert_eq!(enc.lookup(65, &charset, &strings), Some(1));
        assert_eq!(enc.lookup(66, &charset, &strings), Some(2));
        assert_eq!(enc.lookup(67, &charset, &strings), None);
    }

    #[test]
    fn format1_run_lookup() {
        // n_ranges=1, first=65 ('A'), nLeft=2 → A, B, C → gids 1, 2, 3.
        let mut table = vec![0u8; 2];
        table.push(1); // format = 1
        table.push(1); // nRanges
        table.push(65);
        table.push(2);

        let enc = Encoding::parse(&table, 2).unwrap();
        let charset = Charset::IsoAdobe;
        let custom = Index::parse(&[0u8, 0], 0).unwrap();
        let strings = Strings::new(custom);
        assert_eq!(enc.lookup(65, &charset, &strings), Some(1));
        assert_eq!(enc.lookup(66, &charset, &strings), Some(2));
        assert_eq!(enc.lookup(67, &charset, &strings), Some(3));
        assert_eq!(enc.lookup(68, &charset, &strings), None);
    }
}
