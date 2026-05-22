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
//! Round-95 update: the Standard Encoding table (TN5176 Appendix B
//! §1) is now transcribed in full as [`STANDARD_ENCODING`]. It maps
//! `code: u8` → `SID: u16`. This is the table the deprecated four-arg
//! `endchar` (Type 1 `seac`) form uses to resolve its `bchar` and
//! `achar` glyph-name operands per Adobe TN5177 Appendix C, and we
//! also expose it through `Encoding::Standard::lookup` so legacy
//! Standard-encoded PostScript fonts decode without the sfnt-`cmap`
//! detour.

use crate::cff::charset::Charset;
use crate::cff::strings::{glyph_name_to_codepoint, Strings};
use crate::parser::{read_u16, read_u8};
use crate::Error;

/// CFF Standard Encoding table (Adobe TN5176 Appendix B §1).
///
/// `STANDARD_ENCODING[code]` is the SID a code unit `0..=255` resolves
/// to. SID `0` (`.notdef`) means the code is unassigned in the
/// Standard Encoding. Transcribed verbatim from TN5176 Appendix B §1
/// (4 Dec 03), pages 37-39.
pub(crate) const STANDARD_ENCODING: [u16; 256] = [
    // 0..15 — all unassigned
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, // 16..31 — all unassigned
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    // 32..47 — space, exclam, quotedbl, numbersign, dollar, percent,
    // ampersand, quoteright, parenleft, parenright, asterisk, plus,
    // comma, hyphen, period, slash
    1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16,
    // 48..63 — zero..nine, colon, semicolon, less, equal, greater, question
    17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32,
    // 64..79 — at, A..N, O
    33, 34, 35, 36, 37, 38, 39, 40, 41, 42, 43, 44, 45, 46, 47, 48,
    // 80..95 — P..Z, bracketleft, backslash, bracketright, asciicircum,
    // underscore
    49, 50, 51, 52, 53, 54, 55, 56, 57, 58, 59, 60, 61, 62, 63, 64,
    // 96..111 — quoteleft, a..o
    65, 66, 67, 68, 69, 70, 71, 72, 73, 74, 75, 76, 77, 78, 79, 80,
    // 112..127 — p..z, braceleft, bar, braceright, asciitilde, .notdef
    81, 82, 83, 84, 85, 86, 87, 88, 89, 90, 91, 92, 93, 94, 95, 0,
    // 128..143 — all .notdef
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, // 144..159 — all .notdef
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    // 160 .notdef, 161 exclamdown, 162 cent, 163 sterling, 164 fraction,
    // 165 yen, 166 florin, 167 section, 168 currency, 169 quotesingle,
    // 170 quotedblleft, 171 guillemotleft, 172 guilsinglleft,
    // 173 guilsinglright, 174 fi, 175 fl
    0, 96, 97, 98, 99, 100, 101, 102, 103, 104, 105, 106, 107, 108, 109, 110,
    // 176 .notdef, 177 endash, 178 dagger, 179 daggerdbl,
    // 180 periodcentered, 181 .notdef, 182 paragraph, 183 bullet,
    // 184 quotesinglbase, 185 quotedblbase, 186 quotedblright,
    // 187 guillemotright, 188 ellipsis, 189 perthousand, 190 .notdef,
    // 191 questiondown
    0, 111, 112, 113, 114, 0, 115, 116, 117, 118, 119, 120, 121, 122, 0, 123,
    // 192 .notdef, 193 grave, 194 acute, 195 circumflex, 196 tilde,
    // 197 macron, 198 breve, 199 dotaccent, 200 dieresis, 201 .notdef,
    // 202 ring, 203 cedilla, 204 .notdef, 205 hungarumlaut, 206 ogonek,
    // 207 caron
    0, 124, 125, 126, 127, 128, 129, 130, 131, 0, 132, 133, 0, 134, 135, 136,
    // 208 emdash, 209..223 all .notdef
    137, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    // 224 .notdef, 225 AE, 226 .notdef, 227 ordfeminine,
    // 228..231 .notdef, 232 Lslash, 233 Oslash, 234 OE,
    // 235 ordmasculine, 236..239 .notdef
    0, 138, 0, 139, 0, 0, 0, 0, 140, 141, 142, 143, 0, 0, 0, 0,
    // 240 .notdef, 241 ae, 242..244 .notdef, 245 dotlessi,
    // 246..247 .notdef, 248 lslash, 249 oslash, 250 oe, 251 germandbls,
    // 252..255 .notdef
    0, 144, 0, 0, 0, 145, 0, 0, 146, 147, 148, 149, 0, 0, 0, 0,
];

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
            Self::Standard => {
                // TN5176 §12 + Appendix B §1: the Standard Encoding
                // maps `code` → SID; we then look the SID up in the
                // font's charset to get a GID. SID 0 (.notdef) means
                // the code unit is unassigned in Standard Encoding.
                let _ = strings;
                let sid = STANDARD_ENCODING[code as usize];
                if sid == 0 {
                    return None;
                }
                charset.gid_of_sid(sid)
            }
            Self::Expert => {
                // The Expert encoding table is not transcribed in
                // round 1 (TN5176 Appendix B §2). Callers should
                // route through the sfnt `cmap` table instead — the
                // public `Font::glyph_index` already does.
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
    fn standard_encoding_landmark_codes() {
        // Spot-check entries against TN5176 Appendix B §1.
        // The table is `code → SID`, with 0 = .notdef (unassigned).
        assert_eq!(STANDARD_ENCODING[b' ' as usize], 1); // space
        assert_eq!(STANDARD_ENCODING[b'!' as usize], 2); // exclam
        assert_eq!(STANDARD_ENCODING[b'A' as usize], 34); // 'A'
        assert_eq!(STANDARD_ENCODING[b'Z' as usize], 59); // 'Z'
        assert_eq!(STANDARD_ENCODING[b'a' as usize], 66); // 'a'
        assert_eq!(STANDARD_ENCODING[b'z' as usize], 91); // 'z'
        assert_eq!(STANDARD_ENCODING[0], 0); // unassigned low
        assert_eq!(STANDARD_ENCODING[127], 0); // DEL — explicit .notdef
        assert_eq!(STANDARD_ENCODING[208], 137); // emdash
        assert_eq!(STANDARD_ENCODING[225], 138); // AE
        assert_eq!(STANDARD_ENCODING[241], 144); // ae
        assert_eq!(STANDARD_ENCODING[251], 149); // germandbls
                                                 // Boundary check: everything past the last assignment is 0.
        assert_eq!(STANDARD_ENCODING[252], 0);
        assert_eq!(STANDARD_ENCODING[255], 0);
    }

    #[test]
    fn standard_encoding_routes_through_charset() {
        // Build a Format-0 charset that has glyph 1 = SID 34 (= 'A' in
        // Standard Encoding). The encoding lookup should return GID 1.
        // Charset Format 0 SID layout: 2 bytes per gid starting at gid=1.
        let charset_payload = vec![0x00, 0x22]; // SID 34 = 'A'
        let charset = Charset::Format0 {
            bytes: &charset_payload,
            num_glyphs: 2, // .notdef + 1 real glyph
        };
        let custom = Index::parse(&[0u8, 0], 0).unwrap();
        let strings = Strings::new(custom);
        let enc = Encoding::Standard;
        assert_eq!(enc.lookup(b'A', &charset, &strings), Some(1));
        // 'B' (SID 35) is not present in the charset → None.
        assert_eq!(enc.lookup(b'B', &charset, &strings), None);
        // Unassigned code 0 (SID 0 in Standard Encoding) → None even
        // though the charset has .notdef at GID 0, because Standard
        // Encoding doesn't assign code 0 to any glyph name.
        assert_eq!(enc.lookup(0, &charset, &strings), None);
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
