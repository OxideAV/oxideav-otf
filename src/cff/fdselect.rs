//! CFF FDSelect (Adobe TN5176 §19).
//!
//! In a CID-keyed CFF font, glyphs are partitioned into groups that
//! each share a Font DICT (FD) — and therefore a Private DICT, its
//! Local Subrs, and its `defaultWidthX` / `nominalWidthX`. The
//! FDSelect structure is the GID → FD-index map that says which Font
//! DICT applies to each glyph.
//!
//! Two on-disk formats (TN5176 Tables 27, 28):
//!
//! - **Format 0** (`Card8 format = 0`): a flat `Card8 fds[nGlyphs]`
//!   array — `fds[gid]` is the FD index for `gid`. (Identical to
//!   charset format 0 except that `.notdef` / GID 0 *is* included
//!   here; charset format 0 omits it.) Used when FD indexes are in a
//!   fairly random order.
//! - **Format 3** (`Card8 format = 3`): run-length encoded. A
//!   `Card16 nRanges` count, then `nRanges` `Range3` records of
//!   `(Card16 first, Card8 fd)`, then a `Card16 sentinel`. Each
//!   Range3 covers GIDs `[first, next.first)`; the first range's
//!   `first` must be 0 and the sentinel equals the glyph count.
//!   Suited to well-ordered FD indexes (the usual case).

use crate::parser::{read_u16, read_u8};
use crate::Error;

/// A parsed FDSelect, retaining the raw payload for O(log n) / O(1)
/// per-glyph lookups (no per-glyph allocation up front).
#[derive(Debug, Clone)]
pub(crate) enum FdSelect<'a> {
    /// Format 0: flat per-glyph FD-index array. `bytes` is the
    /// `fds[nGlyphs]` slice (the format byte already consumed).
    Format0 { fds: &'a [u8], num_glyphs: u32 },
    /// Format 3: range-encoded. `ranges` is the `Range3[nRanges]`
    /// region (3 bytes each: `Card16 first` + `Card8 fd`); `sentinel`
    /// is the trailing GID delimiter (== glyph count).
    Format3 {
        ranges: &'a [u8],
        n_ranges: u16,
        sentinel: u16,
    },
}

impl<'a> FdSelect<'a> {
    /// Parse the FDSelect located at `off` within the CFF table bytes.
    /// `num_glyphs` is the CharStrings INDEX count and is needed both
    /// to bound the format-0 array and to sanity-check the format-3
    /// sentinel.
    pub(crate) fn parse(bytes: &'a [u8], off: usize, num_glyphs: u32) -> Result<Self, Error> {
        let format = read_u8(bytes, off)?;
        match format {
            0 => {
                let start = off + 1;
                let end = start
                    .checked_add(num_glyphs as usize)
                    .ok_or(Error::Cff("FDSelect format 0 overflow"))?;
                let fds = bytes.get(start..end).ok_or(Error::UnexpectedEof)?;
                Ok(Self::Format0 { fds, num_glyphs })
            }
            3 => {
                let n_ranges = read_u16(bytes, off + 1)?;
                let ranges_at = off + 3;
                let ranges_len = (n_ranges as usize) * 3;
                let ranges = bytes
                    .get(ranges_at..ranges_at + ranges_len)
                    .ok_or(Error::UnexpectedEof)?;
                // Sentinel Card16 follows the range records.
                let sentinel = read_u16(bytes, ranges_at + ranges_len)?;
                Ok(Self::Format3 {
                    ranges,
                    n_ranges,
                    sentinel,
                })
            }
            _ => Err(Error::Cff("unknown FDSelect format")),
        }
    }

    /// Resolve `gid` to its FD (Font DICT) index. Returns `None` for a
    /// GID that lies outside the structure's coverage (out of range
    /// for format 0, or at/after the sentinel for format 3).
    pub(crate) fn fd_index(&self, gid: u16) -> Option<u8> {
        match self {
            Self::Format0 { fds, num_glyphs } => {
                if (gid as u32) >= *num_glyphs {
                    return None;
                }
                fds.get(gid as usize).copied()
            }
            Self::Format3 {
                ranges,
                n_ranges,
                sentinel,
            } => {
                if gid >= *sentinel {
                    return None;
                }
                // Ranges are sorted by ascending `first`; each covers
                // [first, next_first). A linear scan is fine — fonts
                // rarely have more than a handful of FDs — but we read
                // `next.first` (or the sentinel for the last range) to
                // bound each range's upper edge.
                let n = *n_ranges as usize;
                for i in 0..n {
                    let rec = i * 3;
                    let first = u16::from_be_bytes([ranges[rec], ranges[rec + 1]]);
                    let fd = ranges[rec + 2];
                    let next_first = if i + 1 < n {
                        let nrec = (i + 1) * 3;
                        u16::from_be_bytes([ranges[nrec], ranges[nrec + 1]])
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
        // format byte 0, then fds[5] = [0, 1, 1, 2, 0].
        let buf = vec![0u8, 0, 1, 1, 2, 0];
        let sel = FdSelect::parse(&buf, 0, 5).expect("parse");
        assert!(matches!(sel, FdSelect::Format0 { .. }));
        assert_eq!(sel.fd_index(0), Some(0));
        assert_eq!(sel.fd_index(1), Some(1));
        assert_eq!(sel.fd_index(2), Some(1));
        assert_eq!(sel.fd_index(3), Some(2));
        assert_eq!(sel.fd_index(4), Some(0));
        assert_eq!(sel.fd_index(5), None); // past nGlyphs
    }

    #[test]
    fn format3_ranges() {
        // format=3, nRanges=2,
        //   Range3[0] = (first=0,  fd=0)  → covers GID 0..=2
        //   Range3[1] = (first=3,  fd=1)  → covers GID 3..=5
        // sentinel = 6 (glyph count).
        let buf = vec![
            3, // format
            0x00, 0x02, // nRanges = 2
            0x00, 0x00, 0x00, // Range3[0]: first=0, fd=0
            0x00, 0x03, 0x01, // Range3[1]: first=3, fd=1
            0x00, 0x06, // sentinel = 6
        ];
        let sel = FdSelect::parse(&buf, 0, 6).expect("parse");
        assert!(matches!(sel, FdSelect::Format3 { .. }));
        for gid in 0u16..=2 {
            assert_eq!(sel.fd_index(gid), Some(0), "gid {gid}");
        }
        for gid in 3u16..=5 {
            assert_eq!(sel.fd_index(gid), Some(1), "gid {gid}");
        }
        assert_eq!(sel.fd_index(6), None); // at sentinel
        assert_eq!(sel.fd_index(99), None);
    }

    #[test]
    fn format3_single_range() {
        // A complete CIDFont commonly has a single range covering all
        // glyphs with FD 0.
        let buf = vec![
            3, // format
            0x00, 0x01, // nRanges = 1
            0x00, 0x00, 0x00, // first=0, fd=0
            0x00, 0x04, // sentinel = 4
        ];
        let sel = FdSelect::parse(&buf, 0, 4).expect("parse");
        for gid in 0u16..=3 {
            assert_eq!(sel.fd_index(gid), Some(0));
        }
        assert_eq!(sel.fd_index(4), None);
    }

    #[test]
    fn rejects_unknown_format() {
        let buf = vec![7u8, 0, 0];
        assert!(FdSelect::parse(&buf, 0, 1).is_err());
    }

    #[test]
    fn rejects_truncated_format0() {
        // Claims 10 glyphs but only 3 fds bytes present.
        let buf = vec![0u8, 0, 1, 2];
        assert!(FdSelect::parse(&buf, 0, 10).is_err());
    }
}
