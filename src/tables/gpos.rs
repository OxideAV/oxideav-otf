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
use crate::tables::gdef::Coverage;
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
}
