//! `hhea` — horizontal header.

use crate::Error;
use crate::parser::{read_i16, read_u16};

#[derive(Debug, Clone, Copy)]
pub struct HheaTable {
    pub ascent: i16,
    pub descent: i16,
    pub line_gap: i16,
    pub advance_width_max: u16,
    pub num_long_hor_metrics: u16,
}

impl HheaTable {
    pub fn parse(bytes: &[u8]) -> Result<Self, Error> {
        // Spec layout, big-endian:
        //   0  / 4 / version (Fixed; should be 1.0)
        //   4  / 2 / Ascender
        //   6  / 2 / Descender
        //   8  / 2 / LineGap
        //  10  / 2 / advanceWidthMax (UFWord)
        //  12  / 2 / minLeftSideBearing
        //  14  / 2 / minRightSideBearing
        //  16  / 2 / xMaxExtent
        //  18  / 2 / caretSlopeRise
        //  20  / 2 / caretSlopeRun
        //  22  / 2 / caretOffset
        //  24  / 8 / reserved (4 * i16)
        //  32  / 2 / metricDataFormat
        //  34  / 2 / numberOfHMetrics
        if bytes.len() < 36 {
            return Err(Error::UnexpectedEof);
        }
        let ascent = read_i16(bytes, 4)?;
        let descent = read_i16(bytes, 6)?;
        let line_gap = read_i16(bytes, 8)?;
        let advance_width_max = read_u16(bytes, 10)?;
        let num_long_hor_metrics = read_u16(bytes, 34)?;
        if num_long_hor_metrics == 0 {
            return Err(Error::BadStructure("hhea.numberOfHMetrics == 0"));
        }
        Ok(Self {
            ascent,
            descent,
            line_gap,
            advance_width_max,
            num_long_hor_metrics,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_minimal() {
        let mut b = vec![0u8; 36];
        b[0..4].copy_from_slice(&0x00010000u32.to_be_bytes());
        b[4..6].copy_from_slice(&(1900i16).to_be_bytes());
        b[6..8].copy_from_slice(&(-500i16).to_be_bytes());
        b[10..12].copy_from_slice(&(2048u16).to_be_bytes());
        b[34..36].copy_from_slice(&(1u16).to_be_bytes());
        let h = HheaTable::parse(&b).unwrap();
        assert_eq!(h.ascent, 1900);
        assert_eq!(h.descent, -500);
        assert_eq!(h.num_long_hor_metrics, 1);
    }
}
