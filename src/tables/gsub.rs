//! `GSUB` — Glyph Substitution Table (header + ScriptList / FeatureList
//! / LookupList walk).
//!
//! Spec: Microsoft / ISO/IEC 14496-22 OpenType `GSUB` table
//! (`docs/text/opentype/otspec-gsub.html`), with the
//! `ScriptList` / `FeatureList` / `LookupList` / `Lookup` /
//! `LookupFlag` structures sourced from
//! `docs/text/opentype/otspec-chapter2-common-layout-tables.html`.
//!
//! Two header versions are defined:
//! ```text
//!   GSUB Header, version 1.0           (10 bytes)
//!   0 / 2 / majorVersion (= 1)
//!   2 / 2 / minorVersion (= 0)
//!   4 / 2 / scriptListOffset    (Offset16, from start of GSUB)
//!   6 / 2 / featureListOffset   (Offset16, from start of GSUB)
//!   8 / 2 / lookupListOffset    (Offset16, from start of GSUB)
//!
//!   GSUB Header, version 1.1           (14 bytes; adds:)
//!  10 / 4 / featureVariationsOffset    (Offset32; may be NULL)
//! ```
//!
//! This module surfaces the header, the three principal sub-tables,
//! and a few shaping-relevant convenience methods. The lookup
//! subtable formats themselves (GsubLookupType 1–8) are left as raw
//! sub-slices via [`super::layout::Lookup::subtable_bytes`]; decoding
//! their interiors is deferred to a future round.

use crate::tables::layout::{FeatureList, LayoutHeader, Lookup, LookupList, Script, ScriptList};
use crate::Error;

/// Parsed `GSUB` header view.
#[derive(Debug, Clone, Copy)]
pub struct GsubTable<'a> {
    bytes: &'a [u8],
    header: LayoutHeader,
}

impl<'a> GsubTable<'a> {
    /// Parse a GSUB table from the raw `bytes` of the table.
    pub fn parse(bytes: &'a [u8]) -> Result<Self, Error> {
        let header = LayoutHeader::parse(bytes)?;
        let len = bytes.len();
        if (header.script_list_off as usize) >= len
            || (header.feature_list_off as usize) >= len
            || (header.lookup_list_off as usize) >= len
        {
            return Err(Error::BadStructure("GSUB: header offset out of range"));
        }
        if header.feature_variations_off != 0 && (header.feature_variations_off as usize) >= len {
            return Err(Error::BadStructure(
                "GSUB: featureVariationsOffset out of range",
            ));
        }
        Ok(Self { bytes, header })
    }

    /// `(majorVersion, minorVersion)` pair (`(1, 0)` or `(1, 1)`).
    pub fn version(&self) -> (u16, u16) {
        (self.header.major, self.header.minor)
    }

    /// Raw `featureVariationsOffset` (`0` = NULL or absent). The
    /// FeatureVariations table itself is not yet decoded; callers
    /// wanting its bytes can index `self.raw()` at this offset.
    pub fn feature_variations_offset(&self) -> u32 {
        self.header.feature_variations_off
    }

    /// `true` iff the v1.1 `featureVariationsOffset` field is present
    /// and non-zero.
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
    /// `b"latn"`). Returns `None` when the tag is absent or the
    /// ScriptList itself fails to parse.
    pub fn find_script(&self, tag: &[u8; 4]) -> Option<Script<'a>> {
        self.script_list().ok()?.find(tag)?.ok()
    }

    /// Convenience: total `lookupCount` (matches
    /// `LookupList::count()`).
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

    /// Borrow lookup `i` by index. Returns `None` for an out-of-range
    /// index or a malformed list.
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

    /// Build a minimal valid v1.0 GSUB byte tower with one DFLT
    /// script + one `liga` feature + one Lookup (type 4 = ligature
    /// substitution; we surface only the header). Tests the offsets +
    /// the three accessors.
    #[test]
    fn parses_minimal_v10_table() {
        // -------- layout planning --------
        // 0   /  10 / header (script=10, feature=22, lookup=44)
        // 10  /  12 / ScriptList: count=1, [DFLT, scriptOffset=8 → 18]
        // 18  /   4 / Script: defaultLangSys=0, langSysCount=0
        // 22  /  10 / FeatureList: count=1, [liga, featureOffset=8 → 30]
        // 30  /   6 / Feature: paramsOffset=0, lookupCount=1, lookupIdx=[0]
        // 44  /   4 / LookupList: count=1, [lookupOffset=4 → 48]
        // 48  /   6 / Lookup: type=4, flag=0, subTableCount=0
        let mut bytes = vec![0u8; 54];
        // header
        bytes[0..2].copy_from_slice(&be(1));
        bytes[2..4].copy_from_slice(&be(0));
        bytes[4..6].copy_from_slice(&be(10));
        bytes[6..8].copy_from_slice(&be(22));
        bytes[8..10].copy_from_slice(&be(44));
        // ScriptList
        bytes[10..12].copy_from_slice(&be(1));
        bytes[12..16].copy_from_slice(b"DFLT");
        bytes[16..18].copy_from_slice(&be(8));
        // Script
        bytes[18..20].copy_from_slice(&be(0));
        bytes[20..22].copy_from_slice(&be(0));
        // FeatureList
        bytes[22..24].copy_from_slice(&be(1));
        bytes[24..28].copy_from_slice(b"liga");
        bytes[28..30].copy_from_slice(&be(8));
        // Feature
        bytes[30..32].copy_from_slice(&be(0));
        bytes[32..34].copy_from_slice(&be(1));
        bytes[34..36].copy_from_slice(&be(0));
        // LookupList
        bytes[44..46].copy_from_slice(&be(1));
        bytes[46..48].copy_from_slice(&be(4));
        // Lookup
        bytes[48..50].copy_from_slice(&be(4));
        bytes[50..52].copy_from_slice(&be(0));
        bytes[52..54].copy_from_slice(&be(0));

        let g = GsubTable::parse(&bytes).unwrap();
        assert_eq!(g.version(), (1, 0));
        assert!(!g.has_feature_variations());
        assert_eq!(g.feature_variations_offset(), 0);
        assert_eq!(g.script_count(), 1);
        assert_eq!(g.feature_count(), 1);
        assert_eq!(g.lookup_count(), 1);

        let scripts = g.script_list().unwrap();
        assert_eq!(scripts.count(), 1);
        assert_eq!(scripts.tag(0), Some(*b"DFLT"));
        let dflt = g.find_script(b"DFLT").expect("DFLT script");
        assert!(!dflt.has_default_lang_sys());
        assert_eq!(dflt.lang_sys_count(), 0);

        let feats = g.feature_list().unwrap();
        assert_eq!(feats.tag(0), Some(*b"liga"));
        let liga = feats.feature(0).unwrap().unwrap();
        assert_eq!(liga.lookup_count(), 1);
        assert_eq!(liga.lookup_index(0), Some(0));

        let l0 = g.lookup(0).unwrap();
        assert_eq!(l0.lookup_type(), 4);
        assert!(!l0.flag().ignore_marks());
    }

    #[test]
    fn rejects_unknown_minor_version() {
        let mut bytes = vec![0u8; 14];
        bytes[0..2].copy_from_slice(&be(1));
        bytes[2..4].copy_from_slice(&be(2));
        assert!(matches!(
            GsubTable::parse(&bytes),
            Err(Error::BadStructure(_))
        ));
    }

    #[test]
    fn rejects_offset_past_table() {
        let mut bytes = vec![0u8; 10];
        bytes[0..2].copy_from_slice(&be(1));
        bytes[2..4].copy_from_slice(&be(0));
        bytes[4..6].copy_from_slice(&be(99));
        assert!(matches!(
            GsubTable::parse(&bytes),
            Err(Error::BadStructure(_))
        ));
    }
}
