//! `MVAR` — metrics variations (ISO/IEC 14496-22:2019 §7.3.6).
//!
//! In a variable font, font-wide metric values found in `OS/2`, `hhea`,
//! `vhea`, `post`, and `gasp` may need to vary per instance. `MVAR`
//! holds an ItemVariationStore plus an array of value records that map a
//! **four-byte value tag** (e.g. `b"hasc"` = `OS/2.sTypoAscender`) to a
//! delta-set index (outer + inner) into the store.
//!
//! Layout (§7.3.6.1):
//!
//! ```text
//! MVAR header
//!   uint16   majorVersion = 1
//!   uint16   minorVersion = 0
//!   uint16   (reserved)   = 0
//!   uint16   valueRecordSize
//!   uint16   valueRecordCount
//!   Offset16 itemVariationStoreOffset   // from start of MVAR
//!   ValueRecord valueRecords[valueRecordCount]   // sorted by tag
//!
//! ValueRecord
//!   Tag    valueTag
//!   uint16 deltaSetOuterIndex
//!   uint16 deltaSetInnerIndex
//! ```
//!
//! Processing (§7.3.6.2): to vary a target metric, look up its value tag
//! (the records are sorted, so binary-searchable). If absent, the metric
//! is constant; if present, the delta-set index drives an
//! ItemVariationStore lookup that yields the per-instance adjustment to
//! add to the base value.

use crate::parser::{read_tag, read_u16};
use crate::tables::ivs::ItemVariationStore;
use crate::Error;

/// One MVAR value record: a tag and its delta-set index.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ValueRecord {
    /// `valueTag` — the font-wide measure this record varies.
    pub tag: [u8; 4],
    /// `deltaSetOuterIndex` — selects an ItemVariationData subtable.
    pub outer_index: u16,
    /// `deltaSetInnerIndex` — selects a delta-set row.
    pub inner_index: u16,
}

/// A parsed `MVAR` table.
#[derive(Debug, Clone)]
pub struct MvarTable {
    records: Vec<ValueRecord>,
    store: Option<ItemVariationStore>,
}

impl MvarTable {
    pub fn parse(bytes: &[u8]) -> Result<Self, Error> {
        if bytes.len() < 12 {
            return Err(Error::UnexpectedEof);
        }
        let major = read_u16(bytes, 0)?;
        if major != 1 {
            return Err(Error::BadStructure("MVAR: unsupported majorVersion"));
        }
        let value_record_size = read_u16(bytes, 6)? as usize;
        let value_record_count = read_u16(bytes, 8)? as usize;
        let ivs_offset = read_u16(bytes, 10)? as usize;

        if value_record_count > 0 && value_record_size < 8 {
            return Err(Error::BadStructure("MVAR: valueRecordSize < 8"));
        }

        // Value records begin at byte 12.
        let mut records = Vec::with_capacity(value_record_count);
        for i in 0..value_record_count {
            let off = 12 + i * value_record_size;
            if off + 8 > bytes.len() {
                return Err(Error::UnexpectedEof);
            }
            records.push(ValueRecord {
                tag: read_tag(bytes, off)?,
                outer_index: read_u16(bytes, off + 4)?,
                inner_index: read_u16(bytes, off + 6)?,
            });
        }

        let store = if ivs_offset != 0 {
            Some(ItemVariationStore::parse_at(bytes, ivs_offset)?)
        } else {
            None
        };

        Ok(Self { records, store })
    }

    /// The value records, in tag order.
    pub fn records(&self) -> &[ValueRecord] {
        &self.records
    }

    /// The ItemVariationStore, if present.
    pub fn store(&self) -> Option<&ItemVariationStore> {
        self.store.as_ref()
    }

    /// Look up a value record by its tag (binary search — records are
    /// spec-sorted).
    pub fn record(&self, tag: &[u8; 4]) -> Option<&ValueRecord> {
        self.records
            .binary_search_by(|r| r.tag.cmp(tag))
            .ok()
            .map(|i| &self.records[i])
    }

    /// The per-instance metric **adjustment** (delta) for a value tag,
    /// given a normalized instance coordinate tuple. Returns `0.0` when
    /// the tag is absent (the metric is constant across the variation
    /// space) or when there is no ItemVariationStore. Add the result to
    /// the table's base value to obtain the instance value.
    pub fn metric_delta(&self, tag: &[u8; 4], instance_coords: &[f32]) -> f32 {
        let (Some(rec), Some(store)) = (self.record(tag), self.store.as_ref()) else {
            return 0.0;
        };
        store.delta(rec.outer_index, rec.inner_index, instance_coords)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f2(v: f32) -> [u8; 2] {
        ((v * 16384.0).round() as i16).to_be_bytes()
    }

    /// Build an MVAR with two value records (sorted) and an IVS that
    /// gives 'hasc' a +50 delta at normalized weight +1.0.
    fn build() -> Vec<u8> {
        // Records: 'hasc' → (0,0), 'hdsc' → (0,1). value record size 8.
        let value_record_count = 2usize;
        let records_len = value_record_count * 8;
        let ivs_offset = 12 + records_len;

        // IVS: 1 region (weight 0,1,1), 1 subtable with 2 rows × 1 region
        // (all int16). Row0 = {50}, row1 = {-30}.
        let mut ivs = Vec::new();
        ivs.extend_from_slice(&1u16.to_be_bytes()); // format
        ivs.extend_from_slice(&12u32.to_be_bytes()); // regionListOffset
        ivs.extend_from_slice(&1u16.to_be_bytes()); // ivdCount
        ivs.extend_from_slice(&22u32.to_be_bytes()); // ivd[0] @ 22
        assert_eq!(ivs.len(), 12);
        ivs.extend_from_slice(&1u16.to_be_bytes()); // axisCount
        ivs.extend_from_slice(&1u16.to_be_bytes()); // regionCount
        ivs.extend_from_slice(&f2(0.0));
        ivs.extend_from_slice(&f2(1.0));
        ivs.extend_from_slice(&f2(1.0));
        assert_eq!(ivs.len(), 22);
        ivs.extend_from_slice(&2u16.to_be_bytes()); // itemCount
        ivs.extend_from_slice(&1u16.to_be_bytes()); // shortDeltaCount
        ivs.extend_from_slice(&1u16.to_be_bytes()); // regionIndexCount
        ivs.extend_from_slice(&0u16.to_be_bytes()); // regionIndex 0
        ivs.extend_from_slice(&50i16.to_be_bytes()); // row0
        ivs.extend_from_slice(&(-30i16).to_be_bytes()); // row1

        let mut b = Vec::new();
        b.extend_from_slice(&1u16.to_be_bytes()); // major
        b.extend_from_slice(&0u16.to_be_bytes()); // minor
        b.extend_from_slice(&0u16.to_be_bytes()); // reserved
        b.extend_from_slice(&8u16.to_be_bytes()); // valueRecordSize
        b.extend_from_slice(&(value_record_count as u16).to_be_bytes());
        b.extend_from_slice(&(ivs_offset as u16).to_be_bytes());
        // records (sorted: 'hasc' < 'hdsc').
        b.extend_from_slice(b"hasc");
        b.extend_from_slice(&0u16.to_be_bytes()); // outer
        b.extend_from_slice(&0u16.to_be_bytes()); // inner
        b.extend_from_slice(b"hdsc");
        b.extend_from_slice(&0u16.to_be_bytes());
        b.extend_from_slice(&1u16.to_be_bytes());
        assert_eq!(b.len(), ivs_offset);
        b.extend_from_slice(&ivs);
        b
    }

    #[test]
    fn parses_and_looks_up_deltas() {
        let m = MvarTable::parse(&build()).unwrap();
        assert_eq!(m.records().len(), 2);
        assert_eq!(&m.record(b"hasc").unwrap().tag, b"hasc");
        assert_eq!(m.record(b"hdsc").unwrap().inner_index, 1);
        assert!(m.record(b"xxxx").is_none());

        // At normalized weight +1.0: 'hasc' (row0) = +50, 'hdsc' (row1)
        // = -30.
        assert!((m.metric_delta(b"hasc", &[1.0]) - 50.0).abs() < 1e-5);
        assert!((m.metric_delta(b"hdsc", &[1.0]) - (-30.0)).abs() < 1e-5);
        // At +0.5: scaled by 0.5.
        assert!((m.metric_delta(b"hasc", &[0.5]) - 25.0).abs() < 1e-5);
        // Absent tag → 0.
        assert_eq!(m.metric_delta(b"zzzz", &[1.0]), 0.0);
    }

    #[test]
    fn empty_records_no_store() {
        let mut b = vec![0u8; 12];
        b[0..2].copy_from_slice(&1u16.to_be_bytes());
        // valueRecordCount 0, ivs offset 0.
        let m = MvarTable::parse(&b).unwrap();
        assert_eq!(m.records().len(), 0);
        assert!(m.store().is_none());
        assert_eq!(m.metric_delta(b"hasc", &[1.0]), 0.0);
    }

    #[test]
    fn rejects_bad_version() {
        let mut b = vec![0u8; 12];
        b[0..2].copy_from_slice(&2u16.to_be_bytes());
        assert!(matches!(MvarTable::parse(&b), Err(Error::BadStructure(_))));
    }
}
