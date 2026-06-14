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
//! The lookup-subtable enumeration for GPOS (lookup types 1–9: Single,
//! Pair, Cursive, MarkToBase, MarkToLig, MarkToMark, Context,
//! ChainContext, Extension) is left as raw sub-slices via
//! [`super::layout::Lookup::subtable_bytes`]; decoding the
//! Anchor / MarkArray interiors are deferred to a future round; the
//! ValueRecord primitive and Lookup Type 1 (single adjustment
//! positioning) are now decoded as typed views (this round).

use crate::parser::{read_i16, read_u16};
use crate::tables::gdef::{ClassDef, Coverage};
use crate::tables::layout::{FeatureList, LayoutHeader, Lookup, LookupList, Script, ScriptList};
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
/// Only the design-unit placement/advance values are surfaced as typed
/// fields; the four optional Device/VariationIndex offsets are kept as
/// raw `Offset16` values (`0` = NULL) because Device-table decoding is
/// deferred to a later round. Every field that the originating
/// `ValueFormat` does not declare is reported as `0`, matching the
/// spec's "empty ValueRecord ⇒ no positioning change" semantics.
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
}
