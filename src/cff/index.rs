//! CFF INDEX structure (Adobe TN5176 §5).
//!
//! INDEX is the universal "array of variable-sized objects" CFF
//! container. Format:
//!
//! ```text
//! Card16 count                           // number of objects
//! Card8  offSize                         // 1..4; size of each offset below
//! Offset offset[count + 1]               // offSize-byte big-endian offsets
//!                                        //   relative to (data_start - 1)
//! Card8  data[offset[count] - 1]         // contiguous object payloads
//! ```
//!
//! Offsets are 1-based: `offset[0]` is always 1, and the byte at
//! position `offset[i] - 1` (relative to `data_start`) is the first
//! byte of object `i`. The trailing `offset[count]` is the size of
//! the data section + 1 (so the difference between consecutive
//! offsets gives each object's length).
//!
//! When `count == 0` the structure is just `Card16(0)` — only 2 bytes
//! total, no `offSize` / `offsets` / `data` sections at all.

use crate::parser::{read_u16, read_u8};
use crate::Error;

/// A parsed CFF INDEX. Stores the original byte slice + lazily-parsed
/// offset metadata; entries are returned by zero-copy `&[u8]` slices.
#[derive(Debug, Clone)]
pub(crate) struct Index<'a> {
    /// Whole CFF table (entries are slices into this).
    pub(crate) bytes: &'a [u8],
    /// Number of entries in the INDEX.
    pub(crate) count: u32,
    /// Offset of the first byte AFTER this INDEX (i.e. where the next
    /// adjacent structure starts). Useful when walking a sequence of
    /// INDEXes (Name → Top → String → Global Subrs).
    pub(crate) end: usize,
    /// Width of each offset entry in bytes (1..4).
    off_size: u8,
    /// Offset of the first byte of `offset[0]` within `bytes`.
    offsets_at: usize,
    /// Offset of the first byte of `data[0]` within `bytes`. Note: the
    /// CFF spec defines this as `(offset_array_end - 1)` — i.e. byte
    /// offsets in the offset array are 1-based with respect to this
    /// position.
    data_at: usize,
}

impl<'a> Index<'a> {
    /// Parse an INDEX starting at `at` within `bytes`.
    pub(crate) fn parse(bytes: &'a [u8], at: usize) -> Result<Self, Error> {
        if at > bytes.len() {
            return Err(Error::UnexpectedEof);
        }
        let count = read_u16(bytes, at)? as u32;
        if count == 0 {
            // Empty INDEX is just the Card16 — no offSize / offsets / data.
            return Ok(Self {
                bytes,
                count: 0,
                end: at + 2,
                off_size: 1,
                offsets_at: at + 2,
                data_at: at + 2,
            });
        }
        let off_size = read_u8(bytes, at + 2)?;
        if !(1..=4).contains(&off_size) {
            return Err(Error::Cff("INDEX offSize out of range"));
        }
        let offsets_at = at + 3;
        let offsets_len = (count as usize + 1) * off_size as usize;
        let data_at = offsets_at + offsets_len;
        if data_at > bytes.len() {
            return Err(Error::UnexpectedEof);
        }
        // Read the trailing offset to know the data section length.
        let last = read_offset(bytes, offsets_at, off_size, count as usize)?;
        if last < 1 {
            return Err(Error::Cff("INDEX last offset == 0"));
        }
        let data_len = (last - 1) as usize;
        let end = data_at + data_len;
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

    /// Borrow entry `i` as a slice. Both empty-INDEX (no entries) and
    /// out-of-range `i` return `Error::Cff(...)`.
    pub(crate) fn entry(&self, i: u32) -> Result<&'a [u8], Error> {
        if i >= self.count {
            return Err(Error::Cff("INDEX entry out of range"));
        }
        let start = read_offset(self.bytes, self.offsets_at, self.off_size, i as usize)?;
        let end_off = read_offset(self.bytes, self.offsets_at, self.off_size, i as usize + 1)?;
        if start < 1 || end_off < start {
            return Err(Error::Cff("malformed INDEX offsets"));
        }
        let s = self.data_at + (start - 1) as usize;
        let e = self.data_at + (end_off - 1) as usize;
        if e > self.bytes.len() {
            return Err(Error::UnexpectedEof);
        }
        Ok(&self.bytes[s..e])
    }
}

/// Read offset entry `i` (0-indexed) from an INDEX offset array.
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

    /// Build a minimal INDEX with three byte-string entries: "abc",
    /// "de", "" (empty). Useful as a regression-target for the
    /// 1-based offset arithmetic.
    fn build_three_entry_index() -> Vec<u8> {
        // count = 3, offSize = 1
        // offsets: 1, 4, 6, 6  (1-based; entries are abc / de / ε)
        // data: a b c d e
        vec![
            0x00, 0x03, // count
            0x01, // offSize
            0x01, 0x04, 0x06, 0x06, // offsets[0..=3]
            b'a', b'b', b'c', b'd', b'e',
        ]
    }

    #[test]
    fn parses_three_entries() {
        let v = build_three_entry_index();
        let idx = Index::parse(&v, 0).expect("parse");
        assert_eq!(idx.count, 3);
        assert_eq!(idx.entry(0).unwrap(), b"abc");
        assert_eq!(idx.entry(1).unwrap(), b"de");
        assert_eq!(idx.entry(2).unwrap(), b"");
        assert!(idx.entry(3).is_err());
    }

    #[test]
    fn empty_index_is_two_bytes() {
        let v = vec![0u8, 0]; // count = 0
        let idx = Index::parse(&v, 0).expect("parse");
        assert_eq!(idx.count, 0);
        assert_eq!(idx.end, 2);
        assert!(idx.entry(0).is_err());
    }

    #[test]
    fn end_field_points_past_data() {
        let v = build_three_entry_index();
        let idx = Index::parse(&v, 0).expect("parse");
        assert_eq!(idx.end, v.len());
    }

    #[test]
    fn rejects_truncated_data() {
        // Same header as build_three_entry_index but truncate the
        // trailing 'e' byte.
        let mut v = build_three_entry_index();
        v.pop();
        assert!(Index::parse(&v, 0).is_err());
    }

    #[test]
    fn handles_off_size_2() {
        // count = 1, offSize = 2, single 4-byte entry "WXYZ".
        // offsets: [0001, 0005] (big-endian u16).
        let v = vec![
            0x00, 0x01, // count
            0x02, // offSize
            0x00, 0x01, 0x00, 0x05, // offsets
            b'W', b'X', b'Y', b'Z',
        ];
        let idx = Index::parse(&v, 0).expect("parse");
        assert_eq!(idx.entry(0).unwrap(), b"WXYZ");
    }
}
