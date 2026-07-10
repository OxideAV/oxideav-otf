//! `avar` — axis variations (ISO/IEC 14496-22:2019 §7.3.1).
//!
//! The optional `avar` table refines the default coordinate
//! normalization performed by `fvar`. After `fvar` maps a user-scale
//! axis value to the `[-1, 1]` normalized scale (default→0, min→-1,
//! max→1, linear between), `avar` applies a per-axis piecewise-linear
//! **segment map** that further warps the normalized value, making the
//! variation along an axis less linear.
//!
//! On-disk layout (§7.3.1.2):
//!
//! ```text
//! avar header
//!   uint16 majorVersion = 1
//!   uint16 minorVersion = 0
//!   uint16 (reserved)   = 0
//!   uint16 axisCount        // must equal fvar.axisCount
//!   SegmentMaps axisSegmentMaps[axisCount]
//!
//! SegmentMaps
//!   uint16 positionMapCount
//!   AxisValueMap axisValueMaps[positionMapCount]
//!
//! AxisValueMap
//!   F2DOT14 fromCoordinate    // default-normalized input
//!   F2DOT14 toCoordinate      // modified-normalized output
//! ```
//!
//! Processing (§7.3.1.3): for a default-normalized value, find the first
//! map record whose `fromCoordinate >= value` (`endSeg`). If it equals
//! the value, return its `toCoordinate`; otherwise interpolate linearly
//! between the preceding record (`startSeg`) and `endSeg`. A segment map
//! with no records (or one missing the required -1/0/+1 anchors) is the
//! identity.
//!
//! **Version 2** appends two offsets after the segment-maps array (whose
//! count may then be zero):
//!
//! ```text
//! Offset32To<DeltaSetIndexMap>   axisIndexMapOffset
//! Offset32To<ItemVariationStore> varStoreOffset
//! ```
//!
//! enabling **cross-axis** remapping: after the v1 segment-map stage,
//! a per-axis delta is interpolated from `varStore` using the
//! intermediate coordinates themselves (the delta-set for axis *i* is
//! found through `axisIndexMap`, or is *i* directly when the map is
//! absent), added to the axis coordinate in F2DOT14 integer units
//! (deltas are stored as true value × 16384), and clamped to `[-1, 1]`.
//! The resulting coordinates drive all downstream variation data.

use crate::parser::{read_f2dot14, read_u16, read_u32};
use crate::tables::ivs::{DeltaSetIndexMap, ItemVariationStore};
use crate::Error;

/// One `(fromCoordinate, toCoordinate)` correspondence in a segment map.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AxisValueMap {
    /// `fromCoordinate` — a default-normalized input value in `[-1, 1]`.
    pub from: f32,
    /// `toCoordinate` — the modified-normalized output value.
    pub to: f32,
}

/// A per-axis piecewise-linear segment map.
#[derive(Debug, Clone, PartialEq)]
pub struct SegmentMap {
    maps: Vec<AxisValueMap>,
}

impl SegmentMap {
    /// Apply the segment map to a default-normalized value, returning the
    /// modified-normalized value (§7.3.1.3). An empty map, or one
    /// lacking enough records to bracket the value, returns the input
    /// unchanged (the identity map).
    pub fn apply(&self, value: f32) -> f32 {
        // An axis with no value maps (or fewer than the required three
        // anchor maps) is not modified. The spec says the -1/0/+1
        // anchors are required for any modification to take effect; we
        // treat a too-short map as the identity.
        if self.maps.len() < 3 {
            return value;
        }
        // Find endSeg: the first record whose fromCoordinate >= value.
        for (i, m) in self.maps.iter().enumerate() {
            if m.from >= value {
                if m.from == value || i == 0 {
                    // Exact hit, or value below the first anchor: the
                    // first record (for -1) cannot be an endSeg with a
                    // preceding startSeg, so use its toCoordinate.
                    return m.to;
                }
                let start = self.maps[i - 1];
                let denom = m.from - start.from;
                if denom == 0.0 {
                    return m.to;
                }
                return start.to + (m.to - start.to) * (value - start.from) / denom;
            }
        }
        // value is above the last anchor: clamp to the last toCoordinate.
        self.maps.last().map(|m| m.to).unwrap_or(value)
    }

    /// The raw value-map records.
    pub fn maps(&self) -> &[AxisValueMap] {
        &self.maps
    }
}

/// A parsed `avar` table — one segment map per axis, plus (version 2)
/// the cross-axis delta mapping.
///
/// Version 2 appends two offsets to the version-1 layout: an
/// `axisIndexMap` (`DeltaSetIndexMap`) mapping `fvar` axis indices to
/// delta-set indices, and a `varStore` (`ItemVariationStore`) whose
/// interpolated deltas — computed from the *segment-mapped*
/// intermediate coordinates — are added to the axis coordinates and
/// clamped to `[-1, 1]`. This enables designspace warping, axis
/// cloning (higher-order interpolation), and parametric-axis fonts.
#[derive(Debug, Clone)]
pub struct AvarTable {
    major_version: u16,
    segment_maps: Vec<SegmentMap>,
    /// v2 `axisIndexMap`: `fvar` axis index → delta-set index. When
    /// absent, the axis index itself is the delta-set index (high 16
    /// bits outer, low 16 inner).
    axis_index_map: Option<DeltaSetIndexMap>,
    /// v2 `varStore`: per-axis interpolated coordinate deltas, stored
    /// in F2DOT14 integer units (16384 = +1.0).
    var_store: Option<ItemVariationStore>,
}

impl AvarTable {
    pub fn parse(bytes: &[u8]) -> Result<Self, Error> {
        if bytes.len() < 8 {
            return Err(Error::UnexpectedEof);
        }
        let major = read_u16(bytes, 0)?;
        if major != 1 && major != 2 {
            return Err(Error::BadStructure("avar: unsupported majorVersion"));
        }
        // v2 names this field axisSegmentMapCount: it may be 0 (no v1
        // segment maps at all), otherwise it must equal fvar.axisCount.
        let axis_count = read_u16(bytes, 6)? as usize;
        let mut segment_maps = Vec::with_capacity(axis_count.min(bytes.len() / 2));
        let mut off = 8usize;
        for _ in 0..axis_count {
            if off + 2 > bytes.len() {
                return Err(Error::UnexpectedEof);
            }
            let n = read_u16(bytes, off)? as usize;
            off += 2;
            let mut maps = Vec::with_capacity(n.min(bytes.len() / 4));
            for _ in 0..n {
                if off + 4 > bytes.len() {
                    return Err(Error::UnexpectedEof);
                }
                maps.push(AxisValueMap {
                    from: read_f2dot14(bytes, off)?,
                    to: read_f2dot14(bytes, off + 2)?,
                });
                off += 4;
            }
            segment_maps.push(SegmentMap { maps });
        }
        let (axis_index_map, var_store) = if major >= 2 {
            let axis_index_map_offset = read_u32(bytes, off)? as usize;
            let var_store_offset = read_u32(bytes, off + 4)? as usize;
            let map = if axis_index_map_offset != 0 {
                Some(DeltaSetIndexMap::parse_at(bytes, axis_index_map_offset)?)
            } else {
                None
            };
            let store = if var_store_offset != 0 {
                Some(ItemVariationStore::parse_at(bytes, var_store_offset)?)
            } else {
                None
            };
            (map, store)
        } else {
            (None, None)
        };
        Ok(Self {
            major_version: major,
            segment_maps,
            axis_index_map,
            var_store,
        })
    }

    /// The table's `majorVersion` (1 or 2).
    pub fn major_version(&self) -> u16 {
        self.major_version
    }

    /// The per-axis segment maps, in axis order.
    pub fn segment_maps(&self) -> &[SegmentMap] {
        &self.segment_maps
    }

    /// Number of segment maps (`axisCount` in v1, `axisSegmentMapCount`
    /// in v2 — a v2 table may carry zero segment maps and still remap
    /// axes through its delta store).
    pub fn axis_count(&self) -> usize {
        self.segment_maps.len()
    }

    /// The v2 `axisIndexMap`, when present.
    pub fn axis_index_map(&self) -> Option<&DeltaSetIndexMap> {
        self.axis_index_map.as_ref()
    }

    /// The v2 `varStore`, when present.
    pub fn var_store(&self) -> Option<&ItemVariationStore> {
        self.var_store.as_ref()
    }

    /// Apply the table to a default-normalized coordinate tuple.
    ///
    /// Stage 1 (v1): each axis's coordinate runs through its segment
    /// map; axes beyond the table's segment-map count (or with no
    /// usable map) pass through unchanged. Stage 2 (v2 only): using
    /// the stage-1 *intermediate* coordinates, a per-axis delta is
    /// interpolated from the `varStore` (axis → delta-set via
    /// `axisIndexMap`, or the axis index directly when absent), added
    /// to the coordinate in F2DOT14 integer units, and the result is
    /// clamped to `[-1, 1]`.
    pub fn apply(&self, normalized: &[f32]) -> Vec<f32> {
        let mut out: Vec<f32> = normalized
            .iter()
            .enumerate()
            .map(|(i, &v)| self.segment_maps.get(i).map(|sm| sm.apply(v)).unwrap_or(v))
            .collect();
        let Some(store) = self.var_store.as_ref() else {
            return out;
        };
        // Every axis's delta is interpolated against the same
        // intermediate coordinate tuple (deltas do not cascade), so
        // snapshot before adjusting.
        let intermediate = out.clone();
        for (i, v) in out.iter_mut().enumerate() {
            let var_idx = i as u32;
            let (outer, inner) = match self.axis_index_map.as_ref() {
                Some(map) => map.index_u32(var_idx),
                None => ((var_idx >> 16) as u16, (var_idx & 0xFFFF) as u16),
            };
            if outer == 0xFFFF && inner == 0xFFFF {
                // "No variation data" mapping.
                continue;
            }
            let delta = store.delta(outer, inner, &intermediate);
            // The reference algorithm works in F2DOT14 integer units:
            // round the interpolated delta, add, clamp to ±16384
            // (±1.0), then convert back.
            let v_f2 = (*v * 16384.0).round() + delta.round();
            *v = v_f2.clamp(-16384.0, 16384.0) / 16384.0;
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f2(v: f32) -> [u8; 2] {
        ((v * 16384.0).round() as i16).to_be_bytes()
    }

    /// Build a one-axis avar with the spec §7.3.1.4 example mapping.
    fn build_example() -> Vec<u8> {
        let maps: &[(f32, f32)] = &[
            (-1.0, -1.0),
            (-0.75, -0.5),
            (0.0, 0.0),
            (0.4, 0.4),
            (0.6, 0.9),
            (1.0, 1.0),
        ];
        let mut b = Vec::new();
        b.extend_from_slice(&1u16.to_be_bytes()); // major
        b.extend_from_slice(&0u16.to_be_bytes()); // minor
        b.extend_from_slice(&0u16.to_be_bytes()); // reserved
        b.extend_from_slice(&1u16.to_be_bytes()); // axisCount
        b.extend_from_slice(&(maps.len() as u16).to_be_bytes());
        for (from, to) in maps {
            b.extend_from_slice(&f2(*from));
            b.extend_from_slice(&f2(*to));
        }
        b
    }

    #[test]
    fn spec_example_table() {
        // §7.3.1.4 example: verify several default→final mappings.
        let a = AvarTable::parse(&build_example()).unwrap();
        let sm = &a.segment_maps()[0];
        let approx = |x: f32, y: f32| (x - y).abs() < 0.01;
        assert!(approx(sm.apply(-1.0), -1.0));
        assert!(approx(sm.apply(-0.75), -0.5));
        assert!(approx(sm.apply(-0.5), -0.3333));
        assert!(approx(sm.apply(-0.25), -0.1667));
        assert!(approx(sm.apply(0.0), 0.0));
        assert!(approx(sm.apply(0.25), 0.25));
        assert!(approx(sm.apply(0.5), 0.65));
        assert!(approx(sm.apply(0.75), 0.9375));
        assert!(approx(sm.apply(1.0), 1.0));
    }

    #[test]
    fn empty_map_is_identity() {
        let mut b = Vec::new();
        b.extend_from_slice(&1u16.to_be_bytes());
        b.extend_from_slice(&0u16.to_be_bytes());
        b.extend_from_slice(&0u16.to_be_bytes());
        b.extend_from_slice(&1u16.to_be_bytes()); // axisCount = 1
        b.extend_from_slice(&0u16.to_be_bytes()); // positionMapCount = 0
        let a = AvarTable::parse(&b).unwrap();
        assert_eq!(a.apply(&[0.37]), vec![0.37]);
    }

    #[test]
    fn apply_multi_axis_passthrough() {
        // One-axis table applied to a two-axis tuple: axis 1 passes
        // through unchanged.
        let a = AvarTable::parse(&build_example()).unwrap();
        let out = a.apply(&[0.5, -0.5]);
        assert!((out[0] - 0.65).abs() < 0.01);
        assert_eq!(out[1], -0.5);
    }

    #[test]
    fn rejects_bad_version() {
        let mut b = vec![0u8; 8];
        b[0..2].copy_from_slice(&3u16.to_be_bytes());
        assert!(matches!(AvarTable::parse(&b), Err(Error::BadStructure(_))));
    }

    // ---- version 2 ----------------------------------------------------------

    /// A two-axis `ItemVariationStore`: one region peaking on axis 0
    /// (start 0, peak 1, end 1; axis 1 ignored), one `ItemVariationData`
    /// with the given int16 delta rows.
    fn v2_store(rows: &[i16]) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(&1u16.to_be_bytes()); // format
        v.extend_from_slice(&12u32.to_be_bytes()); // regionListOffset
        v.extend_from_slice(&1u16.to_be_bytes()); // ivdCount
        v.extend_from_slice(&28u32.to_be_bytes()); // ivd[0]
        assert_eq!(v.len(), 12);
        v.extend_from_slice(&2u16.to_be_bytes()); // axisCount
        v.extend_from_slice(&1u16.to_be_bytes()); // regionCount
        v.extend_from_slice(&f2(0.0)); // axis0 start
        v.extend_from_slice(&f2(1.0)); // axis0 peak
        v.extend_from_slice(&f2(1.0)); // axis0 end
        v.extend_from_slice(&f2(0.0)); // axis1 start (peak 0 = ignored)
        v.extend_from_slice(&f2(0.0));
        v.extend_from_slice(&f2(0.0));
        assert_eq!(v.len(), 28);
        v.extend_from_slice(&(rows.len() as u16).to_be_bytes()); // itemCount
        v.extend_from_slice(&1u16.to_be_bytes()); // shortDeltaCount
        v.extend_from_slice(&1u16.to_be_bytes()); // regionIndexCount
        v.extend_from_slice(&0u16.to_be_bytes()); // regionIndex 0
        for &d in rows {
            v.extend_from_slice(&d.to_be_bytes());
        }
        v
    }

    /// Build a two-axis avar v2 table: identity segment maps, an
    /// optional axisIndexMap blob, and a varStore blob.
    fn build_v2(axis_index_map: Option<&[u8]>, store: &[u8]) -> Vec<u8> {
        let mut b = Vec::new();
        b.extend_from_slice(&2u16.to_be_bytes()); // major
        b.extend_from_slice(&0u16.to_be_bytes()); // minor
        b.extend_from_slice(&0u16.to_be_bytes()); // reserved
        b.extend_from_slice(&2u16.to_be_bytes()); // axisSegmentMapCount
        for _ in 0..2 {
            // identity segment map: -1/-1, 0/0, +1/+1.
            b.extend_from_slice(&3u16.to_be_bytes());
            for v in [-1.0f32, 0.0, 1.0] {
                b.extend_from_slice(&f2(v));
                b.extend_from_slice(&f2(v));
            }
        }
        let header_end = b.len() + 8;
        let map_at = if axis_index_map.is_some() {
            header_end
        } else {
            0
        };
        let store_at = header_end + axis_index_map.map_or(0, |m| m.len());
        b.extend_from_slice(&(map_at as u32).to_be_bytes());
        b.extend_from_slice(&(store_at as u32).to_be_bytes());
        if let Some(m) = axis_index_map {
            b.extend_from_slice(m);
        }
        b.extend_from_slice(store);
        b
    }

    #[test]
    fn v2_identity_axis_mapping_deltas() {
        // No axisIndexMap: axis i uses delta-set (0, i). Row 0 shifts
        // axis 0 by -0.5 at full coordinate; row 1 shifts axis 1 by
        // +axis0-coordinate (cross-axis remap).
        let b = build_v2(None, &v2_store(&[-8192, 16384]));
        let a = AvarTable::parse(&b).unwrap();
        assert_eq!(a.major_version(), 2);
        assert!(a.axis_index_map().is_none());
        assert!(a.var_store().is_some());

        let out = a.apply(&[1.0, 0.0]);
        assert!((out[0] - 0.5).abs() < 1e-4, "{out:?}");
        assert!((out[1] - 1.0).abs() < 1e-4, "{out:?}");
        // At half coordinate the region scalar halves both deltas.
        let out = a.apply(&[0.5, 0.0]);
        assert!((out[0] - 0.25).abs() < 1e-4, "{out:?}");
        assert!((out[1] - 0.5).abs() < 1e-4, "{out:?}");
        // At the default instance nothing moves.
        assert_eq!(a.apply(&[0.0, 0.0]), vec![0.0, 0.0]);
    }

    #[test]
    fn v2_axis_index_map_and_clamp() {
        // axisIndexMap (format 0, 4-byte entries, 16 inner bits):
        // axis 0 → 0xFFFFFFFF (no variation data), axis 1 → (0, 0).
        let mut m = vec![0u8, 0x3F];
        m.extend_from_slice(&2u16.to_be_bytes());
        m.extend_from_slice(&0xFFFF_FFFFu32.to_be_bytes());
        m.extend_from_slice(&0u32.to_be_bytes());
        // Row 0: +1.0 delta driven by axis 0's coordinate.
        let b = build_v2(Some(&m), &v2_store(&[16384]));
        let a = AvarTable::parse(&b).unwrap();
        assert!(a.axis_index_map().is_some());

        // Axis 0 is unmapped (sentinel) and keeps its value; axis 1
        // gains +1.0 but clamps to the [-1, 1] range.
        let out = a.apply(&[1.0, 0.75]);
        assert!((out[0] - 1.0).abs() < 1e-4, "{out:?}");
        assert!((out[1] - 1.0).abs() < 1e-4, "{out:?}");
        // Negative side clamp: drive axis 1 down past -1.
        let b2 = build_v2(Some(&m), &v2_store(&[-16384]));
        let a2 = AvarTable::parse(&b2).unwrap();
        let out = a2.apply(&[1.0, -0.75]);
        assert!((out[1] + 1.0).abs() < 1e-4, "{out:?}");
    }

    #[test]
    fn v2_segment_maps_feed_delta_stage() {
        // Axis 0's segment map warps 0.5 → 0.65 (spec example); the
        // delta stage must interpolate against the *intermediate*
        // coordinate: with row 1 = +1.0 scaled by axis 0's region
        // scalar, axis 1 becomes 0.65, not 0.5.
        let maps: &[(f32, f32)] = &[
            (-1.0, -1.0),
            (-0.75, -0.5),
            (0.0, 0.0),
            (0.4, 0.4),
            (0.6, 0.9),
            (1.0, 1.0),
        ];
        let mut b = Vec::new();
        b.extend_from_slice(&2u16.to_be_bytes());
        b.extend_from_slice(&0u16.to_be_bytes());
        b.extend_from_slice(&0u16.to_be_bytes());
        b.extend_from_slice(&2u16.to_be_bytes());
        b.extend_from_slice(&(maps.len() as u16).to_be_bytes());
        for (from, to) in maps {
            b.extend_from_slice(&f2(*from));
            b.extend_from_slice(&f2(*to));
        }
        b.extend_from_slice(&0u16.to_be_bytes()); // axis 1: no maps
        let store_at = b.len() + 8;
        b.extend_from_slice(&0u32.to_be_bytes()); // no axisIndexMap
        b.extend_from_slice(&(store_at as u32).to_be_bytes());
        b.extend_from_slice(&v2_store(&[0, 16384]));

        let a = AvarTable::parse(&b).unwrap();
        let out = a.apply(&[0.5, 0.0]);
        assert!((out[0] - 0.65).abs() < 0.01, "{out:?}");
        assert!((out[1] - 0.65).abs() < 0.01, "{out:?}");
    }

    /// Single-byte mutation + truncation robustness: every mutant
    /// must either fail to parse or apply without panicking.
    #[test]
    fn v2_mutation_robustness() {
        let mut m = vec![0u8, 0x3F];
        m.extend_from_slice(&2u16.to_be_bytes());
        m.extend_from_slice(&0xFFFF_FFFFu32.to_be_bytes());
        m.extend_from_slice(&0u32.to_be_bytes());
        let base = build_v2(Some(&m), &v2_store(&[16384, -8192]));
        let exercise = |bytes: &[u8]| {
            if let Ok(a) = AvarTable::parse(bytes) {
                let _ = a.apply(&[1.0, -0.5]);
                let _ = a.apply(&[]);
            }
        };
        for i in 0..base.len() {
            for v in [0x00u8, 0xFF, base[i].wrapping_add(1)] {
                let mut mutant = base.clone();
                mutant[i] = v;
                exercise(&mutant);
            }
        }
        for len in 0..base.len() {
            exercise(&base[..len]);
        }
    }

    #[test]
    fn v2_without_store_is_v1_behavior() {
        let b = build_v2(None, &[]);
        // varStore offset points at empty data → parse fails… build a
        // variant with a zero varStore offset instead.
        let mut b2 = b[..b.len() - 8].to_vec();
        b2.extend_from_slice(&0u32.to_be_bytes());
        b2.extend_from_slice(&0u32.to_be_bytes());
        let a = AvarTable::parse(&b2).unwrap();
        assert!(a.var_store().is_none());
        let out = a.apply(&[0.37, -0.2]);
        assert!((out[0] - 0.37).abs() < 1e-4, "{out:?}");
        assert!((out[1] + 0.2).abs() < 1e-4, "{out:?}");
    }
}
