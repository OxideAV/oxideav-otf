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
//! (ExpertSubset). ISOAdobe resolves every glyph to SID `gid` (the
//! identity for the first 229 SIDs). Expert and ExpertSubset are
//! fixed `GID → SID` lists transcribed from TN5176 Appendix C
//! ([`EXPERT_SIDS`] / [`EXPERT_SUBSET_SIDS`]); a font selecting one
//! of them resolves glyph names through the same standard-strings
//! table the rest of the charset code uses.

use crate::parser::{read_u16, read_u8};
use crate::Error;

/// Expert predefined charset — TN5176 Appendix C, in GID order
/// beginning with GID 1 (`.notdef` is the implicit GID 0 and is not
/// listed). `EXPERT_SIDS[gid - 1]` is the SID for glyph `gid`. The
/// appendix lays the entries out column-major across three columns
/// per page block; this array linearises them back into GID order.
/// 166 glyphs total (GID 0..=165).
pub(crate) const EXPERT_SIDS: [u16; 165] = [
    // GID 1..=15 (appendix block 1)
    1, 229, 230, 231, 232, 233, 234, 235, 236, 237, 238, 13, 14, 15, 99,
    // GID 16..=111 (appendix block 2)
    239, 240, 241, 242, 243, 244, 245, 246, 247, 248, 27, 28, 249, 250, 251, 252, 253, 254, 255,
    256, 257, 258, 259, 260, 261, 262, 263, 264, 265, 266, 109, 110, 267, 268, 269, 270, 271, 272,
    273, 274, 275, 276, 277, 278, 279, 280, 281, 282, 283, 284, 285, 286, 287, 288, 289, 290, 291,
    292, 293, 294, 295, 296, 297, 298, 299, 300, 301, 302, 303, 304, 305, 306, 307, 308, 309, 310,
    311, 312, 313, 314, 315, 316, 317, 318, 158, 155, 163, 319, 320, 321, 322, 323, 324, 325, 326,
    150, // GID 112..=165 (appendix block 3)
    164, 169, 327, 328, 329, 330, 331, 332, 333, 334, 335, 336, 337, 338, 339, 340, 341, 342, 343,
    344, 345, 346, 347, 348, 349, 350, 351, 352, 353, 354, 355, 356, 357, 358, 359, 360, 361, 362,
    363, 364, 365, 366, 367, 368, 369, 370, 371, 372, 373, 374, 375, 376, 377, 378,
];

/// Expert Subset predefined charset — TN5176 Appendix C, GID order
/// from GID 1 (`.notdef` is the implicit GID 0). `EXPERT_SUBSET_SIDS
/// [gid - 1]` is the SID for glyph `gid`. 87 glyphs total
/// (GID 0..=86).
pub(crate) const EXPERT_SUBSET_SIDS: [u16; 86] = [
    // GID 1..=33 (appendix block 1)
    1, 231, 232, 235, 236, 237, 238, 13, 14, 15, 99, 239, 240, 241, 242, 243, 244, 245, 246, 247,
    248, 27, 28, 249, 250, 251, 253, 254, 255, 256, 257, 258, 259,
    // GID 34..=86 (appendix block 2)
    260, 261, 262, 263, 264, 265, 266, 109, 110, 267, 268, 269, 270, 272, 300, 301, 302, 305, 314,
    315, 158, 155, 163, 320, 321, 322, 323, 324, 325, 326, 150, 164, 169, 327, 328, 329, 330, 331,
    332, 333, 334, 335, 336, 337, 338, 339, 340, 341, 342, 343, 344, 345, 346,
];

#[derive(Debug, Clone)]
pub(crate) enum Charset<'a> {
    /// Predefined ISOAdobe — gid `i` maps to SID `i` for `i <= 228`,
    /// gid 0 is `.notdef`. Anything past 228 with this charset is
    /// invalid (real fonts switch to a custom format then).
    IsoAdobe,
    /// Predefined Expert charset — fixed GID → SID list from TN5176
    /// Appendix C, exposed as [`EXPERT_SIDS`].
    Expert,
    /// Predefined Expert Subset charset — fixed GID → SID list from
    /// TN5176 Appendix C, exposed as [`EXPERT_SUBSET_SIDS`].
    ExpertSubset,
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
    /// \>= 3 = custom offset into `bytes`. `num_glyphs` comes from the
    /// CharStrings INDEX count.
    pub(crate) fn parse(bytes: &'a [u8], top_off: i32, num_glyphs: u32) -> Result<Self, Error> {
        match top_off {
            0 => Ok(Self::IsoAdobe),
            1 => Ok(Self::Expert),
            2 => Ok(Self::ExpertSubset),
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

    /// Reverse-map a SID to its glyph id, scanning the charset's
    /// records. Returns `None` if no glyph in this charset has the
    /// requested SID.
    ///
    /// Used by the Type 2 charstring interpreter's `seac` handler
    /// (TN5177 Appendix C, the deprecated four-operand `endchar`
    /// form): once `bchar` / `achar` are resolved through the
    /// Standard Encoding table to SIDs, the actual component glyph
    /// IDs come from the per-font charset.
    pub(crate) fn gid_of_sid(&self, sid: u16) -> Option<u16> {
        if sid == 0 {
            return Some(0); // .notdef is always GID 0
        }
        match self {
            Self::IsoAdobe => {
                // ISOAdobe is the identity for SIDs 1..=228.
                if (sid as usize) < 229 {
                    Some(sid)
                } else {
                    None
                }
            }
            Self::Expert => predefined_gid_of_sid(&EXPERT_SIDS, sid),
            Self::ExpertSubset => predefined_gid_of_sid(&EXPERT_SUBSET_SIDS, sid),
            Self::Format0 { bytes, num_glyphs } => {
                // Linear scan of the format-0 SID array.
                let n = (*num_glyphs).saturating_sub(1) as usize;
                for i in 0..n {
                    let off = i * 2;
                    if let Ok(entry) = read_u16(bytes, off) {
                        if entry == sid {
                            return Some((i + 1) as u16);
                        }
                    } else {
                        return None;
                    }
                }
                None
            }
            Self::Format1 { bytes, num_glyphs } => walk_runs_for_sid(bytes, *num_glyphs, sid, 1),
            Self::Format2 { bytes, num_glyphs } => walk_runs_for_sid(bytes, *num_glyphs, sid, 2),
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
            Self::Expert => predefined_sid_of(&EXPERT_SIDS, gid),
            Self::ExpertSubset => predefined_sid_of(&EXPERT_SUBSET_SIDS, gid),
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

/// `gid → SID` for a predefined charset table (Expert / ExpertSubset).
/// GID 0 is `.notdef` (SID 0); GID `i >= 1` is `table[i - 1]`. Any
/// gid past the table's last glyph returns `None`.
fn predefined_sid_of(table: &[u16], gid: u16) -> Option<u16> {
    if gid == 0 {
        return Some(0); // .notdef
    }
    table.get(gid as usize - 1).copied()
}

/// `SID → gid` for a predefined charset table (the inverse of
/// [`predefined_sid_of`]). SID 0 is `.notdef` at GID 0; otherwise the
/// first table slot holding `target_sid` wins (the predefined tables
/// contain no duplicate SIDs, so the first hit is the only hit).
fn predefined_gid_of_sid(table: &[u16], target_sid: u16) -> Option<u16> {
    if target_sid == 0 {
        return Some(0); // .notdef
    }
    table
        .iter()
        .position(|&s| s == target_sid)
        .map(|i| (i + 1) as u16)
}

/// Reverse-direction sibling of [`walk_runs`]: scans the run table to
/// find the gid that holds `target_sid`.
fn walk_runs_for_sid(
    bytes: &[u8],
    num_glyphs: u32,
    target_sid: u16,
    n_left_size: usize,
) -> Option<u16> {
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
        let run_last_sid = (first as u32) + n_left;
        if (target_sid as u32) >= (first as u32) && (target_sid as u32) <= run_last_sid {
            let in_run = (target_sid as u32) - (first as u32);
            let resolved = gid + in_run;
            if resolved > u16::MAX as u32 {
                return None;
            }
            return Some(resolved as u16);
        }
        gid = gid + n_left + 1;
    }
    None
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
    fn gid_of_sid_iso_adobe() {
        let cs = Charset::IsoAdobe;
        assert_eq!(cs.gid_of_sid(0), Some(0));
        assert_eq!(cs.gid_of_sid(1), Some(1));
        assert_eq!(cs.gid_of_sid(228), Some(228));
        assert_eq!(cs.gid_of_sid(229), None);
    }

    #[test]
    fn gid_of_sid_format0() {
        // num_glyphs = 4 → 3 SIDs after .notdef.
        let payload = vec![0x00, 100, 0x00, 200, 0x01, 0x2C]; // 100, 200, 300
        let cs = Charset::Format0 {
            bytes: &payload,
            num_glyphs: 4,
        };
        assert_eq!(cs.gid_of_sid(100), Some(1));
        assert_eq!(cs.gid_of_sid(200), Some(2));
        assert_eq!(cs.gid_of_sid(300), Some(3));
        assert_eq!(cs.gid_of_sid(999), None);
        // .notdef forwarding.
        assert_eq!(cs.gid_of_sid(0), Some(0));
    }

    #[test]
    fn gid_of_sid_format1_within_run() {
        // Run 1: first SID 50, nLeft 2 → covers gid 1..=3 (sids 50, 51, 52).
        // Run 2: first SID 70, nLeft 1 → covers gid 4..=5 (sids 70, 71).
        let payload = vec![0x00, 50, 0x02, 0x00, 70, 0x01];
        let cs = Charset::Format1 {
            bytes: &payload,
            num_glyphs: 6,
        };
        assert_eq!(cs.gid_of_sid(50), Some(1));
        assert_eq!(cs.gid_of_sid(51), Some(2));
        assert_eq!(cs.gid_of_sid(52), Some(3));
        assert_eq!(cs.gid_of_sid(70), Some(4));
        assert_eq!(cs.gid_of_sid(71), Some(5));
        // SID in a hole between runs → None.
        assert_eq!(cs.gid_of_sid(60), None);
    }

    #[test]
    fn expert_table_lengths() {
        // TN5176 Appendix C: Expert has 166 glyphs (GID 0..=165),
        // Expert Subset has 87 (GID 0..=86). The arrays omit the
        // implicit GID-0 .notdef, hence one fewer entry each.
        assert_eq!(EXPERT_SIDS.len(), 165);
        assert_eq!(EXPERT_SUBSET_SIDS.len(), 86);
    }

    #[test]
    fn expert_landmark_sids() {
        let cs = Charset::Expert;
        // GID 0 = .notdef.
        assert_eq!(cs.sid_of(0), Some(0));
        // GID 1 = space (SID 1) — both Expert and ISOAdobe start here.
        assert_eq!(cs.sid_of(1), Some(1));
        // GID 2 = exclamsmall (SID 229) — the first "expert" glyph.
        assert_eq!(cs.sid_of(2), Some(229));
        // GID 12 = comma (SID 13) — a standard SID reused mid-table.
        assert_eq!(cs.sid_of(12), Some(13));
        // GID 15 = fraction (SID 99), last of appendix block 1.
        assert_eq!(cs.sid_of(15), Some(99));
        // GID 16 = zerooldstyle (SID 239), first of block 2.
        assert_eq!(cs.sid_of(16), Some(239));
        // GID 165 = Ydieresissmall (SID 378), the final Expert glyph.
        assert_eq!(cs.sid_of(165), Some(378));
        // One past the end → None.
        assert_eq!(cs.sid_of(166), None);
    }

    #[test]
    fn expert_subset_landmark_sids() {
        let cs = Charset::ExpertSubset;
        assert_eq!(cs.sid_of(0), Some(0));
        assert_eq!(cs.sid_of(1), Some(1)); // space
        assert_eq!(cs.sid_of(2), Some(231)); // dollaroldstyle
        assert_eq!(cs.sid_of(11), Some(99)); // fraction, last of block 1
        assert_eq!(cs.sid_of(12), Some(239)); // zerooldstyle
        assert_eq!(cs.sid_of(86), Some(346)); // commainferior, the last glyph
        assert_eq!(cs.sid_of(87), None);
    }

    #[test]
    fn expert_round_trip_sid_gid() {
        let cs = Charset::Expert;
        for gid in 0u16..=165 {
            let sid = cs.sid_of(gid).expect("every Expert gid has a SID");
            assert_eq!(
                cs.gid_of_sid(sid),
                Some(gid),
                "Expert gid {gid} (sid {sid}) failed to round-trip"
            );
        }
    }

    #[test]
    fn expert_subset_round_trip_sid_gid() {
        let cs = Charset::ExpertSubset;
        for gid in 0u16..=86 {
            let sid = cs.sid_of(gid).expect("every ExpertSubset gid has a SID");
            assert_eq!(
                cs.gid_of_sid(sid),
                Some(gid),
                "ExpertSubset gid {gid} (sid {sid}) failed to round-trip"
            );
        }
    }

    #[test]
    fn expert_sids_resolve_to_standard_strings() {
        // Every Expert / ExpertSubset SID must be <= 390 (a predefined
        // standard string), so a font using these charsets resolves
        // glyph names without a per-font String INDEX. This is what
        // makes the predefined charsets usable through `glyph_name`.
        use crate::cff::strings::STANDARD_STRINGS;
        for &sid in EXPERT_SIDS.iter().chain(EXPERT_SUBSET_SIDS.iter()) {
            assert!(
                (sid as usize) < STANDARD_STRINGS.len(),
                "predefined-charset SID {sid} exceeds the standard-strings table"
            );
        }
        // Spot-check a couple of names via the standard-strings table.
        assert_eq!(STANDARD_STRINGS[229], "exclamsmall");
        assert_eq!(STANDARD_STRINGS[378], "Ydieresissmall");
        assert_eq!(STANDARD_STRINGS[346], "commainferior");
    }

    #[test]
    fn parse_dispatches_predefined_expert() {
        // top_off 1 → Expert, 2 → ExpertSubset (num_glyphs unused for
        // predefined charsets).
        let cs = Charset::parse(&[], 1, 0).expect("expert parse");
        assert!(matches!(cs, Charset::Expert));
        let cs = Charset::parse(&[], 2, 0).expect("expert-subset parse");
        assert!(matches!(cs, Charset::ExpertSubset));
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
