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
use crate::parser::{read_f2dot14, read_u16, read_u32, read_u8};
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

/// A `DeltaSetIndexMap` table (ISO/IEC 14496-22:2019 §7.3.5.2 + the
/// format-0/format-1 header): maps an array index (a glyph ID, an axis
/// index, or a COLR variation index) to a delta-set `(outer, inner)`
/// index pair, in a packed representation.
///
/// Two header forms are decoded:
///
/// - **Format 0** — `format(1) entryFormat(1) mapCount(2) mapData[]`.
///   This is byte-identical to the ISO 14496-22:2019 headerless
///   `entryFormat(2) mapCount(2)` form used by `HVAR`/`VVAR`, because
///   the legacy 16-bit `entryFormat`'s high byte is permanently
///   reserved (zero) — the same byte the formatted header reads as
///   `format = 0`.
/// - **Format 1** — `format(1) entryFormat(1) mapCount(4) mapData[]`,
///   allowing a 32-bit entry count (used by `COLR` v1 and `avar` v2).
///
/// The `entryFormat` byte packs the inner-index bit count (low 4 bits,
/// minus 1) and the per-entry byte size (bits 4-5, minus 1). An index
/// past `mapCount - 1` clamps to the last entry.
#[derive(Debug, Clone)]
pub struct DeltaSetIndexMap {
    inner_bit_count: u32,
    entry_size: usize,
    map_count: usize,
    /// The raw `mapData` slice.
    data: Vec<u8>,
}

impl DeltaSetIndexMap {
    /// Parse a DeltaSetIndexMap (format 0 or 1) at `offset` within
    /// `parent`.
    pub fn parse_at(parent: &[u8], offset: usize) -> Result<Self, Error> {
        let format = read_u8(parent, offset)?;
        let entry_format = read_u8(parent, offset + 1)? as u16;
        let (map_count, data_start) = match format {
            0 => (read_u16(parent, offset + 2)? as usize, offset + 4),
            1 => (read_u32(parent, offset + 2)? as usize, offset + 6),
            _ => {
                return Err(Error::BadStructure("DeltaSetIndexMap: unknown format"));
            }
        };
        let inner_bit_count = (entry_format & 0x000F) as u32 + 1;
        let entry_size = (((entry_format & 0x0030) >> 4) + 1) as usize;
        let data_len = entry_size
            .checked_mul(map_count)
            .ok_or(Error::UnexpectedEof)?;
        let data = parent
            .get(
                data_start
                    ..data_start
                        .checked_add(data_len)
                        .ok_or(Error::UnexpectedEof)?,
            )
            .ok_or(Error::UnexpectedEof)?
            .to_vec();
        Ok(Self {
            inner_bit_count,
            entry_size,
            map_count,
            data,
        })
    }

    /// Resolve a glyph ID to its `(outerIndex, innerIndex)` delta-set
    /// index pair. A glyph past the last entry clamps to the last entry
    /// (per spec).
    pub fn index(&self, glyph_id: u16) -> (u16, u16) {
        self.index_u32(glyph_id as u32)
    }

    /// Resolve a 32-bit map index (a `COLR` `varIndexBase`-derived
    /// index or an `avar` v2 axis index) to its `(outerIndex,
    /// innerIndex)` delta-set index pair. An index past the last entry
    /// clamps to the last entry (per spec).
    pub fn index_u32(&self, idx: u32) -> (u16, u16) {
        if self.map_count == 0 {
            return (0, 0);
        }
        let i = (idx as usize).min(self.map_count - 1);
        let off = i * self.entry_size;
        let mut entry: u32 = 0;
        for b in 0..self.entry_size {
            entry = (entry << 8) | self.data[off + b] as u32;
        }
        let inner_mask = (1u32 << self.inner_bit_count) - 1;
        let outer = (entry >> self.inner_bit_count) as u16;
        let inner = (entry & inner_mask) as u16;
        (outer, inner)
    }

    /// Number of mapping entries.
    pub fn map_count(&self) -> usize {
        self.map_count
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

    #[test]
    fn delta_set_index_map_unpacks_entries() {
        // entryFormat: inner bit count 4 (low nibble = 3), entry size 2
        // bytes (bits 4-5 = 1). 3 entries.
        let entry_format: u16 = 0x0013; // (0x0010 size=2) | (0x0003 inner=4)
        let mut p = Vec::new();
        p.extend_from_slice(&entry_format.to_be_bytes());
        p.extend_from_slice(&3u16.to_be_bytes()); // mapCount
                                                  // entry = (outer << 4) | inner.
                                                  // glyph0 -> (1, 2) = 0x12
        p.extend_from_slice(&0x0012u16.to_be_bytes());
        // glyph1 -> (0, 5) = 0x05
        p.extend_from_slice(&0x0005u16.to_be_bytes());
        // glyph2 -> (2, 0) = 0x20
        p.extend_from_slice(&0x0020u16.to_be_bytes());

        let m = DeltaSetIndexMap::parse_at(&p, 0).unwrap();
        assert_eq!(m.map_count(), 3);
        assert_eq!(m.index(0), (1, 2));
        assert_eq!(m.index(1), (0, 5));
        assert_eq!(m.index(2), (2, 0));
        // glyph past the end clamps to the last entry.
        assert_eq!(m.index(99), (2, 0));
    }

    #[test]
    fn delta_set_index_map_format_1() {
        // format 1: uint32 mapCount. inner bit count 8 (low nibble 7),
        // entry size 2 (bits 4-5 = 1).
        let mut p = Vec::new();
        p.push(1u8); // format
        p.push(0x17u8); // entryFormat
        p.extend_from_slice(&2u32.to_be_bytes()); // mapCount
                                                  // entry = (outer << 8) | inner.
        p.extend_from_slice(&0x0103u16.to_be_bytes()); // idx0 -> (1, 3)
        p.extend_from_slice(&0x0210u16.to_be_bytes()); // idx1 -> (2, 16)

        let m = DeltaSetIndexMap::parse_at(&p, 0).unwrap();
        assert_eq!(m.map_count(), 2);
        assert_eq!(m.index_u32(0), (1, 3));
        assert_eq!(m.index_u32(1), (2, 16));
        // clamps to the last entry, including far past u16 range.
        assert_eq!(m.index_u32(0x0001_0000), (2, 16));
    }

    #[test]
    fn delta_set_index_map_rejects_unknown_format() {
        let p = [7u8, 0x13, 0, 1, 0, 0];
        assert!(matches!(
            DeltaSetIndexMap::parse_at(&p, 0),
            Err(Error::BadStructure(_))
        ));
    }
}
