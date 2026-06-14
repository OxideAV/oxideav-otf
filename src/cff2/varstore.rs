//! CFF2 VariationStore / ItemVariationStore (OpenType 1.9.1 `CFF2`
//! table, §12 "VariationStore data contents").
//!
//! A variable CFF2 font carries an embedded **ItemVariationStore**
//! (IVS) pointed at by the Top DICT `VariationStoreOffset` operator.
//! The IVS defines the global list of variation regions of the design
//! space (`VariationRegionList`) plus one or more `ItemVariationData`
//! subtables. In CFF2 the delta values themselves are NOT stored in
//! the IVS — they live as operands of `blend` operators in CharStrings
//! and Private DICTs — so each `ItemVariationData` carries only its
//! `regionIndexes` array (and the spec mandates `itemCount == 0` and
//! `wordDeltaCount == 0`, leaving no `deltaSets` array). What the IVS
//! supplies a `blend` is therefore the number of active regions `k`
//! (the length of the selected `regionIndexes` array) and, per region,
//! the axis intervals used to compute the per-region interpolation
//! scalars.
//!
//! On-disk layout (CFF2 §12; byte offsets match the spec's worked
//! "Example CFF2 table"):
//!
//! ```text
//! VariationStore                       // wrapper inside the CFF2 table
//!   uint16 length                      // byte length of the IVS that follows
//!   uint8  data[length]                // the ItemVariationStore
//!
//! ItemVariationStore                   // `data` above
//!   uint16 format                      // = 1
//!   uint32 variationRegionListOffset   // from start of the IVS
//!   uint16 itemVariationDataCount
//!   uint32 itemVariationDataOffsets[itemVariationDataCount]
//!                                      // each from start of the IVS
//!
//! VariationRegionList
//!   uint16 axisCount
//!   uint16 regionCount
//!   VariationRegion variationRegions[regionCount]
//!
//! VariationRegion                      // regionAxes[axisCount]
//!   RegionAxisCoordinates regionAxes[axisCount]
//!
//! RegionAxisCoordinates
//!   F2DOT14 startCoord
//!   F2DOT14 peakCoord
//!   F2DOT14 endCoord
//!
//! ItemVariationData                    // CFF2: itemCount and
//!   uint16 itemCount                   //   shortDeltaCount are 0 and
//!   uint16 shortDeltaCount             //   the deltaSets array is
//!   uint16 regionIndexCount            //   omitted entirely.
//!   uint16 regionIndexes[regionIndexCount]
//! ```
//!
//! Spec: `docs/text/opentype/otspec-cff2.html` §12 + worked
//! "Example CFF2 table".

use crate::parser::{read_u16, read_u32};
use crate::Error;

/// The only ItemVariationStore format defined by the spec is `1`.
const ITEM_VARIATION_STORE_FORMAT: u16 = 1;

/// One axis interval of a [`VariationRegion`], stored on disk as three
/// consecutive `F2DOT14` values (§12 "RegionAxisCoordinates").
///
/// The three coordinates bound the region's span on one axis of the
/// normalized design space (each in the closed interval `[-1.0, 1.0]`):
/// the region is active between `start` and `end`, with full effect at
/// `peak`. They are retained as the raw `f32` decode of the F2DOT14
/// fixed-point words; the per-region scalar computation that combines
/// them with an instance's axis settings is not performed here (it is
/// specified in the OpenType *Font Variations Common Table Formats*
/// chapter, which is not part of the staged CFF2 doc).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RegionAxisCoordinates {
    /// `startCoord` — the axis value at which the region begins to take
    /// effect. F2DOT14, normalized to `[-1.0, 1.0]`.
    pub start: f32,
    /// `peakCoord` — the axis value at which the region has full
    /// effect (scalar 1.0). A `peak` of exactly `0.0` means the region
    /// does not vary along this axis.
    pub peak: f32,
    /// `endCoord` — the axis value at which the region stops taking
    /// effect. F2DOT14, normalized to `[-1.0, 1.0]`.
    pub end: f32,
}

/// One region of the design variation space: an axis interval
/// (`RegionAxisCoordinates`) for each of the IVS's `axisCount` axes.
#[derive(Debug, Clone, PartialEq)]
pub struct VariationRegion {
    /// `regionAxes[axisCount]` — one interval per design axis, in axis
    /// order. Always exactly `axisCount` long.
    pub region_axes: Vec<RegionAxisCoordinates>,
}

/// One `ItemVariationData` subtable. In CFF2 only the `regionIndexes`
/// array is meaningful; `itemCount` and `wordDeltaCount`
/// (`shortDeltaCount`) are required to be `0`, so there are no delta
/// sets stored. We retain both fields verbatim so a caller can detect
/// a spec-violating non-CFF2 IVS embedded by mistake.
#[derive(Debug, Clone, PartialEq)]
pub struct ItemVariationData {
    /// `itemCount` — number of delta-set rows. CFF2 mandates `0`.
    pub item_count: u16,
    /// `shortDeltaCount` (a.k.a. `wordDeltaCount`) — number of "short"
    /// (16-bit) deltas per row. CFF2 mandates `0`.
    pub short_delta_count: u16,
    /// `regionIndexes` — 0-based indices into the IVS's
    /// `VariationRegionList`, naming the active regions for this
    /// subtable. Its length is the `k` value a `blend` operator uses
    /// (the number of deltas per blended operand).
    pub region_indexes: Vec<u16>,
}

impl ItemVariationData {
    /// `k` — the number of active variation regions selected by this
    /// subtable (the length of `regionIndexes`). A `blend` operator
    /// that uses the subtable expects `k` delta operands per default
    /// value (CFF2 §9 "blend").
    pub fn region_count(&self) -> usize {
        self.region_indexes.len()
    }
}

/// A parsed CFF2 ItemVariationStore: the global `VariationRegionList`
/// plus the array of `ItemVariationData` subtables. Constructed from
/// the bytes pointed at by the Top DICT `VariationStoreOffset`.
#[derive(Debug, Clone, PartialEq)]
pub struct ItemVariationStore {
    /// Number of design axes (the per-region `regionAxes` length).
    pub axis_count: u16,
    /// `VariationRegionList` — every region used anywhere in the font,
    /// in index order. `ItemVariationData::region_indexes` entries
    /// index into this list.
    pub regions: Vec<VariationRegion>,
    /// `ItemVariationData` subtables in index order. Subtable `0` is
    /// the default; `vsindex` selects a different one (CFF2 §9).
    pub item_variation_data: Vec<ItemVariationData>,
}

impl ItemVariationStore {
    /// Parse the `VariationStore` wrapper at `offset` within the CFF2
    /// table `bytes`. This consumes the leading `uint16 length` field
    /// and then parses the `length`-byte ItemVariationStore that
    /// follows, with all internal offsets taken relative to the start
    /// of the ItemVariationStore (i.e. the byte after `length`).
    ///
    /// `offset` is the value of the Top DICT `VariationStoreOffset`
    /// operator, relative to byte 0 of the CFF2 table.
    pub fn parse_variation_store(bytes: &[u8], offset: usize) -> Result<Self, Error> {
        let length = read_u16(bytes, offset)? as usize;
        let ivs_start = offset
            .checked_add(2)
            .ok_or(Error::Cff("CFF2 VariationStore offset overflow"))?;
        let ivs_end = ivs_start
            .checked_add(length)
            .ok_or(Error::Cff("CFF2 VariationStore length overflow"))?;
        if ivs_end > bytes.len() {
            return Err(Error::UnexpectedEof);
        }
        // Confine parsing to the declared ItemVariationStore extent so a
        // truncated/lying length can never read into adjacent CFF2
        // structures.
        Self::parse(&bytes[ivs_start..ivs_end])
    }

    /// Parse a bare ItemVariationStore (no leading `length` field) from
    /// a slice whose byte 0 is the IVS `format` field. All internal
    /// offsets are relative to byte 0 of `ivs`.
    pub fn parse(ivs: &[u8]) -> Result<Self, Error> {
        let format = read_u16(ivs, 0)?;
        if format != ITEM_VARIATION_STORE_FORMAT {
            return Err(Error::Cff("CFF2 ItemVariationStore format must be 1"));
        }
        let region_list_offset = read_u32(ivs, 2)? as usize;
        let ivd_count = read_u16(ivs, 6)? as usize;

        // itemVariationDataOffsets[ivd_count] begins at byte 8.
        let mut ivd_offsets = Vec::with_capacity(ivd_count);
        for i in 0..ivd_count {
            let off = 8usize
                .checked_add(i * 4)
                .ok_or(Error::Cff("CFF2 IVS subtable offset table overflow"))?;
            ivd_offsets.push(read_u32(ivs, off)? as usize);
        }

        let (axis_count, regions) = Self::parse_region_list(ivs, region_list_offset)?;

        let mut item_variation_data = Vec::with_capacity(ivd_count);
        for &ivd_off in &ivd_offsets {
            item_variation_data.push(Self::parse_item_variation_data(
                ivs,
                ivd_off,
                regions.len(),
            )?);
        }

        Ok(Self {
            axis_count,
            regions,
            item_variation_data,
        })
    }

    /// Parse the `VariationRegionList` at `offset` within the IVS.
    /// Returns `(axisCount, regions)`.
    fn parse_region_list(ivs: &[u8], offset: usize) -> Result<(u16, Vec<VariationRegion>), Error> {
        let axis_count = read_u16(ivs, offset)?;
        let region_count = read_u16(ivs, offset + 2)? as usize;
        // Each VariationRegion is axisCount RegionAxisCoordinates, each
        // 6 bytes (three F2DOT14 words). regionAxes start at offset + 4.
        let axis_count_usize = axis_count as usize;
        let mut regions = Vec::with_capacity(region_count);
        let mut cursor = offset
            .checked_add(4)
            .ok_or(Error::Cff("CFF2 VariationRegionList offset overflow"))?;
        for _ in 0..region_count {
            let mut region_axes = Vec::with_capacity(axis_count_usize);
            for _ in 0..axis_count_usize {
                let start = read_f2dot14(ivs, cursor)?;
                let peak = read_f2dot14(ivs, cursor + 2)?;
                let end = read_f2dot14(ivs, cursor + 4)?;
                region_axes.push(RegionAxisCoordinates { start, peak, end });
                cursor = cursor
                    .checked_add(6)
                    .ok_or(Error::Cff("CFF2 VariationRegion extent overflow"))?;
            }
            regions.push(VariationRegion { region_axes });
        }
        Ok((axis_count, regions))
    }

    /// Parse one `ItemVariationData` subtable at `offset` within the
    /// IVS. `region_total` is the IVS's `VariationRegionList` length,
    /// used to validate every `regionIndexes` entry.
    fn parse_item_variation_data(
        ivs: &[u8],
        offset: usize,
        region_total: usize,
    ) -> Result<ItemVariationData, Error> {
        let item_count = read_u16(ivs, offset)?;
        let short_delta_count = read_u16(ivs, offset + 2)?;
        let region_index_count = read_u16(ivs, offset + 4)? as usize;
        let mut region_indexes = Vec::with_capacity(region_index_count);
        for i in 0..region_index_count {
            let off = offset
                .checked_add(6 + i * 2)
                .ok_or(Error::Cff("CFF2 regionIndexes overflow"))?;
            let ri = read_u16(ivs, off)?;
            if (ri as usize) >= region_total {
                return Err(Error::Cff(
                    "CFF2 regionIndex out of range of VariationRegionList",
                ));
            }
            region_indexes.push(ri);
        }
        Ok(ItemVariationData {
            item_count,
            short_delta_count,
            region_indexes,
        })
    }

    /// Number of `ItemVariationData` subtables. A `vsindex` operand
    /// must be `< this`.
    pub fn item_variation_data_count(&self) -> usize {
        self.item_variation_data.len()
    }

    /// The `ItemVariationData` selected by a `vsindex` value `ivd`
    /// (default `0`). `None` if `ivd` is out of range.
    pub fn item_variation_data_at(&self, ivd: usize) -> Option<&ItemVariationData> {
        self.item_variation_data.get(ivd)
    }
}

/// Decode a big-endian F2DOT14 fixed-point value to `f32`.
///
/// F2DOT14 is a signed 16-bit value with a 2-bit integer part and a
/// 14-bit fraction (OpenType data types): the stored `i16` divided by
/// `16384`. e.g. `0xC000` = `-1.0`, `0xE000` = `-0.5`, `0x0000` =
/// `0.0`, `0x7FFF` ≈ `1.999939`.
#[inline]
fn read_f2dot14(bytes: &[u8], off: usize) -> Result<f32, Error> {
    let raw = read_u16(bytes, off)? as i16;
    Ok(raw as f32 / 16384.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The exact bytes of the ItemVariationStore from the CFF2 spec's
    /// worked "Example CFF2 table" (§ worked example, CFF2 offsets
    /// 0x10..0x37):
    ///   VariationStore: length = 0x0026 (38).
    ///   IVS: format=1, regionListOffset=12, ivdCount=1, ivdOffsets=[28].
    ///   RegionList: axisCount=1, regionCount=2,
    ///       region0 axis0: start=-1.0 (C000) peak=-0.5 (E000) end=0.0 (0000)
    ///       region1 axis0: start=-1.0 (C000) peak=-1.0 (C000) end=-0.5 (E000)
    ///   IVD0: itemCount=0, shortDeltaCount=0, regionIndexCount=2,
    ///       regionIndexes={0,1}.
    fn spec_example_variation_store() -> Vec<u8> {
        let mut v = Vec::new();
        // VariationStore length = 38.
        v.extend_from_slice(&[0x00, 0x26]);
        // --- ItemVariationStore starts here (byte index 2 of v) ---
        v.extend_from_slice(&[0x00, 0x01]); // format = 1
        v.extend_from_slice(&[0x00, 0x00, 0x00, 0x0C]); // regionListOffset = 12
        v.extend_from_slice(&[0x00, 0x01]); // itemVariationDataCount = 1
        v.extend_from_slice(&[0x00, 0x00, 0x00, 0x1C]); // ivdOffsets[0] = 28
                                                        // VariationRegionList @ IVS offset 12.
        v.extend_from_slice(&[0x00, 0x01]); // axisCount = 1
        v.extend_from_slice(&[0x00, 0x02]); // regionCount = 2
        v.extend_from_slice(&[0xC0, 0x00]); // region0 start = -1.0
        v.extend_from_slice(&[0xE0, 0x00]); // region0 peak  = -0.5
        v.extend_from_slice(&[0x00, 0x00]); // region0 end   =  0.0
        v.extend_from_slice(&[0xC0, 0x00]); // region1 start = -1.0
        v.extend_from_slice(&[0xC0, 0x00]); // region1 peak  = -1.0
        v.extend_from_slice(&[0xE0, 0x00]); // region1 end   = -0.5
                                            // ItemVariationData subtable 0 @ IVS offset 28.
        v.extend_from_slice(&[0x00, 0x00]); // itemCount = 0
        v.extend_from_slice(&[0x00, 0x00]); // shortDeltaCount = 0
        v.extend_from_slice(&[0x00, 0x02]); // regionIndexCount = 2
        v.extend_from_slice(&[0x00, 0x00, 0x00, 0x01]); // regionIndexes = {0, 1}
        v
    }

    #[test]
    fn parses_spec_example_via_wrapper() {
        let v = spec_example_variation_store();
        let ivs = ItemVariationStore::parse_variation_store(&v, 0).expect("parse");
        assert_eq!(ivs.axis_count, 1);
        assert_eq!(ivs.regions.len(), 2);
        assert_eq!(ivs.item_variation_data_count(), 1);

        // Region 0: -1.0 / -0.5 / 0.0
        let r0 = &ivs.regions[0].region_axes[0];
        assert_eq!(
            *r0,
            RegionAxisCoordinates {
                start: -1.0,
                peak: -0.5,
                end: 0.0
            }
        );
        // Region 1: -1.0 / -1.0 / -0.5
        let r1 = &ivs.regions[1].region_axes[0];
        assert_eq!(
            *r1,
            RegionAxisCoordinates {
                start: -1.0,
                peak: -1.0,
                end: -0.5
            }
        );

        // IVD0: CFF2 mandates itemCount == 0, shortDeltaCount == 0.
        let ivd = ivs.item_variation_data_at(0).unwrap();
        assert_eq!(ivd.item_count, 0);
        assert_eq!(ivd.short_delta_count, 0);
        assert_eq!(ivd.region_indexes, vec![0, 1]);
        assert_eq!(ivd.region_count(), 2); // k = 2
    }

    #[test]
    fn parses_bare_ivs() {
        // Same example, skipping the 2-byte VariationStore length wrapper.
        let v = spec_example_variation_store();
        let ivs = ItemVariationStore::parse(&v[2..]).expect("parse");
        assert_eq!(ivs.axis_count, 1);
        assert_eq!(ivs.regions.len(), 2);
        assert_eq!(ivs.item_variation_data[0].region_indexes, vec![0, 1]);
    }

    #[test]
    fn f2dot14_decode() {
        // Spec values: C000=-1.0, E000=-0.5, 0000=0.0, 4000=1.0, 7FFF≈2.0.
        assert_eq!(read_f2dot14(&[0xC0, 0x00], 0).unwrap(), -1.0);
        assert_eq!(read_f2dot14(&[0xE0, 0x00], 0).unwrap(), -0.5);
        assert_eq!(read_f2dot14(&[0x00, 0x00], 0).unwrap(), 0.0);
        assert_eq!(read_f2dot14(&[0x40, 0x00], 0).unwrap(), 1.0);
        assert_eq!(read_f2dot14(&[0x20, 0x00], 0).unwrap(), 0.5);
        // 0x7FFF / 16384 = 1.99993896...
        let v = read_f2dot14(&[0x7F, 0xFF], 0).unwrap();
        assert!((v - 1.999_939).abs() < 1e-6);
    }

    #[test]
    fn rejects_bad_format() {
        let mut v = spec_example_variation_store();
        // Set IVS format (bytes 2..4 of v) to 2.
        v[2] = 0x00;
        v[3] = 0x02;
        let err = ItemVariationStore::parse_variation_store(&v, 0).unwrap_err();
        match err {
            Error::Cff(s) => assert!(s.contains("format must be 1")),
            _ => panic!("unexpected: {err:?}"),
        }
    }

    #[test]
    fn rejects_region_index_out_of_range() {
        let mut v = spec_example_variation_store();
        // regionIndexes are the last 4 bytes: {0, 1}. Make the second
        // index 5 (only 2 regions exist).
        let n = v.len();
        v[n - 1] = 0x05;
        let err = ItemVariationStore::parse_variation_store(&v, 0).unwrap_err();
        match err {
            Error::Cff(s) => assert!(s.contains("out of range")),
            _ => panic!("unexpected: {err:?}"),
        }
    }

    #[test]
    fn rejects_length_past_eof() {
        let mut v = spec_example_variation_store();
        // Claim length 0xFFFF.
        v[0] = 0xFF;
        v[1] = 0xFF;
        let err = ItemVariationStore::parse_variation_store(&v, 0).unwrap_err();
        assert!(matches!(err, Error::UnexpectedEof));
    }

    #[test]
    fn multiple_item_variation_data_subtables() {
        // axisCount=1, regionCount=3; two IVDs selecting different
        // region subsets ({0} and {1,2}). Exercises vsindex-style
        // multi-subtable layout.
        let mut v = Vec::new();
        // No wrapper; build a bare IVS.
        v.extend_from_slice(&[0x00, 0x01]); // format = 1
        v.extend_from_slice(&[0x00, 0x00, 0x00, 0x10]); // regionListOffset = 16
        v.extend_from_slice(&[0x00, 0x02]); // ivdCount = 2
        v.extend_from_slice(&[0x00, 0x00, 0x00, 0x26]); // ivd[0] @ 38
        v.extend_from_slice(&[0x00, 0x00, 0x00, 0x2E]); // ivd[1] @ 46
                                                        // RegionList @ 16: axisCount=1, regionCount=3.
        assert_eq!(v.len(), 16);
        v.extend_from_slice(&[0x00, 0x01]); // axisCount
        v.extend_from_slice(&[0x00, 0x03]); // regionCount
        for _ in 0..3 {
            v.extend_from_slice(&[0x40, 0x00, 0x40, 0x00, 0x40, 0x00]); // 1.0/1.0/1.0
        }
        // IVD0 @ 38: {0}.
        assert_eq!(v.len(), 38);
        v.extend_from_slice(&[0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00]);
        // IVD1 @ 46: {1, 2}.
        assert_eq!(v.len(), 46);
        v.extend_from_slice(&[0x00, 0x00, 0x00, 0x00, 0x00, 0x02, 0x00, 0x01, 0x00, 0x02]);

        let ivs = ItemVariationStore::parse(&v).expect("parse");
        assert_eq!(ivs.regions.len(), 3);
        assert_eq!(ivs.item_variation_data_count(), 2);
        assert_eq!(
            ivs.item_variation_data_at(0).unwrap().region_indexes,
            vec![0]
        );
        assert_eq!(
            ivs.item_variation_data_at(1).unwrap().region_indexes,
            vec![1, 2]
        );
        assert!(ivs.item_variation_data_at(2).is_none());
    }
}
