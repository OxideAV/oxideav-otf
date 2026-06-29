//! `vmtx` — vertical metrics (ISO/IEC 14496-22:2019 §5.7.10).
//!
//! The vertical analogue of `hmtx`. Layout: `numOfLongVerMetrics`
//! `(advanceHeight: u16, topSideBearing: i16)` pairs (the `vMetrics`
//! array) followed by `numGlyphs - numOfLongVerMetrics` bare
//! `topSideBearing: i16` values. Tail glyphs share the advance height
//! of the *last* full metric pair (the spec's monospaced-run optimisation:
//! the second array is at the end of the font and every glyph in it has
//! the same advance height as the last `vMetrics` entry).

use crate::parser::{read_i16, read_u16};
use crate::Error;

#[derive(Debug, Clone)]
pub struct VmtxTable<'a> {
    bytes: &'a [u8],
    num_long_ver_metrics: u16,
    num_glyphs: u16,
}

impl<'a> VmtxTable<'a> {
    pub fn parse(
        bytes: &'a [u8],
        num_long_ver_metrics: u16,
        num_glyphs: u16,
    ) -> Result<Self, Error> {
        if num_long_ver_metrics == 0 {
            return Err(Error::BadStructure("vmtx: numOfLongVerMetrics == 0"));
        }
        if num_long_ver_metrics > num_glyphs {
            return Err(Error::BadStructure("vmtx: numOfLongVerMetrics > numGlyphs"));
        }
        let expected =
            num_long_ver_metrics as usize * 4 + (num_glyphs - num_long_ver_metrics) as usize * 2;
        if bytes.len() < expected {
            return Err(Error::UnexpectedEof);
        }
        Ok(Self {
            bytes,
            num_long_ver_metrics,
            num_glyphs,
        })
    }

    /// Per-glyph advance height in font units. Returns 0 for an
    /// out-of-range id.
    pub fn advance(&self, glyph_id: u16) -> u16 {
        if glyph_id >= self.num_glyphs {
            return 0;
        }
        let idx = glyph_id.min(self.num_long_ver_metrics - 1) as usize;
        read_u16(self.bytes, idx * 4).unwrap_or(0)
    }

    /// Per-glyph top side bearing in font units. Returns 0 for an
    /// out-of-range id.
    pub fn top_side_bearing(&self, glyph_id: u16) -> i16 {
        if glyph_id >= self.num_glyphs {
            return 0;
        }
        if glyph_id < self.num_long_ver_metrics {
            read_i16(self.bytes, glyph_id as usize * 4 + 2).unwrap_or(0)
        } else {
            let tail_idx = (glyph_id - self.num_long_ver_metrics) as usize;
            let tail_off = self.num_long_ver_metrics as usize * 4 + tail_idx * 2;
            read_i16(self.bytes, tail_off).unwrap_or(0)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pairs_then_tail() {
        let mut b = Vec::new();
        b.extend_from_slice(&1000u16.to_be_bytes());
        b.extend_from_slice(&50i16.to_be_bytes());
        b.extend_from_slice(&1000u16.to_be_bytes());
        b.extend_from_slice(&(-30i16).to_be_bytes());
        b.extend_from_slice(&77i16.to_be_bytes());
        let v = VmtxTable::parse(&b, 2, 3).unwrap();
        assert_eq!(v.advance(0), 1000);
        assert_eq!(v.advance(1), 1000);
        // tail glyph inherits the last full advance height.
        assert_eq!(v.advance(2), 1000);
        assert_eq!(v.top_side_bearing(0), 50);
        assert_eq!(v.top_side_bearing(1), -30);
        assert_eq!(v.top_side_bearing(2), 77);
    }

    #[test]
    fn out_of_range() {
        let mut b = Vec::new();
        b.extend_from_slice(&1000u16.to_be_bytes());
        b.extend_from_slice(&50i16.to_be_bytes());
        let v = VmtxTable::parse(&b, 1, 1).unwrap();
        assert_eq!(v.advance(5), 0);
        assert_eq!(v.top_side_bearing(5), 0);
    }

    #[test]
    fn rejects_mismatch() {
        let b = vec![0u8; 8];
        assert!(matches!(
            VmtxTable::parse(&b, 3, 2),
            Err(Error::BadStructure(_))
        ));
    }
}
