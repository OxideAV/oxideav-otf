//! CFF2 header (OpenType 1.9.1 `CFF2` table, §6 "CFF2 Header").
//!
//! Format (Table 8 / "headerFormat" anchor in the spec):
//! ```text
//! uint8  majorVersion   // must be 2
//! uint8  minorVersion   // must be 0
//! uint8  headerSize     // must be 5; future versions may extend
//! uint16 topDICTSize    // length of the Top DICT subtable
//! ```
//!
//! The TopDICT subtable starts immediately after the header — for the
//! 1.9.1 layout, at byte offset 5 from the start of the CFF2 table. The
//! `headerSize` field exists so future versions can interpose more
//! header fields after `topDICTSize` while older parsers still locate
//! the Top DICT correctly: a CFF2 parser MUST add `headerSize` to the
//! table-base offset, not the constant `5`, to skip the header.
//!
//! The sum `headerSize + topDICTSize` is the location within the CFF2
//! table of the required `GlobalSubrINDEX` subtable (spec §6).

use crate::parser::{read_u16, read_u8};
use crate::Error;

/// Parsed CFF2 table header.
#[derive(Debug, Clone, Copy)]
pub struct Cff2Header {
    /// Major version. Always `2` for a valid CFF2 table.
    pub major: u8,
    /// Minor version. Always `0` for the OpenType 1.9.1 format.
    pub minor: u8,
    /// Header size in bytes. Always `5` for the OpenType 1.9.1 format;
    /// future versions may grow it (see module-level docs). A parser
    /// MUST honour this value when locating the Top DICT rather than
    /// assuming `5`.
    pub header_size: u8,
    /// Length of the Top DICT subtable that follows the header, in
    /// bytes.
    pub top_dict_size: u16,
}

impl Cff2Header {
    /// Parse the 5-byte CFF2 header. Returns `Error::Cff(...)` on any
    /// spec violation (wrong major, header_size < 5).
    pub(crate) fn parse(bytes: &[u8]) -> Result<Self, Error> {
        let major = read_u8(bytes, 0)?;
        let minor = read_u8(bytes, 1)?;
        let header_size = read_u8(bytes, 2)?;
        let top_dict_size = read_u16(bytes, 3)?;

        if major != 2 {
            return Err(Error::Cff("CFF2 header: major != 2"));
        }
        // The spec sets minor to 0 for 1.9.1; we don't reject a higher
        // minor since the field is explicitly there to signal a
        // back-compatible refinement.
        if header_size < 5 {
            return Err(Error::Cff("CFF2 header: headerSize < 5"));
        }
        if (header_size as usize) > bytes.len() {
            return Err(Error::UnexpectedEof);
        }
        Ok(Self {
            major,
            minor,
            header_size,
            top_dict_size,
        })
    }

    /// Byte offset (from CFF2 table start) of the Top DICT subtable —
    /// always `header_size`.
    pub fn top_dict_offset(&self) -> usize {
        self.header_size as usize
    }

    /// Byte offset (from CFF2 table start) of the GlobalSubrINDEX
    /// subtable that follows the Top DICT. Per spec §6 this is the sum
    /// `headerSize + topDICTSize`.
    pub fn global_subr_index_offset(&self) -> usize {
        self.header_size as usize + self.top_dict_size as usize
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_minimal_header() {
        // major=2, minor=0, headerSize=5, topDICTSize=0x000B (11).
        let bytes = [2u8, 0, 5, 0x00, 0x0B];
        let h = Cff2Header::parse(&bytes).expect("parse");
        assert_eq!(h.major, 2);
        assert_eq!(h.minor, 0);
        assert_eq!(h.header_size, 5);
        assert_eq!(h.top_dict_size, 11);
        assert_eq!(h.top_dict_offset(), 5);
        assert_eq!(h.global_subr_index_offset(), 16);
    }

    #[test]
    fn rejects_cff1_major() {
        // major=1 is CFF (TN5176), not CFF2.
        let bytes = [1u8, 0, 5, 0x00, 0x0B];
        assert!(matches!(
            Cff2Header::parse(&bytes),
            Err(Error::Cff("CFF2 header: major != 2"))
        ));
    }

    #[test]
    fn rejects_header_size_below_5() {
        let bytes = [2u8, 0, 4, 0x00, 0x0B];
        assert!(matches!(
            Cff2Header::parse(&bytes),
            Err(Error::Cff("CFF2 header: headerSize < 5"))
        ));
    }

    #[test]
    fn rejects_short_input() {
        // Only 4 bytes — can't read topDICTSize.
        let bytes = [2u8, 0, 5, 0x00];
        assert!(matches!(
            Cff2Header::parse(&bytes),
            Err(Error::UnexpectedEof)
        ));
    }

    #[test]
    fn tolerates_extended_header() {
        // header_size=8 with three padding bytes; topDICTSize must come
        // before the padding per spec format Table 8. A real CFF2 parser
        // should still locate the Top DICT at offset 8, not 5.
        let bytes = [2u8, 0, 8, 0x00, 0x10, 0, 0, 0];
        let h = Cff2Header::parse(&bytes).expect("parse");
        assert_eq!(h.header_size, 8);
        assert_eq!(h.top_dict_offset(), 8);
        assert_eq!(h.global_subr_index_offset(), 8 + 16);
    }

    #[test]
    fn rejects_when_declared_header_exceeds_input() {
        // Claims headerSize=10 but we only supplied 5 bytes.
        let bytes = [2u8, 0, 10, 0x00, 0x0B];
        assert!(matches!(
            Cff2Header::parse(&bytes),
            Err(Error::UnexpectedEof)
        ));
    }
}
