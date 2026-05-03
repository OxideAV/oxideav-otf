//! CFF header (Adobe TN5176 §6, "Header").
//!
//! Format:
//! ```text
//! Card8  major     // major version (1 for CFF, 2 for CFF2 — rejected here)
//! Card8  minor     // minor version
//! Card8  hdrSize   // size of the header in bytes (>= 4)
//! OffSize offSize  // offset size for the absolute-offset operands in
//!                  //   the Top DICT (1..4)
//! ```
//!
//! The header may be larger than 4 bytes — `hdrSize` lets the writer
//! pad it for alignment. Anything past byte 4 is ignored on parse.

use crate::Error;
use crate::parser::read_u8;

/// Parsed CFF header. We don't keep `major`/`minor` because nothing
/// downstream branches on them; the version-1 acceptance check is done
/// up-front and that's the only divergence.
#[derive(Debug, Clone, Copy)]
pub(crate) struct CffHeader {
    /// Header size in bytes (>= 4). Subsequent INDEX parsers start
    /// reading at this offset.
    pub(crate) size: u8,
    /// Offset size for absolute-offset operands. Currently unused —
    /// Top DICT operand offsets are encoded in-line (operator-specific
    /// integer formats) rather than relying on this field. We keep it
    /// recorded for future CFF2 work.
    #[allow(dead_code)]
    pub(crate) off_size: u8,
}

impl CffHeader {
    pub(crate) fn parse(bytes: &[u8]) -> Result<Self, Error> {
        let major = read_u8(bytes, 0)?;
        let _minor = read_u8(bytes, 1)?;
        let size = read_u8(bytes, 2)?;
        let off_size = read_u8(bytes, 3)?;

        if major != 1 {
            // CFF2 (major == 2) gets its own crate / module later.
            // Anything else is malformed.
            return Err(Error::Cff("unsupported CFF major version"));
        }
        if size < 4 {
            return Err(Error::Cff("hdrSize < 4"));
        }
        if off_size < 1 || off_size > 4 {
            return Err(Error::Cff("offSize out of range"));
        }
        if (size as usize) > bytes.len() {
            return Err(Error::UnexpectedEof);
        }
        Ok(Self { size, off_size })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_minimal_header() {
        let bytes = [1u8, 0, 4, 2];
        let h = CffHeader::parse(&bytes).expect("parse");
        assert_eq!(h.size, 4);
        assert_eq!(h.off_size, 2);
    }

    #[test]
    fn rejects_cff2_major() {
        let bytes = [2u8, 0, 5, 2];
        assert!(matches!(
            CffHeader::parse(&bytes),
            Err(Error::Cff("unsupported CFF major version"))
        ));
    }

    #[test]
    fn rejects_short_header() {
        // hdrSize claims 8 but we only supplied 4 bytes.
        let bytes = [1u8, 0, 8, 1];
        assert!(matches!(CffHeader::parse(&bytes), Err(Error::UnexpectedEof)));
    }
}
