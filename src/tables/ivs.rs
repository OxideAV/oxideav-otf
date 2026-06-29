//! ItemVariationStore with delta-set storage (ISO/IEC 14496-22:2019
//! §7.2.3) — the variation-data structure shared by `MVAR`, `HVAR`,
//! `VVAR`, `GDEF`, and `BASE`.
//!
//! This differs from the CFF2 ItemVariationStore in
//! [`crate::cff2::varstore`]: in CFF2 the deltas live as `blend`
//! operands inside CharStrings, so the IVS `ItemVariationData` carries
//! only its `regionIndexes` (and `itemCount`/`shortDeltaCount` are 0).
//! In the metrics/positioning variation tables, by contrast, the IVS
//! **stores the delta sets directly**: a two-dimensional array of
//! `itemCount` rows × `regionIndexCount` columns, with the first
//! `shortDeltaCount` columns as `int16` and the rest as `int8`.
//!
//! Layout (§7.2.3.2):
//!
//! ```text
//! ItemVariationStore
//!   uint16   format = 1
//!   Offset32 variationRegionListOffset
//!   uint16   itemVariationDataCount
//!   Offset32 itemVariationDataOffsets[itemVariationDataCount]
//!
//! VariationRegionList
//!   uint16 axisCount
//!   uint16 regionCount
//!   VariationRegion regions[regionCount]   // axisCount RegionAxisCoordinates each
//!
//! ItemVariationData subtable
//!   uint16 itemCount
//!   uint16 shortDeltaCount
//!   uint16 regionIndexCount
//!   uint16 regionIndexes[regionIndexCount]
//!   DeltaSet deltaSets[itemCount]
//!     int16 [shortDeltaCount] + int8 [regionIndexCount - shortDeltaCount]
//! ```
//!
//! The interpolation (§7.2.3.3, §7.1.7) selects a delta-set row by an
//! outer (subtable) + inner (row) index pair, computes a per-region
//! scalar for the active instance, and sums `scalar * delta` across the
//! subtable's regions.

use crate::cff2::varstore::{RegionAxisCoordinates, VariationRegion};
use crate::parser::{read_f2dot14, read_u16, read_u32};
use crate::Error;

/// One delta-set-storing `ItemVariationData` subtable.
#[derive(Debug, Clone)]
pub struct ItemVariationData {
    /// `regionIndexes` — indices into the store's region list, in column
    /// order of the delta sets.
    region_indexes: Vec<u16>,
    /// Decoded delta-set rows. `delta_sets[row][col]` is the delta for
    /// `region_indexes[col]`. Rows = `itemCount`.
    delta_sets: Vec<Vec<i32>>,
}

impl ItemVariationData {
    fn parse(bytes: &[u8], off: usize, region_total: usize) -> Result<Self, Error> {
        let item_count = read_u16(bytes, off)? as usize;
        let short_delta_count = read_u16(bytes, off + 2)? as usize;
        let region_index_count = read_u16(bytes, off + 4)? as usize;
        if short_delta_count > region_index_count {
            return Err(Error::BadStructure(
                "IVS: shortDeltaCount > regionIndexCount",
            ));
        }
        let mut region_indexes = Vec::with_capacity(region_index_count);
        for i in 0..region_index_count {
            let ri = read_u16(bytes, off + 6 + i * 2)?;
            if (ri as usize) >= region_total {
                return Err(Error::BadStructure("IVS: regionIndex out of range"));
            }
            region_indexes.push(ri);
        }
        // Each delta-set row is shortDeltaCount int16 + (regionIndexCount
        // - shortDeltaCount) int8 bytes.
        let row_bytes = short_delta_count * 2 + (region_index_count - short_delta_count);
        let mut cursor = off + 6 + region_index_count * 2;
        let mut delta_sets = Vec::with_capacity(item_count);
        for _ in 0..item_count {
            if cursor + row_bytes > bytes.len() {
                return Err(Error::UnexpectedEof);
            }
            let mut row = Vec::with_capacity(region_index_count);
            let mut c = cursor;
            for _ in 0..short_delta_count {
                row.push(read_u16(bytes, c)? as i16 as i32);
                c += 2;
            }
            for _ in short_delta_count..region_index_count {
                row.push(bytes[c] as i8 as i32);
                c += 1;
            }
            delta_sets.push(row);
            cursor += row_bytes;
        }
        Ok(Self {
            region_indexes,
            delta_sets,
        })
    }

    /// Number of delta-set rows (`itemCount`).
    pub fn item_count(&self) -> usize {
        self.delta_sets.len()
    }

    /// The region indices, in delta-set column order.
    pub fn region_indexes(&self) -> &[u16] {
        &self.region_indexes
    }
}

/// A delta-set-storing ItemVariationStore.
#[derive(Debug, Clone)]
pub struct ItemVariationStore {
    /// `axisCount` of the region list.
    pub axis_count: u16,
    regions: Vec<VariationRegion>,
    subtables: Vec<ItemVariationData>,
}

impl ItemVariationStore {
    /// Parse a bare ItemVariationStore whose byte 0 is the `format`
    /// field. All internal offsets are relative to byte 0 of `ivs`.
    pub fn parse(ivs: &[u8]) -> Result<Self, Error> {
        let format = read_u16(ivs, 0)?;
        if format != 1 {
            return Err(Error::BadStructure("IVS: format must be 1"));
        }
        let region_list_offset = read_u32(ivs, 2)? as usize;
        let ivd_count = read_u16(ivs, 6)? as usize;
        let mut ivd_offsets = Vec::with_capacity(ivd_count);
        for i in 0..ivd_count {
            ivd_offsets.push(read_u32(ivs, 8 + i * 4)? as usize);
        }
        let (axis_count, regions) = parse_region_list(ivs, region_list_offset)?;
        let mut subtables = Vec::with_capacity(ivd_count);
        for &o in &ivd_offsets {
            subtables.push(ItemVariationData::parse(ivs, o, regions.len())?);
        }
        Ok(Self {
            axis_count,
            regions,
            subtables,
        })
    }

    /// Parse an ItemVariationStore reached through an `Offset32` from a
    /// parent table — `parent` is the parent table slice and `offset` is
    /// the field value (relative to the parent's byte 0).
    pub fn parse_at(parent: &[u8], offset: usize) -> Result<Self, Error> {
        let ivs = parent.get(offset..).ok_or(Error::UnexpectedEof)?;
        Self::parse(ivs)
    }

    /// The variation region list.
    pub fn regions(&self) -> &[VariationRegion] {
        &self.regions
    }

    /// Number of `ItemVariationData` subtables (the valid range of an
    /// outer index).
    pub fn subtable_count(&self) -> usize {
        self.subtables.len()
    }

    /// Borrow a subtable by outer index.
    pub fn subtable(&self, outer: usize) -> Option<&ItemVariationData> {
        self.subtables.get(outer)
    }

    /// Compute the interpolated **net adjustment** (delta) for the
    /// delta-set selected by `(outer, inner)`, given a normalized
    /// instance coordinate tuple (§7.1.7 + §7.2.3.3).
    ///
    /// Returns `0.0` if the index pair is out of range (the spec's
    /// "item is constant" case is handled by the parent table, which
    /// simply has no index for a constant item).
    pub fn delta(&self, outer: u16, inner: u16, instance_coords: &[f32]) -> f32 {
        let Some(ivd) = self.subtables.get(outer as usize) else {
            return 0.0;
        };
        let Some(row) = ivd.delta_sets.get(inner as usize) else {
            return 0.0;
        };
        let mut net = 0.0f32;
        for (col, &delta) in row.iter().enumerate() {
            let Some(&ri) = ivd.region_indexes.get(col) else {
                continue;
            };
            let Some(region) = self.regions.get(ri as usize) else {
                continue;
            };
            net += region.scalar(instance_coords) * delta as f32;
        }
        net
    }
}

/// Parse the `VariationRegionList` at `offset`; returns
/// `(axisCount, regions)`.
fn parse_region_list(ivs: &[u8], offset: usize) -> Result<(u16, Vec<VariationRegion>), Error> {
    let axis_count = read_u16(ivs, offset)?;
    let region_count = read_u16(ivs, offset + 2)? as usize;
    let axis_count_usize = axis_count as usize;
    let mut regions = Vec::with_capacity(region_count);
    let mut cursor = offset + 4;
    for _ in 0..region_count {
        let mut region_axes = Vec::with_capacity(axis_count_usize);
        for _ in 0..axis_count_usize {
            let start = read_f2dot14(ivs, cursor)?;
            let peak = read_f2dot14(ivs, cursor + 2)?;
            let end = read_f2dot14(ivs, cursor + 4)?;
            region_axes.push(RegionAxisCoordinates { start, peak, end });
            cursor += 6;
        }
        regions.push(VariationRegion { region_axes });
    }
    Ok((axis_count, regions))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a single-axis IVS: 2 regions, 1 subtable with 1 delta-set
    /// row of 2 deltas (one int16, one int8).
    fn build() -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(&1u16.to_be_bytes()); // format
        v.extend_from_slice(&12u32.to_be_bytes()); // regionListOffset = 12
        v.extend_from_slice(&1u16.to_be_bytes()); // ivdCount
        v.extend_from_slice(&28u32.to_be_bytes()); // ivd[0] @ 28
        assert_eq!(v.len(), 12);
        // region list: axisCount 1, regionCount 2.
        v.extend_from_slice(&1u16.to_be_bytes());
        v.extend_from_slice(&2u16.to_be_bytes());
        // region0 weight (0,1,1).
        v.extend_from_slice(&f2(0.0));
        v.extend_from_slice(&f2(1.0));
        v.extend_from_slice(&f2(1.0));
        // region1 weight (-1,-1,0).
        v.extend_from_slice(&f2(-1.0));
        v.extend_from_slice(&f2(-1.0));
        v.extend_from_slice(&f2(0.0));
        assert_eq!(v.len(), 28);
        // IVD0: itemCount 1, shortDeltaCount 1, regionIndexCount 2.
        v.extend_from_slice(&1u16.to_be_bytes());
        v.extend_from_slice(&1u16.to_be_bytes());
        v.extend_from_slice(&2u16.to_be_bytes());
        v.extend_from_slice(&0u16.to_be_bytes()); // regionIndex 0
        v.extend_from_slice(&1u16.to_be_bytes()); // regionIndex 1
                                                  // delta-set row: int16 = 100 (region0), int8 = -20 (region1).
        v.extend_from_slice(&100i16.to_be_bytes());
        v.push((-20i8) as u8);
        v
    }

    fn f2(v: f32) -> [u8; 2] {
        ((v * 16384.0).round() as i16).to_be_bytes()
    }

    #[test]
    fn parses_and_interpolates() {
        let ivs = ItemVariationStore::parse(&build()).unwrap();
        assert_eq!(ivs.axis_count, 1);
        assert_eq!(ivs.regions().len(), 2);
        assert_eq!(ivs.subtable_count(), 1);
        assert_eq!(ivs.subtable(0).unwrap().item_count(), 1);
        assert_eq!(ivs.subtable(0).unwrap().region_indexes(), &[0, 1]);

        // At normalized weight +1.0: region0 scalar 1.0, region1 (peak
        // -1, coord +1) out of range → 0. delta = 1.0*100 + 0*-20 = 100.
        assert!((ivs.delta(0, 0, &[1.0]) - 100.0).abs() < 1e-5);
        // At +0.5: region0 scalar 0.5 → 50.
        assert!((ivs.delta(0, 0, &[0.5]) - 50.0).abs() < 1e-5);
        // At -1.0: region0 out of range (coord < start 0) → 0; region1
        // scalar 1.0 → -20.
        assert!((ivs.delta(0, 0, &[-1.0]) - (-20.0)).abs() < 1e-5);
        // At default 0.0: both regions inactive → 0.
        assert_eq!(ivs.delta(0, 0, &[0.0]), 0.0);
    }

    #[test]
    fn out_of_range_index_is_zero() {
        let ivs = ItemVariationStore::parse(&build()).unwrap();
        assert_eq!(ivs.delta(5, 0, &[1.0]), 0.0);
        assert_eq!(ivs.delta(0, 9, &[1.0]), 0.0);
    }

    #[test]
    fn rejects_bad_format() {
        let mut v = build();
        v[0..2].copy_from_slice(&2u16.to_be_bytes());
        assert!(matches!(
            ItemVariationStore::parse(&v),
            Err(Error::BadStructure(_))
        ));
    }
}
