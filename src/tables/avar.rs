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

use crate::parser::{read_f2dot14, read_u16};
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

/// A parsed `avar` table — one segment map per axis.
#[derive(Debug, Clone)]
pub struct AvarTable {
    segment_maps: Vec<SegmentMap>,
}

impl AvarTable {
    pub fn parse(bytes: &[u8]) -> Result<Self, Error> {
        if bytes.len() < 8 {
            return Err(Error::UnexpectedEof);
        }
        let major = read_u16(bytes, 0)?;
        if major != 1 {
            return Err(Error::BadStructure("avar: unsupported majorVersion"));
        }
        let axis_count = read_u16(bytes, 6)? as usize;
        let mut segment_maps = Vec::with_capacity(axis_count);
        let mut off = 8usize;
        for _ in 0..axis_count {
            if off + 2 > bytes.len() {
                return Err(Error::UnexpectedEof);
            }
            let n = read_u16(bytes, off)? as usize;
            off += 2;
            let mut maps = Vec::with_capacity(n);
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
        Ok(Self { segment_maps })
    }

    /// The per-axis segment maps, in axis order.
    pub fn segment_maps(&self) -> &[SegmentMap] {
        &self.segment_maps
    }

    /// Number of axes (`axisCount`).
    pub fn axis_count(&self) -> usize {
        self.segment_maps.len()
    }

    /// Apply each axis's segment map to a default-normalized coordinate
    /// tuple in place-by-value. Axes beyond the table's `axisCount` (or
    /// for which the table has no map) pass through unchanged.
    pub fn apply(&self, normalized: &[f32]) -> Vec<f32> {
        normalized
            .iter()
            .enumerate()
            .map(|(i, &v)| self.segment_maps.get(i).map(|sm| sm.apply(v)).unwrap_or(v))
            .collect()
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
}
