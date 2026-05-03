//! `head` — font header.
//!
//! Spec: Microsoft OpenType `head`. We decode the few fields that
//! round 1 actually needs: `unitsPerEm` and the glyph-extent bbox.
//! `indexToLocFormat` exists too but is meaningless for CFF fonts
//! (no `loca` table); we record it for parity but don't validate.

use crate::Error;
use crate::parser::{read_i16, read_u16};

#[derive(Debug, Clone, Copy)]
pub struct HeadTable {
    pub units_per_em: u16,
    pub x_min: i16,
    pub y_min: i16,
    pub x_max: i16,
    pub y_max: i16,
    pub mac_style: u16,
}

impl HeadTable {
    pub fn parse(bytes: &[u8]) -> Result<Self, Error> {
        // Layout (offset / size / field):
        //   0  / 4 / version (Fixed, expect 1.0)
        //   4  / 4 / fontRevision (Fixed)
        //   8  / 4 / checkSumAdjustment
        //  12  / 4 / magicNumber (0x5F0F3CF5)
        //  16  / 2 / flags
        //  18  / 2 / unitsPerEm
        //  20  / 8 / created (LONGDATETIME)
        //  28  / 8 / modified
        //  36  / 2 / xMin
        //  38  / 2 / yMin
        //  40  / 2 / xMax
        //  42  / 2 / yMax
        //  44  / 2 / macStyle
        //  46  / 2 / lowestRecPPEM
        //  48  / 2 / fontDirectionHint
        //  50  / 2 / indexToLocFormat
        //  52  / 2 / glyphDataFormat
        if bytes.len() < 54 {
            return Err(Error::UnexpectedEof);
        }
        let units_per_em = read_u16(bytes, 18)?;
        if units_per_em == 0 {
            return Err(Error::BadStructure("head.unitsPerEm == 0"));
        }
        let x_min = read_i16(bytes, 36)?;
        let y_min = read_i16(bytes, 38)?;
        let x_max = read_i16(bytes, 40)?;
        let y_max = read_i16(bytes, 42)?;
        let mac_style = read_u16(bytes, 44)?;
        Ok(Self {
            units_per_em,
            x_min,
            y_min,
            x_max,
            y_max,
            mac_style,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build_head(units: u16) -> Vec<u8> {
        let mut b = vec![0u8; 54];
        b[0..4].copy_from_slice(&0x00010000u32.to_be_bytes());
        b[12..16].copy_from_slice(&0x5F0F3CF5u32.to_be_bytes());
        b[18..20].copy_from_slice(&units.to_be_bytes());
        b[36..38].copy_from_slice(&(-100i16).to_be_bytes());
        b[38..40].copy_from_slice(&(-200i16).to_be_bytes());
        b[40..42].copy_from_slice(&(1500i16).to_be_bytes());
        b[42..44].copy_from_slice(&(2000i16).to_be_bytes());
        b
    }

    #[test]
    fn parses_minimal() {
        let h = HeadTable::parse(&build_head(1000)).unwrap();
        assert_eq!(h.units_per_em, 1000);
        assert_eq!(h.x_min, -100);
        assert_eq!(h.y_max, 2000);
    }

    #[test]
    fn rejects_zero_upem() {
        assert!(HeadTable::parse(&build_head(0)).is_err());
    }
}
