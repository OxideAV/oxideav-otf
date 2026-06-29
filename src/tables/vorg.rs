//! `VORG` — vertical origin (ISO/IEC 14496-22:2019 §5.4).
//!
//! An optional CFF-OFF table giving the Y coordinate of each glyph's
//! vertical origin directly, so a vertical-writing client need not
//! compute the glyph bounding-box top from the charstring + `vmtx` top
//! side bearing. (Ignored in TrueType-outline fonts, where `glyf` +
//! `vmtx` already suffice.)
//!
//! Layout:
//!
//! ```text
//! VORG
//!   uint16 majorVersion = 1
//!   uint16 minorVersion = 0
//!   int16  defaultVertOriginY
//!   uint16 numVertOriginYMetrics
//!   { uint16 glyphIndex; int16 vertOriginY } vertOriginYMetrics[]
//!                                              // sorted by glyphIndex
//! ```
//!
//! A glyph absent from the metrics array uses `defaultVertOriginY`.

use crate::parser::{read_i16, read_u16};
use crate::Error;

/// A parsed `VORG` table.
#[derive(Debug, Clone)]
pub struct VorgTable {
    default_vert_origin_y: i16,
    /// `(glyphIndex, vertOriginY)` pairs, sorted by glyphIndex.
    metrics: Vec<(u16, i16)>,
}

impl VorgTable {
    pub fn parse(bytes: &[u8]) -> Result<Self, Error> {
        if bytes.len() < 8 {
            return Err(Error::UnexpectedEof);
        }
        let major = read_u16(bytes, 0)?;
        if major != 1 {
            return Err(Error::BadStructure("VORG: unsupported majorVersion"));
        }
        let default_vert_origin_y = read_i16(bytes, 4)?;
        let count = read_u16(bytes, 6)? as usize;
        let need = 8 + count * 4;
        if bytes.len() < need {
            return Err(Error::UnexpectedEof);
        }
        let mut metrics = Vec::with_capacity(count);
        for i in 0..count {
            let off = 8 + i * 4;
            metrics.push((read_u16(bytes, off)?, read_i16(bytes, off + 2)?));
        }
        Ok(Self {
            default_vert_origin_y,
            metrics,
        })
    }

    /// The default vertical-origin Y for glyphs not in the metrics array.
    pub fn default_vert_origin_y(&self) -> i16 {
        self.default_vert_origin_y
    }

    /// The vertical-origin Y coordinate (design units) for a glyph: its
    /// explicit entry if present (binary-searched — the array is sorted),
    /// else `defaultVertOriginY`.
    pub fn vert_origin_y(&self, glyph_id: u16) -> i16 {
        match self.metrics.binary_search_by(|(g, _)| g.cmp(&glyph_id)) {
            Ok(i) => self.metrics[i].1,
            Err(_) => self.default_vert_origin_y,
        }
    }

    /// The explicit `(glyphIndex, vertOriginY)` entries.
    pub fn metrics(&self) -> &[(u16, i16)] {
        &self.metrics
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build() -> Vec<u8> {
        // Spec §5.4 example: default 880, entries (10,889) (12,861) (13,849).
        let mut b = Vec::new();
        b.extend_from_slice(&1u16.to_be_bytes()); // major
        b.extend_from_slice(&0u16.to_be_bytes()); // minor
        b.extend_from_slice(&880i16.to_be_bytes()); // default
        b.extend_from_slice(&3u16.to_be_bytes()); // count
        for (g, y) in [(10u16, 889i16), (12, 861), (13, 849)] {
            b.extend_from_slice(&g.to_be_bytes());
            b.extend_from_slice(&y.to_be_bytes());
        }
        b
    }

    #[test]
    fn spec_example() {
        let v = VorgTable::parse(&build()).unwrap();
        assert_eq!(v.default_vert_origin_y(), 880);
        assert_eq!(v.vert_origin_y(10), 889);
        assert_eq!(v.vert_origin_y(12), 861);
        assert_eq!(v.vert_origin_y(13), 849);
        // glyphs without an entry use the default.
        assert_eq!(v.vert_origin_y(0), 880);
        assert_eq!(v.vert_origin_y(11), 880);
        assert_eq!(v.vert_origin_y(999), 880);
        assert_eq!(v.metrics().len(), 3);
    }

    #[test]
    fn no_metrics_array() {
        let mut b = vec![0u8; 8];
        b[0..2].copy_from_slice(&1u16.to_be_bytes());
        b[4..6].copy_from_slice(&500i16.to_be_bytes());
        let v = VorgTable::parse(&b).unwrap();
        assert_eq!(v.vert_origin_y(42), 500);
        assert_eq!(v.metrics().len(), 0);
    }

    #[test]
    fn rejects_bad_version() {
        let mut b = vec![0u8; 8];
        b[0..2].copy_from_slice(&2u16.to_be_bytes());
        assert!(matches!(VorgTable::parse(&b), Err(Error::BadStructure(_))));
    }
}
