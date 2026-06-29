//! `HVAR` / `VVAR` — horizontal/vertical metrics variations (ISO/IEC
//! 14496-22:2019 §7.3.5, §7.3.8).
//!
//! Both tables share the same shape: a major/minor version, an
//! `Offset32` to an ItemVariationStore, and a set of optional
//! `Offset32`s to `DeltaSetIndexMap` tables — one per metric the table
//! varies. `HVAR` carries advance-width + left/right-side-bearing maps;
//! `VVAR` carries advance-height + top/bottom-side-bearing + vertical-
//! origin maps. The advance map is optional: when absent, the glyph ID
//! is used as the inner delta-set index with outer index 0.
//!
//! Layout (§7.3.5.2 / §7.3.8.2):
//!
//! ```text
//! HVAR/VVAR header
//!   uint16   majorVersion = 1
//!   uint16   minorVersion = 0
//!   Offset32 itemVariationStoreOffset
//!   Offset32 advanceMappingOffset       (may be NULL)
//!   Offset32 sb1MappingOffset           (lsb / tsb; may be NULL)
//!   Offset32 sb2MappingOffset           (rsb / bsb; may be NULL)
//!   Offset32 vorgMappingOffset          (VVAR only; may be NULL)
//! ```
//!
//! `advance(glyph, &normalized)` resolves the per-instance advance
//! adjustment; the side-bearing / vertical-origin accessors do the same
//! for their respective maps when present.

use crate::parser::{read_u16, read_u32};
use crate::tables::ivs::{DeltaSetIndexMap, ItemVariationStore};
use crate::Error;

/// A parsed `HVAR` or `VVAR` table.
#[derive(Debug, Clone)]
pub struct MetricsVariations {
    store: ItemVariationStore,
    advance_map: Option<DeltaSetIndexMap>,
    sb1_map: Option<DeltaSetIndexMap>,
    sb2_map: Option<DeltaSetIndexMap>,
    vorg_map: Option<DeltaSetIndexMap>,
}

impl MetricsVariations {
    /// Parse an `HVAR` table (advance-width + lsb + rsb maps).
    pub fn parse_hvar(bytes: &[u8]) -> Result<Self, Error> {
        Self::parse(bytes, false)
    }

    /// Parse a `VVAR` table (advance-height + tsb + bsb + vorg maps).
    pub fn parse_vvar(bytes: &[u8]) -> Result<Self, Error> {
        Self::parse(bytes, true)
    }

    fn parse(bytes: &[u8], has_vorg: bool) -> Result<Self, Error> {
        let min_len = if has_vorg { 24 } else { 20 };
        if bytes.len() < min_len {
            return Err(Error::UnexpectedEof);
        }
        let major = read_u16(bytes, 0)?;
        if major != 1 {
            return Err(Error::BadStructure("HVAR/VVAR: unsupported majorVersion"));
        }
        let ivs_offset = read_u32(bytes, 4)? as usize;
        let advance_off = read_u32(bytes, 8)? as usize;
        let sb1_off = read_u32(bytes, 12)? as usize;
        let sb2_off = read_u32(bytes, 16)? as usize;
        let vorg_off = if has_vorg {
            read_u32(bytes, 20)? as usize
        } else {
            0
        };

        let store = ItemVariationStore::parse_at(bytes, ivs_offset)?;
        let map = |off: usize| -> Result<Option<DeltaSetIndexMap>, Error> {
            if off == 0 {
                Ok(None)
            } else {
                Ok(Some(DeltaSetIndexMap::parse_at(bytes, off)?))
            }
        };
        Ok(Self {
            store,
            advance_map: map(advance_off)?,
            sb1_map: map(sb1_off)?,
            sb2_map: map(sb2_off)?,
            vorg_map: map(vorg_off)?,
        })
    }

    /// The embedded ItemVariationStore.
    pub fn store(&self) -> &ItemVariationStore {
        &self.store
    }

    /// The per-instance **advance** adjustment for a glyph (advance width
    /// for `HVAR`, advance height for `VVAR`), given normalized instance
    /// coordinates. When no advance mapping table is present, the glyph
    /// ID is the inner index with outer index 0 (§7.3.5.3).
    pub fn advance(&self, glyph_id: u16, instance_coords: &[f32]) -> f32 {
        let (outer, inner) = match &self.advance_map {
            Some(m) => m.index(glyph_id),
            None => (0, glyph_id),
        };
        self.store.delta(outer, inner, instance_coords)
    }

    /// The per-instance left-side-bearing (`HVAR`) / top-side-bearing
    /// (`VVAR`) adjustment, or `None` when no such mapping table is
    /// present.
    pub fn side_bearing_1(&self, glyph_id: u16, instance_coords: &[f32]) -> Option<f32> {
        self.sb1_map.as_ref().map(|m| {
            let (o, i) = m.index(glyph_id);
            self.store.delta(o, i, instance_coords)
        })
    }

    /// The per-instance right-side-bearing (`HVAR`) / bottom-side-bearing
    /// (`VVAR`) adjustment, or `None` when no such mapping table is
    /// present.
    pub fn side_bearing_2(&self, glyph_id: u16, instance_coords: &[f32]) -> Option<f32> {
        self.sb2_map.as_ref().map(|m| {
            let (o, i) = m.index(glyph_id);
            self.store.delta(o, i, instance_coords)
        })
    }

    /// The per-instance vertical-origin adjustment (`VVAR` only), or
    /// `None` when no vertical-origin mapping is present.
    pub fn vertical_origin(&self, glyph_id: u16, instance_coords: &[f32]) -> Option<f32> {
        self.vorg_map.as_ref().map(|m| {
            let (o, i) = m.index(glyph_id);
            self.store.delta(o, i, instance_coords)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f2(v: f32) -> [u8; 2] {
        ((v * 16384.0).round() as i16).to_be_bytes()
    }

    /// Build an HVAR with an IVS (1 region weight 0/1/1, 1 subtable with
    /// 3 rows of one int16 delta each) and an advance map of 2 entries.
    fn build_hvar() -> Vec<u8> {
        // IVS body.
        let mut ivs = Vec::new();
        ivs.extend_from_slice(&1u16.to_be_bytes()); // format
        ivs.extend_from_slice(&12u32.to_be_bytes()); // regionListOffset
        ivs.extend_from_slice(&1u16.to_be_bytes()); // ivdCount
        ivs.extend_from_slice(&22u32.to_be_bytes()); // ivd[0] @ 22
        ivs.extend_from_slice(&1u16.to_be_bytes()); // axisCount
        ivs.extend_from_slice(&1u16.to_be_bytes()); // regionCount
        ivs.extend_from_slice(&f2(0.0));
        ivs.extend_from_slice(&f2(1.0));
        ivs.extend_from_slice(&f2(1.0));
        // ivd @ 22: itemCount 3, shortDeltaCount 1, regionIndexCount 1.
        ivs.extend_from_slice(&3u16.to_be_bytes());
        ivs.extend_from_slice(&1u16.to_be_bytes());
        ivs.extend_from_slice(&1u16.to_be_bytes());
        ivs.extend_from_slice(&0u16.to_be_bytes()); // regionIndex 0
        ivs.extend_from_slice(&10i16.to_be_bytes()); // row0
        ivs.extend_from_slice(&20i16.to_be_bytes()); // row1
        ivs.extend_from_slice(&30i16.to_be_bytes()); // row2

        // advance map: entryFormat inner=4 bits, size=1 byte; 2 entries.
        // glyph0 -> (0,2)=0x02, glyph1 -> (0,0)=0x00.
        let mut amap = Vec::new();
        amap.extend_from_slice(&0x0003u16.to_be_bytes()); // inner=4 (low nibble 3), size=1
        amap.extend_from_slice(&2u16.to_be_bytes()); // mapCount
        amap.push(0x02); // glyph0 -> inner 2
        amap.push(0x00); // glyph1 -> inner 0

        // Header: version + 4 Offset32 (ivs, advance, sb1=0, sb2=0).
        let header_len = 20;
        let ivs_off = header_len;
        let amap_off = ivs_off + ivs.len();
        let mut b = Vec::new();
        b.extend_from_slice(&1u16.to_be_bytes()); // major
        b.extend_from_slice(&0u16.to_be_bytes()); // minor
        b.extend_from_slice(&(ivs_off as u32).to_be_bytes());
        b.extend_from_slice(&(amap_off as u32).to_be_bytes());
        b.extend_from_slice(&0u32.to_be_bytes()); // lsb map NULL
        b.extend_from_slice(&0u32.to_be_bytes()); // rsb map NULL
        assert_eq!(b.len(), header_len);
        b.extend_from_slice(&ivs);
        b.extend_from_slice(&amap);
        b
    }

    #[test]
    fn hvar_advance_with_mapping() {
        let h = MetricsVariations::parse_hvar(&build_hvar()).unwrap();
        // At normalized weight +1.0: region scalar 1.0.
        // glyph0 -> inner 2 -> row2 = 30.
        assert!((h.advance(0, &[1.0]) - 30.0).abs() < 1e-5);
        // glyph1 -> inner 0 -> row0 = 10.
        assert!((h.advance(1, &[1.0]) - 10.0).abs() < 1e-5);
        // glyph past mapCount clamps to last entry (glyph1 -> 10).
        assert!((h.advance(99, &[1.0]) - 10.0).abs() < 1e-5);
        // At +0.5: scaled.
        assert!((h.advance(0, &[0.5]) - 15.0).abs() < 1e-5);
        // No side-bearing maps.
        assert!(h.side_bearing_1(0, &[1.0]).is_none());
        assert!(h.side_bearing_2(0, &[1.0]).is_none());
    }

    #[test]
    fn hvar_advance_implicit_index_without_mapping() {
        // Strip the advance map (set its offset to 0) and verify the
        // glyph ID becomes the inner index directly.
        let mut b = build_hvar();
        // advanceMappingOffset is bytes 8..12.
        b[8..12].copy_from_slice(&0u32.to_be_bytes());
        let h = MetricsVariations::parse_hvar(&b).unwrap();
        // glyph0 -> (0, 0) -> row0 = 10; glyph2 -> (0,2) -> row2 = 30.
        assert!((h.advance(0, &[1.0]) - 10.0).abs() < 1e-5);
        assert!((h.advance(2, &[1.0]) - 30.0).abs() < 1e-5);
    }

    #[test]
    fn rejects_bad_version() {
        let mut b = vec![0u8; 20];
        b[0..2].copy_from_slice(&2u16.to_be_bytes());
        assert!(matches!(
            MetricsVariations::parse_hvar(&b),
            Err(Error::BadStructure(_))
        ));
    }
}
