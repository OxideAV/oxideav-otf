//! CFF2 FontDICTSelect (OpenType 1.9.1 `CFF2` table, FontDICTSelect).
//!
//! When a CFF2 font has more than one FontDICT, a FontDICTSelect table
//! routes each glyph (CharString) to the FontDICT — and therefore the
//! PrivateDICT, LocalSubrINDEX, and default `vsindex` — that applies to
//! it. The location is the `FontDICTSelectOffset` Top DICT operator
//! (`0x0c25`). When the font has a single FontDICT, no FontDICTSelect
//! is present and every glyph maps to FontDICT 0.
//!
//! Three on-disk formats are defined:
//!
//! - **Format 0** (`uint8 format = 0`): a flat `uint8 fds[numGlyphs]`
//!   array — `fds[gid]` is the FontDICT index for `gid`.
//! - **Format 3** (`uint8 format = 3`): run-length encoded with 16-bit
//!   glyph IDs. `uint16 numRanges`, then `numRanges` Range3 records of
//!   `(uint16 first, uint8 fontDICTID)`, then a `uint16 sentinel`
//!   (== `numGlyphs`). The first range's `first` is 0; each range
//!   covers `[first, next.first)`.
//! - **Format 4** (`uint8 format = 4`): like format 3 but with 32-bit
//!   `numRanges`/`sentinel` and a Range4 record of `(uint32 first,
//!   uint16 fontDICTID)`, allowing > 65,534 glyphs and up to 65,535
//!   FontDICTs.
//!
//! This mirrors the CFF1 `cff::fdselect` formats 0/3 and adds the
//! CFF2-only format 4; FD indices are widened to `u16` to carry the
//! format-4 range.
//!
//! Spec: `docs/text/opentype/otspec-cff2.html`
//! (FontDICTINDEX, FontDICTSelect and FontDICT).

use crate::parser::{read_u16, read_u32, read_u8};
use crate::Error;

/// A parsed CFF2 FontDICTSelect, retaining the raw payload for
/// per-glyph lookups without an up-front per-glyph allocation.
#[derive(Debug, Clone)]
pub enum Cff2FdSelect<'a> {
    /// Format 0: flat per-glyph FontDICT-index array (`fds[numGlyphs]`,
    /// format byte already consumed).
    Format0 { fds: &'a [u8], num_glyphs: u32 },
    /// Format 3: 16-bit range-encoded. `ranges` is the
    /// `Range3[numRanges]` region (3 bytes each: `uint16 first` +
    /// `uint8 fontDICTID`); `sentinel` is the trailing GID delimiter.
    Format3 {
        ranges: &'a [u8],
        n_ranges: u32,
        sentinel: u32,
    },
    /// Format 4: 32-bit range-encoded. `ranges` is the
    /// `Range4[numRanges]` region (6 bytes each: `uint32 first` +
    /// `uint16 fontDICTID`); `sentinel` is the 32-bit GID delimiter.
    Format4 {
        ranges: &'a [u8],
        n_ranges: u32,
        sentinel: u32,
    },
}

impl<'a> Cff2FdSelect<'a> {
    /// Parse the FontDICTSelect at `off` within the CFF2 table bytes.
    /// `num_glyphs` (the CharStringINDEX count) bounds the format-0
    /// array and sanity-checks range sentinels.
    pub fn parse(bytes: &'a [u8], off: usize, num_glyphs: u32) -> Result<Self, Error> {
        let format = read_u8(bytes, off)?;
        match format {
            0 => {
                let start = off + 1;
                let end = start
                    .checked_add(num_glyphs as usize)
                    .ok_or(Error::Cff("CFF2 FontDICTSelect format 0 overflow"))?;
                let fds = bytes.get(start..end).ok_or(Error::UnexpectedEof)?;
                Ok(Self::Format0 { fds, num_glyphs })
            }
            3 => {
                let n_ranges = read_u16(bytes, off + 1)? as u32;
                let ranges_at = off + 3;
                let ranges_len = (n_ranges as usize)
                    .checked_mul(3)
                    .ok_or(Error::Cff("CFF2 FontDICTSelect format 3 overflow"))?;
                let ranges = bytes
                    .get(ranges_at..ranges_at + ranges_len)
                    .ok_or(Error::UnexpectedEof)?;
                let sentinel = read_u16(bytes, ranges_at + ranges_len)? as u32;
                Ok(Self::Format3 {
                    ranges,
                    n_ranges,
                    sentinel,
                })
            }
            4 => {
                let n_ranges = read_u32(bytes, off + 1)?;
                let ranges_at = off + 5;
                let ranges_len = (n_ranges as usize)
                    .checked_mul(6)
                    .ok_or(Error::Cff("CFF2 FontDICTSelect format 4 overflow"))?;
                let ranges = bytes
                    .get(ranges_at..ranges_at + ranges_len)
                    .ok_or(Error::UnexpectedEof)?;
                let sentinel = read_u32(bytes, ranges_at + ranges_len)?;
                Ok(Self::Format4 {
                    ranges,
                    n_ranges,
                    sentinel,
                })
            }
            _ => Err(Error::Cff("unknown CFF2 FontDICTSelect format")),
        }
    }

    /// Resolve `gid` to its FontDICT index. Returns `None` for a GID
    /// outside the structure's coverage (past `numGlyphs` for format 0,
    /// at/after the sentinel for formats 3/4).
    pub fn fd_index(&self, gid: u32) -> Option<u16> {
        match self {
            Self::Format0 { fds, num_glyphs } => {
                if gid >= *num_glyphs {
                    return None;
                }
                fds.get(gid as usize).map(|&v| v as u16)
            }
            Self::Format3 {
                ranges,
                n_ranges,
                sentinel,
            } => {
                if gid >= *sentinel {
                    return None;
                }
                let n = *n_ranges as usize;
                for i in 0..n {
                    let rec = i * 3;
                    let first = u16::from_be_bytes([ranges[rec], ranges[rec + 1]]) as u32;
                    let fd = ranges[rec + 2] as u16;
                    let next_first = if i + 1 < n {
                        let nrec = (i + 1) * 3;
                        u16::from_be_bytes([ranges[nrec], ranges[nrec + 1]]) as u32
                    } else {
                        *sentinel
                    };
                    if gid >= first && gid < next_first {
                        return Some(fd);
                    }
                }
                None
            }
            Self::Format4 {
                ranges,
                n_ranges,
                sentinel,
            } => {
                if gid >= *sentinel {
                    return None;
                }
                let n = *n_ranges as usize;
                for i in 0..n {
                    let rec = i * 6;
                    let first = u32::from_be_bytes([
                        ranges[rec],
                        ranges[rec + 1],
                        ranges[rec + 2],
                        ranges[rec + 3],
                    ]);
                    let fd = u16::from_be_bytes([ranges[rec + 4], ranges[rec + 5]]);
                    let next_first = if i + 1 < n {
                        let nrec = (i + 1) * 6;
                        u32::from_be_bytes([
                            ranges[nrec],
                            ranges[nrec + 1],
                            ranges[nrec + 2],
                            ranges[nrec + 3],
                        ])
                    } else {
                        *sentinel
                    };
                    if gid >= first && gid < next_first {
                        return Some(fd);
                    }
                }
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format0_flat_array() {
        let buf = vec![0u8, 0, 1, 1, 2, 0];
        let sel = Cff2FdSelect::parse(&buf, 0, 5).expect("parse");
        assert_eq!(sel.fd_index(0), Some(0));
        assert_eq!(sel.fd_index(3), Some(2));
        assert_eq!(sel.fd_index(4), Some(0));
        assert_eq!(sel.fd_index(5), None);
    }

    #[test]
    fn format3_ranges() {
        let buf = vec![
            3, // format
            0x00, 0x02, // nRanges = 2
            0x00, 0x00, 0x00, // first=0, fd=0
            0x00, 0x03, 0x01, // first=3, fd=1
            0x00, 0x06, // sentinel = 6
        ];
        let sel = Cff2FdSelect::parse(&buf, 0, 6).expect("parse");
        for gid in 0u32..=2 {
            assert_eq!(sel.fd_index(gid), Some(0));
        }
        for gid in 3u32..=5 {
            assert_eq!(sel.fd_index(gid), Some(1));
        }
        assert_eq!(sel.fd_index(6), None);
    }

    #[test]
    fn format4_ranges_wide() {
        // numRanges=2 (uint32), Range4 (uint32 first, uint16 fd):
        //   (0, fd=7) covers 0..=99_999
        //   (100_000, fd=300) covers 100_000..=199_999
        // sentinel (uint32) = 200_000.
        let mut buf = vec![4u8]; // format
        buf.extend_from_slice(&2u32.to_be_bytes()); // numRanges
        buf.extend_from_slice(&0u32.to_be_bytes()); // first=0
        buf.extend_from_slice(&7u16.to_be_bytes()); // fd=7
        buf.extend_from_slice(&100_000u32.to_be_bytes()); // first=100000
        buf.extend_from_slice(&300u16.to_be_bytes()); // fd=300
        buf.extend_from_slice(&200_000u32.to_be_bytes()); // sentinel
        let sel = Cff2FdSelect::parse(&buf, 0, 200_000).expect("parse");
        assert_eq!(sel.fd_index(0), Some(7));
        assert_eq!(sel.fd_index(99_999), Some(7));
        assert_eq!(sel.fd_index(100_000), Some(300));
        assert_eq!(sel.fd_index(199_999), Some(300));
        assert_eq!(sel.fd_index(200_000), None);
    }

    #[test]
    fn rejects_unknown_format() {
        let buf = vec![7u8, 0, 0];
        assert!(Cff2FdSelect::parse(&buf, 0, 1).is_err());
    }

    #[test]
    fn rejects_truncated_format4() {
        // numRanges=5 but no records.
        let mut buf = vec![4u8];
        buf.extend_from_slice(&5u32.to_be_bytes());
        assert!(Cff2FdSelect::parse(&buf, 0, 10).is_err());
    }
}
