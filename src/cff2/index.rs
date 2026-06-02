//! CFF2 INDEX structure (OpenType 1.9.1 `CFF2` table, §6 "INDEX data").
//!
//! CFF2 INDEX is the same shape as the CFF1 INDEX from Adobe TN5176 §5
//! except the `count` field is widened from `Card16` (CFF1) to
//! `uint32` (CFF2). An empty CFF2 INDEX is consequently `4` bytes
//! (zero `count` and no following fields), versus CFF1's `2` bytes.
//!
//! Format:
//! ```text
//! uint32 count                              // number of objects
//! uint8  offsetSize                         // 1..4
//! Offset offsets[count + 1]                 // offsetSize-byte big-endian offsets
//!                                           //   relative to the byte preceding object data
//! uint8  data[offsets[count] - 1]           // contiguous object payloads
//! ```
//!
//! Like CFF1, offsets are 1-based: `offsets[0]` is always `1`, the
//! byte at position `offsets[i] - 1` relative to the data section is
//! the first byte of object `i`, and consecutive offsets give each
//! object's length. An object may have a zero size (e.g. a CharString
//! for a non-printing glyph: §8 "Non-printing glyphs").

use crate::parser::read_u8;
use crate::Error;

/// A parsed CFF2 INDEX. Entries are returned as zero-copy `&[u8]` slices
/// into the underlying CFF2 table bytes.
#[derive(Debug, Clone)]
pub struct Cff2Index<'a> {
    /// Whole CFF2 table (entries are slices into this).
    pub(crate) bytes: &'a [u8],
    /// Number of entries in the INDEX.
    pub(crate) count: u32,
    /// Offset of the first byte after this INDEX (where the next
    /// adjacent structure would start). Used when walking the
    /// Header → TopDICT → GlobalSubrINDEX prefix in order, or when
    /// a caller wants to confirm the on-disk size of the INDEX after
    /// parse.
    #[allow(dead_code)] // referenced by tests + retained for future sequential walkers.
    pub(crate) end: usize,
    /// Width of each offset array element in bytes (1..4).
    off_size: u8,
    /// Offset of the first byte of `offsets[0]` within `bytes`.
    offsets_at: usize,
    /// Offset of the byte the spec defines offsets as "relative to the
    /// byte preceding object data" — i.e. our `data_at = offset_array_end - 1`
    /// in spec terms. With offsets 1-based, the data byte for offset
    /// value `o` lives at `bytes[data_at + (o - 1)]`. We pre-compute
    /// `data_at = offset_array_end` so the +1 cancels the -1.
    data_at: usize,
}

impl<'a> Cff2Index<'a> {
    /// Parse a CFF2 INDEX starting at byte offset `at` within `bytes`.
    pub(crate) fn parse(bytes: &'a [u8], at: usize) -> Result<Self, Error> {
        if at > bytes.len() {
            return Err(Error::UnexpectedEof);
        }
        let count = read_u32(bytes, at)?;
        if count == 0 {
            // Empty CFF2 INDEX is exactly the 4-byte count field.
            return Ok(Self {
                bytes,
                count: 0,
                end: at + 4,
                off_size: 1,
                offsets_at: at + 4,
                data_at: at + 4,
            });
        }
        let off_size = read_u8(bytes, at + 4)?;
        if !(1..=4).contains(&off_size) {
            return Err(Error::Cff("CFF2 INDEX offsetSize out of range"));
        }
        let offsets_at = at + 5;
        let n_offsets = (count as usize)
            .checked_add(1)
            .ok_or(Error::Cff("CFF2 INDEX count overflow"))?;
        let offsets_len = n_offsets
            .checked_mul(off_size as usize)
            .ok_or(Error::Cff("CFF2 INDEX offsets length overflow"))?;
        let data_at = offsets_at
            .checked_add(offsets_len)
            .ok_or(Error::Cff("CFF2 INDEX data offset overflow"))?;
        if data_at > bytes.len() {
            return Err(Error::UnexpectedEof);
        }
        let last = read_offset(bytes, offsets_at, off_size, count as usize)?;
        if last < 1 {
            return Err(Error::Cff("CFF2 INDEX last offset == 0"));
        }
        let data_len = (last - 1) as usize;
        let end = data_at
            .checked_add(data_len)
            .ok_or(Error::Cff("CFF2 INDEX end overflow"))?;
        if end > bytes.len() {
            return Err(Error::UnexpectedEof);
        }
        Ok(Self {
            bytes,
            count,
            end,
            off_size,
            offsets_at,
            data_at,
        })
    }

    /// Borrow entry `i` as a slice into the underlying bytes. Empty
    /// INDEXes and out-of-range indices return `Error::Cff(...)`.
    pub(crate) fn entry(&self, i: u32) -> Result<&'a [u8], Error> {
        if i >= self.count {
            return Err(Error::Cff("CFF2 INDEX entry out of range"));
        }
        let start = read_offset(self.bytes, self.offsets_at, self.off_size, i as usize)?;
        let end_off = read_offset(self.bytes, self.offsets_at, self.off_size, i as usize + 1)?;
        if start < 1 || end_off < start {
            return Err(Error::Cff("CFF2 INDEX: malformed offsets"));
        }
        let s = self.data_at + (start - 1) as usize;
        let e = self.data_at + (end_off - 1) as usize;
        if e > self.bytes.len() {
            return Err(Error::UnexpectedEof);
        }
        Ok(&self.bytes[s..e])
    }
}

/// Read a big-endian `u32` from `bytes[off..off+4]`.
fn read_u32(bytes: &[u8], off: usize) -> Result<u32, Error> {
    let s = bytes.get(off..off + 4).ok_or(Error::UnexpectedEof)?;
    Ok(u32::from_be_bytes([s[0], s[1], s[2], s[3]]))
}

/// Read offset element `i` from a CFF2 INDEX offset array.
fn read_offset(bytes: &[u8], offsets_at: usize, off_size: u8, i: usize) -> Result<u32, Error> {
    let off = offsets_at + i * off_size as usize;
    let s = bytes
        .get(off..off + off_size as usize)
        .ok_or(Error::UnexpectedEof)?;
    Ok(match off_size {
        1 => s[0] as u32,
        2 => u16::from_be_bytes([s[0], s[1]]) as u32,
        3 => ((s[0] as u32) << 16) | ((s[1] as u32) << 8) | s[2] as u32,
        4 => u32::from_be_bytes([s[0], s[1], s[2], s[3]]),
        _ => unreachable!("off_size validated to 1..=4 in parse()"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal CFF2 INDEX with three byte-string entries:
    /// "abc", "de", "" (empty).
    fn build_three_entry_index() -> Vec<u8> {
        // count = 3 (uint32!), offsetSize = 1
        // offsets: 1, 4, 6, 6 (1-based)
        // data: a b c d e
        let mut v = vec![0, 0, 0, 3]; // count
        v.push(1); // offsetSize
        v.extend_from_slice(&[1, 4, 6, 6]); // offsets[0..=3]
        v.extend_from_slice(b"abcde");
        v
    }

    #[test]
    fn parses_three_entries() {
        let v = build_three_entry_index();
        let idx = Cff2Index::parse(&v, 0).expect("parse");
        assert_eq!(idx.count, 3);
        assert_eq!(idx.entry(0).unwrap(), b"abc");
        assert_eq!(idx.entry(1).unwrap(), b"de");
        assert_eq!(idx.entry(2).unwrap(), b"");
        assert!(idx.entry(3).is_err());
    }

    #[test]
    fn empty_index_is_four_bytes() {
        // Per spec §6 "An empty INDEX is represented by a count field
        // with a 0 value and no additional fields. Thus, the total
        // size of an empty INDEX is 4 bytes."
        let v = vec![0u8, 0, 0, 0];
        let idx = Cff2Index::parse(&v, 0).expect("parse");
        assert_eq!(idx.count, 0);
        assert_eq!(idx.end, 4);
        assert!(idx.entry(0).is_err());
    }

    #[test]
    fn handles_off_size_2() {
        // count = 1 (uint32), offsetSize = 2, single 4-byte entry "WXYZ".
        let mut v = vec![0, 0, 0, 1];
        v.push(2);
        v.extend_from_slice(&[0, 1, 0, 5]); // offsets (u16 big-endian)
        v.extend_from_slice(b"WXYZ");
        let idx = Cff2Index::parse(&v, 0).expect("parse");
        assert_eq!(idx.entry(0).unwrap(), b"WXYZ");
        assert_eq!(idx.end, v.len());
    }

    #[test]
    fn handles_off_size_3() {
        // count = 2, offsetSize = 3, entries "x" and "yz".
        let mut v = vec![0, 0, 0, 2];
        v.push(3);
        // offsets: 1, 2, 4 (each 3 bytes, big-endian)
        v.extend_from_slice(&[0, 0, 1, 0, 0, 2, 0, 0, 4]);
        v.extend_from_slice(b"xyz");
        let idx = Cff2Index::parse(&v, 0).expect("parse");
        assert_eq!(idx.entry(0).unwrap(), b"x");
        assert_eq!(idx.entry(1).unwrap(), b"yz");
    }

    #[test]
    fn handles_off_size_4() {
        // count = 1, offsetSize = 4, single 1-byte entry "Q".
        let mut v = vec![0, 0, 0, 1];
        v.push(4);
        v.extend_from_slice(&[0, 0, 0, 1, 0, 0, 0, 2]); // u32 offsets
        v.extend_from_slice(b"Q");
        let idx = Cff2Index::parse(&v, 0).expect("parse");
        assert_eq!(idx.entry(0).unwrap(), b"Q");
    }

    #[test]
    fn rejects_truncated_data() {
        let mut v = build_three_entry_index();
        v.pop();
        assert!(Cff2Index::parse(&v, 0).is_err());
    }

    #[test]
    fn rejects_off_size_zero() {
        let mut v = vec![0, 0, 0, 1];
        v.push(0); // off_size = 0
        assert!(matches!(
            Cff2Index::parse(&v, 0),
            Err(Error::Cff("CFF2 INDEX offsetSize out of range"))
        ));
    }

    #[test]
    fn rejects_off_size_five() {
        let mut v = vec![0, 0, 0, 1];
        v.push(5);
        assert!(matches!(
            Cff2Index::parse(&v, 0),
            Err(Error::Cff("CFF2 INDEX offsetSize out of range"))
        ));
    }

    #[test]
    fn rejects_first_offset_zero() {
        // First offset must be 1 per spec; we test the simpler invariant
        // that an INDEX whose declared last-offset is 0 (i.e. data_len
        // would underflow) is rejected.
        let mut v = vec![0, 0, 0, 1, 1, 0, 0]; // count=1, off_size=1, offsets=[0, 0]
        v.extend_from_slice(b"");
        assert!(matches!(
            Cff2Index::parse(&v, 0),
            Err(Error::Cff("CFF2 INDEX last offset == 0"))
        ));
    }

    #[test]
    fn end_field_points_past_data() {
        let v = build_three_entry_index();
        let idx = Cff2Index::parse(&v, 0).expect("parse");
        assert_eq!(idx.end, v.len());
    }
}
