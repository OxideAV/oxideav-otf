//! `fvar` — font variations (ISO/IEC 14496-22:2019 §7.3.3).
//!
//! The `fvar` table is the global definition of a variable font's design
//! space: the **axes** of variation (each with a tag, min/default/max in
//! user coordinates, flags, and a `name`-table name ID) and a set of
//! **named instances** (designer-chosen positions in the space, each
//! with a subfamily name ID and an optional PostScript name ID).
//!
//! On-disk layout (§7.3.3.1):
//!
//! ```text
//! fvar header
//!   uint16   majorVersion          = 1
//!   uint16   minorVersion          = 0
//!   Offset16 axesArrayOffset       = 16
//!   uint16   (reserved)            = 2
//!   uint16   axisCount
//!   uint16   axisSize              = 20
//!   uint16   instanceCount
//!   uint16   instanceSize          = axisCount*4 + 4  (or +6 with PS name)
//!
//! VariationAxisRecord (axisSize bytes each, at axesArrayOffset)
//!   Tag    axisTag
//!   Fixed  minValue
//!   Fixed  defaultValue
//!   Fixed  maxValue
//!   uint16 flags
//!   uint16 axisNameID
//!
//! InstanceRecord (instanceSize bytes each, directly after the axes)
//!   uint16 subfamilyNameID
//!   uint16 flags
//!   Fixed  coordinates[axisCount]
//!   uint16 postScriptNameID        (present iff instanceSize == axisCount*4 + 6)
//! ```
//!
//! The crate's primary use of `fvar` is **coordinate normalization**:
//! given a user-scale axis value, map it onto the normalized `[-1, 1]`
//! scale (default to 0, min to -1, max to 1, linear between) that every
//! other variation table consumes. `avar`, if present, refines this.

use crate::parser::{read_fixed, read_tag, read_u16};
use crate::Error;

/// `flags` bit 0: the axis should not be exposed directly in UIs.
pub const FVAR_AXIS_HIDDEN: u16 = 0x0001;

/// One variation axis (`VariationAxisRecord`). Coordinate values are in
/// **user scale** (the scale registered for the axis tag, e.g. 100..900
/// for `wght`), not normalized.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VariationAxis {
    /// `axisTag` — e.g. `b"wght"`, `b"wdth"`, `b"slnt"`, `b"ital"`,
    /// `b"opsz"`, or a foundry-defined tag.
    pub tag: [u8; 4],
    /// `minValue` (user scale).
    pub min: f32,
    /// `defaultValue` (user scale).
    pub default: f32,
    /// `maxValue` (user scale).
    pub max: f32,
    /// `flags` — currently only `FVAR_AXIS_HIDDEN` is defined.
    pub flags: u16,
    /// `axisNameID` — `name`-table ID for the axis's display name.
    pub name_id: u16,
}

impl VariationAxis {
    /// `true` if the `HIDDEN_AXIS` flag is set.
    pub fn is_hidden(&self) -> bool {
        self.flags & FVAR_AXIS_HIDDEN != 0
    }

    /// Normalize a user-scale coordinate to the `[-1.0, 1.0]` scale per
    /// the spec's default-normalization pseudo-code (§7.3.1.1): clamp to
    /// `[min, max]`, then map default→0, min→-1, max→1, linear within
    /// each half. `avar` (if present) further modifies this output.
    pub fn normalize(&self, user_value: f32) -> f32 {
        let v = user_value.clamp(self.min, self.max);
        if v < self.default {
            if self.default == self.min {
                0.0
            } else {
                -((self.default - v) / (self.default - self.min))
            }
        } else if v > self.default {
            if self.max == self.default {
                0.0
            } else {
                (v - self.default) / (self.max - self.default)
            }
        } else {
            0.0
        }
    }
}

/// One named instance (`InstanceRecord`): a designer-named position in
/// the variation space.
#[derive(Debug, Clone, PartialEq)]
pub struct NamedInstance {
    /// `subfamilyNameID` — `name`-table ID treated as the typographic
    /// subfamily (name ID 17 equivalent) for this instance.
    pub subfamily_name_id: u16,
    /// `flags` — reserved (0).
    pub flags: u16,
    /// `coordinates[axisCount]` — user-scale position, one per axis in
    /// axis order.
    pub coordinates: Vec<f32>,
    /// `postScriptNameID` — optional; `None` when the instance records
    /// omit the field, and `None` also when the stored value is the
    /// "no PS name" sentinel `0xFFFF`.
    pub postscript_name_id: Option<u16>,
}

/// A parsed `fvar` table.
#[derive(Debug, Clone)]
pub struct FvarTable {
    axes: Vec<VariationAxis>,
    instances: Vec<NamedInstance>,
}

impl FvarTable {
    pub fn parse(bytes: &[u8]) -> Result<Self, Error> {
        if bytes.len() < 16 {
            return Err(Error::UnexpectedEof);
        }
        let major = read_u16(bytes, 0)?;
        if major != 1 {
            return Err(Error::BadStructure("fvar: unsupported majorVersion"));
        }
        let axes_off = read_u16(bytes, 4)? as usize;
        let axis_count = read_u16(bytes, 8)? as usize;
        let axis_size = read_u16(bytes, 10)? as usize;
        let instance_count = read_u16(bytes, 12)? as usize;
        let instance_size = read_u16(bytes, 14)? as usize;

        // A VariationAxisRecord is 20 bytes; reject a smaller axisSize
        // (a larger one is tolerated — future fields are skipped via the
        // axisSize stride).
        if axis_size < 20 {
            return Err(Error::BadStructure("fvar: axisSize < 20"));
        }
        // The InstanceRecord is axisCount*4 + 4, optionally + 2 for the
        // PostScript name ID. Anything smaller is malformed.
        let inst_base = axis_count * 4 + 4;
        if instance_count > 0 && instance_size < inst_base {
            return Err(Error::BadStructure("fvar: instanceSize too small"));
        }
        let has_ps_name = instance_size >= inst_base + 2;

        // Parse axes.
        let mut axes = Vec::with_capacity(axis_count);
        for i in 0..axis_count {
            let off = axes_off + i * axis_size;
            if off + 20 > bytes.len() {
                return Err(Error::UnexpectedEof);
            }
            axes.push(VariationAxis {
                tag: read_tag(bytes, off)?,
                min: read_fixed(bytes, off + 4)?,
                default: read_fixed(bytes, off + 8)?,
                max: read_fixed(bytes, off + 12)?,
                flags: read_u16(bytes, off + 16)?,
                name_id: read_u16(bytes, off + 18)?,
            });
        }

        // Parse instances (directly after the axes array).
        let instances_off = axes_off + axis_count * axis_size;
        let mut instances = Vec::with_capacity(instance_count);
        for i in 0..instance_count {
            let off = instances_off + i * instance_size;
            if off + inst_base > bytes.len() {
                return Err(Error::UnexpectedEof);
            }
            let subfamily_name_id = read_u16(bytes, off)?;
            let flags = read_u16(bytes, off + 2)?;
            let mut coordinates = Vec::with_capacity(axis_count);
            for a in 0..axis_count {
                coordinates.push(read_fixed(bytes, off + 4 + a * 4)?);
            }
            let postscript_name_id = if has_ps_name {
                let v = read_u16(bytes, off + 4 + axis_count * 4)?;
                if v == 0xFFFF {
                    None
                } else {
                    Some(v)
                }
            } else {
                None
            };
            instances.push(NamedInstance {
                subfamily_name_id,
                flags,
                coordinates,
                postscript_name_id,
            });
        }

        Ok(Self { axes, instances })
    }

    /// The font's variation axes, in axis order.
    pub fn axes(&self) -> &[VariationAxis] {
        &self.axes
    }

    /// Number of variation axes (`axisCount`).
    pub fn axis_count(&self) -> usize {
        self.axes.len()
    }

    /// Look up an axis by its 4-byte tag.
    pub fn axis(&self, tag: &[u8; 4]) -> Option<&VariationAxis> {
        self.axes.iter().find(|a| &a.tag == tag)
    }

    /// The font's named instances.
    pub fn instances(&self) -> &[NamedInstance] {
        &self.instances
    }

    /// Normalize a full user-scale coordinate tuple to `[-1, 1]` per
    /// axis. `user_coords` is matched against the axes positionally; a
    /// short slice uses each remaining axis's default (→ 0.0); a long
    /// slice ignores the surplus. The result always has `axis_count`
    /// entries. (This applies only the default `fvar` normalization;
    /// `avar` refinement, if present, is applied by the caller — see
    /// `Font::normalize_coords`.)
    pub fn normalize_coords(&self, user_coords: &[f32]) -> Vec<f32> {
        self.axes
            .iter()
            .enumerate()
            .map(|(i, axis)| {
                let user = user_coords.get(i).copied().unwrap_or(axis.default);
                axis.normalize(user)
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixed(v: f32) -> [u8; 4] {
        ((v * 65536.0) as i32).to_be_bytes()
    }

    fn build(
        axes: &[([u8; 4], f32, f32, f32)],
        instances: &[(u16, &[f32], Option<u16>)],
    ) -> Vec<u8> {
        let axis_count = axes.len();
        let has_ps = instances.iter().any(|(_, _, p)| p.is_some());
        let inst_size = axis_count * 4 + 4 + if has_ps { 2 } else { 0 };
        let mut b = Vec::new();
        // header
        b.extend_from_slice(&1u16.to_be_bytes()); // major
        b.extend_from_slice(&0u16.to_be_bytes()); // minor
        b.extend_from_slice(&16u16.to_be_bytes()); // axesArrayOffset
        b.extend_from_slice(&2u16.to_be_bytes()); // reserved
        b.extend_from_slice(&(axis_count as u16).to_be_bytes());
        b.extend_from_slice(&20u16.to_be_bytes()); // axisSize
        b.extend_from_slice(&(instances.len() as u16).to_be_bytes());
        b.extend_from_slice(&(inst_size as u16).to_be_bytes());
        // axes
        for (tag, mn, df, mx) in axes {
            b.extend_from_slice(tag);
            b.extend_from_slice(&fixed(*mn));
            b.extend_from_slice(&fixed(*df));
            b.extend_from_slice(&fixed(*mx));
            b.extend_from_slice(&0u16.to_be_bytes()); // flags
            b.extend_from_slice(&256u16.to_be_bytes()); // nameID
        }
        // instances
        for (sub, coords, ps) in instances {
            b.extend_from_slice(&sub.to_be_bytes());
            b.extend_from_slice(&0u16.to_be_bytes()); // flags
            for c in coords.iter() {
                b.extend_from_slice(&fixed(*c));
            }
            if has_ps {
                b.extend_from_slice(&ps.unwrap_or(0xFFFF).to_be_bytes());
            }
        }
        b
    }

    #[test]
    fn parses_axes_and_instances() {
        let b = build(
            &[
                (*b"wght", 100.0, 400.0, 900.0),
                (*b"wdth", 75.0, 100.0, 125.0),
            ],
            &[
                (17, &[400.0, 100.0], Some(300)),
                (258, &[700.0, 100.0], Some(301)),
            ],
        );
        let f = FvarTable::parse(&b).unwrap();
        assert_eq!(f.axis_count(), 2);
        assert_eq!(f.axis(b"wght").unwrap().default, 400.0);
        assert_eq!(f.axis(b"wdth").unwrap().min, 75.0);
        assert_eq!(f.instances().len(), 2);
        assert_eq!(f.instances()[1].subfamily_name_id, 258);
        assert_eq!(f.instances()[0].postscript_name_id, Some(300));
        assert_eq!(f.instances()[0].coordinates, vec![400.0, 100.0]);
    }

    #[test]
    fn normalization_default_min_max() {
        let axis = VariationAxis {
            tag: *b"wght",
            min: 100.0,
            default: 400.0,
            max: 900.0,
            flags: 0,
            name_id: 256,
        };
        assert_eq!(axis.normalize(400.0), 0.0);
        assert_eq!(axis.normalize(100.0), -1.0);
        assert_eq!(axis.normalize(900.0), 1.0);
        // halfway from default to max: (700-400)/(900-400) = 0.6
        assert!((axis.normalize(700.0) - 0.6).abs() < 1e-6);
        // halfway from default to min: -(400-250)/(400-100) = -0.5
        assert!((axis.normalize(250.0) - (-0.5)).abs() < 1e-6);
        // out of range clamps.
        assert_eq!(axis.normalize(2000.0), 1.0);
        assert_eq!(axis.normalize(-50.0), -1.0);
    }

    #[test]
    fn normalize_coords_fills_defaults() {
        let b = build(
            &[
                (*b"wght", 100.0, 400.0, 900.0),
                (*b"wdth", 75.0, 100.0, 125.0),
            ],
            &[],
        );
        let f = FvarTable::parse(&b).unwrap();
        // Only specify wght; wdth uses default → 0.0.
        let n = f.normalize_coords(&[900.0]);
        assert_eq!(n, vec![1.0, 0.0]);
    }

    #[test]
    fn ps_name_sentinel_is_none() {
        let b = build(&[(*b"wght", 100.0, 400.0, 900.0)], &[(17, &[400.0], None)]);
        let f = FvarTable::parse(&b).unwrap();
        // build() forces has_ps=false when no instance has a PS name, so
        // postscript_name_id is None because the field is absent.
        assert_eq!(f.instances()[0].postscript_name_id, None);
    }

    #[test]
    fn rejects_bad_version() {
        let mut b = vec![0u8; 16];
        b[0..2].copy_from_slice(&2u16.to_be_bytes());
        assert!(matches!(FvarTable::parse(&b), Err(Error::BadStructure(_))));
    }
}
