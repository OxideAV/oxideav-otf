//! CFF Charset (Adobe TN5176 §13).
//!
//! Maps glyph id → SID (string id). GID 0 is always `.notdef` and
//! is *not* stored in the charset — every format describes glyphs
//! `1..=numGlyphs-1` only.
//!
//! Three on-disk formats:
//!
//! - **Format 0** (`.0[]`): array of `Card16` SIDs, one per glyph
//!   from gid=1 up. Length = `(num_glyphs - 1) * 2` bytes (+1 for
//!   the format tag).
//! - **Format 1** (`.1[]`): run-length encoded as
//!   `(SID first, Card8 nLeft)*` — SID `first` is for gid=1, then
//!   `first+1..=first+nLeft` cover gid=2..=2+nLeft, then the next
//!   range starts at gid=2+nLeft+1, and so on until every glyph is
//!   covered.
//! - **Format 2** (`.2[]`): same as format 1 but `nLeft` is `Card16`
//!   for fonts with very long contiguous SID runs (CJK).
//!
//! Three predefined charsets are signalled by Top DICT operator 15
//! holding the special offset values 0 (ISOAdobe), 1 (Expert), 2
//! (ExpertSubset). We accept these but resolve every glyph to SID
//! `gid` for ISOAdobe (which is the identity for the first 229 SIDs)
//! and emit `Error::Cff` for the Expert variants — they're for
//! Adobe's specialized "expert" character sets and we'd need the
//! full predefined-charset tables in the spec appendix to handle
//! them, which is out of round-1 scope.

use crate::parser::{read_u16, read_u8};
use crate::Error;

#[derive(Debug, Clone)]
pub(crate) enum Charset<'a> {
    /// Predefined ISOAdobe — gid `i` maps to SID `i` for `i <= 228`,
    /// gid 0 is `.notdef`. Anything past 228 with this charset is
    /// invalid (real fonts switch to a custom format then).
    IsoAdobe,
    /// Custom format 0 — array of u16 SIDs starting at gid 1.
    Format0 { bytes: &'a [u8], num_glyphs: u32 },
    /// Custom format 1 — variable-length runs of u16 starting SID +
    /// u8 count.
    Format1 { bytes: &'a [u8], num_glyphs: u32 },
    /// Custom format 2 — variable-length runs of u16 starting SID +
    /// u16 count.
    Format2 { bytes: &'a [u8], num_glyphs: u32 },
}

impl<'a> Charset<'a> {
    /// Parse a charset.
    ///
    /// `top_off` is the integer operand for Top DICT operator 15:
    /// 0 = ISOAdobe (predefined), 1/2 = Expert variants (predefined),
    /// >= 3 = custom offset into `bytes`. `num_glyphs` comes from the
    /// CharStrings INDEX count.
    pub(crate) fn parse(bytes: &'a [u8], top_off: i32, num_glyphs: u32) -> Result<Self, Error> {
        match top_off {
            0 => Ok(Self::IsoAdobe),
            1 | 2 => Err(Error::Cff(
                "predefined Expert charset not implemented in round 1",
            )),
            n if n < 0 => Err(Error::Cff("negative charset offset")),
            n => {
                let off = n as usize;
                if off >= bytes.len() {
                    return Err(Error::UnexpectedEof);
                }
                let format = read_u8(bytes, off)?;
                let payload = &bytes[off + 1..];
                match format {
                    0 => Ok(Self::Format0 {
                        bytes: payload,
                        num_glyphs,
                    }),
                    1 => Ok(Self::Format1 {
                        bytes: payload,
                        num_glyphs,
                    }),
                    2 => Ok(Self::Format2 {
                        bytes: payload,
                        num_glyphs,
                    }),
                    _ => Err(Error::Cff("unknown charset format")),
                }
            }
        }
    }

    /// Resolve gid → SID. Returns `None` for out-of-range gid.
    pub(crate) fn sid_of(&self, gid: u16) -> Option<u16> {
        if gid == 0 {
            return Some(0); // .notdef
        }
        match self {
            Self::IsoAdobe => {
                if (gid as usize) < 229 {
                    Some(gid)
                } else {
                    None
                }
            }
            Self::Format0 { bytes, num_glyphs } => {
                if (gid as u32) >= *num_glyphs {
                    return None;
                }
                let off = (gid as usize - 1) * 2;
                read_u16(bytes, off).ok()
            }
            Self::Format1 { bytes, num_glyphs } => walk_runs(bytes, *num_glyphs, gid, 1),
            Self::Format2 { bytes, num_glyphs } => walk_runs(bytes, *num_glyphs, gid, 2),
        }
    }
}

/// Shared run-walker for format 1 (`n_left_size = 1`) and format 2
/// (`n_left_size = 2`).
///
/// Each run is `[u16 first_sid, uN n_left]`. The run starts at the
/// "next gid" pointer (initially 1) and covers `n_left + 1` glyphs.
fn walk_runs(bytes: &[u8], num_glyphs: u32, target_gid: u16, n_left_size: usize) -> Option<u16> {
    let mut gid: u32 = 1;
    let mut off: usize = 0;
    while gid < num_glyphs {
        let first = read_u16(bytes, off).ok()?;
        off += 2;
        let n_left: u32 = match n_left_size {
            1 => read_u8(bytes, off).ok()? as u32,
            2 => read_u16(bytes, off).ok()? as u32,
            _ => unreachable!(),
        };
        off += n_left_size;
        let run_end = gid + n_left;
        if (target_gid as u32) >= gid && (target_gid as u32) <= run_end {
            let in_run = (target_gid as u32) - gid;
            let sid = first as u32 + in_run;
            if sid > u16::MAX as u32 {
                return None;
            }
            return Some(sid as u16);
        }
        gid = run_end + 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iso_adobe_identity() {
        let cs = Charset::IsoAdobe;
        assert_eq!(cs.sid_of(0), Some(0));
        assert_eq!(cs.sid_of(1), Some(1));
        assert_eq!(cs.sid_of(228), Some(228));
        assert_eq!(cs.sid_of(229), None);
    }

    #[test]
    fn format0_walk() {
        // num_glyphs = 4 → 3 SIDs after .notdef.
        // SIDs for gid 1..3: 100, 200, 300.
        let payload = vec![0x00, 100, 0x00, 200, 0x01, 0x2C];
        let cs = Charset::Format0 {
            bytes: &payload,
            num_glyphs: 4,
        };
        assert_eq!(cs.sid_of(1), Some(100));
        assert_eq!(cs.sid_of(2), Some(200));
        assert_eq!(cs.sid_of(3), Some(300));
        assert_eq!(cs.sid_of(4), None);
    }

    #[test]
    fn format1_walk() {
        // num_glyphs = 6.
        // Run 1: first SID 50, nLeft 2 → covers gid 1..=3 (sids 50,51,52).
        // Run 2: first SID 70, nLeft 1 → covers gid 4..=5 (sids 70,71).
        let payload = vec![0x00, 50, 0x02, 0x00, 70, 0x01];
        let cs = Charset::Format1 {
            bytes: &payload,
            num_glyphs: 6,
        };
        assert_eq!(cs.sid_of(1), Some(50));
        assert_eq!(cs.sid_of(2), Some(51));
        assert_eq!(cs.sid_of(3), Some(52));
        assert_eq!(cs.sid_of(4), Some(70));
        assert_eq!(cs.sid_of(5), Some(71));
    }

    #[test]
    fn parse_via_offset_dispatch_format0() {
        // Plant a format-0 charset at offset 4 in a synthetic table.
        let mut table = vec![0u8, 0, 0, 0]; // padding
        table.push(0); // format = 0
        table.extend_from_slice(&[0, 100, 0, 200]); // 2 sids
        let cs = Charset::parse(&table, 4, 3).expect("parse");
        assert!(matches!(cs, Charset::Format0 { .. }));
        assert_eq!(cs.sid_of(1), Some(100));
        assert_eq!(cs.sid_of(2), Some(200));
    }
}
