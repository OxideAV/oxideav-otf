//! ItemVariationStore with delta-set storage (ISO/IEC 14496-22:2019
//! §7.2.3; `docs/text/opentype/otspec-otvarcommonformats.html`,
//! "Item variation stores") — the variation-data structure shared by
//! `MVAR`, `HVAR`, `VVAR`, `GDEF`, `BASE`, and `COLR`.
//!
//! This differs from the CFF2 ItemVariationStore in
//! [`crate::cff2::varstore`]: in CFF2 the deltas live as `blend`
//! operands inside CharStrings, so the IVS `ItemVariationData` carries
//! only its `regionIndexes` (and `itemCount`/`wordDeltaCount` are 0).
//! In the metrics/positioning variation tables, by contrast, the IVS
//! **stores the delta sets directly**: a two-dimensional array of
//! `itemCount` rows × `regionIndexCount` columns.
//!
//! Layout (§7.2.3.2 / the common-formats chapter):
//!
//! ```text
//! ItemVariationStore
//!   uint16   format = 1
//!   Offset32 variationRegionListOffset
//!   uint16   itemVariationDataCount
//!   Offset32 itemVariationDataOffsets[itemVariationDataCount]  // NULL = no subtable
//!
//! VariationRegionList
//!   uint16 axisCount
//!   uint16 regionCount
//!   VariationRegion regions[regionCount]   // axisCount RegionAxisCoordinates each
//!
//! ItemVariationData subtable
//!   uint16 itemCount
//!   uint16 wordDeltaCount        // 0x8000 LONG_WORDS | 0x7FFF WORD_DELTA_COUNT_MASK
//!   uint16 regionIndexCount
//!   uint16 regionIndexes[regionIndexCount]
//!   DeltaSet deltaSets[itemCount]
//! ```
//!
//! Each DeltaSet row logically holds `regionIndexCount` deltas: a run
//! of "word"-typed deltas (`wordDeltaCount & WORD_DELTA_COUNT_MASK` of
//! them, which must be <= `regionIndexCount`) followed by short-typed
//! deltas. Without the `LONG_WORDS` flag the word/short types are
//! `int16`/`int8` (row length `regionIndexCount + wordCount` bytes);
//! with it they are `int32`/`int16` (twice that length). Per the
//! chapter, the flag "should only be used in top-level tables that
//! include 32-bit values that can be variable — currently, only the
//! COLR table."
//!
//! A NULL offset in `itemVariationDataOffsets` means there is no
//! subtable for that outer index: items associated with it have no
//! variation, and any inner index under it is ignored.
//!
//! The interpolation (§7.2.3.3, §7.1.7) selects a delta-set row by an
//! outer (subtable) + inner (row) index pair, computes a per-region
//! scalar for the active instance, and sums `scalar * delta` across the
//! subtable's regions.

use crate::cff2::varstore::{RegionAxisCoordinates, VariationRegion};
use crate::parser::{read_f2dot14, read_u16, read_u32, read_u8};
use crate::Error;

/// `wordDeltaCount` bit 15: word deltas are `int32` (and short deltas
/// `int16`) instead of `int16`/`int8`.
pub const IVS_LONG_WORDS: u16 = 0x8000;
/// `wordDeltaCount` mask for the count of word-typed deltas at the
/// start of each DeltaSet row.
pub const IVS_WORD_DELTA_COUNT_MASK: u16 = 0x7FFF;

/// One delta-set-storing `ItemVariationData` subtable.
#[derive(Debug, Clone)]
pub struct ItemVariationData {
    /// `regionIndexes` — indices into the store's region list, in column
    /// order of the delta sets.
    region_indexes: Vec<u16>,
    /// Decoded delta-set rows. `delta_sets[row][col]` is the delta for
    /// `region_indexes[col]`. Rows = `itemCount`.
    delta_sets: Vec<Vec<i32>>,
    /// Whether the subtable used the `LONG_WORDS` (int32/int16) delta
    /// representation.
    long_words: bool,
}

impl ItemVariationData {
    fn parse(bytes: &[u8], off: usize, region_total: usize) -> Result<Self, Error> {
        let item_count = read_u16(bytes, off)? as usize;
        // Packed field: 0x8000 LONG_WORDS flag + 0x7FFF word count.
        let packed = read_u16(bytes, off + 2)?;
        let long_words = packed & IVS_LONG_WORDS != 0;
        let word_delta_count = (packed & IVS_WORD_DELTA_COUNT_MASK) as usize;
        let region_index_count = read_u16(bytes, off + 4)? as usize;
        // "must be less than or equal to regionIndexCount."
        if word_delta_count > region_index_count {
            return Err(Error::BadStructure(
                "IVS: wordDeltaCount > regionIndexCount",
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
        // Row length: regionIndexCount + wordCount bytes, doubled when
        // LONG_WORDS is set (word deltas int32 + short deltas int16
        // instead of int16 + int8).
        let base_row_bytes = region_index_count + word_delta_count;
        let row_bytes = if long_words {
            base_row_bytes * 2
        } else {
            base_row_bytes
        };
        let mut cursor = off + 6 + region_index_count * 2;
        let mut delta_sets = Vec::with_capacity(item_count);
        for _ in 0..item_count {
            if cursor + row_bytes > bytes.len() {
                return Err(Error::UnexpectedEof);
            }
            let mut row = Vec::with_capacity(region_index_count);
            let mut c = cursor;
            // Word-typed deltas first, then short-typed deltas.
            for _ in 0..word_delta_count {
                if long_words {
                    row.push(read_u32(bytes, c)? as i32);
                    c += 4;
                } else {
                    row.push(read_u16(bytes, c)? as i16 as i32);
                    c += 2;
                }
            }
            for _ in word_delta_count..region_index_count {
                if long_words {
                    row.push(read_u16(bytes, c)? as i16 as i32);
                    c += 2;
                } else {
                    row.push(bytes[c] as i8 as i32);
                    c += 1;
                }
            }
            delta_sets.push(row);
            cursor += row_bytes;
        }
        Ok(Self {
            region_indexes,
            delta_sets,
            long_words,
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

    /// Whether this subtable stores its deltas in the `LONG_WORDS`
    /// (int32/int16) representation. Only meaningful for top-level
    /// tables with variable 32-bit values (currently `COLR`).
    pub fn long_words(&self) -> bool {
        self.long_words
    }
}

/// A delta-set-storing ItemVariationStore.
#[derive(Debug, Clone)]
pub struct ItemVariationStore {
    /// `axisCount` of the region list.
    pub axis_count: u16,
    regions: Vec<VariationRegion>,
    /// `None` slots are NULL `itemVariationDataOffsets` entries: no
    /// subtable for that outer index (items under it do not vary).
    subtables: Vec<Option<ItemVariationData>>,
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
            // "A NULL offset in the array indicates that there is no
            // item variation data subtable for that index."
            if o == 0 {
                subtables.push(None);
            } else {
                subtables.push(Some(ItemVariationData::parse(ivs, o, regions.len())?));
            }
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

    /// Number of `ItemVariationData` offset-array slots (the valid
    /// range of an outer index; a slot may be a NULL offset).
    pub fn subtable_count(&self) -> usize {
        self.subtables.len()
    }

    /// Borrow a subtable by outer index. `None` for an out-of-range
    /// index **or** a NULL `itemVariationDataOffsets` entry (no
    /// variation data for that outer index).
    pub fn subtable(&self, outer: usize) -> Option<&ItemVariationData> {
        self.subtables.get(outer)?.as_ref()
    }

    /// Compute the interpolated **net adjustment** (delta) for the
    /// delta-set selected by `(outer, inner)`, given a normalized
    /// instance coordinate tuple (§7.1.7 + §7.2.3.3).
    ///
    /// Returns `0.0` if the index pair is out of range or the outer
    /// index selects a NULL subtable offset (the spec's "items
    /// associated with this index do not have any variation" rule; the
    /// "item is constant" case is likewise handled by the parent table,
    /// which simply has no index for a constant item).
    pub fn delta(&self, outer: u16, inner: u16, instance_coords: &[f32]) -> f32 {
        let Some(ivd) = self.subtable(outer as usize) else {
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

    /// Single-axis IVS with a NULL first subtable offset and a real
    /// second subtable.
    fn build_with_null_subtable() -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(&1u16.to_be_bytes()); // format
        v.extend_from_slice(&16u32.to_be_bytes()); // regionListOffset = 16
        v.extend_from_slice(&2u16.to_be_bytes()); // ivdCount
        v.extend_from_slice(&0u32.to_be_bytes()); // ivd[0] NULL
        v.extend_from_slice(&26u32.to_be_bytes()); // ivd[1] @ 26
        assert_eq!(v.len(), 16);
        // region list: axisCount 1, regionCount 1, region (0,1,1).
        v.extend_from_slice(&1u16.to_be_bytes());
        v.extend_from_slice(&1u16.to_be_bytes());
        v.extend_from_slice(&f2(0.0));
        v.extend_from_slice(&f2(1.0));
        v.extend_from_slice(&f2(1.0));
        assert_eq!(v.len(), 26);
        // IVD1: itemCount 1, wordDeltaCount 1, regionIndexCount 1.
        v.extend_from_slice(&1u16.to_be_bytes());
        v.extend_from_slice(&1u16.to_be_bytes());
        v.extend_from_slice(&1u16.to_be_bytes());
        v.extend_from_slice(&0u16.to_be_bytes()); // regionIndex 0
        v.extend_from_slice(&7i16.to_be_bytes()); // delta 7
        v
    }

    #[test]
    fn null_subtable_offset_means_no_variation() {
        let ivs = ItemVariationStore::parse(&build_with_null_subtable()).unwrap();
        assert_eq!(ivs.subtable_count(), 2);
        // Outer index 0 selects the NULL slot: no subtable, delta 0 for
        // any inner index.
        assert!(ivs.subtable(0).is_none());
        assert_eq!(ivs.delta(0, 0, &[1.0]), 0.0);
        assert_eq!(ivs.delta(0, 3, &[1.0]), 0.0);
        // Outer index 1 is the real subtable.
        assert_eq!(ivs.subtable(1).unwrap().item_count(), 1);
        assert!((ivs.delta(1, 0, &[1.0]) - 7.0).abs() < 1e-5);
    }

    /// Single-axis IVS whose one subtable uses the LONG_WORDS
    /// representation: 2 regions, 1 word (int32) delta + 1 short
    /// (int16) delta per row.
    fn build_long_words() -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(&1u16.to_be_bytes()); // format
        v.extend_from_slice(&12u32.to_be_bytes()); // regionListOffset = 12
        v.extend_from_slice(&1u16.to_be_bytes()); // ivdCount
        v.extend_from_slice(&28u32.to_be_bytes()); // ivd[0] @ 28
        assert_eq!(v.len(), 12);
        // region list: axisCount 1, regionCount 2.
        v.extend_from_slice(&1u16.to_be_bytes());
        v.extend_from_slice(&2u16.to_be_bytes());
        v.extend_from_slice(&f2(0.0));
        v.extend_from_slice(&f2(1.0));
        v.extend_from_slice(&f2(1.0));
        v.extend_from_slice(&f2(-1.0));
        v.extend_from_slice(&f2(-1.0));
        v.extend_from_slice(&f2(0.0));
        assert_eq!(v.len(), 28);
        // IVD0: itemCount 2, wordDeltaCount LONG_WORDS | 1,
        // regionIndexCount 2. Row bytes = (2 + 1) * 2 = 6.
        v.extend_from_slice(&2u16.to_be_bytes());
        v.extend_from_slice(&(IVS_LONG_WORDS | 1).to_be_bytes());
        v.extend_from_slice(&2u16.to_be_bytes());
        v.extend_from_slice(&0u16.to_be_bytes()); // regionIndex 0
        v.extend_from_slice(&1u16.to_be_bytes()); // regionIndex 1
                                                  // Row 0: int32 = 131072 (2.0 in Fixed 1/65536ths), int16 = -300.
        v.extend_from_slice(&131_072i32.to_be_bytes());
        v.extend_from_slice(&(-300i16).to_be_bytes());
        // Row 1: int32 = -70000 (outside int16 range), int16 = 25.
        v.extend_from_slice(&(-70_000i32).to_be_bytes());
        v.extend_from_slice(&25i16.to_be_bytes());
        v
    }

    #[test]
    fn long_words_rows_decode_int32_and_int16() {
        let ivs = ItemVariationStore::parse(&build_long_words()).unwrap();
        let ivd = ivs.subtable(0).unwrap();
        assert!(ivd.long_words());
        assert_eq!(ivd.item_count(), 2);
        // Region 0 active at +1.0 (scalar 1), region 1 inactive.
        assert!((ivs.delta(0, 0, &[1.0]) - 131_072.0).abs() < 1e-3);
        assert!((ivs.delta(0, 1, &[1.0]) - (-70_000.0)).abs() < 1e-3);
        // Region 1 active at -1.0: the int16 short deltas.
        assert!((ivs.delta(0, 0, &[-1.0]) - (-300.0)).abs() < 1e-5);
        assert!((ivs.delta(0, 1, &[-1.0]) - 25.0).abs() < 1e-5);
    }

    #[test]
    fn rejects_word_count_above_region_count() {
        // wordDeltaCount (3) > regionIndexCount (2) must be rejected,
        // with and without LONG_WORDS.
        for flag in [0u16, IVS_LONG_WORDS] {
            let mut v = build_long_words();
            v[30..32].copy_from_slice(&(flag | 3).to_be_bytes());
            assert!(matches!(
                ItemVariationStore::parse(&v),
                Err(Error::BadStructure(_))
            ));
        }
    }

    #[test]
    fn long_words_row_truncation_is_eof() {
        // Chop the last row short: the doubled row length must be
        // enforced.
        let v = build_long_words();
        let v = &v[..v.len() - 1];
        assert!(matches!(
            ItemVariationStore::parse(v),
            Err(Error::UnexpectedEof)
        ));
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
