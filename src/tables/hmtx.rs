//! `hmtx` — horizontal metrics.
//!
//! Layout: `numberOfHMetrics` (from `hhea`) `(advanceWidth: u16, lsb:
//! i16)` pairs followed by `numGlyphs - numberOfHMetrics` bare `lsb:
//! i16` values. Tail glyphs share the advance of the *last* full
//! metric pair.

use crate::parser::{read_i16, read_u16};
use crate::Error;

#[derive(Debug, Clone)]
pub struct HmtxTable<'a> {
    bytes: &'a [u8],
    num_long_hor_metrics: u16,
    num_glyphs: u16,
}

impl<'a> HmtxTable<'a> {
    pub fn parse(
        bytes: &'a [u8],
        num_long_hor_metrics: u16,
        num_glyphs: u16,
    ) -> Result<Self, Error> {
        if num_long_hor_metrics == 0 {
            return Err(Error::BadStructure("hmtx: numberOfHMetrics == 0"));
        }
        if num_long_hor_metrics > num_glyphs {
            return Err(Error::BadStructure("hmtx: numberOfHMetrics > numGlyphs"));
        }
        let expected =
            num_long_hor_metrics as usize * 4 + (num_glyphs - num_long_hor_metrics) as usize * 2;
        if bytes.len() < expected {
            return Err(Error::UnexpectedEof);
        }
        Ok(Self {
            bytes,
            num_long_hor_metrics,
            num_glyphs,
        })
    }

    /// Per-glyph advance in font units. Returns 0 for an out-of-range id.
    pub fn advance(&self, glyph_id: u16) -> u16 {
        if glyph_id >= self.num_glyphs {
            return 0;
        }
        let idx = glyph_id.min(self.num_long_hor_metrics - 1) as usize;
        read_u16(self.bytes, idx * 4).unwrap_or(0)
    }

    /// Per-glyph left-side bearing in font units. Returns 0 for an
    /// out-of-range id.
    pub fn lsb(&self, glyph_id: u16) -> i16 {
        if glyph_id >= self.num_glyphs {
            return 0;
        }
        if glyph_id < self.num_long_hor_metrics {
            read_i16(self.bytes, glyph_id as usize * 4 + 2).unwrap_or(0)
        } else {
            let tail_idx = (glyph_id - self.num_long_hor_metrics) as usize;
            let tail_off = self.num_long_hor_metrics as usize * 4 + tail_idx * 2;
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
        b.extend_from_slice(&300u16.to_be_bytes());
        b.extend_from_slice(&10i16.to_be_bytes());
        b.extend_from_slice(&400u16.to_be_bytes());
        b.extend_from_slice(&(-20i16).to_be_bytes());
        b.extend_from_slice(&99i16.to_be_bytes());
        let h = HmtxTable::parse(&b, 2, 3).unwrap();
        assert_eq!(h.advance(0), 300);
        assert_eq!(h.advance(1), 400);
        assert_eq!(h.advance(2), 400);
        assert_eq!(h.lsb(2), 99);
    }
}
