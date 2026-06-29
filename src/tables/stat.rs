//! `STAT` — style attributes (ISO/IEC 14496-22:2019 §7.3.7).
//!
//! The style attributes table describes the design attributes that
//! distinguish a font's style within its family, and associates those
//! attributes with `name`-table strings for UI presentation. It is
//! required in all variable fonts and recommended for new non-variable
//! fonts.
//!
//! Structure (§7.3.7.1): a header with version, two arrays
//! (design-axis records and axis-value-table offsets), and an
//! `elidedFallbackNameID`:
//!
//! ```text
//! STAT header
//!   uint16   majorVersion = 1
//!   uint16   minorVersion = 1 or 2  (1.0 omitted elidedFallbackNameID)
//!   uint16   designAxisSize
//!   uint16   designAxisCount
//!   Offset32 offsetToDesignAxes
//!   uint16   axisValueCount
//!   Offset32 offsetToAxisValueOffsets
//!   uint16   elidedFallbackNameID   (>= v1.1)
//!
//! AxisRecord (designAxisSize bytes each)
//!   Tag    axisTag
//!   uint16 axisNameID
//!   uint16 axisOrdering
//!
//! axisValueOffsets[axisValueCount]   // Offset16 from the array start
//! ```
//!
//! Four axis-value-table formats (§7.3.7.3): formats 1/2/3 are
//! single-axis (value, value+range, value+style-link), format 4 is a
//! multi-axis combination.

use crate::parser::{read_fixed, read_tag, read_u16, read_u32};
use crate::Error;

/// `flags` bit 0: this record describes other (older-sibling) family
/// members, not the containing font.
pub const STAT_OLDER_SIBLING_FONT_ATTRIBUTE: u16 = 0x0001;
/// `flags` bit 1: this axis value is the "normal" value and may be
/// elided from composed names.
pub const STAT_ELIDABLE_AXIS_VALUE_NAME: u16 = 0x0002;

/// A `STAT` design-axis record (§7.3.7.2).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StatAxisRecord {
    /// `axisTag` — the design-variation axis tag (e.g. `b"wght"`).
    pub tag: [u8; 4],
    /// `axisNameID` — `name`-table ID of the axis's display name.
    pub name_id: u16,
    /// `axisOrdering` — sort/priority hint for UI ordering.
    pub ordering: u16,
}

/// A decoded axis-value table (§7.3.7.3), one of the four formats.
#[derive(Debug, Clone, PartialEq)]
pub enum AxisValue {
    /// Format 1: a single axis value with a name.
    Format1 {
        axis_index: u16,
        flags: u16,
        value_name_id: u16,
        value: f32,
    },
    /// Format 2: a nominal value plus a `[min, max]` range.
    Format2 {
        axis_index: u16,
        flags: u16,
        value_name_id: u16,
        nominal_value: f32,
        range_min: f32,
        range_max: f32,
    },
    /// Format 3: a value plus a style-linked counterpart (e.g. the
    /// "bold" weight paired with a "regular" weight).
    Format3 {
        axis_index: u16,
        flags: u16,
        value_name_id: u16,
        value: f32,
        linked_value: f32,
    },
    /// Format 4: a multi-axis combination with one name.
    Format4 {
        flags: u16,
        value_name_id: u16,
        /// `(axisIndex, value)` pairs, one per contributing axis.
        values: Vec<(u16, f32)>,
    },
}

impl AxisValue {
    /// The `flags` field common to all formats.
    pub fn flags(&self) -> u16 {
        match self {
            AxisValue::Format1 { flags, .. }
            | AxisValue::Format2 { flags, .. }
            | AxisValue::Format3 { flags, .. }
            | AxisValue::Format4 { flags, .. } => *flags,
        }
    }

    /// The `valueNameID` field common to all formats.
    pub fn value_name_id(&self) -> u16 {
        match self {
            AxisValue::Format1 { value_name_id, .. }
            | AxisValue::Format2 { value_name_id, .. }
            | AxisValue::Format3 { value_name_id, .. }
            | AxisValue::Format4 { value_name_id, .. } => *value_name_id,
        }
    }

    /// The on-disk format number (1..=4).
    pub fn format(&self) -> u16 {
        match self {
            AxisValue::Format1 { .. } => 1,
            AxisValue::Format2 { .. } => 2,
            AxisValue::Format3 { .. } => 3,
            AxisValue::Format4 { .. } => 4,
        }
    }

    /// `true` if the `ELIDABLE_AXIS_VALUE_NAME` flag is set.
    pub fn is_elidable(&self) -> bool {
        self.flags() & STAT_ELIDABLE_AXIS_VALUE_NAME != 0
    }

    /// `true` if the `OLDER_SIBLING_FONT_ATTRIBUTE` flag is set.
    pub fn is_older_sibling(&self) -> bool {
        self.flags() & STAT_OLDER_SIBLING_FONT_ATTRIBUTE != 0
    }
}

/// A parsed `STAT` table.
#[derive(Debug, Clone)]
pub struct StatTable {
    major: u16,
    minor: u16,
    elided_fallback_name_id: u16,
    axes: Vec<StatAxisRecord>,
    axis_values: Vec<AxisValue>,
}

impl StatTable {
    pub fn parse(bytes: &[u8]) -> Result<Self, Error> {
        if bytes.len() < 20 {
            return Err(Error::UnexpectedEof);
        }
        let major = read_u16(bytes, 0)?;
        let minor = read_u16(bytes, 2)?;
        if major != 1 {
            return Err(Error::BadStructure("STAT: unsupported majorVersion"));
        }
        let design_axis_size = read_u16(bytes, 4)? as usize;
        let design_axis_count = read_u16(bytes, 6)? as usize;
        let offset_to_design_axes = read_u32(bytes, 8)? as usize;
        let axis_value_count = read_u16(bytes, 12)? as usize;
        let offset_to_axis_value_offsets = read_u32(bytes, 16)? as usize;
        // elidedFallbackNameID exists from v1.1 on (the header is 20
        // bytes when present, 18 in the deprecated v1.0).
        let elided_fallback_name_id = if minor >= 1 {
            read_u16(bytes, 20).unwrap_or(0)
        } else {
            0
        };

        // An AxisRecord is 8 bytes (Tag + 2 uint16); reject a smaller
        // designAxisSize (a larger one is tolerated via the stride).
        if design_axis_count > 0 && design_axis_size < 8 {
            return Err(Error::BadStructure("STAT: designAxisSize < 8"));
        }

        // Parse the design-axis records.
        let mut axes = Vec::with_capacity(design_axis_count);
        for i in 0..design_axis_count {
            let off = offset_to_design_axes + i * design_axis_size;
            if off + 8 > bytes.len() {
                return Err(Error::UnexpectedEof);
            }
            axes.push(StatAxisRecord {
                tag: read_tag(bytes, off)?,
                name_id: read_u16(bytes, off + 4)?,
                ordering: read_u16(bytes, off + 6)?,
            });
        }

        // Parse the axis-value tables (each reached through an Offset16
        // relative to the start of the offsets array).
        let mut axis_values = Vec::with_capacity(axis_value_count);
        for i in 0..axis_value_count {
            let off_pos = offset_to_axis_value_offsets + i * 2;
            if off_pos + 2 > bytes.len() {
                return Err(Error::UnexpectedEof);
            }
            let rel = read_u16(bytes, off_pos)? as usize;
            if rel == 0 {
                continue; // NULL offset — no table
            }
            let av_off = offset_to_axis_value_offsets + rel;
            if let Some(av) = parse_axis_value(bytes, av_off)? {
                axis_values.push(av);
            }
        }

        Ok(Self {
            major,
            minor,
            elided_fallback_name_id,
            axes,
            axis_values,
        })
    }

    /// `(majorVersion, minorVersion)`.
    pub fn version(&self) -> (u16, u16) {
        (self.major, self.minor)
    }

    /// `elidedFallbackNameID` (0 for a deprecated v1.0 table).
    pub fn elided_fallback_name_id(&self) -> u16 {
        self.elided_fallback_name_id
    }

    /// The design-axis records, in table order.
    pub fn axes(&self) -> &[StatAxisRecord] {
        &self.axes
    }

    /// The decoded axis-value tables (NULL and unrecognised-format
    /// entries are skipped).
    pub fn axis_values(&self) -> &[AxisValue] {
        &self.axis_values
    }
}

/// Parse one axis-value table at `off`. Returns `None` for an
/// unrecognised format (per spec: ignore it).
fn parse_axis_value(bytes: &[u8], off: usize) -> Result<Option<AxisValue>, Error> {
    let format = read_u16(bytes, off)?;
    let av = match format {
        1 => {
            if off + 12 > bytes.len() {
                return Err(Error::UnexpectedEof);
            }
            AxisValue::Format1 {
                axis_index: read_u16(bytes, off + 2)?,
                flags: read_u16(bytes, off + 4)?,
                value_name_id: read_u16(bytes, off + 6)?,
                value: read_fixed(bytes, off + 8)?,
            }
        }
        2 => {
            if off + 20 > bytes.len() {
                return Err(Error::UnexpectedEof);
            }
            AxisValue::Format2 {
                axis_index: read_u16(bytes, off + 2)?,
                flags: read_u16(bytes, off + 4)?,
                value_name_id: read_u16(bytes, off + 6)?,
                nominal_value: read_fixed(bytes, off + 8)?,
                range_min: read_fixed(bytes, off + 12)?,
                range_max: read_fixed(bytes, off + 16)?,
            }
        }
        3 => {
            if off + 16 > bytes.len() {
                return Err(Error::UnexpectedEof);
            }
            AxisValue::Format3 {
                axis_index: read_u16(bytes, off + 2)?,
                flags: read_u16(bytes, off + 4)?,
                value_name_id: read_u16(bytes, off + 6)?,
                value: read_fixed(bytes, off + 8)?,
                linked_value: read_fixed(bytes, off + 12)?,
            }
        }
        4 => {
            if off + 8 > bytes.len() {
                return Err(Error::UnexpectedEof);
            }
            let axis_count = read_u16(bytes, off + 2)? as usize;
            let flags = read_u16(bytes, off + 4)?;
            let value_name_id = read_u16(bytes, off + 6)?;
            let mut values = Vec::with_capacity(axis_count);
            for i in 0..axis_count {
                let rec = off + 8 + i * 6;
                if rec + 6 > bytes.len() {
                    return Err(Error::UnexpectedEof);
                }
                values.push((read_u16(bytes, rec)?, read_fixed(bytes, rec + 2)?));
            }
            AxisValue::Format4 {
                flags,
                value_name_id,
                values,
            }
        }
        _ => return Ok(None),
    };
    Ok(Some(av))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixed(v: f32) -> [u8; 4] {
        ((v * 65536.0) as i32).to_be_bytes()
    }

    #[test]
    fn parses_axes_and_format1_values() {
        // Header (20 bytes for v1.1) + 1 axis + 1 axis-value offset + a
        // format-1 axis value.
        let header_len = 22; // include elidedFallbackNameID (2 bytes)
        let design_axes_off = header_len;
        let axis_rec_len = 8;
        let av_offsets_off = design_axes_off + axis_rec_len;
        let av_off = av_offsets_off + 2; // 1 offset entry of 2 bytes

        let mut b = vec![0u8; av_off + 12];
        b[0..2].copy_from_slice(&1u16.to_be_bytes()); // major
        b[2..4].copy_from_slice(&1u16.to_be_bytes()); // minor
        b[4..6].copy_from_slice(&(axis_rec_len as u16).to_be_bytes()); // designAxisSize
        b[6..8].copy_from_slice(&1u16.to_be_bytes()); // designAxisCount
        b[8..12].copy_from_slice(&(design_axes_off as u32).to_be_bytes());
        b[12..14].copy_from_slice(&1u16.to_be_bytes()); // axisValueCount
        b[16..20].copy_from_slice(&(av_offsets_off as u32).to_be_bytes());
        b[20..22].copy_from_slice(&2u16.to_be_bytes()); // elidedFallbackNameID
                                                        // axis record.
        b[design_axes_off..design_axes_off + 4].copy_from_slice(b"wght");
        b[design_axes_off + 4..design_axes_off + 6].copy_from_slice(&256u16.to_be_bytes());
        b[design_axes_off + 6..design_axes_off + 8].copy_from_slice(&0u16.to_be_bytes());
        // axis-value offset (relative to av_offsets_off): points to av_off.
        let rel = (av_off - av_offsets_off) as u16;
        b[av_offsets_off..av_offsets_off + 2].copy_from_slice(&rel.to_be_bytes());
        // format-1 axis value: Bold weight 700, elidable=false.
        b[av_off..av_off + 2].copy_from_slice(&1u16.to_be_bytes()); // format
        b[av_off + 2..av_off + 4].copy_from_slice(&0u16.to_be_bytes()); // axisIndex
        b[av_off + 4..av_off + 6].copy_from_slice(&0u16.to_be_bytes()); // flags
        b[av_off + 6..av_off + 8].copy_from_slice(&300u16.to_be_bytes()); // valueNameID
        b[av_off + 8..av_off + 12].copy_from_slice(&fixed(700.0));

        let stat = StatTable::parse(&b).unwrap();
        assert_eq!(stat.version(), (1, 1));
        assert_eq!(stat.elided_fallback_name_id(), 2);
        assert_eq!(stat.axes().len(), 1);
        assert_eq!(&stat.axes()[0].tag, b"wght");
        assert_eq!(stat.axes()[0].name_id, 256);
        assert_eq!(stat.axis_values().len(), 1);
        match &stat.axis_values()[0] {
            AxisValue::Format1 {
                axis_index,
                value_name_id,
                value,
                ..
            } => {
                assert_eq!(*axis_index, 0);
                assert_eq!(*value_name_id, 300);
                assert_eq!(*value, 700.0);
            }
            other => panic!("expected format 1, got {other:?}"),
        }
        assert_eq!(stat.axis_values()[0].format(), 1);
        assert!(!stat.axis_values()[0].is_elidable());
    }

    #[test]
    fn parses_format2_3_4() {
        // Build a STAT with three axis values: format 2, 3, 4.
        let header_len = 22;
        let axis_rec_len = 8;
        let design_axes_off = header_len;
        let av_offsets_off = design_axes_off + axis_rec_len; // 1 axis
        let av_count = 3;
        let av_base = av_offsets_off + av_count * 2;
        let f2_off = av_base;
        let f3_off = f2_off + 20;
        let f4_off = f3_off + 16;
        let total = f4_off + 8 + 2 * 6; // format 4 with 2 axis values

        let mut b = vec![0u8; total];
        b[0..2].copy_from_slice(&1u16.to_be_bytes());
        b[2..4].copy_from_slice(&2u16.to_be_bytes()); // minor = 2
        b[4..6].copy_from_slice(&(axis_rec_len as u16).to_be_bytes());
        b[6..8].copy_from_slice(&1u16.to_be_bytes());
        b[8..12].copy_from_slice(&(design_axes_off as u32).to_be_bytes());
        b[12..14].copy_from_slice(&(av_count as u16).to_be_bytes());
        b[16..20].copy_from_slice(&(av_offsets_off as u32).to_be_bytes());
        b[20..22].copy_from_slice(&17u16.to_be_bytes());
        b[design_axes_off..design_axes_off + 4].copy_from_slice(b"opsz");
        // offsets (relative to av_offsets_off).
        for (i, &abs) in [f2_off, f3_off, f4_off].iter().enumerate() {
            let rel = (abs - av_offsets_off) as u16;
            let p = av_offsets_off + i * 2;
            b[p..p + 2].copy_from_slice(&rel.to_be_bytes());
        }
        // format 2.
        b[f2_off..f2_off + 2].copy_from_slice(&2u16.to_be_bytes());
        b[f2_off + 6..f2_off + 8].copy_from_slice(&400u16.to_be_bytes());
        b[f2_off + 8..f2_off + 12].copy_from_slice(&fixed(11.0)); // nominal
        b[f2_off + 12..f2_off + 16].copy_from_slice(&fixed(9.0)); // min
        b[f2_off + 16..f2_off + 20].copy_from_slice(&fixed(13.0)); // max
                                                                   // format 3.
        b[f3_off..f3_off + 2].copy_from_slice(&3u16.to_be_bytes());
        b[f3_off + 6..f3_off + 8].copy_from_slice(&401u16.to_be_bytes());
        b[f3_off + 8..f3_off + 12].copy_from_slice(&fixed(400.0)); // value
        b[f3_off + 12..f3_off + 16].copy_from_slice(&fixed(700.0)); // linked
                                                                    // format 4 with 2 axis values + elidable flag.
        b[f4_off..f4_off + 2].copy_from_slice(&4u16.to_be_bytes());
        b[f4_off + 2..f4_off + 4].copy_from_slice(&2u16.to_be_bytes()); // axisCount
        b[f4_off + 4..f4_off + 6].copy_from_slice(&STAT_ELIDABLE_AXIS_VALUE_NAME.to_be_bytes());
        b[f4_off + 6..f4_off + 8].copy_from_slice(&500u16.to_be_bytes());
        b[f4_off + 8..f4_off + 10].copy_from_slice(&0u16.to_be_bytes()); // axisIndex 0
        b[f4_off + 10..f4_off + 14].copy_from_slice(&fixed(700.0));
        b[f4_off + 14..f4_off + 16].copy_from_slice(&1u16.to_be_bytes()); // axisIndex 1
        b[f4_off + 16..f4_off + 20].copy_from_slice(&fixed(75.0));

        let stat = StatTable::parse(&b).unwrap();
        assert_eq!(stat.version(), (1, 2));
        assert_eq!(stat.axis_values().len(), 3);
        match &stat.axis_values()[0] {
            AxisValue::Format2 {
                nominal_value,
                range_min,
                range_max,
                ..
            } => {
                assert_eq!(*nominal_value, 11.0);
                assert_eq!(*range_min, 9.0);
                assert_eq!(*range_max, 13.0);
            }
            o => panic!("expected format 2, got {o:?}"),
        }
        match &stat.axis_values()[1] {
            AxisValue::Format3 {
                value,
                linked_value,
                ..
            } => {
                assert_eq!(*value, 400.0);
                assert_eq!(*linked_value, 700.0);
            }
            o => panic!("expected format 3, got {o:?}"),
        }
        match &stat.axis_values()[2] {
            AxisValue::Format4 { values, .. } => {
                assert_eq!(values.len(), 2);
                assert_eq!(values[0], (0, 700.0));
                assert_eq!(values[1], (1, 75.0));
            }
            o => panic!("expected format 4, got {o:?}"),
        }
        assert!(stat.axis_values()[2].is_elidable());
    }

    #[test]
    fn unrecognised_format_skipped() {
        let header_len = 22;
        let av_offsets_off = header_len;
        let av_off = av_offsets_off + 2;
        let mut b = vec![0u8; av_off + 4];
        b[0..2].copy_from_slice(&1u16.to_be_bytes());
        b[2..4].copy_from_slice(&1u16.to_be_bytes());
        b[4..6].copy_from_slice(&8u16.to_be_bytes());
        b[6..8].copy_from_slice(&0u16.to_be_bytes()); // 0 axes
        b[8..12].copy_from_slice(&(header_len as u32).to_be_bytes());
        b[12..14].copy_from_slice(&1u16.to_be_bytes()); // 1 axis value
        b[16..20].copy_from_slice(&(av_offsets_off as u32).to_be_bytes());
        let rel = (av_off - av_offsets_off) as u16;
        b[av_offsets_off..av_offsets_off + 2].copy_from_slice(&rel.to_be_bytes());
        b[av_off..av_off + 2].copy_from_slice(&99u16.to_be_bytes()); // unknown format
        let stat = StatTable::parse(&b).unwrap();
        assert_eq!(stat.axis_values().len(), 0);
    }

    #[test]
    fn rejects_bad_version() {
        let mut b = vec![0u8; 20];
        b[0..2].copy_from_slice(&2u16.to_be_bytes());
        assert!(matches!(StatTable::parse(&b), Err(Error::BadStructure(_))));
    }
}
