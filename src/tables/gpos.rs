//! `GPOS` — Glyph Positioning Table (header + ScriptList / FeatureList
//! / LookupList walk).
//!
//! Spec: Microsoft / ISO/IEC 14496-22 OpenType `GPOS` table
//! (`docs/text/opentype/otspec-gpos.html`), with the
//! `ScriptList` / `FeatureList` / `LookupList` / `Lookup` /
//! `LookupFlag` structures sourced from
//! `docs/text/opentype/otspec-chapter2-common-layout-tables.html`.
//!
//! Two header versions are defined:
//! ```text
//!   GPOS Header, version 1.0           (10 bytes)
//!   0 / 2 / majorVersion (= 1)
//!   2 / 2 / minorVersion (= 0)
//!   4 / 2 / scriptListOffset    (Offset16, from start of GPOS)
//!   6 / 2 / featureListOffset   (Offset16, from start of GPOS)
//!   8 / 2 / lookupListOffset    (Offset16, from start of GPOS)
//!
//!   GPOS Header, version 1.1           (14 bytes; adds:)
//!  10 / 4 / featureVariationsOffset    (Offset32; may be NULL)
//! ```
//!
//! Typed lookup-subtable views are decoded for Lookup Type 1 (single
//! adjustment), Type 2 (pair adjustment), Type 3 (cursive attachment),
//! Type 4 (mark-to-base attachment), Type 6 (mark-to-mark attachment),
//! and Type 9 (positioning extension), the last wrapping any of the
//! already-decoded types via a 32-bit indirection. The remaining lookup
//! types (5 Mark-to-ligature, 7–8 Context/Chained) are left as raw
//! sub-slices via [`super::layout::Lookup::subtable_bytes`]; their
//! MarkArray interiors are deferred to a future round. The shared
//! ValueRecord, Anchor, and MarkArray/MarkRecord primitives are decoded
//! (the last two by the mark-to-base path; mark-to-mark reuses them, and
//! mark-to-ligature will too); the cursive path reuses the Anchor
//! primitive for its EntryExit records.

use crate::parser::{read_i16, read_u16, read_u32};
use crate::tables::context::{ChainedSequenceContext, SequenceContext};
use crate::tables::device::DeviceOrVariationIndex;
use crate::tables::gdef::{ClassDef, Coverage};
use crate::tables::layout::{
    FeatureList, FeatureVariations, LayoutHeader, Lookup, LookupList, Script, ScriptList,
};
use crate::Error;

/// GPOS Lookup Type 1 — single adjustment positioning.
pub const GPOS_LOOKUP_TYPE_SINGLE: u16 = 1;
/// GPOS Lookup Type 2 — pair adjustment positioning.
pub const GPOS_LOOKUP_TYPE_PAIR: u16 = 2;
/// GPOS Lookup Type 3 — cursive attachment positioning.
pub const GPOS_LOOKUP_TYPE_CURSIVE: u16 = 3;
/// GPOS Lookup Type 4 — mark-to-base attachment positioning.
pub const GPOS_LOOKUP_TYPE_MARK_TO_BASE: u16 = 4;
/// GPOS Lookup Type 5 — mark-to-ligature attachment positioning.
pub const GPOS_LOOKUP_TYPE_MARK_TO_LIGATURE: u16 = 5;
/// GPOS Lookup Type 6 — mark-to-mark attachment positioning.
pub const GPOS_LOOKUP_TYPE_MARK_TO_MARK: u16 = 6;
/// GPOS Lookup Type 7 — contextual positioning.
pub const GPOS_LOOKUP_TYPE_CONTEXT: u16 = 7;
/// GPOS Lookup Type 8 — chained contextual positioning.
pub const GPOS_LOOKUP_TYPE_CHAINED_CONTEXT: u16 = 8;
/// GPOS Lookup Type 9 — positioning extension.
pub const GPOS_LOOKUP_TYPE_EXTENSION: u16 = 9;

// ValueFormat flag masks (GPOS §"ValueRecord", `otspec-gpos.html`).
const VF_X_PLACEMENT: u16 = 0x0001;
const VF_Y_PLACEMENT: u16 = 0x0002;
const VF_X_ADVANCE: u16 = 0x0004;
const VF_Y_ADVANCE: u16 = 0x0008;
const VF_X_PLACEMENT_DEVICE: u16 = 0x0010;
const VF_Y_PLACEMENT_DEVICE: u16 = 0x0020;
const VF_X_ADVANCE_DEVICE: u16 = 0x0040;
const VF_Y_ADVANCE_DEVICE: u16 = 0x0080;
/// Bits reserved for future use (must be zero in a conforming font).
const VF_RESERVED: u16 = 0xFF00;

/// A `ValueFormat` flags field: a bitmask declaring which fields are
/// present in each `ValueRecord` of a SinglePos / PairPos subtable.
///
/// Spec: `docs/text/opentype/otspec-gpos.html` §"ValueRecord".
/// Each defined bit corresponds to one `int16`/`Offset16` field and
/// increases the on-disk `ValueRecord` size by 2 bytes; a value of
/// `0x0000` is an empty record (no positioning change).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ValueFormat(pub u16);

impl ValueFormat {
    /// Raw flag bits.
    pub fn bits(self) -> u16 {
        self.0
    }

    /// `true` iff every set bit is a defined (non-reserved) flag.
    pub fn is_valid(self) -> bool {
        self.0 & VF_RESERVED == 0
    }

    /// Includes an `xPlacement` (`int16`) field.
    pub fn has_x_placement(self) -> bool {
        self.0 & VF_X_PLACEMENT != 0
    }
    /// Includes a `yPlacement` (`int16`) field.
    pub fn has_y_placement(self) -> bool {
        self.0 & VF_Y_PLACEMENT != 0
    }
    /// Includes an `xAdvance` (`int16`) field.
    pub fn has_x_advance(self) -> bool {
        self.0 & VF_X_ADVANCE != 0
    }
    /// Includes a `yAdvance` (`int16`) field.
    pub fn has_y_advance(self) -> bool {
        self.0 & VF_Y_ADVANCE != 0
    }
    /// Includes an `xPlaDeviceOffset` (`Offset16`) field.
    pub fn has_x_placement_device(self) -> bool {
        self.0 & VF_X_PLACEMENT_DEVICE != 0
    }
    /// Includes a `yPlaDeviceOffset` (`Offset16`) field.
    pub fn has_y_placement_device(self) -> bool {
        self.0 & VF_Y_PLACEMENT_DEVICE != 0
    }
    /// Includes an `xAdvDeviceOffset` (`Offset16`) field.
    pub fn has_x_advance_device(self) -> bool {
        self.0 & VF_X_ADVANCE_DEVICE != 0
    }
    /// Includes a `yAdvDeviceOffset` (`Offset16`) field.
    pub fn has_y_advance_device(self) -> bool {
        self.0 & VF_Y_ADVANCE_DEVICE != 0
    }

    /// On-disk size, in bytes, of one `ValueRecord` with this format:
    /// `2 * popcount(definedBits)`.
    pub fn record_size(self) -> usize {
        2 * (self.0 & !VF_RESERVED).count_ones() as usize
    }
}

/// A decoded `ValueRecord`: positioning adjustments for one glyph
/// position. Spec: `docs/text/opentype/otspec-gpos.html` §"ValueRecord".
///
/// The design-unit placement/advance values are surfaced as typed
/// fields, and the four optional Device/VariationIndex offsets are kept
/// as raw `Offset16` values (`0` = NULL); the referenced tables are
/// decoded by [`ValueRecord::x_placement_device`] and friends (given
/// the subtable base, since the offsets are from the start of the
/// subtable that contained the `ValueRecord`). Every field that the
/// originating `ValueFormat` does not declare is reported as `0`,
/// matching the spec's "empty ValueRecord ⇒ no positioning change"
/// semantics.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ValueRecord {
    /// Horizontal placement adjustment, design units.
    pub x_placement: i16,
    /// Vertical placement adjustment, design units.
    pub y_placement: i16,
    /// Horizontal advance adjustment, design units.
    pub x_advance: i16,
    /// Vertical advance adjustment, design units.
    pub y_advance: i16,
    /// Raw `xPlaDeviceOffset` (`0` = NULL).
    pub x_placement_device_offset: u16,
    /// Raw `yPlaDeviceOffset` (`0` = NULL).
    pub y_placement_device_offset: u16,
    /// Raw `xAdvDeviceOffset` (`0` = NULL).
    pub x_advance_device_offset: u16,
    /// Raw `yAdvDeviceOffset` (`0` = NULL).
    pub y_advance_device_offset: u16,
}

impl ValueRecord {
    /// Parse a `ValueRecord` from `data` starting at byte `off`, reading
    /// exactly the fields declared by `format`. Fields appear in the
    /// fixed flag-bit order (placement before advance, X before Y,
    /// values before device offsets). Returns the parsed record and the
    /// number of bytes consumed.
    pub fn parse(data: &[u8], off: usize, format: ValueFormat) -> Result<(Self, usize), Error> {
        let mut cur = off;
        let mut rec = ValueRecord::default();

        macro_rules! take_i16 {
            ($cond:expr, $field:ident) => {
                if $cond {
                    rec.$field = read_i16(data, cur)?;
                    cur += 2;
                }
            };
        }
        macro_rules! take_off {
            ($cond:expr, $field:ident) => {
                if $cond {
                    rec.$field = read_u16(data, cur)?;
                    cur += 2;
                }
            };
        }

        take_i16!(format.has_x_placement(), x_placement);
        take_i16!(format.has_y_placement(), y_placement);
        take_i16!(format.has_x_advance(), x_advance);
        take_i16!(format.has_y_advance(), y_advance);
        take_off!(format.has_x_placement_device(), x_placement_device_offset);
        take_off!(format.has_y_placement_device(), y_placement_device_offset);
        take_off!(format.has_x_advance_device(), x_advance_device_offset);
        take_off!(format.has_y_advance_device(), y_advance_device_offset);

        Ok((rec, cur - off))
    }

    /// Decode the `xPlaDeviceOffset` Device / VariationIndex table.
    /// `subtable` is the slice whose index 0 is the start of the GPOS
    /// subtable that owned this `ValueRecord` (the offset is from the
    /// subtable start). `None` for a NULL offset.
    pub fn x_placement_device<'a>(
        &self,
        subtable: &'a [u8],
    ) -> Option<Result<DeviceOrVariationIndex<'a>, Error>> {
        decode_device_at(subtable, self.x_placement_device_offset)
    }

    /// Decode the `yPlaDeviceOffset` Device / VariationIndex table.
    /// See [`ValueRecord::x_placement_device`] for the `subtable`
    /// convention.
    pub fn y_placement_device<'a>(
        &self,
        subtable: &'a [u8],
    ) -> Option<Result<DeviceOrVariationIndex<'a>, Error>> {
        decode_device_at(subtable, self.y_placement_device_offset)
    }

    /// Decode the `xAdvDeviceOffset` Device / VariationIndex table.
    /// See [`ValueRecord::x_placement_device`] for the `subtable`
    /// convention.
    pub fn x_advance_device<'a>(
        &self,
        subtable: &'a [u8],
    ) -> Option<Result<DeviceOrVariationIndex<'a>, Error>> {
        decode_device_at(subtable, self.x_advance_device_offset)
    }

    /// Decode the `yAdvDeviceOffset` Device / VariationIndex table.
    /// See [`ValueRecord::x_placement_device`] for the `subtable`
    /// convention.
    pub fn y_advance_device<'a>(
        &self,
        subtable: &'a [u8],
    ) -> Option<Result<DeviceOrVariationIndex<'a>, Error>> {
        decode_device_at(subtable, self.y_advance_device_offset)
    }
}

/// GPOS Lookup Type 1 — single adjustment positioning subtable.
///
/// Spec: `docs/text/opentype/otspec-gpos.html` §"Lookup type 1
/// subtable: single adjustment positioning". Two on-disk formats:
///
/// * **Format 1** — one `ValueRecord` applied to *every* covered glyph.
/// * **Format 2** — a parallel array of `ValueRecords`, one per glyph
///   in Coverage order (`valueCount == coverage glyph count`).
#[derive(Debug, Clone, Copy)]
pub struct SinglePos<'a> {
    bytes: &'a [u8],
    coverage: Coverage<'a>,
    value_format: ValueFormat,
    inner: SinglePosInner,
}

#[derive(Debug, Clone, Copy)]
enum SinglePosInner {
    /// Format 1: a single shared `ValueRecord` at `value_off`.
    Format1 { value_off: usize },
    /// Format 2: `count` records starting at `values_off`.
    Format2 { values_off: usize, count: u16 },
}

impl<'a> SinglePos<'a> {
    /// Parse a SinglePos subtable from its raw `bytes`.
    pub fn parse(bytes: &'a [u8]) -> Result<Self, Error> {
        let format = read_u16(bytes, 0)?;
        let coverage_off = read_u16(bytes, 2)? as usize;
        let value_format = ValueFormat(read_u16(bytes, 4)?);
        if !value_format.is_valid() {
            return Err(Error::BadStructure(
                "GPOS/SinglePos: reserved valueFormat bit set",
            ));
        }
        if coverage_off == 0 || coverage_off >= bytes.len() {
            return Err(Error::BadStructure(
                "GPOS/SinglePos: coverageOffset out of range",
            ));
        }
        let coverage = Coverage::parse(&bytes[coverage_off..])?;

        match format {
            1 => {
                // header is 6 bytes; the shared ValueRecord follows.
                let value_off = 6usize;
                // Validate the record fits.
                ValueRecord::parse(bytes, value_off, value_format)?;
                Ok(Self {
                    bytes,
                    coverage,
                    value_format,
                    inner: SinglePosInner::Format1 { value_off },
                })
            }
            2 => {
                let count = read_u16(bytes, 6)?;
                let values_off = 8usize;
                let rec_size = value_format.record_size();
                let need = values_off
                    .checked_add(
                        rec_size
                            .checked_mul(count as usize)
                            .ok_or(Error::BadStructure("GPOS/SinglePos: valueCount overflow"))?,
                    )
                    .ok_or(Error::BadStructure("GPOS/SinglePos: length overflow"))?;
                if bytes.len() < need {
                    return Err(Error::UnexpectedEof);
                }
                Ok(Self {
                    bytes,
                    coverage,
                    value_format,
                    inner: SinglePosInner::Format2 { values_off, count },
                })
            }
            _ => Err(Error::BadStructure("GPOS/SinglePos: unknown format")),
        }
    }

    /// Subtable format discriminant (1 or 2).
    pub fn format(&self) -> u16 {
        match self.inner {
            SinglePosInner::Format1 { .. } => 1,
            SinglePosInner::Format2 { .. } => 2,
        }
    }

    /// The subtable's [`ValueFormat`].
    pub fn value_format(&self) -> ValueFormat {
        self.value_format
    }

    /// The raw subtable bytes (index 0 = start of the SinglePos
    /// subtable). Device / VariationIndex offsets inside this
    /// subtable's `ValueRecord`s are measured "from beginning of the
    /// immediate parent table", which for SinglePos is the subtable
    /// itself.
    pub fn raw(&self) -> &'a [u8] {
        self.bytes
    }

    /// The subtable's [`Coverage`] table.
    pub fn coverage(&self) -> Coverage<'a> {
        self.coverage
    }

    /// Number of `ValueRecords` physically stored: `1` for format 1,
    /// `valueCount` for format 2.
    pub fn value_count(&self) -> u16 {
        match self.inner {
            SinglePosInner::Format1 { .. } => 1,
            SinglePosInner::Format2 { count, .. } => count,
        }
    }

    /// The positioning [`ValueRecord`] for `glyph_id`, or `None` if the
    /// glyph is not covered by this subtable.
    ///
    /// Format 1 returns the single shared record for any covered glyph;
    /// format 2 indexes its record array by the glyph's Coverage Index.
    pub fn value(&self, glyph_id: u16) -> Option<Result<ValueRecord, Error>> {
        let idx = self.coverage.index_of(glyph_id)?;
        match self.inner {
            SinglePosInner::Format1 { value_off } => {
                Some(ValueRecord::parse(self.bytes, value_off, self.value_format).map(|(r, _)| r))
            }
            SinglePosInner::Format2 { values_off, count } => {
                if idx >= count {
                    // Coverage index points past the value array: the
                    // font is malformed (spec requires valueCount ==
                    // coverage glyph count).
                    return Some(Err(Error::BadStructure(
                        "GPOS/SinglePos2: coverage index >= valueCount",
                    )));
                }
                let off = values_off + idx as usize * self.value_format.record_size();
                Some(ValueRecord::parse(self.bytes, off, self.value_format).map(|(r, _)| r))
            }
        }
    }

    /// Iterate `(glyph_id, ValueRecord)` for every covered glyph in
    /// ascending glyph-ID order.
    pub fn iter(&self) -> SinglePosIter<'a> {
        SinglePosIter {
            sub: *self,
            cov: self.coverage.iter(),
        }
    }
}

/// Iterator over the `(glyph_id, ValueRecord)` pairs of a [`SinglePos`].
#[derive(Debug, Clone)]
pub struct SinglePosIter<'a> {
    sub: SinglePos<'a>,
    cov: crate::tables::gdef::CoverageIter<'a>,
}

impl<'a> Iterator for SinglePosIter<'a> {
    type Item = (u16, Result<ValueRecord, Error>);
    fn next(&mut self) -> Option<Self::Item> {
        let (glyph, _idx) = self.cov.next()?;
        let val = self.sub.value(glyph)?;
        Some((glyph, val))
    }
}

/// A decoded pair of `ValueRecord`s — the positioning data a PairPos
/// subtable applies to one glyph pair.
///
/// `first` adjusts the first (left, in LTR) glyph and `second` adjusts
/// the second glyph. Either is the all-zero [`ValueRecord::default`]
/// when the corresponding `valueFormat` was zero (the spec's "empty
/// ValueRecord ⇒ glyph not repositioned").
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PairValue {
    /// Positioning data for the first glyph of the pair.
    pub first: ValueRecord,
    /// Positioning data for the second glyph of the pair.
    pub second: ValueRecord,
}

/// GPOS Lookup Type 2 — pair adjustment positioning subtable.
///
/// Spec: `docs/text/opentype/otspec-gpos.html` §"Lookup type 2
/// subtable: pair adjustment positioning". Two on-disk formats:
///
/// * **Format 1** — pairs identified individually by glyph index. The
///   Coverage table lists every first glyph; a parallel array of
///   PairSet tables holds, per first glyph, the `(secondGlyph,
///   valueRecord1, valueRecord2)` records sorted by `secondGlyph`.
/// * **Format 2** — pairs identified by glyph *class*. Two ClassDef
///   tables map first/second glyph to a class; a `class1Count ×
///   class2Count` matrix of `(valueRecord1, valueRecord2)` cells holds
///   the adjustment for every class pair.
///
/// Both formats share the two `valueFormat` fields (`valueFormat1` for
/// the first glyph, `valueFormat2` for the second); a zero
/// `valueFormat` means the corresponding `ValueRecord` is absent from
/// the on-disk layout and reads back as the all-zero record.
#[derive(Debug, Clone, Copy)]
pub struct PairPos<'a> {
    bytes: &'a [u8],
    coverage: Coverage<'a>,
    value_format1: ValueFormat,
    value_format2: ValueFormat,
    inner: PairPosInner<'a>,
}

#[derive(Debug, Clone, Copy)]
enum PairPosInner<'a> {
    /// Format 1: `pairSetOffsets[pairSetCount]` start at byte 10.
    Format1 { pair_set_count: u16 },
    /// Format 2: class-pair matrix.
    Format2 {
        class_def1: ClassDef<'a>,
        class_def2: ClassDef<'a>,
        class1_count: u16,
        class2_count: u16,
        /// Byte offset (from start of subtable) of class1Records[].
        matrix_off: usize,
    },
}

impl<'a> PairPos<'a> {
    /// Parse a PairPos subtable from its raw `bytes`.
    pub fn parse(bytes: &'a [u8]) -> Result<Self, Error> {
        let format = read_u16(bytes, 0)?;
        let coverage_off = read_u16(bytes, 2)? as usize;
        let value_format1 = ValueFormat(read_u16(bytes, 4)?);
        let value_format2 = ValueFormat(read_u16(bytes, 6)?);
        if !value_format1.is_valid() || !value_format2.is_valid() {
            return Err(Error::BadStructure(
                "GPOS/PairPos: reserved valueFormat bit set",
            ));
        }
        if coverage_off == 0 || coverage_off >= bytes.len() {
            return Err(Error::BadStructure(
                "GPOS/PairPos: coverageOffset out of range",
            ));
        }
        let coverage = Coverage::parse(&bytes[coverage_off..])?;

        match format {
            1 => {
                let pair_set_count = read_u16(bytes, 8)?;
                // pairSetOffsets[pairSetCount] occupy bytes 10..10+2*count.
                let need = 10usize
                    .checked_add(
                        (pair_set_count as usize)
                            .checked_mul(2)
                            .ok_or(Error::BadStructure("GPOS/PairPos: pairSetCount overflow"))?,
                    )
                    .ok_or(Error::BadStructure("GPOS/PairPos: length overflow"))?;
                if bytes.len() < need {
                    return Err(Error::UnexpectedEof);
                }
                // The spec requires one PairSet per Coverage glyph.
                if pair_set_count as usize != coverage.len() {
                    return Err(Error::BadStructure(
                        "GPOS/PairPos1: pairSetCount != coverage length",
                    ));
                }
                Ok(Self {
                    bytes,
                    coverage,
                    value_format1,
                    value_format2,
                    inner: PairPosInner::Format1 { pair_set_count },
                })
            }
            2 => {
                let class_def1_off = read_u16(bytes, 8)? as usize;
                let class_def2_off = read_u16(bytes, 10)? as usize;
                let class1_count = read_u16(bytes, 12)?;
                let class2_count = read_u16(bytes, 14)?;
                if class_def1_off == 0
                    || class_def1_off >= bytes.len()
                    || class_def2_off == 0
                    || class_def2_off >= bytes.len()
                {
                    return Err(Error::BadStructure(
                        "GPOS/PairPos2: classDefOffset out of range",
                    ));
                }
                let class_def1 = ClassDef::parse(&bytes[class_def1_off..])?;
                let class_def2 = ClassDef::parse(&bytes[class_def2_off..])?;
                let matrix_off = 16usize;
                // Each Class2 record holds valueRecord1 + valueRecord2.
                let cell_size = value_format1
                    .record_size()
                    .checked_add(value_format2.record_size())
                    .ok_or(Error::BadStructure("GPOS/PairPos2: cell size overflow"))?;
                let cells = (class1_count as usize)
                    .checked_mul(class2_count as usize)
                    .ok_or(Error::BadStructure("GPOS/PairPos2: class count overflow"))?;
                let matrix_bytes = cells
                    .checked_mul(cell_size)
                    .ok_or(Error::BadStructure("GPOS/PairPos2: matrix size overflow"))?;
                let need = matrix_off
                    .checked_add(matrix_bytes)
                    .ok_or(Error::BadStructure("GPOS/PairPos2: length overflow"))?;
                if bytes.len() < need {
                    return Err(Error::UnexpectedEof);
                }
                Ok(Self {
                    bytes,
                    coverage,
                    value_format1,
                    value_format2,
                    inner: PairPosInner::Format2 {
                        class_def1,
                        class_def2,
                        class1_count,
                        class2_count,
                        matrix_off,
                    },
                })
            }
            _ => Err(Error::BadStructure("GPOS/PairPos: unknown format")),
        }
    }

    /// Subtable format discriminant (1 or 2).
    pub fn format(&self) -> u16 {
        match self.inner {
            PairPosInner::Format1 { .. } => 1,
            PairPosInner::Format2 { .. } => 2,
        }
    }

    /// [`ValueFormat`] applied to the first glyph of every pair.
    pub fn value_format1(&self) -> ValueFormat {
        self.value_format1
    }

    /// [`ValueFormat`] applied to the second glyph of every pair.
    pub fn value_format2(&self) -> ValueFormat {
        self.value_format2
    }

    /// The subtable's [`Coverage`] table (lists every first glyph).
    pub fn coverage(&self) -> Coverage<'a> {
        self.coverage
    }

    /// The raw subtable bytes (index 0 = start of the PairPos
    /// subtable).
    pub fn raw(&self) -> &'a [u8] {
        self.bytes
    }

    /// The base offset (within [`PairPos::raw`]) that Device /
    /// VariationIndex offsets inside this subtable's `ValueRecord`s
    /// are measured from, for pairs whose first glyph is
    /// `first_glyph`. Per the spec's ValueRecord definition the
    /// "immediate parent table" is the PairPosFormat2 subtable itself
    /// (base 0) but the *PairSet* table within a PairPosFormat1
    /// subtable. `None` when `first_glyph` is not covered or the
    /// PairSet offset is out of range.
    pub fn value_device_base(&self, first_glyph: u16) -> Option<usize> {
        let idx = self.coverage.index_of(first_glyph)?;
        match self.inner {
            PairPosInner::Format1 { pair_set_count } => {
                if idx >= pair_set_count {
                    return None;
                }
                let off_pos = 10 + idx as usize * 2;
                let pair_set_off = read_u16(self.bytes, off_pos).ok()? as usize;
                if pair_set_off == 0 || pair_set_off >= self.bytes.len() {
                    return None;
                }
                Some(pair_set_off)
            }
            PairPosInner::Format2 { .. } => Some(0),
        }
    }

    /// Read the `(valueRecord1, valueRecord2)` pair at byte `off`,
    /// honouring the two value formats in field order. Returns the
    /// decoded [`PairValue`] and the total number of bytes consumed.
    fn read_pair(&self, off: usize) -> Result<(PairValue, usize), Error> {
        let (first, used1) = ValueRecord::parse(self.bytes, off, self.value_format1)?;
        let (second, used2) = ValueRecord::parse(self.bytes, off + used1, self.value_format2)?;
        Ok((PairValue { first, second }, used1 + used2))
    }

    /// Look up the positioning adjustment for the ordered pair
    /// `(first_glyph, second_glyph)`.
    ///
    /// Returns:
    /// * `None` — `first_glyph` is not covered, or (format 1) the pair
    ///   has no record in the first glyph's PairSet.
    /// * `Some(Err(_))` — the on-disk records are malformed.
    /// * `Some(Ok(PairValue))` — the decoded adjustment. For format 2,
    ///   a covered first glyph always yields a record (possibly the
    ///   all-zero cell for the `(class, class)` pair).
    pub fn pair(&self, first_glyph: u16, second_glyph: u16) -> Option<Result<PairValue, Error>> {
        let idx = self.coverage.index_of(first_glyph)?;
        match self.inner {
            PairPosInner::Format1 { pair_set_count } => {
                if idx >= pair_set_count {
                    return Some(Err(Error::BadStructure(
                        "GPOS/PairPos1: coverage index >= pairSetCount",
                    )));
                }
                let off_pos = 10 + idx as usize * 2;
                let pair_set_off = match read_u16(self.bytes, off_pos) {
                    Ok(v) => v as usize,
                    Err(e) => return Some(Err(e)),
                };
                if pair_set_off == 0 || pair_set_off >= self.bytes.len() {
                    return Some(Err(Error::BadStructure(
                        "GPOS/PairPos1: pairSetOffset out of range",
                    )));
                }
                self.pair_in_set(pair_set_off, second_glyph)
            }
            PairPosInner::Format2 {
                class_def1,
                class_def2,
                class1_count,
                class2_count,
                matrix_off,
            } => {
                let c1 = class_def1.class_of(first_glyph);
                let c2 = class_def2.class_of(second_glyph);
                if c1 >= class1_count || c2 >= class2_count {
                    // A glyph whose class is outside the declared matrix
                    // dimensions: the spec's matrix has no cell for it,
                    // so there is no adjustment.
                    return None;
                }
                let cell_size = self.value_format1.record_size() + self.value_format2.record_size();
                let cell_index = c1 as usize * class2_count as usize + c2 as usize;
                let off = matrix_off + cell_index * cell_size;
                Some(self.read_pair(off).map(|(pv, _)| pv))
            }
        }
    }

    /// Format-1 helper: binary-search a PairSet (sorted by `secondGlyph`)
    /// for `second_glyph`.
    fn pair_in_set(
        &self,
        pair_set_off: usize,
        second_glyph: u16,
    ) -> Option<Result<PairValue, Error>> {
        let count = match read_u16(self.bytes, pair_set_off) {
            Ok(v) => v as usize,
            Err(e) => return Some(Err(e)),
        };
        // Each PairValue record = secondGlyph(2) + valueRecord1 + valueRecord2.
        let rec_size = 2 + self.value_format1.record_size() + self.value_format2.record_size();
        let records_off = pair_set_off + 2;
        // Records are sorted by secondGlyph; binary-search.
        let (mut lo, mut hi) = (0usize, count);
        while lo < hi {
            let mid = (lo + hi) / 2;
            let rec_off = records_off + mid * rec_size;
            let sg = match read_u16(self.bytes, rec_off) {
                Ok(v) => v,
                Err(e) => return Some(Err(e)),
            };
            match sg.cmp(&second_glyph) {
                core::cmp::Ordering::Equal => {
                    return Some(self.read_pair(rec_off + 2).map(|(pv, _)| pv));
                }
                core::cmp::Ordering::Less => lo = mid + 1,
                core::cmp::Ordering::Greater => hi = mid,
            }
        }
        None
    }

    /// Iterate every `(first_glyph, second_glyph, PairValue)` triple this
    /// subtable defines, in ascending `(first_glyph, second_glyph)` order
    /// for format 1.
    ///
    /// Format 2 is a dense class matrix, not an enumeration of explicit
    /// glyph pairs, so its iterator yields nothing — use [`pair`] with a
    /// concrete glyph pair, or [`class_pair`] with class values, instead.
    ///
    /// [`pair`]: PairPos::pair
    /// [`class_pair`]: PairPos::class_pair
    pub fn iter(&self) -> PairPosIter<'a> {
        let cov = self.coverage.iter();
        PairPosIter {
            sub: *self,
            cov,
            cur_first: None,
            set_off: 0,
            set_count: 0,
            set_idx: 0,
        }
    }

    /// Format-2 only: look up the adjustment for an ordered *class* pair
    /// `(class1, class2)` directly, bypassing the ClassDef lookups.
    ///
    /// Returns `None` on a format-1 subtable or when either class index
    /// is outside the matrix dimensions.
    pub fn class_pair(&self, class1: u16, class2: u16) -> Option<Result<PairValue, Error>> {
        match self.inner {
            PairPosInner::Format2 {
                class1_count,
                class2_count,
                matrix_off,
                ..
            } => {
                if class1 >= class1_count || class2 >= class2_count {
                    return None;
                }
                let cell_size = self.value_format1.record_size() + self.value_format2.record_size();
                let cell_index = class1 as usize * class2_count as usize + class2 as usize;
                let off = matrix_off + cell_index * cell_size;
                Some(self.read_pair(off).map(|(pv, _)| pv))
            }
            PairPosInner::Format1 { .. } => None,
        }
    }
}

/// Iterator over the `(first_glyph, second_glyph, PairValue)` triples of
/// a format-1 [`PairPos`]. Empty for format-2 subtables.
#[derive(Debug, Clone)]
pub struct PairPosIter<'a> {
    sub: PairPos<'a>,
    cov: crate::tables::gdef::CoverageIter<'a>,
    /// The first glyph whose PairSet is currently being walked.
    cur_first: Option<u16>,
    /// Byte offset of the first PairValue record in the active PairSet.
    set_off: usize,
    /// `pairValueCount` of the active PairSet.
    set_count: usize,
    /// Index of the next record to emit within the active PairSet.
    set_idx: usize,
}

impl<'a> Iterator for PairPosIter<'a> {
    type Item = (u16, u16, Result<PairValue, Error>);

    fn next(&mut self) -> Option<Self::Item> {
        let pair_set_count = match self.sub.inner {
            PairPosInner::Format1 { pair_set_count } => pair_set_count,
            PairPosInner::Format2 { .. } => return None,
        };
        let rec_size =
            2 + self.sub.value_format1.record_size() + self.sub.value_format2.record_size();
        loop {
            // Emit the next record of the active PairSet, if any.
            if let Some(first) = self.cur_first {
                if self.set_idx < self.set_count {
                    let rec_off = self.set_off + self.set_idx * rec_size;
                    self.set_idx += 1;
                    let sg = match read_u16(self.sub.bytes, rec_off) {
                        Ok(v) => v,
                        Err(e) => return Some((first, 0, Err(e))),
                    };
                    let pv = self.sub.read_pair(rec_off + 2).map(|(pv, _)| pv);
                    return Some((first, sg, pv));
                }
                self.cur_first = None;
            }
            // Advance to the next covered first glyph and open its PairSet.
            let (glyph, idx) = self.cov.next()?;
            if idx >= pair_set_count {
                return Some((
                    glyph,
                    0,
                    Err(Error::BadStructure(
                        "GPOS/PairPos1: coverage index >= pairSetCount",
                    )),
                ));
            }
            let off_pos = 10 + idx as usize * 2;
            let pair_set_off = match read_u16(self.sub.bytes, off_pos) {
                Ok(v) => v as usize,
                Err(e) => return Some((glyph, 0, Err(e))),
            };
            if pair_set_off == 0 || pair_set_off >= self.sub.bytes.len() {
                return Some((
                    glyph,
                    0,
                    Err(Error::BadStructure(
                        "GPOS/PairPos1: pairSetOffset out of range",
                    )),
                ));
            }
            let count = match read_u16(self.sub.bytes, pair_set_off) {
                Ok(v) => v as usize,
                Err(e) => return Some((glyph, 0, Err(e))),
            };
            self.cur_first = Some(glyph);
            self.set_off = pair_set_off + 2;
            self.set_count = count;
            self.set_idx = 0;
        }
    }
}

/// Parsed `ExtensionPos` subtable — the GPOS `lookupType = 9` payload.
///
/// Spec: `docs/text/opentype/otspec-gpos.html` §"Lookup type 9 subtable:
/// positioning subtable extension".
///
/// Like the GSUB type-7 extension, this lookup type is a *format
/// extension mechanism*, not a positioning action: it lets a Lookup
/// reach its real subtable through a 32-bit offset, for fonts whose
/// accumulated subtable sizes exceed what the usual 16-bit offsets can
/// address. The spec's processing model: proceed as though the Lookup's
/// `lookupType` were the `extensionLookupType` of the subtables, and as
/// though each extension subtable referenced by `extensionOffset`
/// replaced the type-9 subtable that referenced it.
///
/// One on-disk format is defined.
///
/// ```text
/// PosExtensionFormat1 subtable (8 bytes)
///   0 / 2 / format = 1
///   2 / 2 / extensionLookupType   (any GposLookupType other than 9)
///   4 / 4 / extensionOffset       (Offset32, from start of this
///                                  PosExtensionFormat1 subtable)
/// ```
///
/// Parse-time validation: `format == 1`; `extensionLookupType` must be a
/// defined GposLookupType (`1..=8`) **other than 9** (the spec forbids an
/// extension pointing at another extension); and `extensionOffset`
/// (relative to the start of the PosExtensionFormat1 subtable) must be
/// non-NULL and land inside the supplied byte window. The wrapped
/// subtable is surfaced both raw ([`Self::extension_subtable_bytes`]) and
/// through typed resolvers for the positioning lookup types this crate
/// already decodes ([`Self::as_single_pos`] / [`Self::as_pair_pos`]).
#[derive(Debug, Clone, Copy)]
pub struct ExtensionPos<'a> {
    /// Raw subtable bytes (`extensionOffset` is relative to this
    /// buffer's start).
    bytes: &'a [u8],
    ext_lookup_type: u16,
    ext_offset: u32,
}

impl<'a> ExtensionPos<'a> {
    /// Parse a PosExtensionFormat1 subtable from a buffer whose first two
    /// bytes are the `format` identifier.
    pub fn parse(bytes: &'a [u8]) -> Result<Self, Error> {
        let format = read_u16(bytes, 0)?;
        if format != 1 {
            return Err(Error::BadStructure(
                "GPOS/ExtensionPos: unknown subtable format",
            ));
        }
        let ext_lookup_type = read_u16(bytes, 2)?;
        // Spec: "The extensionLookupType field must be set to any lookup
        // type other than 9." The GposLookupType vocabulary is 1..=8 (9
        // being the extension itself), so anything outside that range is
        // equally undefined.
        if ext_lookup_type == GPOS_LOOKUP_TYPE_EXTENSION {
            return Err(Error::BadStructure(
                "GPOS/ExtensionPos: extensionLookupType must not be 9",
            ));
        }
        if !(GPOS_LOOKUP_TYPE_SINGLE..=GPOS_LOOKUP_TYPE_CHAINED_CONTEXT).contains(&ext_lookup_type)
        {
            return Err(Error::BadStructure(
                "GPOS/ExtensionPos: extensionLookupType out of range",
            ));
        }
        let ext_offset = read_u32(bytes, 4)?;
        // "All offsets to extension subtables are set in the usual
        // way—that is, relative to the start of the PosExtensionFormat1
        // subtable." A NULL offset has no defined meaning here, and the
        // wrapped subtable must start inside the byte window.
        if ext_offset == 0 || (ext_offset as usize) >= bytes.len() {
            return Err(Error::BadStructure(
                "GPOS/ExtensionPos: extensionOffset out of range",
            ));
        }
        Ok(Self {
            bytes,
            ext_lookup_type,
            ext_offset,
        })
    }

    /// Subtable format discriminant (always `1`).
    pub fn format(&self) -> u16 {
        1
    }

    /// `extensionLookupType` — the lookup type of the wrapped subtable.
    /// Guaranteed to be in `1..=8` and never `9`.
    pub fn extension_lookup_type(&self) -> u16 {
        self.ext_lookup_type
    }

    /// `extensionOffset` — byte offset of the wrapped subtable, relative
    /// to the start of this PosExtensionFormat1 subtable.
    pub fn extension_offset(&self) -> u32 {
        self.ext_offset
    }

    /// Raw bytes of the wrapped ("extension") subtable, starting at
    /// `extensionOffset`. Feed these to the typed parser matching
    /// [`Self::extension_lookup_type`] — or use the `as_*` resolvers
    /// below for the lookup types this crate already decodes.
    pub fn extension_subtable_bytes(&self) -> &'a [u8] {
        &self.bytes[self.ext_offset as usize..]
    }

    /// Resolve the wrapped subtable as a [`SinglePos`]
    /// (`extensionLookupType = 1`). `Err(BadStructure)` when the declared
    /// type disagrees or the wrapped bytes are malformed.
    pub fn as_single_pos(&self) -> Result<SinglePos<'a>, Error> {
        if self.ext_lookup_type != GPOS_LOOKUP_TYPE_SINGLE {
            return Err(Error::BadStructure(
                "GPOS/ExtensionPos: extensionLookupType is not 1",
            ));
        }
        SinglePos::parse(self.extension_subtable_bytes())
    }

    /// Resolve the wrapped subtable as a [`PairPos`]
    /// (`extensionLookupType = 2`). `Err(BadStructure)` when the declared
    /// type disagrees or the wrapped bytes are malformed.
    pub fn as_pair_pos(&self) -> Result<PairPos<'a>, Error> {
        if self.ext_lookup_type != GPOS_LOOKUP_TYPE_PAIR {
            return Err(Error::BadStructure(
                "GPOS/ExtensionPos: extensionLookupType is not 2",
            ));
        }
        PairPos::parse(self.extension_subtable_bytes())
    }

    /// Resolve the wrapped subtable as a [`MarkBasePos`]
    /// (`extensionLookupType = 4`). `Err(BadStructure)` when the declared
    /// type disagrees or the wrapped bytes are malformed.
    pub fn as_mark_base_pos(&self) -> Result<MarkBasePos<'a>, Error> {
        if self.ext_lookup_type != GPOS_LOOKUP_TYPE_MARK_TO_BASE {
            return Err(Error::BadStructure(
                "GPOS/ExtensionPos: extensionLookupType is not 4",
            ));
        }
        MarkBasePos::parse(self.extension_subtable_bytes())
    }

    /// Resolve the wrapped subtable as a [`CursivePos`]
    /// (`extensionLookupType = 3`). `Err(BadStructure)` when the declared
    /// type disagrees or the wrapped bytes are malformed.
    pub fn as_cursive_pos(&self) -> Result<CursivePos<'a>, Error> {
        if self.ext_lookup_type != GPOS_LOOKUP_TYPE_CURSIVE {
            return Err(Error::BadStructure(
                "GPOS/ExtensionPos: extensionLookupType is not 3",
            ));
        }
        CursivePos::parse(self.extension_subtable_bytes())
    }

    /// Resolve the wrapped subtable as a [`MarkMarkPos`]
    /// (`extensionLookupType = 6`). `Err(BadStructure)` when the declared
    /// type disagrees or the wrapped bytes are malformed.
    pub fn as_mark_mark_pos(&self) -> Result<MarkMarkPos<'a>, Error> {
        if self.ext_lookup_type != GPOS_LOOKUP_TYPE_MARK_TO_MARK {
            return Err(Error::BadStructure(
                "GPOS/ExtensionPos: extensionLookupType is not 6",
            ));
        }
        MarkMarkPos::parse(self.extension_subtable_bytes())
    }

    /// Resolve the wrapped subtable as a [`MarkLigPos`]
    /// (`extensionLookupType = 5`). `Err(BadStructure)` when the declared
    /// type disagrees or the wrapped bytes are malformed.
    pub fn as_mark_lig_pos(&self) -> Result<MarkLigPos<'a>, Error> {
        if self.ext_lookup_type != GPOS_LOOKUP_TYPE_MARK_TO_LIGATURE {
            return Err(Error::BadStructure(
                "GPOS/ExtensionPos: extensionLookupType is not 5",
            ));
        }
        MarkLigPos::parse(self.extension_subtable_bytes())
    }

    /// Resolve the wrapped subtable as a [`SequenceContext`]
    /// (`extensionLookupType = 7`). `Err(BadStructure)` when the declared
    /// type disagrees or the wrapped bytes are malformed.
    pub fn as_context_pos(&self) -> Result<SequenceContext<'a>, Error> {
        if self.ext_lookup_type != GPOS_LOOKUP_TYPE_CONTEXT {
            return Err(Error::BadStructure(
                "GPOS/ExtensionPos: extensionLookupType is not 7",
            ));
        }
        SequenceContext::parse(self.extension_subtable_bytes())
    }

    /// Resolve the wrapped subtable as a [`ChainedSequenceContext`]
    /// (`extensionLookupType = 8`). `Err(BadStructure)` when the declared
    /// type disagrees or the wrapped bytes are malformed.
    pub fn as_chained_context_pos(&self) -> Result<ChainedSequenceContext<'a>, Error> {
        if self.ext_lookup_type != GPOS_LOOKUP_TYPE_CHAINED_CONTEXT {
            return Err(Error::BadStructure(
                "GPOS/ExtensionPos: extensionLookupType is not 8",
            ));
        }
        ChainedSequenceContext::parse(self.extension_subtable_bytes())
    }
}

/// A decoded `Anchor` table — one attachment point used by the GPOS
/// mark-attachment and cursive lookups.
///
/// Spec: `docs/text/opentype/otspec-gpos.html` §"Anchor Tables". Three
/// on-disk formats share the `(xCoordinate, yCoordinate)` design-unit
/// pair; the later two add refinement data:
///
/// * **Format 1** — design units only.
/// * **Format 2** — design units plus an `anchorPoint` index into the
///   glyph's contour points (a hinting refinement). The contour-point
///   index is surfaced via [`Anchor::contour_point`].
/// * **Format 3** — design units plus `Offset16` references to Device /
///   VariationIndex tables for X and Y (each may be NULL). The raw
///   offsets are surfaced via [`Anchor::x_device_offset`] /
///   [`Anchor::y_device_offset`], and the referenced tables are decoded
///   by [`Anchor::x_device`] / [`Anchor::y_device`] (given the Anchor
///   table's byte slice).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Anchor {
    /// Subtable format discriminant (1, 2, or 3).
    pub format: u16,
    /// Horizontal anchor coordinate, design units.
    pub x: i16,
    /// Vertical anchor coordinate, design units.
    pub y: i16,
    /// Format-2 `anchorPoint` — index of the on-outline contour point
    /// the anchor is pinned to. `None` for formats 1 and 3.
    pub anchor_point: Option<u16>,
    /// Format-3 raw `xDeviceOffset` (`0` = NULL; always `0` for formats
    /// 1 and 2). Relative to the start of the Anchor table.
    pub x_device_offset: u16,
    /// Format-3 raw `yDeviceOffset` (`0` = NULL; always `0` for formats
    /// 1 and 2). Relative to the start of the Anchor table.
    pub y_device_offset: u16,
    /// Byte offset of this Anchor table within the slice it was
    /// parsed from (the `off` argument of [`Anchor::parse`]) — for
    /// anchors decoded by the GPOS typed views this is the offset
    /// within the owning subtable, letting a client rebase the
    /// format-3 Device / VariationIndex offsets (which are relative
    /// to the Anchor table start).
    pub table_offset: usize,
}

impl Anchor {
    /// Parse an Anchor table from `data` starting at byte `off`.
    ///
    /// The format identifier is validated against the three defined
    /// values; an unknown format is rejected as `BadStructure`.
    pub fn parse(data: &[u8], off: usize) -> Result<Self, Error> {
        let format = read_u16(data, off)?;
        let x = read_i16(data, off + 2)?;
        let y = read_i16(data, off + 4)?;
        match format {
            1 => Ok(Anchor {
                format,
                x,
                y,
                table_offset: off,
                ..Anchor::default()
            }),
            2 => {
                let anchor_point = read_u16(data, off + 6)?;
                Ok(Anchor {
                    format,
                    x,
                    y,
                    anchor_point: Some(anchor_point),
                    table_offset: off,
                    ..Anchor::default()
                })
            }
            3 => {
                let x_device_offset = read_u16(data, off + 6)?;
                let y_device_offset = read_u16(data, off + 8)?;
                Ok(Anchor {
                    format,
                    x,
                    y,
                    anchor_point: None,
                    x_device_offset,
                    y_device_offset,
                    table_offset: off,
                })
            }
            _ => Err(Error::BadStructure("GPOS/Anchor: unknown format")),
        }
    }

    /// Format-2 contour-point refinement index, if present.
    pub fn contour_point(&self) -> Option<u16> {
        self.anchor_point
    }

    /// Raw format-3 `xDeviceOffset` (`0` = NULL).
    pub fn x_device_offset(&self) -> u16 {
        self.x_device_offset
    }

    /// Raw format-3 `yDeviceOffset` (`0` = NULL).
    pub fn y_device_offset(&self) -> u16 {
        self.y_device_offset
    }

    /// Decode the format-3 X Device / VariationIndex table, if present.
    ///
    /// `anchor_table` is the byte slice whose index 0 is the start of
    /// this Anchor table — i.e. `&data[off..]` for the `(data, off)`
    /// pair passed to [`Anchor::parse`] (the `xDeviceOffset` is relative
    /// to the Anchor table start). Returns `None` when the offset is
    /// NULL or the anchor is not format 3; `Some(Err(..))` when the
    /// referenced bytes are malformed.
    pub fn x_device<'a>(
        &self,
        anchor_table: &'a [u8],
    ) -> Option<Result<DeviceOrVariationIndex<'a>, Error>> {
        decode_device_at(anchor_table, self.x_device_offset)
    }

    /// Decode the format-3 Y Device / VariationIndex table, if present.
    /// See [`Anchor::x_device`] for the `anchor_table` convention.
    pub fn y_device<'a>(
        &self,
        anchor_table: &'a [u8],
    ) -> Option<Result<DeviceOrVariationIndex<'a>, Error>> {
        decode_device_at(anchor_table, self.y_device_offset)
    }
}

/// Decode a Device / VariationIndex table at `offset` within `base`.
/// `None` for a NULL (`0`) or out-of-range offset; `Some(Err(..))` for
/// malformed bytes.
fn decode_device_at(base: &[u8], offset: u16) -> Option<Result<DeviceOrVariationIndex<'_>, Error>> {
    if offset == 0 {
        return None;
    }
    let off = offset as usize;
    if off >= base.len() {
        return Some(Err(Error::BadStructure("GPOS/Device: offset out of range")));
    }
    Some(DeviceOrVariationIndex::parse(&base[off..]))
}

/// A decoded `MarkRecord` — the class and anchor of one mark glyph.
///
/// Spec: `docs/text/opentype/otspec-gpos.html` §"Mark array table". A
/// MarkRecord is `{ uint16 markClass; Offset16 markAnchorOffset }`; the
/// offset is resolved to a fully-decoded [`Anchor`] at parse time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MarkRecord {
    /// Mark-class index this mark belongs to (`0..markClassCount`).
    pub mark_class: u16,
    /// The mark's attachment-point [`Anchor`].
    pub anchor: Anchor,
}

/// The attachment geometry a [`MarkBasePos`] subtable computes for a
/// `(mark, base)` glyph pair: the mark's own anchor and the base anchor
/// for that mark's class.
///
/// A text-processing client aligns `mark_anchor` over `base_anchor`,
/// positioning the mark relative to the base glyph's final pen point
/// (spec §"Lookup type 4 subtable"). Either anchor may carry a
/// format-2/3 refinement; see [`Anchor`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MarkAttachment {
    /// The mark glyph's class (`0..markClassCount`).
    pub mark_class: u16,
    /// The mark glyph's attachment anchor.
    pub mark_anchor: Anchor,
    /// The base glyph's anchor for the mark's class.
    pub base_anchor: Anchor,
}

/// GPOS Lookup Type 4 — mark-to-base attachment positioning subtable.
///
/// Spec: `docs/text/opentype/otspec-gpos.html` §"Lookup type 4 subtable:
/// mark-to-base attachment positioning". One on-disk format,
/// `MarkBasePosFormat1`:
///
/// ```text
/// MarkBasePosFormat1 subtable (12 bytes)
///   0 / 2 / format = 1
///   2 / 2 / markCoverageOffset  (Offset16, from start of subtable)
///   4 / 2 / baseCoverageOffset  (Offset16, from start of subtable)
///   6 / 2 / markClassCount
///   8 / 2 / markArrayOffset     (Offset16, from start of subtable)
///  10 / 2 / baseArrayOffset     (Offset16, from start of subtable)
/// ```
///
/// The MarkArray holds one [`MarkRecord`] per mark-Coverage glyph (in
/// Coverage order); the BaseArray holds, per base-Coverage glyph, an
/// array of `markClassCount` [`Anchor`] offsets (the BaseRecord). To
/// attach a mark to a base, the mark's class selects which base anchor
/// aligns with the mark's anchor — see [`Self::attachment`].
#[derive(Debug, Clone, Copy)]
pub struct MarkBasePos<'a> {
    bytes: &'a [u8],
    mark_coverage: Coverage<'a>,
    base_coverage: Coverage<'a>,
    mark_class_count: u16,
    mark_array_off: usize,
    base_array_off: usize,
    base_count: u16,
}

impl<'a> MarkBasePos<'a> {
    /// Parse a MarkBasePosFormat1 subtable from its raw `bytes`.
    pub fn parse(bytes: &'a [u8]) -> Result<Self, Error> {
        let format = read_u16(bytes, 0)?;
        if format != 1 {
            return Err(Error::BadStructure("GPOS/MarkBasePos: unknown format"));
        }
        let mark_coverage_off = read_u16(bytes, 2)? as usize;
        let base_coverage_off = read_u16(bytes, 4)? as usize;
        let mark_class_count = read_u16(bytes, 6)?;
        let mark_array_off = read_u16(bytes, 8)? as usize;
        let base_array_off = read_u16(bytes, 10)? as usize;
        if mark_coverage_off == 0
            || mark_coverage_off >= bytes.len()
            || base_coverage_off == 0
            || base_coverage_off >= bytes.len()
        {
            return Err(Error::BadStructure(
                "GPOS/MarkBasePos: coverageOffset out of range",
            ));
        }
        if mark_array_off == 0
            || mark_array_off >= bytes.len()
            || base_array_off == 0
            || base_array_off >= bytes.len()
        {
            return Err(Error::BadStructure(
                "GPOS/MarkBasePos: arrayOffset out of range",
            ));
        }
        if mark_class_count == 0 {
            return Err(Error::BadStructure(
                "GPOS/MarkBasePos: markClassCount is zero",
            ));
        }
        let mark_coverage = Coverage::parse(&bytes[mark_coverage_off..])?;
        let base_coverage = Coverage::parse(&bytes[base_coverage_off..])?;
        // baseCount is the first uint16 of the BaseArray table; each
        // BaseRecord is `markClassCount` Offset16 anchor offsets.
        let base_count = read_u16(bytes, base_array_off)?;
        // Validate the BaseArray extent: baseCount records of
        // markClassCount Offset16s each, following the 2-byte baseCount.
        let record_size = (mark_class_count as usize)
            .checked_mul(2)
            .ok_or(Error::BadStructure(
                "GPOS/MarkBasePos: record size overflow",
            ))?;
        let base_array_bytes =
            (base_count as usize)
                .checked_mul(record_size)
                .ok_or(Error::BadStructure(
                    "GPOS/MarkBasePos: BaseArray size overflow",
                ))?;
        let need = base_array_off
            .checked_add(2)
            .and_then(|v| v.checked_add(base_array_bytes))
            .ok_or(Error::BadStructure(
                "GPOS/MarkBasePos: BaseArray extent overflow",
            ))?;
        if need > bytes.len() {
            return Err(Error::UnexpectedEof);
        }
        Ok(Self {
            bytes,
            mark_coverage,
            base_coverage,
            mark_class_count,
            mark_array_off,
            base_array_off,
            base_count,
        })
    }

    /// Subtable format discriminant (always `1`).
    pub fn format(&self) -> u16 {
        1
    }

    /// `markClassCount` — number of distinct mark classes.
    pub fn mark_class_count(&self) -> u16 {
        self.mark_class_count
    }

    /// The mark [`Coverage`] table (lists every mark glyph).
    pub fn mark_coverage(&self) -> Coverage<'a> {
        self.mark_coverage
    }

    /// The base [`Coverage`] table (lists every base glyph).
    pub fn base_coverage(&self) -> Coverage<'a> {
        self.base_coverage
    }

    /// The raw subtable bytes (index 0 = start of the MarkBasePos
    /// subtable) — [`Anchor::table_offset`] values from this
    /// subtable's anchors index into it.
    pub fn raw(&self) -> &'a [u8] {
        self.bytes
    }

    /// The decoded [`MarkRecord`] for `mark_glyph`, or `None` if the
    /// glyph is not in the mark Coverage table.
    ///
    /// The MarkArray's `markCount` is required by the spec to equal the
    /// mark-Coverage glyph count, so the Coverage Index directly indexes
    /// the record array.
    pub fn mark_record(&self, mark_glyph: u16) -> Option<Result<MarkRecord, Error>> {
        let idx = self.mark_coverage.index_of(mark_glyph)?;
        Some(self.mark_record_at(idx))
    }

    /// Resolve MarkRecord at mark-Coverage index `idx`.
    fn mark_record_at(&self, idx: u16) -> Result<MarkRecord, Error> {
        // MarkArray: uint16 markCount, then markCount MarkRecords of
        // 4 bytes each (uint16 markClass + Offset16 markAnchorOffset),
        // the offset being from the start of the MarkArray table.
        let mark_count = read_u16(self.bytes, self.mark_array_off)?;
        if idx >= mark_count {
            return Err(Error::BadStructure(
                "GPOS/MarkBasePos: mark coverage index >= markCount",
            ));
        }
        let rec_off = self.mark_array_off + 2 + idx as usize * 4;
        let mark_class = read_u16(self.bytes, rec_off)?;
        if mark_class >= self.mark_class_count {
            return Err(Error::BadStructure(
                "GPOS/MarkBasePos: markClass >= markClassCount",
            ));
        }
        let anchor_off = read_u16(self.bytes, rec_off + 2)? as usize;
        // A NULL markAnchorOffset is not meaningful for a mark (the spec
        // requires every mark to have an anchor).
        if anchor_off == 0 {
            return Err(Error::BadStructure(
                "GPOS/MarkBasePos: NULL mark anchor offset",
            ));
        }
        let anchor = Anchor::parse(self.bytes, self.mark_array_off + anchor_off)?;
        Ok(MarkRecord { mark_class, anchor })
    }

    /// The base [`Anchor`] for base glyph `base_glyph` and mark class
    /// `mark_class`.
    ///
    /// Returns:
    /// * `None` — `base_glyph` is not in the base Coverage table.
    /// * `Some(Ok(None))` — the BaseRecord's anchor offset for that class
    ///   is NULL (the spec permits a base to omit an anchor for a class,
    ///   in which case no adjustment is applied for marks of that class).
    /// * `Some(Ok(Some(Anchor)))` — the decoded base anchor.
    pub fn base_anchor(
        &self,
        base_glyph: u16,
        mark_class: u16,
    ) -> Option<Result<Option<Anchor>, Error>> {
        let idx = self.base_coverage.index_of(base_glyph)?;
        Some(self.base_anchor_at(idx, mark_class))
    }

    /// Resolve the base anchor at base-Coverage index `idx` for
    /// `mark_class`.
    fn base_anchor_at(&self, idx: u16, mark_class: u16) -> Result<Option<Anchor>, Error> {
        if idx >= self.base_count {
            return Err(Error::BadStructure(
                "GPOS/MarkBasePos: base coverage index >= baseCount",
            ));
        }
        if mark_class >= self.mark_class_count {
            return Err(Error::BadStructure(
                "GPOS/MarkBasePos: markClass >= markClassCount",
            ));
        }
        // BaseArray: uint16 baseCount, then baseCount BaseRecords; each
        // BaseRecord is markClassCount Offset16 anchor offsets, the
        // offsets being from the start of the BaseArray table.
        let record_size = self.mark_class_count as usize * 2;
        let rec_off = self.base_array_off + 2 + idx as usize * record_size;
        let anchor_off = read_u16(self.bytes, rec_off + mark_class as usize * 2)? as usize;
        if anchor_off == 0 {
            return Ok(None);
        }
        let anchor = Anchor::parse(self.bytes, self.base_array_off + anchor_off)?;
        Ok(Some(anchor))
    }

    /// Compute the attachment geometry for the ordered pair
    /// `(mark_glyph, base_glyph)`.
    ///
    /// Returns:
    /// * `None` — the mark is not covered, the base is not covered, or
    ///   the base has no (non-NULL) anchor for the mark's class (no
    ///   adjustment applies).
    /// * `Some(Err(_))` — the on-disk records are malformed.
    /// * `Some(Ok(MarkAttachment))` — the mark + base anchors a shaper
    ///   aligns to position the mark over the base.
    pub fn attachment(
        &self,
        mark_glyph: u16,
        base_glyph: u16,
    ) -> Option<Result<MarkAttachment, Error>> {
        let mark = match self.mark_record(mark_glyph)? {
            Ok(m) => m,
            Err(e) => return Some(Err(e)),
        };
        let base = match self.base_anchor(base_glyph, mark.mark_class)? {
            Ok(Some(a)) => a,
            Ok(None) => return None,
            Err(e) => return Some(Err(e)),
        };
        Some(Ok(MarkAttachment {
            mark_class: mark.mark_class,
            mark_anchor: mark.anchor,
            base_anchor: base,
        }))
    }
}

/// The attachment geometry a [`MarkLigPos`] subtable computes for a
/// `(mark, ligature, component)` triple: the mark's own anchor and the
/// ligature-component anchor for that mark's class.
///
/// A text-processing client aligns `mark_anchor` over `ligature_anchor`,
/// positioning the combining mark relative to the identified ligature
/// component's attachment point (spec §"Lookup type 5 subtable"). The
/// roles mirror [`MarkAttachment`] exactly, except the base anchor is
/// chosen by `(component, mark_class)` rather than `mark_class` alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LigatureAttachment {
    /// The mark glyph's class (`0..markClassCount`).
    pub mark_class: u16,
    /// The ligature component the mark attaches to (`0..componentCount`).
    pub component: u16,
    /// The mark glyph's attachment anchor.
    pub mark_anchor: Anchor,
    /// The ligature component's anchor for the mark's class.
    pub ligature_anchor: Anchor,
}

/// GPOS Lookup Type 5 — mark-to-ligature attachment positioning subtable.
///
/// Spec: `docs/text/opentype/otspec-gpos.html` §"Lookup type 5 subtable:
/// mark-to-ligature attachment positioning". One on-disk format,
/// `MarkLigPosFormat1`:
///
/// ```text
/// MarkLigPosFormat1 subtable (12 bytes)
///   0 / 2 / format = 1
///   2 / 2 / markCoverageOffset     (Offset16, from start of subtable)
///   4 / 2 / ligatureCoverageOffset (Offset16, from start of subtable)
///   6 / 2 / markClassCount
///   8 / 2 / markArrayOffset        (Offset16, from start of subtable)
///  10 / 2 / ligatureArrayOffset    (Offset16, from start of subtable)
/// ```
///
/// The MarkArray mirrors [`MarkBasePos`] precisely: one [`MarkRecord`]
/// per mark-Coverage glyph (class + anchor). The difference is the base
/// side, which is *two-dimensional*. The LigatureArray holds a
/// `ligatureCount` and one Offset16 per ligature-Coverage glyph (in
/// Coverage order) to a LigatureAttach table. A LigatureAttach is a
/// `componentCount` followed by `componentCount` ComponentRecords, each
/// of which is `markClassCount` Offset16 anchor offsets (one per mark
/// class, any of which may be NULL). To attach a mark to a ligature, the
/// caller supplies the component index (which the spec notes must be
/// tracked by the client from the original character string) and the
/// mark's class selects which component anchor aligns with the mark's
/// anchor — see [`Self::attachment`].
#[derive(Debug, Clone, Copy)]
pub struct MarkLigPos<'a> {
    bytes: &'a [u8],
    mark_coverage: Coverage<'a>,
    ligature_coverage: Coverage<'a>,
    mark_class_count: u16,
    mark_array_off: usize,
    ligature_array_off: usize,
    ligature_count: u16,
}

impl<'a> MarkLigPos<'a> {
    /// Parse a MarkLigPosFormat1 subtable from its raw `bytes`.
    pub fn parse(bytes: &'a [u8]) -> Result<Self, Error> {
        let format = read_u16(bytes, 0)?;
        if format != 1 {
            return Err(Error::BadStructure("GPOS/MarkLigPos: unknown format"));
        }
        let mark_coverage_off = read_u16(bytes, 2)? as usize;
        let ligature_coverage_off = read_u16(bytes, 4)? as usize;
        let mark_class_count = read_u16(bytes, 6)?;
        let mark_array_off = read_u16(bytes, 8)? as usize;
        let ligature_array_off = read_u16(bytes, 10)? as usize;
        if mark_coverage_off == 0
            || mark_coverage_off >= bytes.len()
            || ligature_coverage_off == 0
            || ligature_coverage_off >= bytes.len()
        {
            return Err(Error::BadStructure(
                "GPOS/MarkLigPos: coverageOffset out of range",
            ));
        }
        if mark_array_off == 0
            || mark_array_off >= bytes.len()
            || ligature_array_off == 0
            || ligature_array_off >= bytes.len()
        {
            return Err(Error::BadStructure(
                "GPOS/MarkLigPos: arrayOffset out of range",
            ));
        }
        if mark_class_count == 0 {
            return Err(Error::BadStructure(
                "GPOS/MarkLigPos: markClassCount is zero",
            ));
        }
        let mark_coverage = Coverage::parse(&bytes[mark_coverage_off..])?;
        let ligature_coverage = Coverage::parse(&bytes[ligature_coverage_off..])?;
        // ligatureCount is the first uint16 of the LigatureArray table; it
        // is followed by ligatureCount Offset16s to LigatureAttach tables.
        let ligature_count = read_u16(bytes, ligature_array_off)?;
        // Validate the LigatureArray offset-array extent: ligatureCount
        // Offset16s following the 2-byte ligatureCount.
        let offsets_bytes = (ligature_count as usize)
            .checked_mul(2)
            .ok_or(Error::BadStructure(
                "GPOS/MarkLigPos: LigatureArray size overflow",
            ))?;
        let need = ligature_array_off
            .checked_add(2)
            .and_then(|v| v.checked_add(offsets_bytes))
            .ok_or(Error::BadStructure(
                "GPOS/MarkLigPos: LigatureArray extent overflow",
            ))?;
        if need > bytes.len() {
            return Err(Error::UnexpectedEof);
        }
        Ok(Self {
            bytes,
            mark_coverage,
            ligature_coverage,
            mark_class_count,
            mark_array_off,
            ligature_array_off,
            ligature_count,
        })
    }

    /// Subtable format discriminant (always `1`).
    pub fn format(&self) -> u16 {
        1
    }

    /// `markClassCount` — number of distinct mark classes.
    pub fn mark_class_count(&self) -> u16 {
        self.mark_class_count
    }

    /// The mark [`Coverage`] table (lists every mark glyph).
    pub fn mark_coverage(&self) -> Coverage<'a> {
        self.mark_coverage
    }

    /// The ligature [`Coverage`] table (lists every ligature glyph).
    pub fn ligature_coverage(&self) -> Coverage<'a> {
        self.ligature_coverage
    }

    /// The raw subtable bytes (index 0 = start of the MarkLigPos
    /// subtable).
    pub fn raw(&self) -> &'a [u8] {
        self.bytes
    }

    /// The decoded [`MarkRecord`] for `mark_glyph`, or `None` if the
    /// glyph is not in the mark Coverage table.
    ///
    /// The MarkArray's `markCount` is required by the spec to equal the
    /// mark-Coverage glyph count, so the Coverage Index directly indexes
    /// the record array.
    pub fn mark_record(&self, mark_glyph: u16) -> Option<Result<MarkRecord, Error>> {
        let idx = self.mark_coverage.index_of(mark_glyph)?;
        Some(self.mark_record_at(idx))
    }

    /// Resolve MarkRecord at mark-Coverage index `idx`.
    fn mark_record_at(&self, idx: u16) -> Result<MarkRecord, Error> {
        // MarkArray: uint16 markCount, then markCount MarkRecords of
        // 4 bytes each (uint16 markClass + Offset16 markAnchorOffset),
        // the offset being from the start of the MarkArray table.
        let mark_count = read_u16(self.bytes, self.mark_array_off)?;
        if idx >= mark_count {
            return Err(Error::BadStructure(
                "GPOS/MarkLigPos: mark coverage index >= markCount",
            ));
        }
        let rec_off = self.mark_array_off + 2 + idx as usize * 4;
        let mark_class = read_u16(self.bytes, rec_off)?;
        if mark_class >= self.mark_class_count {
            return Err(Error::BadStructure(
                "GPOS/MarkLigPos: markClass >= markClassCount",
            ));
        }
        let anchor_off = read_u16(self.bytes, rec_off + 2)? as usize;
        // A NULL markAnchorOffset is not meaningful for a mark (the spec
        // requires every mark to have an anchor).
        if anchor_off == 0 {
            return Err(Error::BadStructure(
                "GPOS/MarkLigPos: NULL mark anchor offset",
            ));
        }
        let anchor = Anchor::parse(self.bytes, self.mark_array_off + anchor_off)?;
        Ok(MarkRecord { mark_class, anchor })
    }

    /// `componentCount` for the ligature glyph `lig_glyph` — the number of
    /// (virtual) components the ligature carries attachment data for.
    ///
    /// Returns:
    /// * `None` — `lig_glyph` is not in the ligature Coverage table.
    /// * `Some(Err(_))` — the on-disk records are malformed.
    /// * `Some(Ok(count))` — the component count.
    pub fn component_count(&self, lig_glyph: u16) -> Option<Result<u16, Error>> {
        let idx = self.ligature_coverage.index_of(lig_glyph)?;
        Some(self.component_count_at(idx))
    }

    /// Resolve the byte offset of the LigatureAttach table at
    /// ligature-Coverage index `idx`.
    fn ligature_attach_off(&self, idx: u16) -> Result<usize, Error> {
        if idx >= self.ligature_count {
            return Err(Error::BadStructure(
                "GPOS/MarkLigPos: ligature coverage index >= ligatureCount",
            ));
        }
        // LigatureArray: uint16 ligatureCount, then ligatureCount Offset16s
        // to LigatureAttach tables, the offsets being from the start of
        // the LigatureArray table.
        let off_field = self.ligature_array_off + 2 + idx as usize * 2;
        let attach_off = read_u16(self.bytes, off_field)? as usize;
        if attach_off == 0 {
            return Err(Error::BadStructure(
                "GPOS/MarkLigPos: NULL ligatureAttach offset",
            ));
        }
        let attach_off =
            self.ligature_array_off
                .checked_add(attach_off)
                .ok_or(Error::BadStructure(
                    "GPOS/MarkLigPos: ligatureAttach offset overflow",
                ))?;
        if attach_off >= self.bytes.len() {
            return Err(Error::BadStructure(
                "GPOS/MarkLigPos: ligatureAttach offset out of range",
            ));
        }
        Ok(attach_off)
    }

    /// Resolve the component count at ligature-Coverage index `idx`,
    /// validating the LigatureAttach extent.
    fn component_count_at(&self, idx: u16) -> Result<u16, Error> {
        let attach_off = self.ligature_attach_off(idx)?;
        // LigatureAttach: uint16 componentCount, then componentCount
        // ComponentRecords; each ComponentRecord is markClassCount
        // Offset16 anchor offsets.
        let component_count = read_u16(self.bytes, attach_off)?;
        let record_size = (self.mark_class_count as usize)
            .checked_mul(2)
            .ok_or(Error::BadStructure("GPOS/MarkLigPos: record size overflow"))?;
        let records_bytes =
            (component_count as usize)
                .checked_mul(record_size)
                .ok_or(Error::BadStructure(
                    "GPOS/MarkLigPos: ComponentRecords size overflow",
                ))?;
        let need = attach_off
            .checked_add(2)
            .and_then(|v| v.checked_add(records_bytes))
            .ok_or(Error::BadStructure(
                "GPOS/MarkLigPos: LigatureAttach extent overflow",
            ))?;
        if need > self.bytes.len() {
            return Err(Error::UnexpectedEof);
        }
        Ok(component_count)
    }

    /// The ligature-component [`Anchor`] for ligature glyph `lig_glyph`,
    /// component `component`, and mark class `mark_class`.
    ///
    /// Returns:
    /// * `None` — `lig_glyph` is not in the ligature Coverage table.
    /// * `Some(Ok(None))` — the ComponentRecord's anchor offset for that
    ///   class is NULL (the spec permits a component to omit an anchor for
    ///   a class, in which case no adjustment applies for marks of that
    ///   class on that component).
    /// * `Some(Ok(Some(Anchor)))` — the decoded component anchor.
    /// * `Some(Err(_))` — the on-disk records are malformed, or
    ///   `component`/`mark_class` is out of range.
    pub fn ligature_anchor(
        &self,
        lig_glyph: u16,
        component: u16,
        mark_class: u16,
    ) -> Option<Result<Option<Anchor>, Error>> {
        let idx = self.ligature_coverage.index_of(lig_glyph)?;
        Some(self.ligature_anchor_at(idx, component, mark_class))
    }

    /// Resolve the component anchor at ligature-Coverage index `idx` for
    /// `(component, mark_class)`.
    fn ligature_anchor_at(
        &self,
        idx: u16,
        component: u16,
        mark_class: u16,
    ) -> Result<Option<Anchor>, Error> {
        if mark_class >= self.mark_class_count {
            return Err(Error::BadStructure(
                "GPOS/MarkLigPos: markClass >= markClassCount",
            ));
        }
        let attach_off = self.ligature_attach_off(idx)?;
        let component_count = read_u16(self.bytes, attach_off)?;
        if component >= component_count {
            return Err(Error::BadStructure(
                "GPOS/MarkLigPos: component >= componentCount",
            ));
        }
        // ComponentRecord array begins after the 2-byte componentCount;
        // each record is markClassCount Offset16s, the anchor offsets being
        // from the start of the LigatureAttach table.
        let record_size = self.mark_class_count as usize * 2;
        let rec_off = attach_off + 2 + component as usize * record_size;
        let anchor_off = read_u16(self.bytes, rec_off + mark_class as usize * 2)? as usize;
        if anchor_off == 0 {
            return Ok(None);
        }
        let anchor_pos = attach_off
            .checked_add(anchor_off)
            .ok_or(Error::BadStructure(
                "GPOS/MarkLigPos: ligature anchor offset overflow",
            ))?;
        let anchor = Anchor::parse(self.bytes, anchor_pos)?;
        Ok(Some(anchor))
    }

    /// Compute the attachment geometry for the ordered triple
    /// `(mark_glyph, lig_glyph, component)`.
    ///
    /// `component` is the zero-based index of the ligature component the
    /// mark is associated with; the spec requires the text-layout client
    /// to track this association from the original character string, as it
    /// is not derivable from the font data alone.
    ///
    /// Returns:
    /// * `None` — the mark is not covered, the ligature is not covered, or
    ///   the identified component has no (non-NULL) anchor for the mark's
    ///   class (no adjustment applies).
    /// * `Some(Err(_))` — the on-disk records are malformed, or
    ///   `component` is out of range for the ligature.
    /// * `Some(Ok(LigatureAttachment))` — the mark + component anchors a
    ///   shaper aligns to position the mark over the ligature component.
    pub fn attachment(
        &self,
        mark_glyph: u16,
        lig_glyph: u16,
        component: u16,
    ) -> Option<Result<LigatureAttachment, Error>> {
        let mark = match self.mark_record(mark_glyph)? {
            Ok(m) => m,
            Err(e) => return Some(Err(e)),
        };
        let lig = match self.ligature_anchor(lig_glyph, component, mark.mark_class)? {
            Ok(Some(a)) => a,
            Ok(None) => return None,
            Err(e) => return Some(Err(e)),
        };
        Some(Ok(LigatureAttachment {
            mark_class: mark.mark_class,
            component,
            mark_anchor: mark.anchor,
            ligature_anchor: lig,
        }))
    }
}

/// The attachment geometry a [`MarkMarkPos`] subtable computes for a
/// `(mark1, mark2)` glyph pair: the attaching mark's own anchor and the
/// `mark2` anchor for that mark's class.
///
/// A text-processing client aligns `mark1_anchor` over `mark2_anchor`,
/// positioning the combining mark relative to the preceding mark glyph
/// (spec §"Lookup type 6 subtable"). The roles mirror [`MarkAttachment`]
/// exactly — `mark1` is the mark being positioned and `mark2` is the
/// mark it attaches to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MarkMarkAttachment {
    /// The attaching mark's class (`0..markClassCount`).
    pub mark_class: u16,
    /// The attaching `mark1` glyph's attachment anchor.
    pub mark1_anchor: Anchor,
    /// The `mark2` glyph's anchor for the attaching mark's class.
    pub mark2_anchor: Anchor,
}

/// GPOS Lookup Type 6 — mark-to-mark attachment positioning subtable.
///
/// Spec: `docs/text/opentype/otspec-gpos.html` §"Lookup type 6 subtable:
/// mark-to-mark attachment positioning". One on-disk format,
/// `MarkMarkPosFormat1`:
///
/// ```text
/// MarkMarkPosFormat1 subtable (12 bytes)
///   0 / 2 / format = 1
///   2 / 2 / mark1CoverageOffset (Offset16, from start of subtable)
///   4 / 2 / mark2CoverageOffset (Offset16, from start of subtable)
///   6 / 2 / markClassCount
///   8 / 2 / mark1ArrayOffset    (Offset16, from start of subtable)
///  10 / 2 / mark2ArrayOffset    (Offset16, from start of subtable)
/// ```
///
/// The structure mirrors [`MarkBasePos`] precisely: `mark1` plays the
/// role of "mark" and `mark2` plays the role of "base". The `mark1Array`
/// holds one [`MarkRecord`] per `mark1`-Coverage glyph (its class +
/// anchor); the `Mark2Array` holds, per `mark2`-Coverage glyph, an array
/// of `markClassCount` [`Anchor`] offsets (one per `mark1` class, in
/// class order, any of which may be NULL). To attach a combining mark to
/// the preceding mark, the attaching mark's class selects which `mark2`
/// anchor aligns with the `mark1` anchor — see [`Self::attachment`].
#[derive(Debug, Clone, Copy)]
pub struct MarkMarkPos<'a> {
    bytes: &'a [u8],
    mark1_coverage: Coverage<'a>,
    mark2_coverage: Coverage<'a>,
    mark_class_count: u16,
    mark1_array_off: usize,
    mark2_array_off: usize,
    mark2_count: u16,
}

impl<'a> MarkMarkPos<'a> {
    /// Parse a MarkMarkPosFormat1 subtable from its raw `bytes`.
    pub fn parse(bytes: &'a [u8]) -> Result<Self, Error> {
        let format = read_u16(bytes, 0)?;
        if format != 1 {
            return Err(Error::BadStructure("GPOS/MarkMarkPos: unknown format"));
        }
        let mark1_coverage_off = read_u16(bytes, 2)? as usize;
        let mark2_coverage_off = read_u16(bytes, 4)? as usize;
        let mark_class_count = read_u16(bytes, 6)?;
        let mark1_array_off = read_u16(bytes, 8)? as usize;
        let mark2_array_off = read_u16(bytes, 10)? as usize;
        if mark1_coverage_off == 0
            || mark1_coverage_off >= bytes.len()
            || mark2_coverage_off == 0
            || mark2_coverage_off >= bytes.len()
        {
            return Err(Error::BadStructure(
                "GPOS/MarkMarkPos: coverageOffset out of range",
            ));
        }
        if mark1_array_off == 0
            || mark1_array_off >= bytes.len()
            || mark2_array_off == 0
            || mark2_array_off >= bytes.len()
        {
            return Err(Error::BadStructure(
                "GPOS/MarkMarkPos: arrayOffset out of range",
            ));
        }
        if mark_class_count == 0 {
            return Err(Error::BadStructure(
                "GPOS/MarkMarkPos: markClassCount is zero",
            ));
        }
        let mark1_coverage = Coverage::parse(&bytes[mark1_coverage_off..])?;
        let mark2_coverage = Coverage::parse(&bytes[mark2_coverage_off..])?;
        // mark2Count is the first uint16 of the Mark2Array table; each
        // Mark2 record is `markClassCount` Offset16 anchor offsets.
        let mark2_count = read_u16(bytes, mark2_array_off)?;
        // Validate the Mark2Array extent: mark2Count records of
        // markClassCount Offset16s each, following the 2-byte mark2Count.
        let record_size = (mark_class_count as usize)
            .checked_mul(2)
            .ok_or(Error::BadStructure(
                "GPOS/MarkMarkPos: record size overflow",
            ))?;
        let mark2_array_bytes =
            (mark2_count as usize)
                .checked_mul(record_size)
                .ok_or(Error::BadStructure(
                    "GPOS/MarkMarkPos: Mark2Array size overflow",
                ))?;
        let need = mark2_array_off
            .checked_add(2)
            .and_then(|v| v.checked_add(mark2_array_bytes))
            .ok_or(Error::BadStructure(
                "GPOS/MarkMarkPos: Mark2Array extent overflow",
            ))?;
        if need > bytes.len() {
            return Err(Error::UnexpectedEof);
        }
        Ok(Self {
            bytes,
            mark1_coverage,
            mark2_coverage,
            mark_class_count,
            mark1_array_off,
            mark2_array_off,
            mark2_count,
        })
    }

    /// Subtable format discriminant (always `1`).
    pub fn format(&self) -> u16 {
        1
    }

    /// `markClassCount` — number of distinct `mark1` classes.
    pub fn mark_class_count(&self) -> u16 {
        self.mark_class_count
    }

    /// The `mark1` (attaching mark) [`Coverage`] table.
    pub fn mark1_coverage(&self) -> Coverage<'a> {
        self.mark1_coverage
    }

    /// The `mark2` (base mark) [`Coverage`] table.
    pub fn mark2_coverage(&self) -> Coverage<'a> {
        self.mark2_coverage
    }

    /// The raw subtable bytes (index 0 = start of the MarkMarkPos
    /// subtable).
    pub fn raw(&self) -> &'a [u8] {
        self.bytes
    }

    /// The decoded [`MarkRecord`] for the attaching `mark1` glyph, or
    /// `None` if the glyph is not in the `mark1` Coverage table.
    ///
    /// The `mark1Array`'s `markCount` is required by the spec to equal
    /// the `mark1`-Coverage glyph count, so the Coverage Index directly
    /// indexes the record array.
    pub fn mark1_record(&self, mark1_glyph: u16) -> Option<Result<MarkRecord, Error>> {
        let idx = self.mark1_coverage.index_of(mark1_glyph)?;
        Some(self.mark1_record_at(idx))
    }

    /// Resolve the `mark1` MarkRecord at `mark1`-Coverage index `idx`.
    fn mark1_record_at(&self, idx: u16) -> Result<MarkRecord, Error> {
        // mark1Array: uint16 markCount, then markCount MarkRecords of
        // 4 bytes each (uint16 markClass + Offset16 markAnchorOffset),
        // the offset being from the start of the mark1Array table.
        let mark_count = read_u16(self.bytes, self.mark1_array_off)?;
        if idx >= mark_count {
            return Err(Error::BadStructure(
                "GPOS/MarkMarkPos: mark1 coverage index >= markCount",
            ));
        }
        let rec_off = self.mark1_array_off + 2 + idx as usize * 4;
        let mark_class = read_u16(self.bytes, rec_off)?;
        if mark_class >= self.mark_class_count {
            return Err(Error::BadStructure(
                "GPOS/MarkMarkPos: markClass >= markClassCount",
            ));
        }
        let anchor_off = read_u16(self.bytes, rec_off + 2)? as usize;
        // A NULL mark anchor offset is not meaningful for a mark (the
        // spec requires every mark to have an anchor).
        if anchor_off == 0 {
            return Err(Error::BadStructure(
                "GPOS/MarkMarkPos: NULL mark1 anchor offset",
            ));
        }
        let anchor = Anchor::parse(self.bytes, self.mark1_array_off + anchor_off)?;
        Ok(MarkRecord { mark_class, anchor })
    }

    /// The `mark2` [`Anchor`] for base-mark glyph `mark2_glyph` and
    /// `mark1` class `mark_class`.
    ///
    /// Returns:
    /// * `None` — `mark2_glyph` is not in the `mark2` Coverage table.
    /// * `Some(Ok(None))` — the Mark2 record's anchor offset for that
    ///   class is NULL (the spec permits a `mark2` to omit an anchor for
    ///   a class, in which case no adjustment is applied for marks of
    ///   that class).
    /// * `Some(Ok(Some(Anchor)))` — the decoded `mark2` anchor.
    pub fn mark2_anchor(
        &self,
        mark2_glyph: u16,
        mark_class: u16,
    ) -> Option<Result<Option<Anchor>, Error>> {
        let idx = self.mark2_coverage.index_of(mark2_glyph)?;
        Some(self.mark2_anchor_at(idx, mark_class))
    }

    /// Resolve the `mark2` anchor at `mark2`-Coverage index `idx` for
    /// `mark_class`.
    fn mark2_anchor_at(&self, idx: u16, mark_class: u16) -> Result<Option<Anchor>, Error> {
        if idx >= self.mark2_count {
            return Err(Error::BadStructure(
                "GPOS/MarkMarkPos: mark2 coverage index >= mark2Count",
            ));
        }
        if mark_class >= self.mark_class_count {
            return Err(Error::BadStructure(
                "GPOS/MarkMarkPos: markClass >= markClassCount",
            ));
        }
        // Mark2Array: uint16 mark2Count, then mark2Count Mark2 records;
        // each Mark2 record is markClassCount Offset16 anchor offsets,
        // the offsets being from the start of the Mark2Array table.
        let record_size = self.mark_class_count as usize * 2;
        let rec_off = self.mark2_array_off + 2 + idx as usize * record_size;
        let anchor_off = read_u16(self.bytes, rec_off + mark_class as usize * 2)? as usize;
        if anchor_off == 0 {
            return Ok(None);
        }
        let anchor = Anchor::parse(self.bytes, self.mark2_array_off + anchor_off)?;
        Ok(Some(anchor))
    }

    /// Compute the attachment geometry for the ordered pair
    /// `(mark1_glyph, mark2_glyph)` — the attaching combining mark and
    /// the preceding mark it joins to.
    ///
    /// Returns:
    /// * `None` — `mark1` is not covered, `mark2` is not covered, or the
    ///   `mark2` glyph has no (non-NULL) anchor for the attaching mark's
    ///   class (no adjustment applies).
    /// * `Some(Err(_))` — the on-disk records are malformed.
    /// * `Some(Ok(MarkMarkAttachment))` — the `mark1` + `mark2` anchors a
    ///   shaper aligns to position the combining mark over the preceding
    ///   mark.
    pub fn attachment(
        &self,
        mark1_glyph: u16,
        mark2_glyph: u16,
    ) -> Option<Result<MarkMarkAttachment, Error>> {
        let mark = match self.mark1_record(mark1_glyph)? {
            Ok(m) => m,
            Err(e) => return Some(Err(e)),
        };
        let mark2 = match self.mark2_anchor(mark2_glyph, mark.mark_class)? {
            Ok(Some(a)) => a,
            Ok(None) => return None,
            Err(e) => return Some(Err(e)),
        };
        Some(Ok(MarkMarkAttachment {
            mark_class: mark.mark_class,
            mark1_anchor: mark.anchor,
            mark2_anchor: mark2,
        }))
    }
}

/// A decoded `EntryExit` record — the entry and exit attachment
/// anchors of one glyph in a [`CursivePos`] subtable.
///
/// Spec: `docs/text/opentype/otspec-gpos.html` §"Lookup type 3 subtable:
/// cursive attachment positioning". Each record is two `Offset16`s, each
/// to an [`Anchor`] table (either of which may be NULL). The exit anchor
/// of one glyph aligns with the entry anchor of the following glyph.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EntryExit {
    /// The glyph's entry [`Anchor`] (`None` when `entryAnchorOffset` is
    /// NULL — the glyph cannot be joined to a preceding glyph).
    pub entry_anchor: Option<Anchor>,
    /// The glyph's exit [`Anchor`] (`None` when `exitAnchorOffset` is
    /// NULL — the glyph cannot be joined to a following glyph).
    pub exit_anchor: Option<Anchor>,
}

/// The attachment geometry a [`CursivePos`] subtable computes for an
/// ordered glyph pair `(first, second)`: the exit anchor of the first
/// glyph and the entry anchor of the second glyph, which a shaper aligns
/// so the two glyphs join cursively.
///
/// Per spec, the line-layout-direction adjustment is applied to the
/// advance of the *first* glyph, while the cross-stream placement of
/// whichever glyph the parent lookup's `RIGHT_TO_LEFT` flag designates
/// is shifted; this view only surfaces the two anchors a client needs to
/// compute that, leaving the flag-dependent direction logic to the
/// caller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CursiveAttachment {
    /// The exit [`Anchor`] of the first (leading) glyph.
    pub exit_anchor: Anchor,
    /// The entry [`Anchor`] of the second (following) glyph.
    pub entry_anchor: Anchor,
}

/// GPOS Lookup Type 3 — cursive attachment positioning subtable.
///
/// Spec: `docs/text/opentype/otspec-gpos.html` §"Lookup type 3 subtable:
/// cursive attachment positioning". One on-disk format,
/// `CursivePosFormat1`:
///
/// ```text
/// CursivePosFormat1 subtable
///   0 / 2 / format = 1
///   2 / 2 / coverageOffset   (Offset16, from start of subtable)
///   4 / 2 / entryExitCount
///   6 / 4·n / entryExitRecords[entryExitCount]
///             each = { Offset16 entryAnchorOffset; Offset16 exitAnchorOffset }
///             (offsets from start of subtable; either may be NULL)
/// ```
///
/// The EntryExit records are stored in Coverage index order: the
/// Coverage index of a glyph selects its record. To join glyph *A* to a
/// following glyph *B*, a client aligns *A*'s exit anchor with *B*'s
/// entry anchor — see [`Self::attachment`].
#[derive(Debug, Clone, Copy)]
pub struct CursivePos<'a> {
    bytes: &'a [u8],
    coverage: Coverage<'a>,
    entry_exit_count: u16,
}

impl<'a> CursivePos<'a> {
    /// Parse a CursivePosFormat1 subtable from its raw `bytes`.
    pub fn parse(bytes: &'a [u8]) -> Result<Self, Error> {
        let format = read_u16(bytes, 0)?;
        if format != 1 {
            return Err(Error::BadStructure("GPOS/CursivePos: unknown format"));
        }
        let coverage_off = read_u16(bytes, 2)? as usize;
        let entry_exit_count = read_u16(bytes, 4)?;
        if coverage_off == 0 || coverage_off >= bytes.len() {
            return Err(Error::BadStructure(
                "GPOS/CursivePos: coverageOffset out of range",
            ));
        }
        // The EntryExit array follows the 6-byte header: entryExitCount
        // records of 4 bytes each. Validate the extent up front so that
        // record accessors can index without further bounds anxiety.
        let array_bytes = (entry_exit_count as usize)
            .checked_mul(4)
            .ok_or(Error::BadStructure(
                "GPOS/CursivePos: EntryExit array size overflow",
            ))?;
        let need = 6usize.checked_add(array_bytes).ok_or(Error::BadStructure(
            "GPOS/CursivePos: EntryExit extent overflow",
        ))?;
        if need > bytes.len() {
            return Err(Error::UnexpectedEof);
        }
        let coverage = Coverage::parse(&bytes[coverage_off..])?;
        Ok(Self {
            bytes,
            coverage,
            entry_exit_count,
        })
    }

    /// Subtable format discriminant (always `1`).
    pub fn format(&self) -> u16 {
        1
    }

    /// `entryExitCount` — the number of EntryExit records, equal to the
    /// Coverage glyph count.
    pub fn entry_exit_count(&self) -> u16 {
        self.entry_exit_count
    }

    /// The [`Coverage`] table listing every glyph with cursive data.
    pub fn coverage(&self) -> Coverage<'a> {
        self.coverage
    }

    /// The raw subtable bytes (index 0 = start of the CursivePos
    /// subtable) — [`Anchor::table_offset`] values from this
    /// subtable's anchors index into it.
    pub fn raw(&self) -> &'a [u8] {
        self.bytes
    }

    /// The decoded [`EntryExit`] record for `glyph`, or `None` if the
    /// glyph is not in the Coverage table.
    ///
    /// Either anchor of the returned record may be `None` (a NULL
    /// `entryAnchorOffset` / `exitAnchorOffset`, meaning the glyph cannot
    /// be cursively joined on that side).
    pub fn entry_exit(&self, glyph: u16) -> Option<Result<EntryExit, Error>> {
        let idx = self.coverage.index_of(glyph)?;
        Some(self.entry_exit_at(idx))
    }

    /// Resolve the EntryExit record at Coverage index `idx`.
    fn entry_exit_at(&self, idx: u16) -> Result<EntryExit, Error> {
        if idx >= self.entry_exit_count {
            return Err(Error::BadStructure(
                "GPOS/CursivePos: coverage index >= entryExitCount",
            ));
        }
        let rec_off = 6 + idx as usize * 4;
        let entry_off = read_u16(self.bytes, rec_off)? as usize;
        let exit_off = read_u16(self.bytes, rec_off + 2)? as usize;
        // Offsets are from the start of the CursivePos subtable; a NULL
        // (zero) offset means "no anchor on this side".
        let entry_anchor = if entry_off == 0 {
            None
        } else {
            Some(Anchor::parse(self.bytes, entry_off)?)
        };
        let exit_anchor = if exit_off == 0 {
            None
        } else {
            Some(Anchor::parse(self.bytes, exit_off)?)
        };
        Ok(EntryExit {
            entry_anchor,
            exit_anchor,
        })
    }

    /// Compute the cursive attachment geometry for the ordered pair
    /// `(first_glyph, second_glyph)`.
    ///
    /// Returns:
    /// * `None` — either glyph is not covered, or one of the two
    ///   participating anchors is NULL (`first` has no exit anchor or
    ///   `second` has no entry anchor): per spec, no positioning
    ///   adjustment is applied in that case.
    /// * `Some(Err(_))` — the on-disk records are malformed.
    /// * `Some(Ok(CursiveAttachment))` — the first glyph's exit anchor
    ///   and the second glyph's entry anchor a shaper aligns to join the
    ///   pair.
    pub fn attachment(
        &self,
        first_glyph: u16,
        second_glyph: u16,
    ) -> Option<Result<CursiveAttachment, Error>> {
        let first = match self.entry_exit(first_glyph)? {
            Ok(ee) => ee,
            Err(e) => return Some(Err(e)),
        };
        let second = match self.entry_exit(second_glyph)? {
            Ok(ee) => ee,
            Err(e) => return Some(Err(e)),
        };
        match (first.exit_anchor, second.entry_anchor) {
            (Some(exit_anchor), Some(entry_anchor)) => Some(Ok(CursiveAttachment {
                exit_anchor,
                entry_anchor,
            })),
            _ => None,
        }
    }
}

/// Parsed `GPOS` header view.
#[derive(Debug, Clone, Copy)]
pub struct GposTable<'a> {
    bytes: &'a [u8],
    header: LayoutHeader,
}

impl<'a> GposTable<'a> {
    /// Parse a GPOS table from the raw `bytes` of the table.
    pub fn parse(bytes: &'a [u8]) -> Result<Self, Error> {
        let header = LayoutHeader::parse(bytes)?;
        let len = bytes.len();
        if (header.script_list_off as usize) >= len
            || (header.feature_list_off as usize) >= len
            || (header.lookup_list_off as usize) >= len
        {
            return Err(Error::BadStructure("GPOS: header offset out of range"));
        }
        if header.feature_variations_off != 0 && (header.feature_variations_off as usize) >= len {
            return Err(Error::BadStructure(
                "GPOS: featureVariationsOffset out of range",
            ));
        }
        Ok(Self { bytes, header })
    }

    /// `(majorVersion, minorVersion)` pair (`(1, 0)` or `(1, 1)`).
    pub fn version(&self) -> (u16, u16) {
        (self.header.major, self.header.minor)
    }

    /// Raw `featureVariationsOffset` (`0` = NULL or absent).
    pub fn feature_variations_offset(&self) -> u32 {
        self.header.feature_variations_off
    }

    /// `true` iff the v1.1 `featureVariationsOffset` is present and
    /// non-zero.
    pub fn has_feature_variations(&self) -> bool {
        self.header.minor >= 1 && self.header.feature_variations_off != 0
    }

    /// Parse the v1.1 `FeatureVariations` table, when present — the
    /// variable-font mechanism that substitutes alternate feature
    /// tables under axis-range conditions (chapter 2 §"Feature
    /// variations").
    pub fn feature_variations(&self) -> Option<Result<FeatureVariations<'a>, Error>> {
        let off = self.header.feature_variations_off as usize;
        if off == 0 {
            return None;
        }
        if off >= self.bytes.len() {
            return Some(Err(Error::BadStructure(
                "featureVariationsOffset out of range",
            )));
        }
        Some(FeatureVariations::parse(&self.bytes[off..]))
    }

    /// Raw table bytes.
    pub fn raw(&self) -> &'a [u8] {
        self.bytes
    }

    /// Parsed [`ScriptList`].
    pub fn script_list(&self) -> Result<ScriptList<'a>, Error> {
        ScriptList::parse(&self.bytes[self.header.script_list_off as usize..])
    }

    /// Parsed [`FeatureList`].
    pub fn feature_list(&self) -> Result<FeatureList<'a>, Error> {
        FeatureList::parse(&self.bytes[self.header.feature_list_off as usize..])
    }

    /// Parsed [`LookupList`].
    pub fn lookup_list(&self) -> Result<LookupList<'a>, Error> {
        LookupList::parse(&self.bytes[self.header.lookup_list_off as usize..])
    }

    /// Convenience: find a [`Script`] by 4-byte tag (e.g. `b"DFLT"`,
    /// `b"latn"`).
    pub fn find_script(&self, tag: &[u8; 4]) -> Option<Script<'a>> {
        self.script_list().ok()?.find(tag)?.ok()
    }

    /// Convenience: total `lookupCount`.
    pub fn lookup_count(&self) -> u16 {
        self.lookup_list().map(|l| l.count()).unwrap_or(0)
    }

    /// Convenience: total `featureCount`.
    pub fn feature_count(&self) -> u16 {
        self.feature_list().map(|f| f.count()).unwrap_or(0)
    }

    /// Convenience: total `scriptCount`.
    pub fn script_count(&self) -> u16 {
        self.script_list().map(|s| s.count()).unwrap_or(0)
    }

    /// Borrow lookup `i` by index.
    pub fn lookup(&self, i: u16) -> Option<Lookup<'a>> {
        self.lookup_list().ok()?.lookup(i)?.ok()
    }

    /// Decode subtable `sub_i` of lookup `lookup_i` as a [`SinglePos`]
    /// (`GposLookupType = 1`, single adjustment positioning).
    ///
    /// Returns:
    /// * `None` — `lookup_i` or `sub_i` is out of range, or the
    ///   referenced subtable bytes are unreachable.
    /// * `Some(Err(Error::BadStructure))` — the lookup is not declared
    ///   as `GPOS_LOOKUP_TYPE_SINGLE`, or the subtable bytes are
    ///   malformed.
    /// * `Some(Ok(SinglePos))` — the typed subtable view.
    pub fn single_pos(&self, lookup_i: u16, sub_i: u16) -> Option<Result<SinglePos<'a>, Error>> {
        let lk = self.lookup(lookup_i)?;
        if lk.lookup_type() != GPOS_LOOKUP_TYPE_SINGLE {
            return Some(Err(Error::BadStructure(
                "GPOS/SinglePos: lookup is not type 1",
            )));
        }
        let bytes = lk.subtable_bytes(sub_i)?;
        Some(SinglePos::parse(bytes))
    }

    /// Decode subtable `sub_i` of lookup `lookup_i` as a [`PairPos`]
    /// (`GposLookupType = 2`, pair adjustment positioning).
    ///
    /// Returns:
    /// * `None` — `lookup_i` or `sub_i` is out of range, or the
    ///   referenced subtable bytes are unreachable.
    /// * `Some(Err(Error::BadStructure))` — the lookup is not declared
    ///   as `GPOS_LOOKUP_TYPE_PAIR`, or the subtable bytes are malformed.
    /// * `Some(Ok(PairPos))` — the typed subtable view.
    pub fn pair_pos(&self, lookup_i: u16, sub_i: u16) -> Option<Result<PairPos<'a>, Error>> {
        let lk = self.lookup(lookup_i)?;
        if lk.lookup_type() != GPOS_LOOKUP_TYPE_PAIR {
            return Some(Err(Error::BadStructure(
                "GPOS/PairPos: lookup is not type 2",
            )));
        }
        let bytes = lk.subtable_bytes(sub_i)?;
        Some(PairPos::parse(bytes))
    }

    /// Decode subtable `sub_i` of lookup `lookup_i` as an
    /// [`ExtensionPos`] (`GposLookupType = 9`, positioning extension).
    ///
    /// Returns:
    /// * `None` — `lookup_i` or `sub_i` is out of range, or the
    ///   referenced subtable bytes are unreachable.
    /// * `Some(Err(Error::BadStructure))` — the lookup is not declared
    ///   as `GPOS_LOOKUP_TYPE_EXTENSION`, or the subtable bytes are
    ///   malformed.
    /// * `Some(Ok(ExtensionPos))` — the typed subtable view; resolve the
    ///   wrapped subtable through
    ///   [`ExtensionPos::extension_subtable_bytes`] or one of the typed
    ///   `as_*` resolvers.
    pub fn extension_pos(
        &self,
        lookup_i: u16,
        sub_i: u16,
    ) -> Option<Result<ExtensionPos<'a>, Error>> {
        let lk = self.lookup(lookup_i)?;
        if lk.lookup_type() != GPOS_LOOKUP_TYPE_EXTENSION {
            return Some(Err(Error::BadStructure(
                "GPOS/ExtensionPos: lookup is not type 9",
            )));
        }
        let bytes = lk.subtable_bytes(sub_i)?;
        Some(ExtensionPos::parse(bytes))
    }

    /// Decode subtable `sub_i` of lookup `lookup_i` as a [`MarkBasePos`]
    /// (`GposLookupType = 4`, mark-to-base attachment positioning).
    ///
    /// Returns:
    /// * `None` — `lookup_i` or `sub_i` is out of range, or the
    ///   referenced subtable bytes are unreachable.
    /// * `Some(Err(Error::BadStructure))` — the lookup is not declared
    ///   as `GPOS_LOOKUP_TYPE_MARK_TO_BASE`, or the subtable bytes are
    ///   malformed.
    /// * `Some(Ok(MarkBasePos))` — the typed subtable view.
    pub fn mark_base_pos(
        &self,
        lookup_i: u16,
        sub_i: u16,
    ) -> Option<Result<MarkBasePos<'a>, Error>> {
        let lk = self.lookup(lookup_i)?;
        if lk.lookup_type() != GPOS_LOOKUP_TYPE_MARK_TO_BASE {
            return Some(Err(Error::BadStructure(
                "GPOS/MarkBasePos: lookup is not type 4",
            )));
        }
        let bytes = lk.subtable_bytes(sub_i)?;
        Some(MarkBasePos::parse(bytes))
    }

    /// Decode subtable `sub_i` of lookup `lookup_i` as a [`CursivePos`]
    /// (`GposLookupType = 3`, cursive attachment positioning).
    ///
    /// Returns:
    /// * `None` — `lookup_i` or `sub_i` is out of range, or the
    ///   referenced subtable bytes are unreachable.
    /// * `Some(Err(Error::BadStructure))` — the lookup is not declared
    ///   as `GPOS_LOOKUP_TYPE_CURSIVE`, or the subtable bytes are
    ///   malformed.
    /// * `Some(Ok(CursivePos))` — the typed subtable view.
    pub fn cursive_pos(&self, lookup_i: u16, sub_i: u16) -> Option<Result<CursivePos<'a>, Error>> {
        let lk = self.lookup(lookup_i)?;
        if lk.lookup_type() != GPOS_LOOKUP_TYPE_CURSIVE {
            return Some(Err(Error::BadStructure(
                "GPOS/CursivePos: lookup is not type 3",
            )));
        }
        let bytes = lk.subtable_bytes(sub_i)?;
        Some(CursivePos::parse(bytes))
    }

    /// Decode subtable `sub_i` of lookup `lookup_i` as a [`MarkMarkPos`]
    /// (`GposLookupType = 6`, mark-to-mark attachment positioning).
    ///
    /// Returns:
    /// * `None` — `lookup_i` or `sub_i` is out of range, or the
    ///   referenced subtable bytes are unreachable.
    /// * `Some(Err(Error::BadStructure))` — the lookup is not declared
    ///   as `GPOS_LOOKUP_TYPE_MARK_TO_MARK`, or the subtable bytes are
    ///   malformed.
    /// * `Some(Ok(MarkMarkPos))` — the typed subtable view.
    pub fn mark_mark_pos(
        &self,
        lookup_i: u16,
        sub_i: u16,
    ) -> Option<Result<MarkMarkPos<'a>, Error>> {
        let lk = self.lookup(lookup_i)?;
        if lk.lookup_type() != GPOS_LOOKUP_TYPE_MARK_TO_MARK {
            return Some(Err(Error::BadStructure(
                "GPOS/MarkMarkPos: lookup is not type 6",
            )));
        }
        let bytes = lk.subtable_bytes(sub_i)?;
        Some(MarkMarkPos::parse(bytes))
    }

    /// Decode subtable `sub_i` of lookup `lookup_i` as a [`MarkLigPos`]
    /// (`GposLookupType = 5`, mark-to-ligature attachment positioning).
    ///
    /// Returns:
    /// * `None` — `lookup_i` or `sub_i` is out of range, or the
    ///   referenced subtable bytes are unreachable.
    /// * `Some(Err(Error::BadStructure))` — the lookup is not declared
    ///   as `GPOS_LOOKUP_TYPE_MARK_TO_LIGATURE`, or the subtable bytes
    ///   are malformed.
    /// * `Some(Ok(MarkLigPos))` — the typed subtable view.
    pub fn mark_lig_pos(&self, lookup_i: u16, sub_i: u16) -> Option<Result<MarkLigPos<'a>, Error>> {
        let lk = self.lookup(lookup_i)?;
        if lk.lookup_type() != GPOS_LOOKUP_TYPE_MARK_TO_LIGATURE {
            return Some(Err(Error::BadStructure(
                "GPOS/MarkLigPos: lookup is not type 5",
            )));
        }
        let bytes = lk.subtable_bytes(sub_i)?;
        Some(MarkLigPos::parse(bytes))
    }

    /// Decode subtable `sub_i` of lookup `lookup_i` as a contextual
    /// positioning subtable ([`SequenceContext`], `GposLookupType = 7`).
    ///
    /// The three on-disk formats (glyph / class / coverage based) are
    /// shared with GSUB type 5; see [`SequenceContext`]. Each match
    /// carries the nested-lookup [`SequenceLookupRecord`]s a shaper
    /// applies.
    ///
    /// Returns:
    /// * `None` — `lookup_i` or `sub_i` is out of range, or the
    ///   referenced subtable bytes are unreachable.
    /// * `Some(Err(Error::BadStructure))` — the lookup is not declared
    ///   as `GPOS_LOOKUP_TYPE_CONTEXT`, or the subtable bytes are
    ///   malformed.
    /// * `Some(Ok(SequenceContext))` — the typed subtable view.
    ///
    /// [`SequenceLookupRecord`]: crate::SequenceLookupRecord
    pub fn context_pos(
        &self,
        lookup_i: u16,
        sub_i: u16,
    ) -> Option<Result<SequenceContext<'a>, Error>> {
        let lk = self.lookup(lookup_i)?;
        if lk.lookup_type() != GPOS_LOOKUP_TYPE_CONTEXT {
            return Some(Err(Error::BadStructure(
                "GPOS/SequenceContext: lookup is not type 7",
            )));
        }
        let bytes = lk.subtable_bytes(sub_i)?;
        Some(SequenceContext::parse(bytes))
    }

    /// Decode subtable `sub_i` of lookup `lookup_i` as a chained
    /// contextual positioning subtable ([`ChainedSequenceContext`],
    /// `GposLookupType = 8`).
    ///
    /// The three on-disk formats are shared with GSUB type 6; see
    /// [`ChainedSequenceContext`]. Each match additionally constrains the
    /// backtrack and lookahead sequences around the input.
    ///
    /// Returns:
    /// * `None` — `lookup_i` or `sub_i` is out of range, or the
    ///   referenced subtable bytes are unreachable.
    /// * `Some(Err(Error::BadStructure))` — the lookup is not declared
    ///   as `GPOS_LOOKUP_TYPE_CHAINED_CONTEXT`, or the subtable bytes are
    ///   malformed.
    /// * `Some(Ok(ChainedSequenceContext))` — the typed subtable view.
    pub fn chained_context_pos(
        &self,
        lookup_i: u16,
        sub_i: u16,
    ) -> Option<Result<ChainedSequenceContext<'a>, Error>> {
        let lk = self.lookup(lookup_i)?;
        if lk.lookup_type() != GPOS_LOOKUP_TYPE_CHAINED_CONTEXT {
            return Some(Err(Error::BadStructure(
                "GPOS/ChainedSequenceContext: lookup is not type 8",
            )));
        }
        let bytes = lk.subtable_bytes(sub_i)?;
        Some(ChainedSequenceContext::parse(bytes))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn be(u: u16) -> [u8; 2] {
        u.to_be_bytes()
    }

    /// Minimal valid v1.0 GPOS byte tower with a single `kern`
    /// feature and a single type-2 lookup (pair-adjustment).
    #[test]
    fn parses_minimal_v10_table() {
        // 0   /  10 / header (script=10, feature=22, lookup=44)
        // 10  /  12 / ScriptList: count=1, [DFLT, scriptOffset=8 → 18]
        // 18  /   4 / Script: defaultLangSys=0, langSysCount=0
        // 22  /  10 / FeatureList: count=1, [kern, featureOffset=8 → 30]
        // 30  /   6 / Feature: paramsOffset=0, lookupCount=1, lookupIdx=[0]
        // 44  /   4 / LookupList: count=1, [lookupOffset=4 → 48]
        // 48  /   6 / Lookup: type=2, flag=0, subTableCount=0
        let mut bytes = vec![0u8; 54];
        bytes[0..2].copy_from_slice(&be(1));
        bytes[2..4].copy_from_slice(&be(0));
        bytes[4..6].copy_from_slice(&be(10));
        bytes[6..8].copy_from_slice(&be(22));
        bytes[8..10].copy_from_slice(&be(44));
        bytes[10..12].copy_from_slice(&be(1));
        bytes[12..16].copy_from_slice(b"DFLT");
        bytes[16..18].copy_from_slice(&be(8));
        bytes[18..20].copy_from_slice(&be(0));
        bytes[20..22].copy_from_slice(&be(0));
        bytes[22..24].copy_from_slice(&be(1));
        bytes[24..28].copy_from_slice(b"kern");
        bytes[28..30].copy_from_slice(&be(8));
        bytes[30..32].copy_from_slice(&be(0));
        bytes[32..34].copy_from_slice(&be(1));
        bytes[34..36].copy_from_slice(&be(0));
        bytes[44..46].copy_from_slice(&be(1));
        bytes[46..48].copy_from_slice(&be(4));
        bytes[48..50].copy_from_slice(&be(2));
        bytes[50..52].copy_from_slice(&be(0));
        bytes[52..54].copy_from_slice(&be(0));

        let g = GposTable::parse(&bytes).unwrap();
        assert_eq!(g.version(), (1, 0));
        assert_eq!(g.script_count(), 1);
        assert_eq!(g.feature_count(), 1);
        assert_eq!(g.lookup_count(), 1);
        assert_eq!(g.find_script(b"DFLT").map(|s| s.lang_sys_count()), Some(0));
        assert_eq!(g.feature_list().unwrap().tag(0), Some(*b"kern"));
        assert_eq!(g.lookup(0).map(|l| l.lookup_type()), Some(2));
    }

    #[test]
    fn rejects_truncated_v11_header() {
        let mut bytes = vec![0u8; 12]; // claims v1.1 but missing 2 trailer bytes
        bytes[0..2].copy_from_slice(&be(1));
        bytes[2..4].copy_from_slice(&be(1));
        assert!(matches!(
            GposTable::parse(&bytes),
            Err(Error::UnexpectedEof)
        ));
    }

    // -- ValueFormat / ValueRecord ---------------------------------------

    #[test]
    fn value_format_field_presence_and_size() {
        let vf = ValueFormat(0x0005); // X_PLACEMENT | X_ADVANCE
        assert!(vf.has_x_placement());
        assert!(!vf.has_y_placement());
        assert!(vf.has_x_advance());
        assert!(!vf.has_y_advance());
        assert!(vf.is_valid());
        assert_eq!(vf.record_size(), 4);

        let all = ValueFormat(0x00FF);
        assert_eq!(all.record_size(), 16);
        assert!(all.is_valid());

        let reserved = ValueFormat(0x0100);
        assert!(!reserved.is_valid());

        assert_eq!(ValueFormat(0).record_size(), 0);
    }

    #[test]
    fn value_record_reads_only_declared_fields_in_order() {
        // valueFormat = X_PLACEMENT | Y_ADVANCE | X_ADVANCE_DEVICE
        //             = 0x0001 | 0x0008 | 0x0040 = 0x0049
        // Fields, in flag-bit order: xPlacement(i16), yAdvance(i16),
        // xAdvDeviceOffset(off16).
        let vf = ValueFormat(0x0049);
        assert_eq!(vf.record_size(), 6);
        let mut data = Vec::new();
        data.extend_from_slice(&(-25i16).to_be_bytes()); // xPlacement
        data.extend_from_slice(&(40i16).to_be_bytes()); // yAdvance
        data.extend_from_slice(&(0x1234u16).to_be_bytes()); // xAdvDeviceOffset
        data.extend_from_slice(&[0xAA, 0xBB]); // trailing noise

        let (rec, used) = ValueRecord::parse(&data, 0, vf).unwrap();
        assert_eq!(used, 6);
        assert_eq!(rec.x_placement, -25);
        assert_eq!(rec.y_advance, 40);
        assert_eq!(rec.x_advance_device_offset, 0x1234);
        // Undeclared fields default to zero.
        assert_eq!(rec.y_placement, 0);
        assert_eq!(rec.x_advance, 0);
        assert_eq!(rec.y_placement_device_offset, 0);
    }

    #[test]
    fn empty_value_record_consumes_nothing() {
        let (rec, used) = ValueRecord::parse(&[], 0, ValueFormat(0)).unwrap();
        assert_eq!(used, 0);
        assert_eq!(rec, ValueRecord::default());
    }

    // -- SinglePos format 1 ----------------------------------------------

    /// Build a standalone SinglePosFormat1 subtable: one shared
    /// ValueRecord (xAdvance only) covering glyphs {10, 11, 20}.
    fn singlepos_f1_subtable() -> Vec<u8> {
        // 0  / 6 / header: format=1, coverageOffset=?, valueFormat=0x0004
        // 6  / 2 / ValueRecord: xAdvance = -50
        // 8  / .. / Coverage format 1: count=3, [10,11,20]
        let mut b = vec![0u8; 8];
        b[0..2].copy_from_slice(&be(1)); // format
        b[2..4].copy_from_slice(&be(8)); // coverageOffset
        b[4..6].copy_from_slice(&be(0x0004)); // valueFormat = X_ADVANCE
        b[6..8].copy_from_slice(&(-50i16).to_be_bytes()); // xAdvance
                                                          // Coverage format 1
        b.extend_from_slice(&be(1)); // coverage format
        b.extend_from_slice(&be(3)); // glyphCount
        b.extend_from_slice(&be(10));
        b.extend_from_slice(&be(11));
        b.extend_from_slice(&be(20));
        b
    }

    #[test]
    fn single_pos_format1_shared_value() {
        let b = singlepos_f1_subtable();
        let sp = SinglePos::parse(&b).unwrap();
        assert_eq!(sp.format(), 1);
        assert_eq!(sp.value_format(), ValueFormat(0x0004));
        assert_eq!(sp.value_count(), 1);

        for g in [10u16, 11, 20] {
            let v = sp.value(g).unwrap().unwrap();
            assert_eq!(v.x_advance, -50);
            assert_eq!(v.x_placement, 0);
        }
        // Uncovered glyph.
        assert!(sp.value(12).is_none());

        let collected: Vec<_> = sp.iter().map(|(g, v)| (g, v.unwrap().x_advance)).collect();
        assert_eq!(collected, vec![(10, -50), (11, -50), (20, -50)]);
    }

    // -- SinglePos format 2 ----------------------------------------------

    #[test]
    fn single_pos_format2_per_glyph_values() {
        // valueFormat = X_PLACEMENT | X_ADVANCE = 0x0005 → 4 bytes/record.
        // Coverage {5, 6} → two records.
        // 0  / 8 / header: format=2, coverageOffset=?, valueFormat, valueCount=2
        // 8  / 4 / record[0]: xPlacement=3, xAdvance=7
        // 12 / 4 / record[1]: xPlacement=-3, xAdvance=-7
        // 16 / .. / Coverage format 1 {5,6}
        let mut b = vec![0u8; 16];
        b[0..2].copy_from_slice(&be(2));
        b[2..4].copy_from_slice(&be(16)); // coverageOffset
        b[4..6].copy_from_slice(&be(0x0005));
        b[6..8].copy_from_slice(&be(2)); // valueCount
        b[8..10].copy_from_slice(&(3i16).to_be_bytes());
        b[10..12].copy_from_slice(&(7i16).to_be_bytes());
        b[12..14].copy_from_slice(&(-3i16).to_be_bytes());
        b[14..16].copy_from_slice(&(-7i16).to_be_bytes());
        b.extend_from_slice(&be(1)); // coverage format
        b.extend_from_slice(&be(2)); // glyphCount
        b.extend_from_slice(&be(5));
        b.extend_from_slice(&be(6));

        let sp = SinglePos::parse(&b).unwrap();
        assert_eq!(sp.format(), 2);
        assert_eq!(sp.value_count(), 2);

        let v5 = sp.value(5).unwrap().unwrap();
        assert_eq!((v5.x_placement, v5.x_advance), (3, 7));
        let v6 = sp.value(6).unwrap().unwrap();
        assert_eq!((v6.x_placement, v6.x_advance), (-3, -7));
        assert!(sp.value(7).is_none());

        let pairs: Vec<_> = sp
            .iter()
            .map(|(g, v)| {
                let v = v.unwrap();
                (g, v.x_placement, v.x_advance)
            })
            .collect();
        assert_eq!(pairs, vec![(5, 3, 7), (6, -3, -7)]);
    }

    #[test]
    fn single_pos_rejects_reserved_value_format() {
        let mut b = singlepos_f1_subtable();
        b[4..6].copy_from_slice(&be(0x0100)); // reserved bit
        assert!(matches!(SinglePos::parse(&b), Err(Error::BadStructure(_))));
    }

    #[test]
    fn single_pos_rejects_bad_coverage_offset() {
        let mut b = singlepos_f1_subtable();
        b[2..4].copy_from_slice(&be(0)); // NULL coverage
        assert!(matches!(SinglePos::parse(&b), Err(Error::BadStructure(_))));
    }

    #[test]
    fn single_pos_format2_truncated_value_array() {
        // valueFormat=0x0005 (4-byte records); valueCount claims a large
        // array that overruns the buffer. Coverage is placed early so it
        // parses cleanly; the value-array length check then fails.
        // 0  / 8 / header: format=2, coverageOffset=8, valueFormat, valueCount=100
        // 8  / .. / Coverage format 1 {5}
        let mut b = vec![0u8; 8];
        b[0..2].copy_from_slice(&be(2));
        b[2..4].copy_from_slice(&be(8)); // coverageOffset → 8
        b[4..6].copy_from_slice(&be(0x0005));
        b[6..8].copy_from_slice(&be(100)); // valueCount=100 → needs 8 + 400 bytes
        b.extend_from_slice(&be(1)); // coverage format
        b.extend_from_slice(&be(1)); // glyphCount
        b.extend_from_slice(&be(5));
        assert!(matches!(SinglePos::parse(&b), Err(Error::UnexpectedEof)));
    }

    #[test]
    fn single_pos_rejects_unknown_format() {
        let mut b = singlepos_f1_subtable();
        b[0..2].copy_from_slice(&be(3));
        assert!(matches!(SinglePos::parse(&b), Err(Error::BadStructure(_))));
    }

    /// End-to-end: a GPOS byte tower whose only lookup is a type-1
    /// single-adjustment, decoded through `GposTable::single_pos`.
    #[test]
    fn gpos_table_single_pos_accessor() {
        let sub = singlepos_f1_subtable();
        // Layout (offsets from start of GPOS):
        // 0   / 10 / header (script=10, feature=22, lookup=44)
        // 10  / 12 / ScriptList (DFLT)
        // 22  / 10 / FeatureList (kern)
        // 30  /  6 / Feature
        // 44  /  4 / LookupList: count=1, [lookupOffset=4 → 48]
        // 48  /  6 / Lookup: type=1, flag=0, subTableCount=1
        // 54  /  2 / subtableOffsets[0]
        // 56  / .. / SinglePos subtable
        let mut bytes = vec![0u8; 54];
        bytes[0..2].copy_from_slice(&be(1));
        bytes[2..4].copy_from_slice(&be(0));
        bytes[4..6].copy_from_slice(&be(10));
        bytes[6..8].copy_from_slice(&be(22));
        bytes[8..10].copy_from_slice(&be(44));
        bytes[10..12].copy_from_slice(&be(1));
        bytes[12..16].copy_from_slice(b"DFLT");
        bytes[16..18].copy_from_slice(&be(8));
        bytes[18..20].copy_from_slice(&be(0));
        bytes[20..22].copy_from_slice(&be(0));
        bytes[22..24].copy_from_slice(&be(1));
        bytes[24..28].copy_from_slice(b"kern");
        bytes[28..30].copy_from_slice(&be(8));
        bytes[30..32].copy_from_slice(&be(0));
        bytes[32..34].copy_from_slice(&be(1));
        bytes[34..36].copy_from_slice(&be(0));
        bytes[44..46].copy_from_slice(&be(1)); // LookupList count
        bytes[46..48].copy_from_slice(&be(4)); // lookupOffset
        bytes[48..50].copy_from_slice(&be(1)); // lookupType = 1
        bytes[50..52].copy_from_slice(&be(0)); // lookupFlag
        bytes[52..54].copy_from_slice(&be(1)); // subTableCount
                                               // subtableOffset is from the start of the Lookup (at 48). The
                                               // subtableOffsets array occupies 54..56, so the subtable bytes
                                               // begin at absolute 56 → relative offset 56 - 48 = 8.
        bytes.extend_from_slice(&be(8)); // subtableOffset
        bytes.extend_from_slice(&sub); // SinglePos at 56

        let g = GposTable::parse(&bytes).unwrap();
        assert_eq!(g.lookup(0).map(|l| l.lookup_type()), Some(1));
        let sp = g.single_pos(0, 0).unwrap().unwrap();
        assert_eq!(sp.value(11).unwrap().unwrap().x_advance, -50);

        // Wrong-type guard: there is no type-2 lookup here.
        // Asking single_pos on the only lookup (type 1) succeeds; an
        // out-of-range lookup yields None.
        assert!(g.single_pos(5, 0).is_none());
    }

    // -- PairPos format 1 ------------------------------------------------

    /// Build a standalone PairPosFormat1 subtable based on the spec's
    /// Example 4 shape: first glyphs {A=8, B=9}; A pairs with V=40
    /// (xAdvance1=-40), B pairs with both T=30 (-30) and V=40 (-50).
    /// valueFormat1 = X_ADVANCE (0x0004), valueFormat2 = 0 (empty).
    fn pairpos_f1_subtable() -> Vec<u8> {
        // Header (10 bytes):
        //  0 /2/ format=1
        //  2 /2/ coverageOffset
        //  4 /2/ valueFormat1 = 0x0004
        //  6 /2/ valueFormat2 = 0
        //  8 /2/ pairSetCount = 2
        // 10 /4/ pairSetOffsets[2]
        // Then: PairSet for A, PairSet for B, then Coverage.
        let vf1 = 0x0004u16;
        // PairSet A: count=1, record (secondGlyph=40, vr1.xAdvance=-40)
        let mut pairset_a = Vec::new();
        pairset_a.extend_from_slice(&be(1)); // pairValueCount
        pairset_a.extend_from_slice(&be(40)); // secondGlyph
        pairset_a.extend_from_slice(&(-40i16).to_be_bytes()); // vr1 xAdvance
                                                              // PairSet B: count=2, sorted by secondGlyph (30, 40)
        let mut pairset_b = Vec::new();
        pairset_b.extend_from_slice(&be(2)); // pairValueCount
        pairset_b.extend_from_slice(&be(30));
        pairset_b.extend_from_slice(&(-30i16).to_be_bytes());
        pairset_b.extend_from_slice(&be(40));
        pairset_b.extend_from_slice(&(-50i16).to_be_bytes());

        let header_len = 10 + 4; // header + 2 pairSetOffsets
        let pairset_a_off = header_len;
        let pairset_b_off = pairset_a_off + pairset_a.len();
        let coverage_off = pairset_b_off + pairset_b.len();

        let mut b = vec![0u8; header_len];
        b[0..2].copy_from_slice(&be(1));
        b[2..4].copy_from_slice(&be(coverage_off as u16));
        b[4..6].copy_from_slice(&be(vf1));
        b[6..8].copy_from_slice(&be(0));
        b[8..10].copy_from_slice(&be(2)); // pairSetCount
        b[10..12].copy_from_slice(&be(pairset_a_off as u16));
        b[12..14].copy_from_slice(&be(pairset_b_off as u16));
        b.extend_from_slice(&pairset_a);
        b.extend_from_slice(&pairset_b);
        // Coverage format 1 {8, 9}
        b.extend_from_slice(&be(1));
        b.extend_from_slice(&be(2));
        b.extend_from_slice(&be(8));
        b.extend_from_slice(&be(9));
        b
    }

    #[test]
    fn pair_pos_format1_lookup() {
        let b = pairpos_f1_subtable();
        let pp = PairPos::parse(&b).unwrap();
        assert_eq!(pp.format(), 1);
        assert_eq!(pp.value_format1(), ValueFormat(0x0004));
        assert_eq!(pp.value_format2(), ValueFormat(0));

        // A(8) + V(40) → -40 on first glyph, empty second record.
        let pv = pp.pair(8, 40).unwrap().unwrap();
        assert_eq!(pv.first.x_advance, -40);
        assert_eq!(pv.second, ValueRecord::default());

        // B(9) + T(30) → -30; B(9) + V(40) → -50.
        assert_eq!(pp.pair(9, 30).unwrap().unwrap().first.x_advance, -30);
        assert_eq!(pp.pair(9, 40).unwrap().unwrap().first.x_advance, -50);

        // A(8) has no pair with T(30).
        assert!(pp.pair(8, 30).is_none());
        // Uncovered first glyph.
        assert!(pp.pair(7, 40).is_none());
    }

    #[test]
    fn pair_pos_format1_iter() {
        let b = pairpos_f1_subtable();
        let pp = PairPos::parse(&b).unwrap();
        let triples: Vec<_> = pp
            .iter()
            .map(|(f, s, v)| (f, s, v.unwrap().first.x_advance))
            .collect();
        assert_eq!(triples, vec![(8, 40, -40), (9, 30, -30), (9, 40, -50)]);
    }

    #[test]
    fn pair_pos_format1_rejects_count_mismatch() {
        let mut b = pairpos_f1_subtable();
        b[8..10].copy_from_slice(&be(1)); // pairSetCount=1 != coverage(2)
        assert!(matches!(PairPos::parse(&b), Err(Error::BadStructure(_))));
    }

    // -- PairPos format 2 ------------------------------------------------

    /// Build a standalone PairPosFormat2 subtable: class1Count=2,
    /// class2Count=2. valueFormat1 = X_ADVANCE, valueFormat2 = 0.
    /// classDef1 maps glyph 8→class1, glyph 9→class0(default).
    /// classDef2 maps glyph 40→class1, others→class0.
    /// Matrix xAdvance cells: [c1=0][c2=0]=0, [0][1]=0,
    /// [1][0]=11, [1][1]=22.
    fn pairpos_f2_subtable() -> Vec<u8> {
        let vf1 = 0x0004u16; // X_ADVANCE → 2 bytes; vf2=0 → 0 bytes; cell=2
        let class1_count = 2u16;
        let class2_count = 2u16;
        let cell = 2usize;
        let matrix_len = class1_count as usize * class2_count as usize * cell;

        let header_len = 16;
        let matrix_off = header_len;
        let class_def1_off = matrix_off + matrix_len;
        // ClassDef format 2, one range {8..8 → class 1}
        let cd1 = {
            let mut v = Vec::new();
            v.extend_from_slice(&be(2)); // format 2
            v.extend_from_slice(&be(1)); // classRangeCount
            v.extend_from_slice(&be(8)); // startGlyphID
            v.extend_from_slice(&be(8)); // endGlyphID
            v.extend_from_slice(&be(1)); // class
            v
        };
        let class_def2_off = class_def1_off + cd1.len();
        let cd2 = {
            let mut v = Vec::new();
            v.extend_from_slice(&be(2));
            v.extend_from_slice(&be(1));
            v.extend_from_slice(&be(40));
            v.extend_from_slice(&be(40));
            v.extend_from_slice(&be(1));
            v
        };
        let coverage_off = class_def2_off + cd2.len();

        let mut b = vec![0u8; header_len];
        b[0..2].copy_from_slice(&be(2)); // format
        b[2..4].copy_from_slice(&be(coverage_off as u16));
        b[4..6].copy_from_slice(&be(vf1));
        b[6..8].copy_from_slice(&be(0)); // vf2
        b[8..10].copy_from_slice(&be(class_def1_off as u16));
        b[10..12].copy_from_slice(&be(class_def2_off as u16));
        b[12..14].copy_from_slice(&be(class1_count));
        b[14..16].copy_from_slice(&be(class2_count));
        // matrix cells: row-major (class1 outer, class2 inner)
        // [0][0]=0, [0][1]=0, [1][0]=11, [1][1]=22
        let cells: [i16; 4] = [0, 0, 11, 22];
        for c in cells {
            b.extend_from_slice(&c.to_be_bytes());
        }
        b.extend_from_slice(&cd1);
        b.extend_from_slice(&cd2);
        // Coverage format 1 {8, 9}
        b.extend_from_slice(&be(1));
        b.extend_from_slice(&be(2));
        b.extend_from_slice(&be(8));
        b.extend_from_slice(&be(9));
        b
    }

    #[test]
    fn pair_pos_format2_class_matrix() {
        let b = pairpos_f2_subtable();
        let pp = PairPos::parse(&b).unwrap();
        assert_eq!(pp.format(), 2);

        // glyph 8 → class1; glyph 40 → class1 ⇒ cell [1][1] = 22.
        let pv = pp.pair(8, 40).unwrap().unwrap();
        assert_eq!(pv.first.x_advance, 22);
        // glyph 8 (c1) + glyph 9 (c0) ⇒ cell [1][0] = 11.
        assert_eq!(pp.pair(8, 9).unwrap().unwrap().first.x_advance, 11);
        // glyph 9 (c0) + glyph 40 (c1) ⇒ cell [0][1] = 0.
        assert_eq!(pp.pair(9, 40).unwrap().unwrap().first.x_advance, 0);

        // Direct class lookup agrees.
        assert_eq!(pp.class_pair(1, 1).unwrap().unwrap().first.x_advance, 22);
        assert_eq!(pp.class_pair(1, 0).unwrap().unwrap().first.x_advance, 11);
        assert!(pp.class_pair(2, 0).is_none()); // class out of range

        // Uncovered first glyph still returns None even though its class
        // would be 0 — the spec keys format 2 off the Coverage table.
        assert!(pp.pair(7, 40).is_none());

        // Format-2 iterator is empty.
        assert_eq!(pp.iter().count(), 0);
        // class_pair on a format-1 subtable is None.
        let f1_bytes = pairpos_f1_subtable();
        let f1 = PairPos::parse(&f1_bytes).unwrap();
        assert!(f1.class_pair(0, 0).is_none());
    }

    #[test]
    fn pair_pos_format2_rejects_bad_classdef_offset() {
        let mut b = pairpos_f2_subtable();
        b[8..10].copy_from_slice(&be(0)); // NULL classDef1Offset
        assert!(matches!(PairPos::parse(&b), Err(Error::BadStructure(_))));
    }

    #[test]
    fn pair_pos_rejects_reserved_value_format() {
        let mut b = pairpos_f1_subtable();
        b[4..6].copy_from_slice(&be(0x0100)); // reserved valueFormat1 bit
        assert!(matches!(PairPos::parse(&b), Err(Error::BadStructure(_))));
    }

    #[test]
    fn pair_pos_rejects_unknown_format() {
        let mut b = pairpos_f1_subtable();
        b[0..2].copy_from_slice(&be(7));
        assert!(matches!(PairPos::parse(&b), Err(Error::BadStructure(_))));
    }

    #[test]
    fn pair_pos_format2_truncated_matrix() {
        // Declare a large class1/class2 count that overruns the buffer.
        let mut b = pairpos_f2_subtable();
        b[12..14].copy_from_slice(&be(100)); // class1Count=100
        assert!(matches!(PairPos::parse(&b), Err(Error::UnexpectedEof)));
    }

    /// End-to-end: a GPOS byte tower whose only lookup is a type-2 pair
    /// adjustment, decoded through `GposTable::pair_pos`.
    #[test]
    fn gpos_table_pair_pos_accessor() {
        let sub = pairpos_f1_subtable();
        // Same layout as gpos_table_single_pos_accessor but lookupType=2.
        let mut bytes = vec![0u8; 54];
        bytes[0..2].copy_from_slice(&be(1));
        bytes[2..4].copy_from_slice(&be(0));
        bytes[4..6].copy_from_slice(&be(10));
        bytes[6..8].copy_from_slice(&be(22));
        bytes[8..10].copy_from_slice(&be(44));
        bytes[10..12].copy_from_slice(&be(1));
        bytes[12..16].copy_from_slice(b"DFLT");
        bytes[16..18].copy_from_slice(&be(8));
        bytes[18..20].copy_from_slice(&be(0));
        bytes[20..22].copy_from_slice(&be(0));
        bytes[22..24].copy_from_slice(&be(1));
        bytes[24..28].copy_from_slice(b"kern");
        bytes[28..30].copy_from_slice(&be(8));
        bytes[30..32].copy_from_slice(&be(0));
        bytes[32..34].copy_from_slice(&be(1));
        bytes[34..36].copy_from_slice(&be(0));
        bytes[44..46].copy_from_slice(&be(1)); // LookupList count
        bytes[46..48].copy_from_slice(&be(4)); // lookupOffset
        bytes[48..50].copy_from_slice(&be(2)); // lookupType = 2
        bytes[50..52].copy_from_slice(&be(0)); // lookupFlag
        bytes[52..54].copy_from_slice(&be(1)); // subTableCount
        bytes.extend_from_slice(&be(8)); // subtableOffset (56 - 48)
        bytes.extend_from_slice(&sub);

        let g = GposTable::parse(&bytes).unwrap();
        assert_eq!(g.lookup(0).map(|l| l.lookup_type()), Some(2));
        let pp = g.pair_pos(0, 0).unwrap().unwrap();
        assert_eq!(pp.pair(9, 40).unwrap().unwrap().first.x_advance, -50);

        // Out-of-range lookup → None.
        assert!(g.pair_pos(5, 0).is_none());
    }

    // -- ExtensionPos (lookup type 9) ------------------------------------

    /// Build a standalone PosExtensionFormat1 subtable (8-byte header
    /// followed by the wrapped subtable bytes) wrapping `inner`, which is
    /// declared as `ext_type`.
    fn build_extension_pos(ext_type: u16, inner: &[u8]) -> Vec<u8> {
        let mut b = Vec::new();
        b.extend_from_slice(&be(1)); // format
        b.extend_from_slice(&be(ext_type)); // extensionLookupType
        b.extend_from_slice(&8u32.to_be_bytes()); // extensionOffset = 8
        b.extend_from_slice(inner); // wrapped subtable at offset 8
        b
    }

    #[test]
    fn extension_pos_round_trip_wrapping_single_pos() {
        let inner = singlepos_f1_subtable();
        let b = build_extension_pos(GPOS_LOOKUP_TYPE_SINGLE, &inner);
        let ext = ExtensionPos::parse(&b).unwrap();
        assert_eq!(ext.format(), 1);
        assert_eq!(ext.extension_lookup_type(), GPOS_LOOKUP_TYPE_SINGLE);
        assert_eq!(ext.extension_offset(), 8);
        assert_eq!(ext.extension_subtable_bytes(), &inner[..]);

        // Typed resolver decodes the wrapped SinglePos.
        let sp = ext.as_single_pos().unwrap();
        assert_eq!(sp.value(11).unwrap().unwrap().x_advance, -50);
        // Wrong-type resolver rejects.
        assert!(ext.as_pair_pos().is_err());
    }

    #[test]
    fn extension_pos_round_trip_wrapping_pair_pos() {
        let inner = pairpos_f1_subtable();
        let b = build_extension_pos(GPOS_LOOKUP_TYPE_PAIR, &inner);
        let ext = ExtensionPos::parse(&b).unwrap();
        assert_eq!(ext.extension_lookup_type(), GPOS_LOOKUP_TYPE_PAIR);
        let pp = ext.as_pair_pos().unwrap();
        assert_eq!(pp.pair(9, 40).unwrap().unwrap().first.x_advance, -50);
        assert!(ext.as_single_pos().is_err());
    }

    #[test]
    fn extension_pos_raw_bytes_for_untyped_wrapped_type() {
        // Wrap a cursive (type-3) subtable this crate does not yet decode;
        // the header still parses and the raw window is reachable.
        let inner = [0xAAu8, 0xBB, 0xCC, 0xDD];
        let b = build_extension_pos(GPOS_LOOKUP_TYPE_CURSIVE, &inner);
        let ext = ExtensionPos::parse(&b).unwrap();
        assert_eq!(ext.extension_lookup_type(), GPOS_LOOKUP_TYPE_CURSIVE);
        assert_eq!(ext.extension_subtable_bytes(), &inner[..]);
        // Typed resolvers reject a non-matching declared type.
        assert!(ext.as_single_pos().is_err());
        assert!(ext.as_pair_pos().is_err());
    }

    #[test]
    fn extension_pos_rejects_unknown_format() {
        let mut b = build_extension_pos(GPOS_LOOKUP_TYPE_SINGLE, &singlepos_f1_subtable());
        b[0..2].copy_from_slice(&be(2)); // format != 1
        assert!(ExtensionPos::parse(&b).is_err());
    }

    #[test]
    fn extension_pos_rejects_extension_pointing_at_extension() {
        // The spec forbids extensionLookupType == 9.
        let inner = singlepos_f1_subtable();
        let b = build_extension_pos(GPOS_LOOKUP_TYPE_EXTENSION, &inner);
        assert!(ExtensionPos::parse(&b).is_err());
    }

    #[test]
    fn extension_pos_rejects_out_of_range_type() {
        for bad in [0u16, 10, 0xFFFF] {
            let b = build_extension_pos(bad, &singlepos_f1_subtable());
            assert!(ExtensionPos::parse(&b).is_err(), "type {bad} should reject");
        }
    }

    #[test]
    fn extension_pos_rejects_null_and_oob_offset() {
        // NULL offset.
        let mut b = build_extension_pos(GPOS_LOOKUP_TYPE_SINGLE, &singlepos_f1_subtable());
        b[4..8].copy_from_slice(&0u32.to_be_bytes());
        assert!(ExtensionPos::parse(&b).is_err());
        // Offset past the end of the byte window.
        let mut b = build_extension_pos(GPOS_LOOKUP_TYPE_SINGLE, &singlepos_f1_subtable());
        let len = b.len() as u32;
        b[4..8].copy_from_slice(&len.to_be_bytes());
        assert!(ExtensionPos::parse(&b).is_err());
    }

    #[test]
    fn extension_pos_rejects_truncated_header() {
        let full = build_extension_pos(GPOS_LOOKUP_TYPE_SINGLE, &singlepos_f1_subtable());
        // Anything shorter than the 8-byte header must fail.
        for len in 0..8 {
            assert!(
                ExtensionPos::parse(&full[..len]).is_err(),
                "len {len} should reject"
            );
        }
    }

    #[test]
    fn gpos_table_extension_pos_accessor() {
        // Same GPOS byte tower shape as `gpos_table_single_pos_accessor`,
        // but the single lookup is declared type 9 and its subtable is a
        // PosExtensionFormat1 wrapping a SinglePos.
        let sub = build_extension_pos(GPOS_LOOKUP_TYPE_SINGLE, &singlepos_f1_subtable());
        let mut bytes = vec![0u8; 54];
        bytes[0..2].copy_from_slice(&be(1));
        bytes[2..4].copy_from_slice(&be(0));
        bytes[4..6].copy_from_slice(&be(10));
        bytes[6..8].copy_from_slice(&be(22));
        bytes[8..10].copy_from_slice(&be(44));
        bytes[10..12].copy_from_slice(&be(1));
        bytes[12..16].copy_from_slice(b"DFLT");
        bytes[16..18].copy_from_slice(&be(8));
        bytes[18..20].copy_from_slice(&be(0));
        bytes[20..22].copy_from_slice(&be(0));
        bytes[22..24].copy_from_slice(&be(1));
        bytes[24..28].copy_from_slice(b"kern");
        bytes[28..30].copy_from_slice(&be(8));
        bytes[30..32].copy_from_slice(&be(0));
        bytes[32..34].copy_from_slice(&be(1));
        bytes[34..36].copy_from_slice(&be(0));
        bytes[44..46].copy_from_slice(&be(1)); // LookupList count
        bytes[46..48].copy_from_slice(&be(4)); // lookupOffset
        bytes[48..50].copy_from_slice(&be(9)); // lookupType = 9
        bytes[50..52].copy_from_slice(&be(0)); // lookupFlag
        bytes[52..54].copy_from_slice(&be(1)); // subTableCount
        bytes.extend_from_slice(&be(8)); // subtableOffset (56 - 48)
        bytes.extend_from_slice(&sub);

        let g = GposTable::parse(&bytes).unwrap();
        assert_eq!(g.lookup(0).map(|l| l.lookup_type()), Some(9));
        let ext = g.extension_pos(0, 0).unwrap().unwrap();
        assert_eq!(ext.extension_lookup_type(), GPOS_LOOKUP_TYPE_SINGLE);
        let sp = ext.as_single_pos().unwrap();
        assert_eq!(sp.value(11).unwrap().unwrap().x_advance, -50);

        // Out-of-range lookup → None.
        assert!(g.extension_pos(5, 0).is_none());
        // Wrong-type accessors on the type-9 lookup → Some(Err).
        assert!(g.single_pos(0, 0).unwrap().is_err());
        assert!(g.pair_pos(0, 0).unwrap().is_err());
    }

    // -- Anchor tables ---------------------------------------------------

    #[test]
    fn anchor_format1_design_units() {
        // format=1, x=100, y=-200.
        let mut b = Vec::new();
        b.extend_from_slice(&be(1));
        b.extend_from_slice(&(100i16).to_be_bytes());
        b.extend_from_slice(&(-200i16).to_be_bytes());
        let a = Anchor::parse(&b, 0).unwrap();
        assert_eq!(a.format, 1);
        assert_eq!(a.x, 100);
        assert_eq!(a.y, -200);
        assert_eq!(a.contour_point(), None);
        assert_eq!(a.x_device_offset(), 0);
    }

    #[test]
    fn anchor_format2_contour_point() {
        // format=2, x=5, y=6, anchorPoint=42.
        let mut b = Vec::new();
        b.extend_from_slice(&be(2));
        b.extend_from_slice(&(5i16).to_be_bytes());
        b.extend_from_slice(&(6i16).to_be_bytes());
        b.extend_from_slice(&be(42));
        let a = Anchor::parse(&b, 0).unwrap();
        assert_eq!(a.format, 2);
        assert_eq!(a.contour_point(), Some(42));
    }

    #[test]
    fn anchor_format3_device_offsets() {
        // format=3, x=1, y=2, xDeviceOffset=0x10, yDeviceOffset=0 (NULL).
        let mut b = Vec::new();
        b.extend_from_slice(&be(3));
        b.extend_from_slice(&(1i16).to_be_bytes());
        b.extend_from_slice(&(2i16).to_be_bytes());
        b.extend_from_slice(&be(0x10));
        b.extend_from_slice(&be(0));
        let a = Anchor::parse(&b, 0).unwrap();
        assert_eq!(a.format, 3);
        assert_eq!(a.x_device_offset(), 0x10);
        assert_eq!(a.y_device_offset(), 0);
        assert_eq!(a.contour_point(), None);
    }

    #[test]
    fn anchor_format3_decodes_device_table() {
        // Anchor format 3 with an X Device table at offset 10 and a NULL
        // Y device. The Device table (deltaFormat 2, 4-bit) carries the
        // spec example {1, 2, 3, -1} for ppem 12..=15.
        let mut b = Vec::new();
        b.extend_from_slice(&be(3)); // format
        b.extend_from_slice(&(1i16).to_be_bytes()); // x
        b.extend_from_slice(&(2i16).to_be_bytes()); // y
        b.extend_from_slice(&be(10)); // xDeviceOffset = 10
        b.extend_from_slice(&be(0)); // yDeviceOffset = NULL
        assert_eq!(b.len(), 10);
        // Device table @ offset 10.
        b.extend_from_slice(&be(12)); // startSize
        b.extend_from_slice(&be(15)); // endSize
        b.extend_from_slice(&be(2)); // deltaFormat (4-bit)
        b.extend_from_slice(&be(0x123F)); // {1,2,3,-1}

        let a = Anchor::parse(&b, 0).unwrap();
        // The anchor table base is the whole slice (off = 0).
        let dev = a.x_device(&b).expect("x device present").expect("decodes");
        let d = dev.as_device().expect("a Device table");
        assert_eq!(d.delta(12), 1);
        assert_eq!(d.delta(15), -1);
        // NULL Y device → None.
        assert!(a.y_device(&b).is_none());
    }

    #[test]
    fn value_record_decodes_advance_device() {
        // A ValueRecord declaring only xAdvDeviceOffset, pointing at a
        // VariationIndex table in the same subtable buffer.
        // Subtable layout: [ValueRecord @ 0 (2 bytes)] [pad] [VarIndex @ 4].
        let mut sub = Vec::new();
        sub.extend_from_slice(&be(4)); // xAdvDeviceOffset = 4
        sub.extend_from_slice(&be(0)); // pad to offset 4
        sub.extend_from_slice(&be(2)); // deltaSetOuterIndex
        sub.extend_from_slice(&be(9)); // deltaSetInnerIndex
        sub.extend_from_slice(&be(0x8000)); // deltaFormat = VARIATION_INDEX
        let fmt = ValueFormat(0x0040); // X_ADVANCE_DEVICE
        let (rec, n) = ValueRecord::parse(&sub, 0, fmt).unwrap();
        assert_eq!(n, 2);
        assert_eq!(rec.x_advance_device_offset, 4);
        let dev = rec
            .x_advance_device(&sub)
            .expect("device present")
            .expect("decodes");
        let vi = dev.as_variation_index().expect("VariationIndex");
        assert_eq!(vi.outer_index, 2);
        assert_eq!(vi.inner_index, 9);
        assert!(rec.x_placement_device(&sub).is_none());
    }

    #[test]
    fn anchor_rejects_unknown_format() {
        let mut b = Vec::new();
        b.extend_from_slice(&be(4));
        b.extend_from_slice(&[0, 0, 0, 0]);
        assert!(matches!(Anchor::parse(&b, 0), Err(Error::BadStructure(_))));
    }

    // -- MarkBasePos -----------------------------------------------------

    /// Build a standalone MarkBasePosFormat1 subtable.
    ///
    /// Marks: glyph 10 → class 0 anchor (10,200); glyph 11 → class 1
    /// anchor (15,-50). Bases: glyph 20 with class-0 anchor (30,210) and
    /// class-1 anchor (32,-40); glyph 21 with a NULL class-1 anchor.
    /// markClassCount = 2.
    fn markbasepos_subtable() -> Vec<u8> {
        // Layout (offsets from subtable start):
        //   0   /  12 / MarkBasePosFormat1 header
        //   12  /   X / markCoverage (format 1, glyphs {10,11})
        //   ..  /   X / baseCoverage (format 1, glyphs {20,21})
        //   ..  /   X / MarkArray
        //   ..  /   X / BaseArray
        //   then the Anchor tables, referenced from Mark/Base arrays.
        //
        // We compute offsets as we append.
        let mut b: Vec<u8> = Vec::new();

        // Header placeholder (12 bytes); fill offsets after layout.
        b.extend_from_slice(&be(1)); // format
        b.extend_from_slice(&be(0)); // markCoverageOffset (patch)
        b.extend_from_slice(&be(0)); // baseCoverageOffset (patch)
        b.extend_from_slice(&be(2)); // markClassCount = 2
        b.extend_from_slice(&be(0)); // markArrayOffset (patch)
        b.extend_from_slice(&be(0)); // baseArrayOffset (patch)

        // markCoverage (format 1: {10, 11}).
        let mark_cov_off = b.len();
        b.extend_from_slice(&be(1)); // coverage format
        b.extend_from_slice(&be(2)); // glyphCount
        b.extend_from_slice(&be(10));
        b.extend_from_slice(&be(11));

        // baseCoverage (format 1: {20, 21}).
        let base_cov_off = b.len();
        b.extend_from_slice(&be(1));
        b.extend_from_slice(&be(2));
        b.extend_from_slice(&be(20));
        b.extend_from_slice(&be(21));

        // MarkArray: markCount=2, two MarkRecords (4 bytes each), then
        // two Anchor tables (format 1, 6 bytes each) appended after.
        let mark_array_off = b.len();
        b.extend_from_slice(&be(2)); // markCount
                                     // markRecords start at mark_array_off+2.
                                     // anchors will sit right after the 2 records:
                                     //   records: 2 + 2*4 = 10 bytes from array start
                                     //   anchor0 at array-relative offset 10
                                     //   anchor1 at array-relative offset 16
        b.extend_from_slice(&be(0)); // markRecord0.markClass = 0
        b.extend_from_slice(&be(10)); // markRecord0.markAnchorOffset (rel array)
        b.extend_from_slice(&be(1)); // markRecord1.markClass = 1
        b.extend_from_slice(&be(16)); // markRecord1.markAnchorOffset (rel array)
                                      // anchor0 (format1, 10,200)
        b.extend_from_slice(&be(1));
        b.extend_from_slice(&(10i16).to_be_bytes());
        b.extend_from_slice(&(200i16).to_be_bytes());
        // anchor1 (format1, 15,-50)
        b.extend_from_slice(&be(1));
        b.extend_from_slice(&(15i16).to_be_bytes());
        b.extend_from_slice(&(-50i16).to_be_bytes());

        // BaseArray: baseCount=2, two BaseRecords (markClassCount=2
        // Offset16s = 4 bytes each), then anchors.
        let base_array_off = b.len();
        b.extend_from_slice(&be(2)); // baseCount
                                     // BaseRecords start at base_array_off+2.
                                     //   2 + 2*4 = 10 bytes of records
                                     //   anchors after: base0c0 @10, base0c1 @16, base1c0 @22
        b.extend_from_slice(&be(10)); // base0, class0 anchorOffset (rel array)
        b.extend_from_slice(&be(16)); // base0, class1 anchorOffset
        b.extend_from_slice(&be(22)); // base1, class0 anchorOffset
        b.extend_from_slice(&be(0)); // base1, class1 anchorOffset = NULL
                                     // base0 class0 anchor (30,210)
        b.extend_from_slice(&be(1));
        b.extend_from_slice(&(30i16).to_be_bytes());
        b.extend_from_slice(&(210i16).to_be_bytes());
        // base0 class1 anchor (32,-40)
        b.extend_from_slice(&be(1));
        b.extend_from_slice(&(32i16).to_be_bytes());
        b.extend_from_slice(&(-40i16).to_be_bytes());
        // base1 class0 anchor (33,205)
        b.extend_from_slice(&be(1));
        b.extend_from_slice(&(33i16).to_be_bytes());
        b.extend_from_slice(&(205i16).to_be_bytes());

        // Patch header offsets.
        b[2..4].copy_from_slice(&be(mark_cov_off as u16));
        b[4..6].copy_from_slice(&be(base_cov_off as u16));
        b[8..10].copy_from_slice(&be(mark_array_off as u16));
        b[10..12].copy_from_slice(&be(base_array_off as u16));
        b
    }

    #[test]
    fn markbasepos_parses_and_resolves_anchors() {
        let sub = markbasepos_subtable();
        let mbp = MarkBasePos::parse(&sub).unwrap();
        assert_eq!(mbp.format(), 1);
        assert_eq!(mbp.mark_class_count(), 2);
        assert!(mbp.mark_coverage().contains(10));
        assert!(mbp.base_coverage().contains(20));

        // Mark records.
        let m0 = mbp.mark_record(10).unwrap().unwrap();
        assert_eq!(m0.mark_class, 0);
        assert_eq!((m0.anchor.x, m0.anchor.y), (10, 200));
        let m1 = mbp.mark_record(11).unwrap().unwrap();
        assert_eq!(m1.mark_class, 1);
        assert_eq!((m1.anchor.x, m1.anchor.y), (15, -50));
        // Uncovered mark.
        assert!(mbp.mark_record(99).is_none());

        // Base anchors per class.
        let b0c0 = mbp.base_anchor(20, 0).unwrap().unwrap().unwrap();
        assert_eq!((b0c0.x, b0c0.y), (30, 210));
        let b0c1 = mbp.base_anchor(20, 1).unwrap().unwrap().unwrap();
        assert_eq!((b0c1.x, b0c1.y), (32, -40));
        // base1 class1 is a NULL offset → Ok(None).
        assert!(mbp.base_anchor(21, 1).unwrap().unwrap().is_none());
        let b1c0 = mbp.base_anchor(21, 0).unwrap().unwrap().unwrap();
        assert_eq!((b1c0.x, b1c0.y), (33, 205));
        // Uncovered base.
        assert!(mbp.base_anchor(99, 0).is_none());
    }

    #[test]
    fn markbasepos_attachment_pairs_mark_class_to_base_anchor() {
        let sub = markbasepos_subtable();
        let mbp = MarkBasePos::parse(&sub).unwrap();

        // Mark 10 (class 0) on base 20 → mark anchor (10,200), base
        // class-0 anchor (30,210).
        let at = mbp.attachment(10, 20).unwrap().unwrap();
        assert_eq!(at.mark_class, 0);
        assert_eq!((at.mark_anchor.x, at.mark_anchor.y), (10, 200));
        assert_eq!((at.base_anchor.x, at.base_anchor.y), (30, 210));

        // Mark 11 (class 1) on base 20 → base class-1 anchor (32,-40).
        let at = mbp.attachment(11, 20).unwrap().unwrap();
        assert_eq!((at.base_anchor.x, at.base_anchor.y), (32, -40));

        // Mark 11 (class 1) on base 21 → base has NULL class-1 anchor →
        // no attachment (None).
        assert!(mbp.attachment(11, 21).is_none());

        // Uncovered mark → None.
        assert!(mbp.attachment(99, 20).is_none());
    }

    #[test]
    fn markbasepos_rejects_bad_format() {
        let mut sub = markbasepos_subtable();
        sub[0..2].copy_from_slice(&be(2)); // format = 2 (undefined)
        assert!(matches!(
            MarkBasePos::parse(&sub),
            Err(Error::BadStructure(_))
        ));
    }

    #[test]
    fn markbasepos_via_gpos_accessor_and_extension() {
        // Build a GPOS table whose single lookup is type 4 with the
        // synthetic MarkBasePos subtable, then resolve via mark_base_pos.
        let sub = markbasepos_subtable();
        let mut bytes = vec![0u8; 54];
        bytes[0..2].copy_from_slice(&be(1));
        bytes[2..4].copy_from_slice(&be(0));
        bytes[4..6].copy_from_slice(&be(10));
        bytes[6..8].copy_from_slice(&be(22));
        bytes[8..10].copy_from_slice(&be(44));
        bytes[10..12].copy_from_slice(&be(1));
        bytes[12..16].copy_from_slice(b"DFLT");
        bytes[16..18].copy_from_slice(&be(8));
        bytes[18..20].copy_from_slice(&be(0));
        bytes[20..22].copy_from_slice(&be(0));
        bytes[22..24].copy_from_slice(&be(1));
        bytes[24..28].copy_from_slice(b"mark");
        bytes[28..30].copy_from_slice(&be(8));
        bytes[30..32].copy_from_slice(&be(0));
        bytes[32..34].copy_from_slice(&be(1));
        bytes[34..36].copy_from_slice(&be(0));
        bytes[44..46].copy_from_slice(&be(1)); // LookupList count
        bytes[46..48].copy_from_slice(&be(4)); // lookupOffset
        bytes[48..50].copy_from_slice(&be(4)); // lookupType = 4
        bytes[50..52].copy_from_slice(&be(0)); // lookupFlag
        bytes[52..54].copy_from_slice(&be(1)); // subTableCount
        bytes.extend_from_slice(&be(8)); // subtableOffset (56 - 48)
        bytes.extend_from_slice(&sub);

        let g = GposTable::parse(&bytes).unwrap();
        assert_eq!(g.lookup(0).map(|l| l.lookup_type()), Some(4));
        let mbp = g.mark_base_pos(0, 0).unwrap().unwrap();
        let at = mbp.attachment(10, 20).unwrap().unwrap();
        assert_eq!((at.base_anchor.x, at.base_anchor.y), (30, 210));
        // Wrong-type accessor on a type-4 lookup → Some(Err).
        assert!(g.single_pos(0, 0).unwrap().is_err());

        // Extension wrapping a type-4 subtable resolves via as_mark_base_pos.
        let ext = build_extension_pos(GPOS_LOOKUP_TYPE_MARK_TO_BASE, &sub);
        let ep = ExtensionPos::parse(&ext).unwrap();
        assert_eq!(ep.extension_lookup_type(), GPOS_LOOKUP_TYPE_MARK_TO_BASE);
        let mbp2 = ep.as_mark_base_pos().unwrap();
        let at2 = mbp2.attachment(11, 20).unwrap().unwrap();
        assert_eq!((at2.base_anchor.x, at2.base_anchor.y), (32, -40));
        // Wrong as_* resolver → Err.
        assert!(ep.as_single_pos().is_err());
    }

    // -- MarkMarkPos (Lookup Type 6) -------------------------------------

    /// Build a standalone MarkMarkPosFormat1 subtable with markClassCount=2:
    ///   mark1 Coverage {10, 11}; mark2 Coverage {20, 21}
    ///   mark1 record 10 → class 0, anchor (10,200)
    ///   mark1 record 11 → class 1, anchor (15,-50)
    ///   mark2 20 → class0 anchor (30,210), class1 anchor (32,-40)
    ///   mark2 21 → class0 anchor (33,205), class1 anchor NULL
    fn markmarkpos_subtable() -> Vec<u8> {
        let mut b: Vec<u8> = Vec::new();
        // Header (12 bytes); fill offsets after layout.
        b.extend_from_slice(&be(1)); // format
        b.extend_from_slice(&be(0)); // mark1CoverageOffset (patch)
        b.extend_from_slice(&be(0)); // mark2CoverageOffset (patch)
        b.extend_from_slice(&be(2)); // markClassCount = 2
        b.extend_from_slice(&be(0)); // mark1ArrayOffset (patch)
        b.extend_from_slice(&be(0)); // mark2ArrayOffset (patch)

        // mark1Coverage (format 1: {10, 11}).
        let mark1_cov_off = b.len();
        b.extend_from_slice(&be(1));
        b.extend_from_slice(&be(2));
        b.extend_from_slice(&be(10));
        b.extend_from_slice(&be(11));

        // mark2Coverage (format 1: {20, 21}).
        let mark2_cov_off = b.len();
        b.extend_from_slice(&be(1));
        b.extend_from_slice(&be(2));
        b.extend_from_slice(&be(20));
        b.extend_from_slice(&be(21));

        // mark1Array: markCount=2, two MarkRecords (4 bytes each), then
        // two Anchor tables (format 1, 6 bytes each).
        let mark1_array_off = b.len();
        b.extend_from_slice(&be(2)); // markCount
                                     // records 2 + 2*4 = 10 bytes; anchor0 @10, anchor1 @16
        b.extend_from_slice(&be(0)); // record0.markClass = 0
        b.extend_from_slice(&be(10)); // record0.markAnchorOffset (rel array)
        b.extend_from_slice(&be(1)); // record1.markClass = 1
        b.extend_from_slice(&be(16)); // record1.markAnchorOffset (rel array)
                                      // anchor0 (10,200)
        b.extend_from_slice(&be(1));
        b.extend_from_slice(&(10i16).to_be_bytes());
        b.extend_from_slice(&(200i16).to_be_bytes());
        // anchor1 (15,-50)
        b.extend_from_slice(&be(1));
        b.extend_from_slice(&(15i16).to_be_bytes());
        b.extend_from_slice(&(-50i16).to_be_bytes());

        // Mark2Array: mark2Count=2, two Mark2 records (markClassCount=2
        // Offset16s = 4 bytes each), then anchors.
        let mark2_array_off = b.len();
        b.extend_from_slice(&be(2)); // mark2Count
                                     // records 2 + 2*4 = 10 bytes
                                     // anchors: m20c0 @10, m20c1 @16, m21c0 @22
        b.extend_from_slice(&be(10)); // mark2[20], class0 anchorOffset (rel array)
        b.extend_from_slice(&be(16)); // mark2[20], class1 anchorOffset
        b.extend_from_slice(&be(22)); // mark2[21], class0 anchorOffset
        b.extend_from_slice(&be(0)); // mark2[21], class1 anchorOffset = NULL
                                     // mark2[20] class0 anchor (30,210)
        b.extend_from_slice(&be(1));
        b.extend_from_slice(&(30i16).to_be_bytes());
        b.extend_from_slice(&(210i16).to_be_bytes());
        // mark2[20] class1 anchor (32,-40)
        b.extend_from_slice(&be(1));
        b.extend_from_slice(&(32i16).to_be_bytes());
        b.extend_from_slice(&(-40i16).to_be_bytes());
        // mark2[21] class0 anchor (33,205)
        b.extend_from_slice(&be(1));
        b.extend_from_slice(&(33i16).to_be_bytes());
        b.extend_from_slice(&(205i16).to_be_bytes());

        // Patch header offsets.
        b[2..4].copy_from_slice(&be(mark1_cov_off as u16));
        b[4..6].copy_from_slice(&be(mark2_cov_off as u16));
        b[8..10].copy_from_slice(&be(mark1_array_off as u16));
        b[10..12].copy_from_slice(&be(mark2_array_off as u16));
        b
    }

    #[test]
    fn markmarkpos_parses_and_resolves_anchors() {
        let sub = markmarkpos_subtable();
        let mmp = MarkMarkPos::parse(&sub).unwrap();
        assert_eq!(mmp.format(), 1);
        assert_eq!(mmp.mark_class_count(), 2);
        assert!(mmp.mark1_coverage().contains(10));
        assert!(mmp.mark2_coverage().contains(20));

        // mark1 records.
        let m0 = mmp.mark1_record(10).unwrap().unwrap();
        assert_eq!(m0.mark_class, 0);
        assert_eq!((m0.anchor.x, m0.anchor.y), (10, 200));
        let m1 = mmp.mark1_record(11).unwrap().unwrap();
        assert_eq!(m1.mark_class, 1);
        assert_eq!((m1.anchor.x, m1.anchor.y), (15, -50));
        // Uncovered mark1.
        assert!(mmp.mark1_record(99).is_none());

        // mark2 anchors per class.
        let m20c0 = mmp.mark2_anchor(20, 0).unwrap().unwrap().unwrap();
        assert_eq!((m20c0.x, m20c0.y), (30, 210));
        let m20c1 = mmp.mark2_anchor(20, 1).unwrap().unwrap().unwrap();
        assert_eq!((m20c1.x, m20c1.y), (32, -40));
        let m21c0 = mmp.mark2_anchor(21, 0).unwrap().unwrap().unwrap();
        assert_eq!((m21c0.x, m21c0.y), (33, 205));
        // mark2[21] class1 is a NULL offset → Ok(None).
        assert!(mmp.mark2_anchor(21, 1).unwrap().unwrap().is_none());
        // Uncovered mark2.
        assert!(mmp.mark2_anchor(99, 0).is_none());
    }

    #[test]
    fn markmarkpos_attachment_pairs_mark_class_to_mark2_anchor() {
        let sub = markmarkpos_subtable();
        let mmp = MarkMarkPos::parse(&sub).unwrap();

        // mark1 10 (class 0) on mark2 20 → mark1 anchor (10,200), mark2
        // class-0 anchor (30,210).
        let at = mmp.attachment(10, 20).unwrap().unwrap();
        assert_eq!(at.mark_class, 0);
        assert_eq!((at.mark1_anchor.x, at.mark1_anchor.y), (10, 200));
        assert_eq!((at.mark2_anchor.x, at.mark2_anchor.y), (30, 210));

        // mark1 11 (class 1) on mark2 20 → mark2 class-1 anchor (32,-40).
        let at = mmp.attachment(11, 20).unwrap().unwrap();
        assert_eq!((at.mark2_anchor.x, at.mark2_anchor.y), (32, -40));

        // mark1 11 (class 1) on mark2 21 → mark2 has NULL class-1 anchor →
        // no attachment (None).
        assert!(mmp.attachment(11, 21).is_none());

        // Uncovered mark1 → None.
        assert!(mmp.attachment(99, 20).is_none());
    }

    #[test]
    fn markmarkpos_rejects_bad_format() {
        let mut sub = markmarkpos_subtable();
        sub[0..2].copy_from_slice(&be(2)); // format = 2 (undefined)
        assert!(matches!(
            MarkMarkPos::parse(&sub),
            Err(Error::BadStructure(_))
        ));
    }

    #[test]
    fn markmarkpos_rejects_overflowing_mark2count() {
        let mut sub = markmarkpos_subtable();
        // Inflate mark2Count so the declared Mark2 record array (mark2Count
        // × markClassCount Offset16s) overruns the buffer → UnexpectedEof.
        let mark2_array_off = u16::from_be_bytes([sub[10], sub[11]]) as usize;
        sub[mark2_array_off..mark2_array_off + 2].copy_from_slice(&be(0x1000));
        assert!(matches!(
            MarkMarkPos::parse(&sub),
            Err(Error::UnexpectedEof)
        ));
    }

    #[test]
    fn markmarkpos_via_gpos_accessor_and_extension() {
        // Build a GPOS table whose single lookup is type 6 with the
        // synthetic MarkMarkPos subtable, then resolve via mark_mark_pos.
        let sub = markmarkpos_subtable();
        let mut bytes = vec![0u8; 54];
        bytes[0..2].copy_from_slice(&be(1));
        bytes[2..4].copy_from_slice(&be(0));
        bytes[4..6].copy_from_slice(&be(10));
        bytes[6..8].copy_from_slice(&be(22));
        bytes[8..10].copy_from_slice(&be(44));
        bytes[10..12].copy_from_slice(&be(1));
        bytes[12..16].copy_from_slice(b"DFLT");
        bytes[16..18].copy_from_slice(&be(8));
        bytes[18..20].copy_from_slice(&be(0));
        bytes[20..22].copy_from_slice(&be(0));
        bytes[22..24].copy_from_slice(&be(1));
        bytes[24..28].copy_from_slice(b"mkmk");
        bytes[28..30].copy_from_slice(&be(8));
        bytes[30..32].copy_from_slice(&be(0));
        bytes[32..34].copy_from_slice(&be(1));
        bytes[34..36].copy_from_slice(&be(0));
        bytes[44..46].copy_from_slice(&be(1)); // LookupList count
        bytes[46..48].copy_from_slice(&be(4)); // lookupOffset
        bytes[48..50].copy_from_slice(&be(6)); // lookupType = 6
        bytes[50..52].copy_from_slice(&be(0)); // lookupFlag
        bytes[52..54].copy_from_slice(&be(1)); // subTableCount
        bytes.extend_from_slice(&be(8)); // subtableOffset (56 - 48)
        bytes.extend_from_slice(&sub);

        let g = GposTable::parse(&bytes).unwrap();
        assert_eq!(g.lookup(0).map(|l| l.lookup_type()), Some(6));
        let mmp = g.mark_mark_pos(0, 0).unwrap().unwrap();
        let at = mmp.attachment(10, 20).unwrap().unwrap();
        assert_eq!((at.mark2_anchor.x, at.mark2_anchor.y), (30, 210));
        // Wrong-type accessor on a type-6 lookup → Some(Err).
        assert!(g.single_pos(0, 0).unwrap().is_err());
        assert!(g.mark_base_pos(0, 0).unwrap().is_err());

        // Extension wrapping a type-6 subtable resolves via as_mark_mark_pos.
        let ext = build_extension_pos(GPOS_LOOKUP_TYPE_MARK_TO_MARK, &sub);
        let ep = ExtensionPos::parse(&ext).unwrap();
        assert_eq!(ep.extension_lookup_type(), GPOS_LOOKUP_TYPE_MARK_TO_MARK);
        let mmp2 = ep.as_mark_mark_pos().unwrap();
        let at2 = mmp2.attachment(11, 20).unwrap().unwrap();
        assert_eq!((at2.mark2_anchor.x, at2.mark2_anchor.y), (32, -40));
        // Wrong as_* resolver → Err.
        assert!(ep.as_mark_base_pos().is_err());
    }

    // -- CursivePos (Lookup Type 3) --------------------------------------

    /// Build a standalone CursivePosFormat1 subtable covering glyphs
    /// {10, 11, 12}:
    ///   * glyph 10: entry (5,100)   exit (90,100)
    ///   * glyph 11: entry (8,100)   exit NULL  (no following join)
    ///   * glyph 12: entry NULL      exit (95,100)  (no preceding join)
    fn cursivepos_subtable() -> Vec<u8> {
        let mut b: Vec<u8> = Vec::new();
        // Header (6 bytes); offsets patched after layout.
        b.extend_from_slice(&be(1)); // format
        b.extend_from_slice(&be(0)); // coverageOffset (patch)
        b.extend_from_slice(&be(3)); // entryExitCount = 3

        // EntryExit records (4 bytes each); offsets patched after the
        // anchors are appended.
        let rec_off = b.len();
        for _ in 0..3 {
            b.extend_from_slice(&be(0)); // entryAnchorOffset (patch)
            b.extend_from_slice(&be(0)); // exitAnchorOffset (patch)
        }

        // Coverage (format 1: {10, 11, 12}).
        let cov_off = b.len();
        b.extend_from_slice(&be(1));
        b.extend_from_slice(&be(3));
        b.extend_from_slice(&be(10));
        b.extend_from_slice(&be(11));
        b.extend_from_slice(&be(12));

        // Anchor tables (format 1, 6 bytes each), offsets from subtable
        // start.
        let anchor = |b: &mut Vec<u8>, x: i16, y: i16| -> u16 {
            let off = b.len() as u16;
            b.extend_from_slice(&be(1));
            b.extend_from_slice(&x.to_be_bytes());
            b.extend_from_slice(&y.to_be_bytes());
            off
        };
        let g10_entry = anchor(&mut b, 5, 100);
        let g10_exit = anchor(&mut b, 90, 100);
        let g11_entry = anchor(&mut b, 8, 100);
        let g12_exit = anchor(&mut b, 95, 100);

        // Patch coverage offset.
        b[2..4].copy_from_slice(&be(cov_off as u16));
        // Patch EntryExit records (entry, exit) per glyph.
        b[rec_off..rec_off + 2].copy_from_slice(&be(g10_entry));
        b[rec_off + 2..rec_off + 4].copy_from_slice(&be(g10_exit));
        b[rec_off + 4..rec_off + 6].copy_from_slice(&be(g11_entry));
        b[rec_off + 6..rec_off + 8].copy_from_slice(&be(0)); // glyph 11 exit NULL
        b[rec_off + 8..rec_off + 10].copy_from_slice(&be(0)); // glyph 12 entry NULL
        b[rec_off + 10..rec_off + 12].copy_from_slice(&be(g12_exit));
        b
    }

    #[test]
    fn cursivepos_parses_and_resolves_entry_exit() {
        let sub = cursivepos_subtable();
        let cp = CursivePos::parse(&sub).unwrap();
        assert_eq!(cp.format(), 1);
        assert_eq!(cp.entry_exit_count(), 3);
        assert!(cp.coverage().contains(10));
        assert!(cp.coverage().contains(12));
        assert!(!cp.coverage().contains(99));

        // Glyph 10 has both anchors.
        let ee = cp.entry_exit(10).unwrap().unwrap();
        let entry = ee.entry_anchor.unwrap();
        let exit = ee.exit_anchor.unwrap();
        assert_eq!((entry.x, entry.y), (5, 100));
        assert_eq!((exit.x, exit.y), (90, 100));

        // Glyph 11: entry present, exit NULL.
        let ee = cp.entry_exit(11).unwrap().unwrap();
        assert_eq!(ee.entry_anchor.map(|a| a.x), Some(8));
        assert!(ee.exit_anchor.is_none());

        // Glyph 12: entry NULL, exit present.
        let ee = cp.entry_exit(12).unwrap().unwrap();
        assert!(ee.entry_anchor.is_none());
        assert_eq!(ee.exit_anchor.map(|a| a.x), Some(95));

        // Uncovered glyph → None.
        assert!(cp.entry_exit(99).is_none());
    }

    #[test]
    fn cursivepos_attachment_aligns_exit_to_entry() {
        let sub = cursivepos_subtable();
        let cp = CursivePos::parse(&sub).unwrap();

        // Join 10 → 11: 10's exit (90,100) aligns with 11's entry (8,100).
        let at = cp.attachment(10, 11).unwrap().unwrap();
        assert_eq!((at.exit_anchor.x, at.exit_anchor.y), (90, 100));
        assert_eq!((at.entry_anchor.x, at.entry_anchor.y), (8, 100));

        // Join 10 → 10 also works (10 has both an exit and an entry).
        let at = cp.attachment(10, 10).unwrap().unwrap();
        assert_eq!(at.exit_anchor.x, 90);
        assert_eq!(at.entry_anchor.x, 5);

        // Join 11 → 10: glyph 11 has NULL exit → no adjustment.
        assert!(cp.attachment(11, 10).is_none());

        // Join 10 → 12: glyph 12 has NULL entry → no adjustment.
        assert!(cp.attachment(10, 12).is_none());

        // Uncovered first/second → None.
        assert!(cp.attachment(99, 11).is_none());
        assert!(cp.attachment(10, 99).is_none());
    }

    #[test]
    fn cursivepos_rejects_bad_format() {
        let mut sub = cursivepos_subtable();
        sub[0..2].copy_from_slice(&be(2)); // format = 2 (undefined)
        assert!(matches!(
            CursivePos::parse(&sub),
            Err(Error::BadStructure(_))
        ));
    }

    #[test]
    fn cursivepos_rejects_truncated_array() {
        // A valid Coverage sits at the buffer's tail, but entryExitCount
        // claims more records than the bytes between the 6-byte header and
        // that Coverage can hold → the EntryExit array overruns into EOF.
        let mut b: Vec<u8> = Vec::new();
        b.extend_from_slice(&be(1)); // format
        b.extend_from_slice(&be(0)); // coverageOffset (patch)
        b.extend_from_slice(&be(3)); // entryExitCount = 3 → needs 12 array bytes
                                     // Only 4 bytes of array space before Coverage starts.
        b.extend_from_slice(&be(0));
        b.extend_from_slice(&be(0));
        let cov_off = b.len();
        b.extend_from_slice(&be(1)); // coverage format 1
        b.extend_from_slice(&be(0)); // glyphCount = 0
        b[2..4].copy_from_slice(&be(cov_off as u16));
        assert!(matches!(CursivePos::parse(&b), Err(Error::UnexpectedEof)));
    }

    #[test]
    fn cursivepos_via_gpos_accessor_and_extension() {
        // Build a GPOS table whose single lookup is type 3 with the
        // synthetic CursivePos subtable, then resolve via cursive_pos.
        let sub = cursivepos_subtable();
        let mut bytes = vec![0u8; 54];
        bytes[0..2].copy_from_slice(&be(1));
        bytes[2..4].copy_from_slice(&be(0));
        bytes[4..6].copy_from_slice(&be(10));
        bytes[6..8].copy_from_slice(&be(22));
        bytes[8..10].copy_from_slice(&be(44));
        bytes[10..12].copy_from_slice(&be(1));
        bytes[12..16].copy_from_slice(b"DFLT");
        bytes[16..18].copy_from_slice(&be(8));
        bytes[18..20].copy_from_slice(&be(0));
        bytes[20..22].copy_from_slice(&be(0));
        bytes[22..24].copy_from_slice(&be(1));
        bytes[24..28].copy_from_slice(b"curs");
        bytes[28..30].copy_from_slice(&be(8));
        bytes[30..32].copy_from_slice(&be(0));
        bytes[32..34].copy_from_slice(&be(1));
        bytes[34..36].copy_from_slice(&be(0));
        bytes[44..46].copy_from_slice(&be(1)); // LookupList count
        bytes[46..48].copy_from_slice(&be(4)); // lookupOffset
        bytes[48..50].copy_from_slice(&be(3)); // lookupType = 3
        bytes[50..52].copy_from_slice(&be(0)); // lookupFlag
        bytes[52..54].copy_from_slice(&be(1)); // subTableCount
        bytes.extend_from_slice(&be(8)); // subtableOffset (56 - 48)
        bytes.extend_from_slice(&sub);

        let g = GposTable::parse(&bytes).unwrap();
        assert_eq!(g.lookup(0).map(|l| l.lookup_type()), Some(3));
        let cp = g.cursive_pos(0, 0).unwrap().unwrap();
        let at = cp.attachment(10, 11).unwrap().unwrap();
        assert_eq!(at.exit_anchor.x, 90);
        // Wrong-type accessor on a type-3 lookup → Some(Err).
        assert!(g.single_pos(0, 0).unwrap().is_err());

        // Extension wrapping a type-3 subtable resolves via as_cursive_pos.
        let ext = build_extension_pos(GPOS_LOOKUP_TYPE_CURSIVE, &sub);
        let ep = ExtensionPos::parse(&ext).unwrap();
        assert_eq!(ep.extension_lookup_type(), GPOS_LOOKUP_TYPE_CURSIVE);
        let cp2 = ep.as_cursive_pos().unwrap();
        let at2 = cp2.attachment(10, 11).unwrap().unwrap();
        assert_eq!(at2.entry_anchor.x, 8);
        // Wrong as_* resolver → Err.
        assert!(ep.as_single_pos().is_err());
    }

    // -- MarkLigPos (Lookup Type 5) --------------------------------------

    /// Build a standalone MarkLigPosFormat1 subtable with markClassCount=2:
    ///   mark Coverage {10, 11}; ligature Coverage {20}
    ///   mark 10 → class 0, anchor (10,200)
    ///   mark 11 → class 1, anchor (15,-50)
    ///   ligature 20 has 2 components:
    ///     comp0: class0 anchor (30,210), class1 anchor (32,-40)
    ///     comp1: class0 anchor (33,205), class1 anchor NULL
    fn markligpos_subtable() -> Vec<u8> {
        let mut b: Vec<u8> = Vec::new();
        // Header (12 bytes); patch offsets after layout.
        b.extend_from_slice(&be(1)); // format
        b.extend_from_slice(&be(0)); // markCoverageOffset (patch)
        b.extend_from_slice(&be(0)); // ligatureCoverageOffset (patch)
        b.extend_from_slice(&be(2)); // markClassCount = 2
        b.extend_from_slice(&be(0)); // markArrayOffset (patch)
        b.extend_from_slice(&be(0)); // ligatureArrayOffset (patch)

        // markCoverage (format 1: {10, 11}).
        let mark_cov_off = b.len();
        b.extend_from_slice(&be(1));
        b.extend_from_slice(&be(2));
        b.extend_from_slice(&be(10));
        b.extend_from_slice(&be(11));

        // ligatureCoverage (format 1: {20}).
        let lig_cov_off = b.len();
        b.extend_from_slice(&be(1));
        b.extend_from_slice(&be(1));
        b.extend_from_slice(&be(20));

        // MarkArray: markCount=2, two MarkRecords (4 bytes each), then two
        // Anchor tables (format 1, 6 bytes each).
        let mark_array_off = b.len();
        b.extend_from_slice(&be(2)); // markCount
                                     // records 2 + 2*4 = 10 bytes; anchor0 @10, anchor1 @16
        b.extend_from_slice(&be(0)); // record0.markClass = 0
        b.extend_from_slice(&be(10)); // record0.markAnchorOffset (rel array)
        b.extend_from_slice(&be(1)); // record1.markClass = 1
        b.extend_from_slice(&be(16)); // record1.markAnchorOffset (rel array)
                                      // anchor0 (10,200)
        b.extend_from_slice(&be(1));
        b.extend_from_slice(&(10i16).to_be_bytes());
        b.extend_from_slice(&(200i16).to_be_bytes());
        // anchor1 (15,-50)
        b.extend_from_slice(&be(1));
        b.extend_from_slice(&(15i16).to_be_bytes());
        b.extend_from_slice(&(-50i16).to_be_bytes());

        // LigatureArray: ligatureCount=1, one Offset16 to a LigatureAttach
        // table (relative to LigatureArray start).
        let lig_array_off = b.len();
        b.extend_from_slice(&be(1)); // ligatureCount
        b.extend_from_slice(&be(4)); // ligatureAttachOffset (rel array): 2 + 1*2 = 4

        // LigatureAttach table at lig_array_off + 4:
        //   componentCount=2, then 2 ComponentRecords (markClassCount=2
        //   Offset16s = 4 bytes each), then anchors.
        // record bytes: 2 + 2*4 = 10; anchors follow at table-rel:
        //   comp0c0 @10, comp0c1 @16, comp1c0 @22
        b.extend_from_slice(&be(2)); // componentCount
        b.extend_from_slice(&be(10)); // comp0, class0 anchorOffset (rel attach)
        b.extend_from_slice(&be(16)); // comp0, class1 anchorOffset
        b.extend_from_slice(&be(22)); // comp1, class0 anchorOffset
        b.extend_from_slice(&be(0)); // comp1, class1 anchorOffset = NULL
                                     // comp0 class0 anchor (30,210)
        b.extend_from_slice(&be(1));
        b.extend_from_slice(&(30i16).to_be_bytes());
        b.extend_from_slice(&(210i16).to_be_bytes());
        // comp0 class1 anchor (32,-40)
        b.extend_from_slice(&be(1));
        b.extend_from_slice(&(32i16).to_be_bytes());
        b.extend_from_slice(&(-40i16).to_be_bytes());
        // comp1 class0 anchor (33,205)
        b.extend_from_slice(&be(1));
        b.extend_from_slice(&(33i16).to_be_bytes());
        b.extend_from_slice(&(205i16).to_be_bytes());

        // Patch header offsets.
        b[2..4].copy_from_slice(&be(mark_cov_off as u16));
        b[4..6].copy_from_slice(&be(lig_cov_off as u16));
        b[8..10].copy_from_slice(&be(mark_array_off as u16));
        b[10..12].copy_from_slice(&be(lig_array_off as u16));
        b
    }

    #[test]
    fn markligpos_parses_and_resolves_anchors() {
        let sub = markligpos_subtable();
        let mlp = MarkLigPos::parse(&sub).unwrap();
        assert_eq!(mlp.format(), 1);
        assert_eq!(mlp.mark_class_count(), 2);
        assert!(mlp.mark_coverage().contains(10));
        assert!(mlp.ligature_coverage().contains(20));

        // Mark records.
        let m0 = mlp.mark_record(10).unwrap().unwrap();
        assert_eq!(m0.mark_class, 0);
        assert_eq!((m0.anchor.x, m0.anchor.y), (10, 200));
        let m1 = mlp.mark_record(11).unwrap().unwrap();
        assert_eq!(m1.mark_class, 1);
        assert_eq!((m1.anchor.x, m1.anchor.y), (15, -50));
        // Uncovered mark.
        assert!(mlp.mark_record(99).is_none());

        // Component count.
        assert_eq!(mlp.component_count(20).unwrap().unwrap(), 2);
        assert!(mlp.component_count(99).is_none());

        // Ligature component anchors per (component, class).
        let c0k0 = mlp.ligature_anchor(20, 0, 0).unwrap().unwrap().unwrap();
        assert_eq!((c0k0.x, c0k0.y), (30, 210));
        let c0k1 = mlp.ligature_anchor(20, 0, 1).unwrap().unwrap().unwrap();
        assert_eq!((c0k1.x, c0k1.y), (32, -40));
        let c1k0 = mlp.ligature_anchor(20, 1, 0).unwrap().unwrap().unwrap();
        assert_eq!((c1k0.x, c1k0.y), (33, 205));
        // comp1 class1 is a NULL offset → Ok(None).
        assert!(mlp.ligature_anchor(20, 1, 1).unwrap().unwrap().is_none());
        // Uncovered ligature.
        assert!(mlp.ligature_anchor(99, 0, 0).is_none());
        // Out-of-range component → Err.
        assert!(mlp.ligature_anchor(20, 2, 0).unwrap().is_err());
        // Out-of-range mark class → Err.
        assert!(mlp.ligature_anchor(20, 0, 2).unwrap().is_err());
    }

    #[test]
    fn markligpos_attachment_selects_component_and_class() {
        let sub = markligpos_subtable();
        let mlp = MarkLigPos::parse(&sub).unwrap();

        // Mark 10 (class 0) on ligature 20 component 0 → mark anchor
        // (10,200), component-0 class-0 anchor (30,210).
        let at = mlp.attachment(10, 20, 0).unwrap().unwrap();
        assert_eq!(at.mark_class, 0);
        assert_eq!(at.component, 0);
        assert_eq!((at.mark_anchor.x, at.mark_anchor.y), (10, 200));
        assert_eq!((at.ligature_anchor.x, at.ligature_anchor.y), (30, 210));

        // Mark 11 (class 1) on ligature 20 component 0 → component-0
        // class-1 anchor (32,-40).
        let at = mlp.attachment(11, 20, 0).unwrap().unwrap();
        assert_eq!((at.ligature_anchor.x, at.ligature_anchor.y), (32, -40));

        // Mark 10 (class 0) on ligature 20 component 1 → component-1
        // class-0 anchor (33,205). Same mark, different component picks a
        // different base anchor — the defining property of Type 5.
        let at = mlp.attachment(10, 20, 1).unwrap().unwrap();
        assert_eq!(at.component, 1);
        assert_eq!((at.ligature_anchor.x, at.ligature_anchor.y), (33, 205));

        // Mark 11 (class 1) on component 1 → NULL class-1 anchor → no
        // attachment (None).
        assert!(mlp.attachment(11, 20, 1).is_none());

        // Uncovered mark → None.
        assert!(mlp.attachment(99, 20, 0).is_none());
        // Uncovered ligature → None.
        assert!(mlp.attachment(10, 99, 0).is_none());
        // Out-of-range component → Some(Err).
        assert!(mlp.attachment(10, 20, 5).unwrap().is_err());
    }

    #[test]
    fn markligpos_rejects_bad_format() {
        let mut sub = markligpos_subtable();
        sub[0..2].copy_from_slice(&be(2)); // format = 2 (undefined)
        assert!(matches!(
            MarkLigPos::parse(&sub),
            Err(Error::BadStructure(_))
        ));
    }

    #[test]
    fn markligpos_rejects_zero_mark_class_count() {
        let mut sub = markligpos_subtable();
        sub[6..8].copy_from_slice(&be(0)); // markClassCount = 0
        assert!(matches!(
            MarkLigPos::parse(&sub),
            Err(Error::BadStructure(_))
        ));
    }

    #[test]
    fn markligpos_rejects_truncated_ligature_array() {
        // Inflate ligatureCount so the offset array overruns the buffer.
        let mut sub = markligpos_subtable();
        let lig_array_off = u16::from_be_bytes([sub[10], sub[11]]) as usize;
        sub[lig_array_off..lig_array_off + 2].copy_from_slice(&be(9999));
        assert!(matches!(MarkLigPos::parse(&sub), Err(Error::UnexpectedEof)));
    }

    #[test]
    fn markligpos_rejects_truncated_ligature_attach() {
        // A valid parse, but componentCount inflated so component_count_at
        // detects the ComponentRecords overrun.
        let sub = markligpos_subtable();
        let mlp = MarkLigPos::parse(&sub).unwrap();
        // Locate the LigatureAttach componentCount field and corrupt it on
        // a private copy, then re-parse + query.
        let mut bad = sub.clone();
        let lig_array_off = u16::from_be_bytes([bad[10], bad[11]]) as usize;
        let attach_rel =
            u16::from_be_bytes([bad[lig_array_off + 2], bad[lig_array_off + 3]]) as usize;
        let attach_off = lig_array_off + attach_rel;
        bad[attach_off..attach_off + 2].copy_from_slice(&be(9999));
        let mlp_bad = MarkLigPos::parse(&bad).unwrap();
        assert!(mlp_bad.component_count(20).unwrap().is_err());
        // The healthy fixture still resolves cleanly.
        assert_eq!(mlp.component_count(20).unwrap().unwrap(), 2);
    }

    #[test]
    fn markligpos_via_gpos_accessor_and_extension() {
        // Build a GPOS table whose single lookup is type 5 with the
        // synthetic MarkLigPos subtable, then resolve via mark_lig_pos.
        let sub = markligpos_subtable();
        let mut bytes = vec![0u8; 54];
        bytes[0..2].copy_from_slice(&be(1));
        bytes[2..4].copy_from_slice(&be(0));
        bytes[4..6].copy_from_slice(&be(10));
        bytes[6..8].copy_from_slice(&be(22));
        bytes[8..10].copy_from_slice(&be(44));
        bytes[10..12].copy_from_slice(&be(1));
        bytes[12..16].copy_from_slice(b"DFLT");
        bytes[16..18].copy_from_slice(&be(8));
        bytes[18..20].copy_from_slice(&be(0));
        bytes[20..22].copy_from_slice(&be(0));
        bytes[22..24].copy_from_slice(&be(1));
        bytes[24..28].copy_from_slice(b"mark");
        bytes[28..30].copy_from_slice(&be(8));
        bytes[30..32].copy_from_slice(&be(0));
        bytes[32..34].copy_from_slice(&be(1));
        bytes[34..36].copy_from_slice(&be(0));
        bytes[44..46].copy_from_slice(&be(1)); // LookupList count
        bytes[46..48].copy_from_slice(&be(4)); // lookupOffset
        bytes[48..50].copy_from_slice(&be(5)); // lookupType = 5
        bytes[50..52].copy_from_slice(&be(0)); // lookupFlag
        bytes[52..54].copy_from_slice(&be(1)); // subTableCount
        bytes.extend_from_slice(&be(8)); // subtableOffset (56 - 48)
        bytes.extend_from_slice(&sub);

        let g = GposTable::parse(&bytes).unwrap();
        assert_eq!(g.lookup(0).map(|l| l.lookup_type()), Some(5));
        let mlp = g.mark_lig_pos(0, 0).unwrap().unwrap();
        let at = mlp.attachment(10, 20, 1).unwrap().unwrap();
        assert_eq!((at.ligature_anchor.x, at.ligature_anchor.y), (33, 205));
        // Wrong-type accessor on a type-5 lookup → Some(Err).
        assert!(g.mark_base_pos(0, 0).unwrap().is_err());

        // Extension wrapping a type-5 subtable resolves via as_mark_lig_pos.
        let ext = build_extension_pos(GPOS_LOOKUP_TYPE_MARK_TO_LIGATURE, &sub);
        let ep = ExtensionPos::parse(&ext).unwrap();
        assert_eq!(
            ep.extension_lookup_type(),
            GPOS_LOOKUP_TYPE_MARK_TO_LIGATURE
        );
        let mlp2 = ep.as_mark_lig_pos().unwrap();
        let at2 = mlp2.attachment(11, 20, 0).unwrap().unwrap();
        assert_eq!((at2.ligature_anchor.x, at2.ligature_anchor.y), (32, -40));
        // Wrong as_* resolver → Err.
        assert!(ep.as_single_pos().is_err());
    }

    // -- Contextual positioning (Lookup Types 7 / 8) ---------------------

    /// A minimal SequenceContextFormat3 subtable: one input position with
    /// Coverage {10}, one SequenceLookup record (seqIndex=0, lookup=3).
    fn seqctx_format3_subtable() -> Vec<u8> {
        let mut b = Vec::new();
        b.extend_from_slice(&be(3)); // format
        b.extend_from_slice(&be(1)); // glyphCount
        b.extend_from_slice(&be(1)); // seqLookupCount
        b.extend_from_slice(&be(0)); // coverageOffset[0] (patch)
        b.extend_from_slice(&be(0)); // seqLookup.sequenceIndex
        b.extend_from_slice(&be(3)); // seqLookup.lookupListIndex
        let cov = b.len();
        b.extend_from_slice(&be(1)); // coverage format 1
        b.extend_from_slice(&be(1)); // glyphCount
        b.extend_from_slice(&be(10)); // glyph 10
        b[6..8].copy_from_slice(&be(cov as u16));
        b
    }

    /// A minimal ChainedSequenceContextFormat3 subtable: empty backtrack,
    /// input Coverage {10}, empty lookahead, lookup (0, 5).
    fn chainctx_format3_subtable() -> Vec<u8> {
        let mut b = Vec::new();
        b.extend_from_slice(&be(3)); // format
        b.extend_from_slice(&be(0)); // backtrackGlyphCount
        b.extend_from_slice(&be(1)); // inputGlyphCount
        b.extend_from_slice(&be(0)); // inputCoverageOffset[0] (patch @ 6)
        b.extend_from_slice(&be(0)); // lookaheadGlyphCount
        b.extend_from_slice(&be(1)); // seqLookupCount
        b.extend_from_slice(&be(0)); // seqLookup.sequenceIndex
        b.extend_from_slice(&be(5)); // seqLookup.lookupListIndex
        let cov = b.len();
        b.extend_from_slice(&be(1));
        b.extend_from_slice(&be(1));
        b.extend_from_slice(&be(10));
        b[6..8].copy_from_slice(&be(cov as u16));
        b
    }

    /// Build a GPOS byte tower whose single lookup has `lookup_type` and
    /// wraps `sub`.
    fn gpos_with_lookup(lookup_type: u16, sub: &[u8]) -> Vec<u8> {
        let mut bytes = vec![0u8; 54];
        bytes[0..2].copy_from_slice(&be(1));
        bytes[4..6].copy_from_slice(&be(10));
        bytes[6..8].copy_from_slice(&be(22));
        bytes[8..10].copy_from_slice(&be(44));
        bytes[10..12].copy_from_slice(&be(1));
        bytes[12..16].copy_from_slice(b"DFLT");
        bytes[16..18].copy_from_slice(&be(8));
        bytes[22..24].copy_from_slice(&be(1));
        bytes[24..28].copy_from_slice(b"test");
        bytes[28..30].copy_from_slice(&be(8));
        bytes[32..34].copy_from_slice(&be(1));
        bytes[44..46].copy_from_slice(&be(1)); // LookupList count
        bytes[46..48].copy_from_slice(&be(4)); // lookupOffset
        bytes[48..50].copy_from_slice(&be(lookup_type));
        bytes[52..54].copy_from_slice(&be(1)); // subTableCount
        bytes.extend_from_slice(&be(8)); // subtableOffset (56 - 48)
        bytes.extend_from_slice(sub);
        bytes
    }

    #[test]
    fn context_pos_via_gpos_accessor_and_extension() {
        let sub = seqctx_format3_subtable();
        let bytes = gpos_with_lookup(GPOS_LOOKUP_TYPE_CONTEXT, &sub);
        let g = GposTable::parse(&bytes).unwrap();
        assert_eq!(g.lookup(0).map(|l| l.lookup_type()), Some(7));
        let ctx = g.context_pos(0, 0).unwrap().unwrap();
        assert_eq!(ctx.format(), 3);
        // Wrong-type accessor on a type-7 lookup → Some(Err).
        assert!(g.chained_context_pos(0, 0).unwrap().is_err());

        // Extension wrapping a type-7 subtable resolves via as_context_pos.
        let ext = build_extension_pos(GPOS_LOOKUP_TYPE_CONTEXT, &sub);
        let ep = ExtensionPos::parse(&ext).unwrap();
        assert_eq!(ep.extension_lookup_type(), GPOS_LOOKUP_TYPE_CONTEXT);
        let ctx2 = ep.as_context_pos().unwrap();
        assert_eq!(ctx2.format(), 3);
        // Wrong as_* resolver → Err.
        assert!(ep.as_chained_context_pos().is_err());
    }

    #[test]
    fn chained_context_pos_via_gpos_accessor_and_extension() {
        let sub = chainctx_format3_subtable();
        let bytes = gpos_with_lookup(GPOS_LOOKUP_TYPE_CHAINED_CONTEXT, &sub);
        let g = GposTable::parse(&bytes).unwrap();
        assert_eq!(g.lookup(0).map(|l| l.lookup_type()), Some(8));
        let ctx = g.chained_context_pos(0, 0).unwrap().unwrap();
        assert_eq!(ctx.format(), 3);
        // Wrong-type accessor on a type-8 lookup → Some(Err).
        assert!(g.context_pos(0, 0).unwrap().is_err());

        // Extension wrapping a type-8 subtable resolves via the
        // as_chained_context_pos resolver.
        let ext = build_extension_pos(GPOS_LOOKUP_TYPE_CHAINED_CONTEXT, &sub);
        let ep = ExtensionPos::parse(&ext).unwrap();
        assert_eq!(ep.extension_lookup_type(), GPOS_LOOKUP_TYPE_CHAINED_CONTEXT);
        let ctx2 = ep.as_chained_context_pos().unwrap();
        assert_eq!(ctx2.format(), 3);
        assert!(ep.as_context_pos().is_err());
    }
}
