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
//! and shaping-relevant convenience methods. Lookup-type subtable
//! decoders are added incrementally; what currently lands as a typed
//! view:
//!
//! * **GsubLookupType 1 — Single substitution** (one glyph → one
//!   glyph) via [`SingleSubst`]. Both on-disk formats are decoded
//!   ([`docs/text/opentype/otspec-gsub.html` §"Lookup type 1 subtable:
//!   single substitution"]):
//!   * Format 1 — `(format, coverageOffset, deltaGlyphID)`. Output
//!     glyph ID = `(input + deltaGlyphID) mod 65536`.
//!   * Format 2 — `(format, coverageOffset, glyphCount,
//!     substituteGlyphIDs[glyphCount])`. Output glyph ID =
//!     `substituteGlyphIDs[coverage_index_of(input)]`.
//!
//! Other GSUB subtable types (2 Multiple, 3 Alternate, 4 Ligature,
//! 5 Contextual, 6 Chained-context, 7 Extension, 8 Reverse-chained-
//! single) remain raw sub-slices via
//! [`super::layout::Lookup::subtable_bytes`]; decoding their
//! interiors is deferred to a future round.

use crate::parser::{read_i16, read_u16};
use crate::tables::gdef::Coverage;
use crate::tables::layout::{FeatureList, LayoutHeader, Lookup, LookupList, Script, ScriptList};
use crate::Error;

// ---------------------------------------------------------------------------
// GsubLookupType enumeration (otspec-gsub.html §"GsubLookupType enumeration")
// ---------------------------------------------------------------------------

/// `GsubLookupType` value for **single substitution** — one glyph
/// replaced by one glyph (§"Lookup type 1 subtable: single
/// substitution").
pub const GSUB_LOOKUP_TYPE_SINGLE: u16 = 1;
/// `GsubLookupType` value for **multiple substitution** — one glyph
/// replaced by a sequence of glyphs (§"Lookup type 2 subtable:
/// multiple substitution").
pub const GSUB_LOOKUP_TYPE_MULTIPLE: u16 = 2;
/// `GsubLookupType` value for **alternate substitution** — one glyph
/// replaced by one of a list of alternates.
pub const GSUB_LOOKUP_TYPE_ALTERNATE: u16 = 3;
/// `GsubLookupType` value for **ligature substitution** — a sequence
/// of glyphs replaced by a single ligature glyph.
pub const GSUB_LOOKUP_TYPE_LIGATURE: u16 = 4;
/// `GsubLookupType` value for **contextual substitution**.
pub const GSUB_LOOKUP_TYPE_CONTEXT: u16 = 5;
/// `GsubLookupType` value for **chained contexts substitution**.
pub const GSUB_LOOKUP_TYPE_CHAINED_CONTEXT: u16 = 6;
/// `GsubLookupType` value for **substitution extension** — 32-bit
/// offset to one of the other lookup-type formats.
pub const GSUB_LOOKUP_TYPE_EXTENSION: u16 = 7;
/// `GsubLookupType` value for **reverse chaining contextual single
/// substitution**.
pub const GSUB_LOOKUP_TYPE_REVERSE_CHAINED_SINGLE: u16 = 8;

// ---------------------------------------------------------------------------
// Lookup type 1: Single substitution (otspec-gsub.html §"Lookup type 1
// subtable: single substitution")
// ---------------------------------------------------------------------------

/// Parsed `SingleSubst` subtable — the GSUB `lookupType = 1` payload.
///
/// Spec: `docs/text/opentype/otspec-gsub.html` §"Lookup type 1
/// subtable: single substitution".
///
/// Two on-disk formats are defined; both carry a Coverage table that
/// names the *input* glyphs:
///
/// * **Format 1** (`SingleSubstFormat1`, 6 bytes): a single
///   `int16 deltaGlyphID` is added (mod 65536) to every covered input
///   glyph to produce the output. The Coverage Index is unused.
///   ```text
///   0 / 2 / format (= 1)
///   2 / 2 / coverageOffset  (Offset16, from start of subtable)
///   4 / 2 / deltaGlyphID    (int16; sum is mod 65536)
///   ```
/// * **Format 2** (`SingleSubstFormat2`): an explicit per-input
///   substitute array indexed by Coverage Index.
///   ```text
///   0 / 2 / format (= 2)
///   2 / 2 / coverageOffset  (Offset16)
///   4 / 2 / glyphCount      (== Coverage.len())
///   6 / 2 / substituteGlyphIDs[glyphCount]
///   ```
///
/// [`Self::substitute`] returns the output glyph (if any) for an input
/// glyph. The parser keeps zero copies — the borrowed byte window
/// covers the whole subtable.
#[derive(Debug, Clone, Copy)]
pub struct SingleSubst<'a> {
    inner: SingleSubstInner<'a>,
}

#[derive(Debug, Clone, Copy)]
enum SingleSubstInner<'a> {
    /// Format 1: a single `deltaGlyphID` applied to every covered glyph.
    Format1 { coverage: Coverage<'a>, delta: i16 },
    /// Format 2: a `glyphCount`-element substitute array indexed by
    /// Coverage Index.
    Format2 {
        coverage: Coverage<'a>,
        /// Raw `substituteGlyphIDs[]` payload (`2 * glyphCount` bytes,
        /// big-endian `u16` each).
        substitutes: &'a [u8],
    },
}

impl<'a> SingleSubst<'a> {
    /// Parse a SingleSubst subtable from a buffer whose first two
    /// bytes are the `format` identifier.
    ///
    /// Validates the format discriminant, the `coverageOffset` window,
    /// and (for format 2) that `glyphCount` matches the Coverage
    /// length and that the trailing array fits inside the supplied
    /// slice.
    pub fn parse(bytes: &'a [u8]) -> Result<Self, Error> {
        let format = read_u16(bytes, 0)?;
        let cov_off = read_u16(bytes, 2)? as usize;
        if cov_off == 0 || cov_off >= bytes.len() {
            return Err(Error::BadStructure(
                "GSUB/SingleSubst: coverageOffset out of range",
            ));
        }
        let coverage = Coverage::parse(&bytes[cov_off..])?;
        match format {
            1 => {
                // 6-byte header: format + coverageOffset + deltaGlyphID.
                let delta = read_i16(bytes, 4)?;
                Ok(Self {
                    inner: SingleSubstInner::Format1 { coverage, delta },
                })
            }
            2 => {
                let glyph_count = read_u16(bytes, 4)? as usize;
                // glyphCount must equal Coverage.len() per the spec:
                // "The substituteGlyphIDs array must contain the same
                // number of glyph indices as the Coverage table".
                if glyph_count != coverage.len() {
                    return Err(Error::BadStructure(
                        "GSUB/SingleSubstFormat2: glyphCount != coverage.len()",
                    ));
                }
                let array_start = 6usize;
                let need = array_start
                    .checked_add(glyph_count.checked_mul(2).ok_or(Error::BadStructure(
                        "GSUB/SingleSubstFormat2 length overflow",
                    ))?)
                    .ok_or(Error::BadStructure(
                        "GSUB/SingleSubstFormat2 length overflow",
                    ))?;
                if bytes.len() < need {
                    return Err(Error::UnexpectedEof);
                }
                Ok(Self {
                    inner: SingleSubstInner::Format2 {
                        coverage,
                        substitutes: &bytes[array_start..need],
                    },
                })
            }
            _ => Err(Error::BadStructure(
                "GSUB/SingleSubst: unknown subtable format",
            )),
        }
    }

    /// Subtable format discriminant (`1` or `2`).
    pub fn format(&self) -> u16 {
        match self.inner {
            SingleSubstInner::Format1 { .. } => 1,
            SingleSubstInner::Format2 { .. } => 2,
        }
    }

    /// The input-side [`Coverage`] table. Glyphs not in this set are
    /// not substituted by this subtable.
    pub fn coverage(&self) -> Coverage<'a> {
        match self.inner {
            SingleSubstInner::Format1 { coverage, .. } => coverage,
            SingleSubstInner::Format2 { coverage, .. } => coverage,
        }
    }

    /// `deltaGlyphID` for a Format 1 subtable, or `None` on Format 2.
    pub fn delta_glyph_id(&self) -> Option<i16> {
        match self.inner {
            SingleSubstInner::Format1 { delta, .. } => Some(delta),
            SingleSubstInner::Format2 { .. } => None,
        }
    }

    /// `glyphCount` (== Coverage.len()) for a Format 2 subtable, or
    /// `None` on Format 1.
    pub fn glyph_count(&self) -> Option<u16> {
        match self.inner {
            SingleSubstInner::Format1 { .. } => None,
            SingleSubstInner::Format2 { substitutes, .. } => Some((substitutes.len() / 2) as u16),
        }
    }

    /// Look up the substitute for `input` — i.e. apply this subtable
    /// as a shaper would.
    ///
    /// Returns `None` when `input` is not covered. Format 1 returns
    /// `(input as i32 + delta as i32) mod 65536` cast back to `u16`
    /// (the spec's "Addition of deltaGlyphID is modulo 65536", "If the
    /// result … is less than zero, add 65536"). Format 2 returns
    /// `substituteGlyphIDs[coverage_index]`.
    pub fn substitute(&self, input: u16) -> Option<u16> {
        match self.inner {
            SingleSubstInner::Format1 { coverage, delta } => {
                coverage.index_of(input)?;
                // Spec: addition is modulo 65536. Sign-extending the
                // i16 delta to i32 and reducing mod 2**16 implements
                // the "if result < 0, add 65536" wrap-around for free.
                let sum = (input as i32 + delta as i32).rem_euclid(65536);
                Some(sum as u16)
            }
            SingleSubstInner::Format2 {
                coverage,
                substitutes,
            } => {
                let idx = coverage.index_of(input)? as usize;
                let off = idx.checked_mul(2).filter(|&o| o + 2 <= substitutes.len())?;
                Some(u16::from_be_bytes([substitutes[off], substitutes[off + 1]]))
            }
        }
    }

    /// Iterate over every `(input_glyph, output_glyph)` pair this
    /// subtable rewrites, in ascending input-glyph order.
    pub fn iter(&self) -> SingleSubstIter<'a> {
        SingleSubstIter {
            cov: self.coverage().iter(),
            sub: self.inner,
        }
    }
}

/// Iterator yielded by [`SingleSubst::iter`].
#[derive(Debug, Clone)]
pub struct SingleSubstIter<'a> {
    cov: crate::tables::gdef::CoverageIter<'a>,
    sub: SingleSubstInner<'a>,
}

impl<'a> Iterator for SingleSubstIter<'a> {
    type Item = (u16, u16);
    fn next(&mut self) -> Option<Self::Item> {
        let (g, idx) = self.cov.next()?;
        let out = match self.sub {
            SingleSubstInner::Format1 { delta, .. } => {
                ((g as i32 + delta as i32).rem_euclid(65536)) as u16
            }
            SingleSubstInner::Format2 { substitutes, .. } => {
                let off = (idx as usize).checked_mul(2)?;
                if off + 2 > substitutes.len() {
                    return None;
                }
                u16::from_be_bytes([substitutes[off], substitutes[off + 1]])
            }
        };
        Some((g, out))
    }
}

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

    /// Decode subtable `sub_i` of lookup `lookup_i` as a
    /// [`SingleSubst`] (`GsubLookupType = 1`).
    ///
    /// Returns:
    /// * `None` — `lookup_i` or `sub_i` is out of range, or the
    ///   referenced subtable bytes are unreachable.
    /// * `Some(Err(Error::BadStructure))` — the lookup is not
    ///   declared as `GSUB_LOOKUP_TYPE_SINGLE`, or the subtable bytes
    ///   are malformed.
    /// * `Some(Ok(SingleSubst))` — the typed subtable view.
    pub fn single_subst(
        &self,
        lookup_i: u16,
        sub_i: u16,
    ) -> Option<Result<SingleSubst<'a>, Error>> {
        let lk = self.lookup(lookup_i)?;
        if lk.lookup_type() != GSUB_LOOKUP_TYPE_SINGLE {
            return Some(Err(Error::BadStructure(
                "GSUB/SingleSubst: lookup is not type 1",
            )));
        }
        let bytes = lk.subtable_bytes(sub_i)?;
        Some(SingleSubst::parse(bytes))
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

    // -------------- SingleSubst Format 1 -------------------------------

    /// Build a SingleSubstFormat1 subtable that maps glyphs
    /// `{20, 21, 22}` to `{120, 121, 122}` via `deltaGlyphID = 100`.
    fn build_single_subst_fmt1(delta: i16, glyphs: &[u16]) -> Vec<u8> {
        // Layout:
        //   0  / 2 / format = 1
        //   2  / 2 / coverageOffset = 6
        //   4  / 2 / deltaGlyphID
        //   6  / 2 / coverage format = 1
        //   8  / 2 / glyphCount
        //  10  / 2*N / glyphArray
        let mut out = Vec::new();
        out.extend_from_slice(&be(1)); // format
        out.extend_from_slice(&be(6)); // coverageOffset
        out.extend_from_slice(&(delta as u16).to_be_bytes()); // deltaGlyphID

        // Coverage Format 1
        out.extend_from_slice(&be(1)); // coverage format
        out.extend_from_slice(&be(glyphs.len() as u16));
        for &g in glyphs {
            out.extend_from_slice(&be(g));
        }
        out
    }

    #[test]
    fn single_subst_fmt1_round_trip() {
        let raw = build_single_subst_fmt1(100, &[20, 21, 22]);
        let ss = SingleSubst::parse(&raw).unwrap();
        assert_eq!(ss.format(), 1);
        assert_eq!(ss.delta_glyph_id(), Some(100));
        assert_eq!(ss.glyph_count(), None);
        assert_eq!(ss.substitute(20), Some(120));
        assert_eq!(ss.substitute(21), Some(121));
        assert_eq!(ss.substitute(22), Some(122));
        // Uncovered glyph: no substitution.
        assert_eq!(ss.substitute(23), None);
        assert_eq!(ss.substitute(0), None);
        // Iteration produces the (input, output) pairs in sorted order.
        let pairs: Vec<_> = ss.iter().collect();
        assert_eq!(pairs, vec![(20, 120), (21, 121), (22, 122)]);
    }

    #[test]
    fn single_subst_fmt1_negative_delta_wraps_mod_65536() {
        // Spec: "If the result after adding deltaGlyphID to the input
        // glyph index is less than zero, add 65536 to obtain a valid
        // glyph ID." Verified with input = 5, delta = -10 → 65531.
        let raw = build_single_subst_fmt1(-10, &[5]);
        let ss = SingleSubst::parse(&raw).unwrap();
        assert_eq!(ss.substitute(5), Some(65531));
    }

    #[test]
    fn single_subst_fmt1_positive_delta_wraps_mod_65536() {
        // Spec: "Addition of deltaGlyphID is modulo 65536." Verified
        // with input = 65530, delta = 10 → 4.
        let raw = build_single_subst_fmt1(10, &[65530]);
        let ss = SingleSubst::parse(&raw).unwrap();
        assert_eq!(ss.substitute(65530), Some(4));
    }

    // -------------- SingleSubst Format 2 -------------------------------

    /// Build a SingleSubstFormat2 subtable.
    fn build_single_subst_fmt2(inputs: &[u16], outputs: &[u16]) -> Vec<u8> {
        assert_eq!(inputs.len(), outputs.len());
        // Layout (Coverage Format 1):
        //   0   / 2 / format = 2
        //   2   / 2 / coverageOffset
        //   4   / 2 / glyphCount
        //   6   / 2*N / substituteGlyphIDs
        //   cov / 2 / coverage format = 1
        //   cov+2 / 2 / glyphCount
        //   cov+4 / 2*N / glyphArray
        let n = inputs.len();
        let cov_off = 6 + 2 * n;
        let mut out = Vec::new();
        out.extend_from_slice(&be(2));
        out.extend_from_slice(&be(cov_off as u16));
        out.extend_from_slice(&be(n as u16));
        for &g in outputs {
            out.extend_from_slice(&be(g));
        }
        // Coverage Format 1
        out.extend_from_slice(&be(1));
        out.extend_from_slice(&be(n as u16));
        for &g in inputs {
            out.extend_from_slice(&be(g));
        }
        out
    }

    #[test]
    fn single_subst_fmt2_round_trip() {
        let raw = build_single_subst_fmt2(&[10, 30, 50], &[1010, 3030, 5050]);
        let ss = SingleSubst::parse(&raw).unwrap();
        assert_eq!(ss.format(), 2);
        assert_eq!(ss.delta_glyph_id(), None);
        assert_eq!(ss.glyph_count(), Some(3));
        assert_eq!(ss.substitute(10), Some(1010));
        assert_eq!(ss.substitute(30), Some(3030));
        assert_eq!(ss.substitute(50), Some(5050));
        // Uncovered glyph IDs (including a value between covered
        // entries) return None.
        assert_eq!(ss.substitute(0), None);
        assert_eq!(ss.substitute(20), None);
        assert_eq!(ss.substitute(60), None);

        let pairs: Vec<_> = ss.iter().collect();
        assert_eq!(pairs, vec![(10, 1010), (30, 3030), (50, 5050)]);
    }

    #[test]
    fn single_subst_fmt2_rejects_glyph_count_mismatch() {
        // Build a valid Format 2 then poke glyphCount to a value that
        // disagrees with the Coverage length.
        let mut raw = build_single_subst_fmt2(&[10, 30], &[100, 300]);
        // glyphCount lives at offset 4..6.
        raw[4..6].copy_from_slice(&be(7));
        assert!(matches!(
            SingleSubst::parse(&raw),
            Err(Error::BadStructure(_))
        ));
    }

    #[test]
    fn single_subst_rejects_unknown_format() {
        // format = 3, plus enough trailing bytes that the
        // coverageOffset window check would succeed.
        let mut raw = vec![0u8; 16];
        raw[0..2].copy_from_slice(&be(3));
        raw[2..4].copy_from_slice(&be(8));
        // Plausible coverage payload at offset 8.
        raw[8..10].copy_from_slice(&be(1));
        raw[10..12].copy_from_slice(&be(1));
        raw[12..14].copy_from_slice(&be(5));
        assert!(matches!(
            SingleSubst::parse(&raw),
            Err(Error::BadStructure(_))
        ));
    }

    #[test]
    fn single_subst_rejects_truncated_array() {
        // Build a Format 2 subtable then chop off the trailing
        // substituteGlyphIDs[] bytes.
        let raw = build_single_subst_fmt2(&[1, 2, 3], &[11, 22, 33]);
        // The trailing array starts at offset 6 and is 2*3 = 6 bytes;
        // dropping the last 2 bytes makes glyphCount-many entries
        // unreadable.
        let truncated = &raw[..raw.len() - 2 /* steal from Coverage tail */];
        // It's the Coverage that gets truncated by this cut, which
        // surfaces as UnexpectedEof when the Coverage parser walks the
        // shortened range.
        assert!(matches!(
            SingleSubst::parse(truncated),
            Err(Error::UnexpectedEof) | Err(Error::BadStructure(_))
        ));
    }

    // -------------- GsubTable::single_subst integration ----------------

    /// Build a tiny GSUB table whose only lookup is a type-1
    /// SingleSubstFormat1 subtable, then drive the whole walk from the
    /// `GsubTable::single_subst` convenience accessor.
    #[test]
    fn gsub_single_subst_end_to_end() {
        // Mostly mirrors `parses_minimal_v10_table` but inflates the
        // Lookup with one real subtable.
        //
        // Layout plan (all offsets relative to start of GSUB):
        //   0   /  10 / header
        //   10  /  12 / ScriptList (1 record, DFLT @18)
        //   18  /   4 / Script (no LangSys)
        //   22  /  10 / FeatureList (1 record, "calt" @ 30 [unused])
        //   30  /   6 / Feature
        //   36  /   4 / LookupList (1 entry → 40)
        //   40  /   8 / Lookup type=1, flag=0, subTableCount=1, subOff=8
        //   48  /  14 / SingleSubstFormat1 subtable:
        //                  format=1, coverageOffset=6, deltaGlyphID=200,
        //                  coverage Format 1: glyphCount=2, glyphs=[50, 51]
        let mut bytes = vec![0u8; 62];
        // header
        bytes[0..2].copy_from_slice(&be(1));
        bytes[2..4].copy_from_slice(&be(0));
        bytes[4..6].copy_from_slice(&be(10));
        bytes[6..8].copy_from_slice(&be(22));
        bytes[8..10].copy_from_slice(&be(36));
        // ScriptList
        bytes[10..12].copy_from_slice(&be(1));
        bytes[12..16].copy_from_slice(b"DFLT");
        bytes[16..18].copy_from_slice(&be(8));
        // Script
        bytes[18..20].copy_from_slice(&be(0));
        bytes[20..22].copy_from_slice(&be(0));
        // FeatureList
        bytes[22..24].copy_from_slice(&be(1));
        bytes[24..28].copy_from_slice(b"calt");
        bytes[28..30].copy_from_slice(&be(8));
        // Feature
        bytes[30..32].copy_from_slice(&be(0));
        bytes[32..34].copy_from_slice(&be(1));
        bytes[34..36].copy_from_slice(&be(0));
        // LookupList
        bytes[36..38].copy_from_slice(&be(1));
        bytes[38..40].copy_from_slice(&be(4));
        // Lookup: type=1, flag=0, subTableCount=1, subtableOffsets=[8]
        bytes[40..42].copy_from_slice(&be(1));
        bytes[42..44].copy_from_slice(&be(0));
        bytes[44..46].copy_from_slice(&be(1));
        bytes[46..48].copy_from_slice(&be(8));
        // SingleSubstFormat1 subtable @ 48
        bytes[48..50].copy_from_slice(&be(1)); // format
        bytes[50..52].copy_from_slice(&be(6)); // coverageOffset
        bytes[52..54].copy_from_slice(&be(200)); // deltaGlyphID
        bytes[54..56].copy_from_slice(&be(1)); // coverage format
        bytes[56..58].copy_from_slice(&be(2)); // glyphCount
        bytes[58..60].copy_from_slice(&be(50));
        bytes[60..62].copy_from_slice(&be(51));

        let g = GsubTable::parse(&bytes).unwrap();
        let ss = g.single_subst(0, 0).expect("subtable exists").unwrap();
        assert_eq!(ss.format(), 1);
        assert_eq!(ss.substitute(50), Some(250));
        assert_eq!(ss.substitute(51), Some(251));
        assert_eq!(ss.substitute(52), None);

        // Wrong subtable index -> None.
        assert!(g.single_subst(0, 1).is_none());
        // Wrong lookup index -> None.
        assert!(g.single_subst(99, 0).is_none());
    }

    #[test]
    fn gsub_single_subst_rejects_non_type_1_lookup() {
        // Reuse the minimal_v10 layout but declare the Lookup as
        // type = 4 (ligature), then assert the typed accessor rejects.
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
        bytes[24..28].copy_from_slice(b"liga");
        bytes[28..30].copy_from_slice(&be(8));
        bytes[30..32].copy_from_slice(&be(0));
        bytes[32..34].copy_from_slice(&be(1));
        bytes[34..36].copy_from_slice(&be(0));
        bytes[44..46].copy_from_slice(&be(1));
        bytes[46..48].copy_from_slice(&be(4));
        // Lookup: declare type = 4 (ligature), not type 1.
        bytes[48..50].copy_from_slice(&be(4));
        bytes[50..52].copy_from_slice(&be(0));
        bytes[52..54].copy_from_slice(&be(0));

        let g = GsubTable::parse(&bytes).unwrap();
        // The lookup exists but is the wrong type; we surface
        // BadStructure rather than None so callers can distinguish a
        // missing lookup from a type mismatch.
        assert!(matches!(g.single_subst(0, 0), Some(Err(_))));
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
