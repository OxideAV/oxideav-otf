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
//! ValueRecord / Anchor / MarkArray interiors is deferred to a future
//! round.

use crate::tables::layout::{FeatureList, LayoutHeader, Lookup, LookupList, Script, ScriptList};
use crate::Error;

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
}
