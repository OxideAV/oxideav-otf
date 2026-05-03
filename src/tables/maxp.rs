//! `maxp` — maximum profile.
//!
//! For OpenType-CFF fonts this is the v0.5 form (6 bytes; just
//! `numGlyphs`). v1.0 (32 bytes) is for TrueType-outline fonts —
//! we accept it but only consult `numGlyphs`.

use crate::Error;
use crate::parser::{read_u16, read_u32};

#[derive(Debug, Clone, Copy)]
pub struct MaxpTable {
    pub num_glyphs: u16,
}

impl MaxpTable {
    pub fn parse(bytes: &[u8]) -> Result<Self, Error> {
        if bytes.len() < 6 {
            return Err(Error::UnexpectedEof);
        }
        let version = read_u32(bytes, 0)?;
        if version != 0x00005000 && version != 0x00010000 {
            return Err(Error::BadStructure("maxp.version not 0.5 or 1.0"));
        }
        let num_glyphs = read_u16(bytes, 4)?;
        if num_glyphs == 0 {
            return Err(Error::BadStructure("maxp.numGlyphs == 0"));
        }
        Ok(Self { num_glyphs })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_v05() {
        let mut b = vec![0u8; 6];
        b[0..4].copy_from_slice(&0x00005000u32.to_be_bytes());
        b[4..6].copy_from_slice(&(123u16).to_be_bytes());
        assert_eq!(MaxpTable::parse(&b).unwrap().num_glyphs, 123);
    }

    #[test]
    fn parses_v10() {
        let mut b = vec![0u8; 32];
        b[0..4].copy_from_slice(&0x00010000u32.to_be_bytes());
        b[4..6].copy_from_slice(&(4567u16).to_be_bytes());
        assert_eq!(MaxpTable::parse(&b).unwrap().num_glyphs, 4567);
    }
}
