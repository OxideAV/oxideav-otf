//! `vhea` — vertical header (ISO/IEC 14496-22:2019 §5.7.9).
//!
//! The vertical analogue of `hhea`: it carries font-wide vertical
//! layout metrics and, crucially, `numOfLongVerMetrics` — the count of
//! full `(advanceHeight, topSideBearing)` pairs in the companion `vmtx`
//! table.
//!
//! Two versions exist. Version 1.0 (`0x00010000`) names the first three
//! int16 fields `ascent` / `descent` / `lineGap`; version 1.1
//! (`0x00011000`) renames them `vertTypoAscender` / `vertTypoDescender`
//! / `vertTypoLineGap`. The on-disk byte layout is **identical** between
//! the two versions — only the field semantics differ — so a single
//! parser handles both. We expose the v1.1 names as the canonical
//! accessors (`vert_typo_ascender`, etc.) with v1.0 aliases.

use crate::parser::{read_i16, read_u16, read_u32};
use crate::Error;

#[derive(Debug, Clone, Copy)]
pub struct VheaTable {
    /// Raw `version` Fixed: `0x00010000` or `0x00011000`.
    pub version: u32,
    /// `vertTypoAscender` (v1.1) / `ascent` (v1.0): distance from the
    /// centerline to the previous line's descent.
    pub ascent: i16,
    /// `vertTypoDescender` (v1.1) / `descent` (v1.0).
    pub descent: i16,
    /// `vertTypoLineGap` (v1.1) / `lineGap` (v1.0). Reserved (0) in v1.0.
    pub line_gap: i16,
    /// `advanceHeightMax` — maximum advance height in font units.
    pub advance_height_max: i16,
    /// `minTopSideBearing`.
    pub min_top_side_bearing: i16,
    /// `minBottomSideBearing`.
    pub min_bottom_side_bearing: i16,
    /// `yMaxExtent` = max(tsb + (yMax - yMin)).
    pub y_max_extent: i16,
    /// `caretSlopeRise`.
    pub caret_slope_rise: i16,
    /// `caretSlopeRun`.
    pub caret_slope_run: i16,
    /// `caretOffset`.
    pub caret_offset: i16,
    /// `numOfLongVerMetrics` — count of full metric pairs in `vmtx`.
    pub num_long_ver_metrics: u16,
}

impl VheaTable {
    pub fn parse(bytes: &[u8]) -> Result<Self, Error> {
        // Spec layout, big-endian (identical for v1.0 and v1.1):
        //   0  / 4 / version (Fixed)
        //   4  / 2 / ascent       (vertTypoAscender)
        //   6  / 2 / descent      (vertTypoDescender)
        //   8  / 2 / lineGap      (vertTypoLineGap)
        //  10  / 2 / advanceHeightMax
        //  12  / 2 / minTopSideBearing
        //  14  / 2 / minBottomSideBearing
        //  16  / 2 / yMaxExtent
        //  18  / 2 / caretSlopeRise
        //  20  / 2 / caretSlopeRun
        //  22  / 2 / caretOffset
        //  24  / 8 / reserved (4 * int16)
        //  32  / 2 / metricDataFormat
        //  34  / 2 / numOfLongVerMetrics
        if bytes.len() < 36 {
            return Err(Error::UnexpectedEof);
        }
        let version = read_u32(bytes, 0)?;
        let num_long_ver_metrics = read_u16(bytes, 34)?;
        if num_long_ver_metrics == 0 {
            return Err(Error::BadStructure("vhea.numOfLongVerMetrics == 0"));
        }
        Ok(Self {
            version,
            ascent: read_i16(bytes, 4)?,
            descent: read_i16(bytes, 6)?,
            line_gap: read_i16(bytes, 8)?,
            advance_height_max: read_i16(bytes, 10)?,
            min_top_side_bearing: read_i16(bytes, 12)?,
            min_bottom_side_bearing: read_i16(bytes, 14)?,
            y_max_extent: read_i16(bytes, 16)?,
            caret_slope_rise: read_i16(bytes, 18)?,
            caret_slope_run: read_i16(bytes, 20)?,
            caret_offset: read_i16(bytes, 22)?,
            num_long_ver_metrics,
        })
    }

    /// v1.1 canonical name for `ascent`.
    #[inline]
    pub fn vert_typo_ascender(&self) -> i16 {
        self.ascent
    }

    /// v1.1 canonical name for `descent`.
    #[inline]
    pub fn vert_typo_descender(&self) -> i16 {
        self.descent
    }

    /// v1.1 canonical name for `line_gap`.
    #[inline]
    pub fn vert_typo_line_gap(&self) -> i16 {
        self.line_gap
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build(version: u32, ascent: i16, descent: i16, num: u16) -> Vec<u8> {
        let mut b = vec![0u8; 36];
        b[0..4].copy_from_slice(&version.to_be_bytes());
        b[4..6].copy_from_slice(&ascent.to_be_bytes());
        b[6..8].copy_from_slice(&descent.to_be_bytes());
        b[34..36].copy_from_slice(&num.to_be_bytes());
        b
    }

    #[test]
    fn parses_v11_example() {
        // Spec §5.7.9 worked example: version 1.1, vertTypoAscender 1024,
        // numOfLongVerMetrics 258.
        let mut b = build(0x0001_1000, 1024, -1024, 258);
        b[10..12].copy_from_slice(&1933i16.to_be_bytes()); // advanceHeightMax
        b[14..16].copy_from_slice(&(-333i16).to_be_bytes()); // minBottomSideBearing
        b[16..18].copy_from_slice(&2036i16.to_be_bytes()); // yMaxExtent
        b[20..22].copy_from_slice(&1i16.to_be_bytes()); // caretSlopeRun
        let v = VheaTable::parse(&b).unwrap();
        assert_eq!(v.version, 0x0001_1000);
        assert_eq!(v.vert_typo_ascender(), 1024);
        assert_eq!(v.advance_height_max, 1933);
        assert_eq!(v.min_bottom_side_bearing, -333);
        assert_eq!(v.y_max_extent, 2036);
        assert_eq!(v.caret_slope_run, 1);
        assert_eq!(v.num_long_ver_metrics, 258);
    }

    #[test]
    fn parses_v10_aliases() {
        let b = build(0x0001_0000, 880, -120, 1);
        let v = VheaTable::parse(&b).unwrap();
        assert_eq!(v.ascent, 880);
        assert_eq!(v.vert_typo_ascender(), 880);
        assert_eq!(v.descent, -120);
        assert_eq!(v.vert_typo_descender(), -120);
    }

    #[test]
    fn rejects_zero_metrics() {
        let b = build(0x0001_0000, 0, 0, 0);
        assert!(matches!(VheaTable::parse(&b), Err(Error::BadStructure(_))));
    }

    #[test]
    fn rejects_short() {
        assert!(matches!(
            VheaTable::parse(&[0u8; 12]),
            Err(Error::UnexpectedEof)
        ));
    }
}
