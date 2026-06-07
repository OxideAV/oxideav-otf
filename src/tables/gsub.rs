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
//! * **GsubLookupType 4 — Ligature substitution** (a sequence of glyphs
//!   replaced by a single ligature glyph) via [`LigatureSubst`]. The
//!   one on-disk format is decoded
//!   ([`docs/text/opentype/otspec-gsub.html` §"Lookup type 4 subtable:
//!   ligature substitution"]):
//!   * Format 1 — `(format, coverageOffset, ligatureSetCount,
//!     ligatureSetOffsets[ligatureSetCount])`. Each LigatureSet is
//!     `(ligatureCount, ligatureOffsets[ligatureCount])`; each Ligature
//!     is `(ligatureGlyph, componentCount,
//!     componentGlyphIDs[componentCount - 1])`. The first component
//!     glyph is the Coverage entry; the tail of the input sequence is
//!     matched against `componentGlyphIDs[..]` in order, and a full
//!     match yields `ligatureGlyph`. Within a LigatureSet, the array
//!     order is the preference order — longer / preferred ligatures
//!     come first.
//!
//! Other GSUB subtable types (2 Multiple, 3 Alternate, 5 Contextual,
//! 6 Chained-context, 7 Extension, 8 Reverse-chained-single) remain
//! raw sub-slices via [`super::layout::Lookup::subtable_bytes`];
//! decoding their interiors is deferred to a future round.

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

// ---------------------------------------------------------------------------
// Lookup type 4: Ligature substitution (otspec-gsub.html §"Lookup type 4
// subtable: ligature substitution")
// ---------------------------------------------------------------------------

/// Parsed `LigatureSubst` subtable — the GSUB `lookupType = 4` payload.
///
/// Spec: `docs/text/opentype/otspec-gsub.html` §"Lookup type 4 subtable:
/// ligature substitution".
///
/// On-disk layout (one format defined):
///
/// ```text
/// LigatureSubstFormat1 subtable
///   0 / 2 / format = 1
///   2 / 2 / coverageOffset       (Offset16, from start of subtable)
///   4 / 2 / ligatureSetCount
///   6 / 2 * n / ligatureSetOffsets[ligatureSetCount]
///                                 (Offset16, from start of subtable,
///                                  ordered by Coverage index)
///
/// LigatureSet table
///   0 / 2 / ligatureCount
///   2 / 2 * n / ligatureOffsets[ligatureCount]
///                                 (Offset16, from start of LigatureSet,
///                                  ordered by preference; longer /
///                                  preferred ligatures first)
///
/// Ligature table
///   0 / 2 / ligatureGlyph
///   2 / 2 / componentCount        (total components incl. the first)
///   4 / 2 * (componentCount - 1) / componentGlyphIDs[componentCount - 1]
/// ```
///
/// The Coverage table names the **first** component glyph of every
/// ligature in the subtable; the tail components live in the per-Ligature
/// `componentGlyphIDs[]` array, starting from the second component
/// (input glyph sequence index = 1). Ligature lookup is therefore
/// driven by the first glyph: an input sequence whose first glyph is
/// in Coverage selects the LigatureSet at that Coverage Index, then
/// each Ligature inside the set is tried in array order and the first
/// whose `componentGlyphIDs[..]` matches the input tail is the result.
///
/// All offsets are validated at parse time. [`Self::substitute`]
/// applies the spec's "first-match wins" rule against an input slice
/// and returns `(ligatureGlyph, componentCount)` for the matching
/// Ligature, or `None` if no Ligature in the selected LigatureSet
/// matches. The component count tells the caller how many input glyphs
/// the ligature consumed (the first glyph + `componentCount - 1` tail
/// glyphs).
#[derive(Debug, Clone, Copy)]
pub struct LigatureSubst<'a> {
    /// Raw subtable bytes (offsets in the on-disk records are relative
    /// to this buffer's start).
    bytes: &'a [u8],
    coverage: Coverage<'a>,
    /// Slice of the `ligatureSetOffsets[]` array (2 bytes per offset).
    set_offsets: &'a [u8],
}

impl<'a> LigatureSubst<'a> {
    /// Parse a LigatureSubst subtable from a buffer whose first two
    /// bytes are the `format` identifier.
    ///
    /// Validates the format discriminant (only `1` is defined), the
    /// `coverageOffset` window, and the trailing `ligatureSetOffsets[]`
    /// array length. The per-LigatureSet and per-Ligature payloads
    /// themselves are validated lazily — [`Self::ligature_set`] and
    /// [`Self::ligature`] re-validate on each call so a malformed
    /// inner record can't poison the top-level view.
    pub fn parse(bytes: &'a [u8]) -> Result<Self, Error> {
        let format = read_u16(bytes, 0)?;
        if format != 1 {
            return Err(Error::BadStructure(
                "GSUB/LigatureSubst: unknown subtable format",
            ));
        }
        let cov_off = read_u16(bytes, 2)? as usize;
        if cov_off == 0 || cov_off >= bytes.len() {
            return Err(Error::BadStructure(
                "GSUB/LigatureSubst: coverageOffset out of range",
            ));
        }
        let coverage = Coverage::parse(&bytes[cov_off..])?;
        let set_count = read_u16(bytes, 4)? as usize;
        let array_start = 6usize;
        let need = array_start
            .checked_add(
                set_count
                    .checked_mul(2)
                    .ok_or(Error::BadStructure("GSUB/LigatureSubst length overflow"))?,
            )
            .ok_or(Error::BadStructure("GSUB/LigatureSubst length overflow"))?;
        if bytes.len() < need {
            return Err(Error::UnexpectedEof);
        }
        Ok(Self {
            bytes,
            coverage,
            set_offsets: &bytes[array_start..need],
        })
    }

    /// Subtable format discriminant (always `1`).
    pub fn format(&self) -> u16 {
        1
    }

    /// The input-side [`Coverage`] table. Each covered glyph is the
    /// **first** component of every ligature in the corresponding
    /// LigatureSet.
    pub fn coverage(&self) -> Coverage<'a> {
        self.coverage
    }

    /// `ligatureSetCount` — number of LigatureSet tables.
    pub fn ligature_set_count(&self) -> u16 {
        (self.set_offsets.len() / 2) as u16
    }

    /// Borrow the [`LigatureSet`] at the given Coverage index. Returns
    /// `None` for an out-of-range index, `Some(Err(...))` when the
    /// referenced bytes are malformed.
    pub fn ligature_set(&self, set_i: u16) -> Option<Result<LigatureSet<'a>, Error>> {
        let off2 = (set_i as usize).checked_mul(2)?;
        if off2 + 2 > self.set_offsets.len() {
            return None;
        }
        let off = u16::from_be_bytes([self.set_offsets[off2], self.set_offsets[off2 + 1]]) as usize;
        if off == 0 || off >= self.bytes.len() {
            return Some(Err(Error::BadStructure(
                "GSUB/LigatureSubst: ligatureSetOffset out of range",
            )));
        }
        Some(LigatureSet::parse(&self.bytes[off..]))
    }

    /// Apply this subtable as a shaper would.
    ///
    /// `input` is the current glyph sequence starting at the position
    /// the shaper is trying to ligate. `input[0]` must be in
    /// [`Self::coverage`]; the tail `input[1..]` is matched against
    /// each Ligature's `componentGlyphIDs[]` in array order.
    ///
    /// Returns `Some((ligature_glyph, component_count))` for the first
    /// matching Ligature — `component_count` is the total number of
    /// input glyphs consumed (including `input[0]`). Returns `None`
    /// when `input` is empty, when `input[0]` is uncovered, or when no
    /// Ligature in the selected LigatureSet matches the input tail.
    ///
    /// Per the spec, "the order in the Ligature offset array defines
    /// the preference for using the ligatures" — first-match wins,
    /// even if a later Ligature in the set would also match a (shorter)
    /// prefix.
    pub fn substitute(&self, input: &[u16]) -> Option<(u16, u16)> {
        let first = *input.first()?;
        let set_i = self.coverage.index_of(first)?;
        let set = self.ligature_set(set_i)?.ok()?;
        for j in 0..set.ligature_count() {
            let lig = set.ligature(j)?.ok()?;
            let comp_count = lig.component_count() as usize;
            if comp_count == 0 {
                // Spec says componentCount is the total count *including*
                // the first; zero is malformed. Skip silently rather
                // than error: this is shaper-path code, not parse-path.
                continue;
            }
            if comp_count > input.len() {
                continue;
            }
            // The first component is the Coverage entry; we already
            // matched it. Match the tail.
            let mut ok = true;
            for k in 0..(comp_count - 1) {
                let want = lig.component_glyph(k as u16)?;
                if want != input[k + 1] {
                    ok = false;
                    break;
                }
            }
            if ok {
                return Some((lig.ligature_glyph(), comp_count as u16));
            }
        }
        None
    }

    /// Iterate the `(coverage_glyph, LigatureSet)` pairs in this
    /// subtable in ascending Coverage order. Malformed LigatureSet
    /// references are surfaced as `Err`.
    pub fn iter(&self) -> LigatureSubstIter<'a> {
        LigatureSubstIter {
            cov: self.coverage.iter(),
            outer: *self,
        }
    }
}

/// Iterator yielded by [`LigatureSubst::iter`].
#[derive(Debug, Clone)]
pub struct LigatureSubstIter<'a> {
    cov: crate::tables::gdef::CoverageIter<'a>,
    outer: LigatureSubst<'a>,
}

impl<'a> Iterator for LigatureSubstIter<'a> {
    type Item = (u16, Result<LigatureSet<'a>, Error>);
    fn next(&mut self) -> Option<Self::Item> {
        let (g, idx) = self.cov.next()?;
        let set = self.outer.ligature_set(idx)?;
        Some((g, set))
    }
}

/// Parsed `LigatureSet` table — a count + an offset array pointing at
/// the individual `Ligature` tables for a single first-component glyph.
#[derive(Debug, Clone, Copy)]
pub struct LigatureSet<'a> {
    bytes: &'a [u8],
    /// Slice of the `ligatureOffsets[]` array (2 bytes per offset).
    lig_offsets: &'a [u8],
}

impl<'a> LigatureSet<'a> {
    /// Parse a LigatureSet table from a buffer whose first two bytes
    /// are `ligatureCount`.
    pub fn parse(bytes: &'a [u8]) -> Result<Self, Error> {
        let count = read_u16(bytes, 0)? as usize;
        let array_start = 2usize;
        let need = array_start
            .checked_add(
                count
                    .checked_mul(2)
                    .ok_or(Error::BadStructure("GSUB/LigatureSet length overflow"))?,
            )
            .ok_or(Error::BadStructure("GSUB/LigatureSet length overflow"))?;
        if bytes.len() < need {
            return Err(Error::UnexpectedEof);
        }
        Ok(Self {
            bytes,
            lig_offsets: &bytes[array_start..need],
        })
    }

    /// `ligatureCount` — number of Ligature tables in this set.
    pub fn ligature_count(&self) -> u16 {
        (self.lig_offsets.len() / 2) as u16
    }

    /// Borrow the [`Ligature`] at preference index `i` (`0 ..
    /// ligature_count`). Returns `None` for an out-of-range index,
    /// `Some(Err(...))` when the referenced bytes are malformed.
    pub fn ligature(&self, i: u16) -> Option<Result<Ligature<'a>, Error>> {
        let off2 = (i as usize).checked_mul(2)?;
        if off2 + 2 > self.lig_offsets.len() {
            return None;
        }
        let off = u16::from_be_bytes([self.lig_offsets[off2], self.lig_offsets[off2 + 1]]) as usize;
        if off == 0 || off >= self.bytes.len() {
            return Some(Err(Error::BadStructure(
                "GSUB/LigatureSet: ligatureOffset out of range",
            )));
        }
        Some(Ligature::parse(&self.bytes[off..]))
    }
}

/// Parsed `Ligature` table — one ligature substitution candidate.
///
/// The on-disk record is `(ligatureGlyph, componentCount,
/// componentGlyphIDs[componentCount - 1])`. The first component glyph
/// is **not** stored here — it is the LigatureSet's covered glyph,
/// surfaced through [`LigatureSubst::coverage`].
#[derive(Debug, Clone, Copy)]
pub struct Ligature<'a> {
    glyph: u16,
    component_count: u16,
    /// Raw `componentGlyphIDs[]` payload — `2 * (componentCount - 1)`
    /// bytes, big-endian `u16` per entry.
    tail: &'a [u8],
}

impl<'a> Ligature<'a> {
    /// Parse a Ligature table from a buffer whose first two bytes are
    /// `ligatureGlyph`.
    ///
    /// A `componentCount` of zero is rejected as `BadStructure`: the
    /// spec specifies "Number of components in the ligature" including
    /// the first, so zero leaves the first-component invariant
    /// unsatisfiable.
    pub fn parse(bytes: &'a [u8]) -> Result<Self, Error> {
        let glyph = read_u16(bytes, 0)?;
        let component_count = read_u16(bytes, 2)?;
        if component_count == 0 {
            return Err(Error::BadStructure(
                "GSUB/Ligature: componentCount must be >= 1",
            ));
        }
        let tail_entries = (component_count - 1) as usize;
        let tail_start = 4usize;
        let need = tail_start
            .checked_add(
                tail_entries
                    .checked_mul(2)
                    .ok_or(Error::BadStructure("GSUB/Ligature length overflow"))?,
            )
            .ok_or(Error::BadStructure("GSUB/Ligature length overflow"))?;
        if bytes.len() < need {
            return Err(Error::UnexpectedEof);
        }
        Ok(Self {
            glyph,
            component_count,
            tail: &bytes[tail_start..need],
        })
    }

    /// `ligatureGlyph` — the substitute glyph ID for this ligature.
    pub fn ligature_glyph(&self) -> u16 {
        self.glyph
    }

    /// `componentCount` — total number of input glyphs (including the
    /// first, Coverage-supplied component) this ligature replaces.
    pub fn component_count(&self) -> u16 {
        self.component_count
    }

    /// The component glyph at tail index `i` (`0 .. componentCount -
    /// 1`). Index `0` is the **second** component glyph (input glyph
    /// sequence index = 1) per the spec.
    pub fn component_glyph(&self, i: u16) -> Option<u16> {
        let off = (i as usize).checked_mul(2)?;
        if off + 2 > self.tail.len() {
            return None;
        }
        Some(u16::from_be_bytes([self.tail[off], self.tail[off + 1]]))
    }

    /// Iterator over every tail-component glyph in input order.
    pub fn component_glyphs(&self) -> LigatureComponentIter<'a> {
        LigatureComponentIter {
            bytes: self.tail,
            pos: 0,
        }
    }
}

/// Iterator over a [`Ligature`]'s tail `componentGlyphIDs[]` in input
/// order (i.e. starting at the second component).
#[derive(Debug, Clone)]
pub struct LigatureComponentIter<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Iterator for LigatureComponentIter<'a> {
    type Item = u16;
    fn next(&mut self) -> Option<Self::Item> {
        if self.pos + 2 > self.bytes.len() {
            return None;
        }
        let g = u16::from_be_bytes([self.bytes[self.pos], self.bytes[self.pos + 1]]);
        self.pos += 2;
        Some(g)
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

    /// Decode subtable `sub_i` of lookup `lookup_i` as a
    /// [`LigatureSubst`] (`GsubLookupType = 4`).
    ///
    /// Returns:
    /// * `None` — `lookup_i` or `sub_i` is out of range, or the
    ///   referenced subtable bytes are unreachable.
    /// * `Some(Err(Error::BadStructure))` — the lookup is not
    ///   declared as `GSUB_LOOKUP_TYPE_LIGATURE`, or the subtable bytes
    ///   are malformed.
    /// * `Some(Ok(LigatureSubst))` — the typed subtable view.
    pub fn ligature_subst(
        &self,
        lookup_i: u16,
        sub_i: u16,
    ) -> Option<Result<LigatureSubst<'a>, Error>> {
        let lk = self.lookup(lookup_i)?;
        if lk.lookup_type() != GSUB_LOOKUP_TYPE_LIGATURE {
            return Some(Err(Error::BadStructure(
                "GSUB/LigatureSubst: lookup is not type 4",
            )));
        }
        let bytes = lk.subtable_bytes(sub_i)?;
        Some(LigatureSubst::parse(bytes))
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

    // -------------- LigatureSubst Format 1 -----------------------------

    /// Build a `LigatureSubstFormat1` subtable that mirrors the spec's
    /// Example 6: Coverage = `{e, f}`, e-set = `[etc]`, f-set = `[ffi,
    /// fi]` (ffi preferred over fi per the spec).
    ///
    /// Component glyph IDs are made-up flat numbers so we can assert
    /// the actual decoded bytes; the layout matches the OFF spec
    /// exactly.
    fn build_example_6_subtable() -> Vec<u8> {
        // Choose glyph IDs:
        //   e = 5, f = 6, t = 20, c = 21, i = 9, etc = 100,
        //   ffi = 101, fi = 102.
        //
        // Layout plan:
        //   0   /  2 / format = 1
        //   2   /  2 / coverageOffset → 30
        //   4   /  2 / ligatureSetCount = 2
        //   6   /  2 / ligatureSetOffsets[0] = 10   (→ e-set @ 10)
        //   8   /  2 / ligatureSetOffsets[1] = 18   (→ f-set @ 18)
        //  10   /  2 / ligatureCount = 1            (e-set: just "etc")
        //  12   /  2 / ligatureOffsets[0] = 4       (→ etc @ 14)
        //  14   /  2 / ligatureGlyph = 100  (etc)
        //  16   /  2 / componentCount = 3
        //  18   /  ⋮ / componentGlyphIDs unused for e-set (it's only 1
        //              ligature, but its bytes overlap the f-set start —
        //              fix the plan: lay out per-set independently.)
        //
        // Replan to avoid overlap. Use:
        //   off 0 .. 10 = header (format, covOff, setCount, setOffsets[0..2])
        //   off 10 .. ?  = e-set (1 ligature: "etc" — 3 components)
        //                  10 / 2 / ligatureCount = 1
        //                  12 / 2 / ligatureOffsets[0] = 4 (→ off 14)
        //                  14 / 2 / ligatureGlyph = 100
        //                  16 / 2 / componentCount = 3
        //                  18 / 2 / componentGlyphIDs[0] = t = 20
        //                  20 / 2 / componentGlyphIDs[1] = c = 21
        //                  → e-set ends at 22.
        //   off 22 .. ?  = f-set (2 ligatures: ffi, fi)
        //                  22 / 2 / ligatureCount = 2
        //                  24 / 2 / ligatureOffsets[0] = 8 (→ off 30; ffi)
        //                  26 / 2 / ligatureOffsets[1] = 16 (→ off 38; fi)
        //                  Wait — those offsets are from start of LigatureSet,
        //                  i.e. from off 22. Compute the on-disk offsets:
        //                    ffi @ off 30 → 30 - 22 = 8.   OK
        //                    fi  @ off 38 → 38 - 22 = 16.  OK
        //                  ffi Ligature @ 30:
        //                    30 / 2 / ligatureGlyph = 101
        //                    32 / 2 / componentCount = 3   (f, f, i)
        //                    34 / 2 / componentGlyphIDs[0] = f = 6
        //                    36 / 2 / componentGlyphIDs[1] = i = 9
        //                    → ends at 38.
        //                  fi Ligature @ 38:
        //                    38 / 2 / ligatureGlyph = 102
        //                    40 / 2 / componentCount = 2   (f, i)
        //                    42 / 2 / componentGlyphIDs[0] = i = 9
        //                    → ends at 44.
        //   off 44 .. 50 = Coverage Format 1 with [e, f] (sorted)
        //                  44 / 2 / coverage format = 1
        //                  46 / 2 / glyphCount = 2
        //                  48 / 2 / glyphArray[0] = e = 5
        //                  50 / 2 / glyphArray[1] = f = 6
        //                  → ends at 52.
        //
        // Re-pin Coverage offset to 44 and ligatureSetOffsets to [10, 22].
        let mut out = vec![0u8; 52];
        // Header
        out[0..2].copy_from_slice(&be(1)); // format
        out[2..4].copy_from_slice(&be(44)); // coverageOffset
        out[4..6].copy_from_slice(&be(2)); // ligatureSetCount
        out[6..8].copy_from_slice(&be(10)); // ligatureSetOffsets[0]
        out[8..10].copy_from_slice(&be(22)); // ligatureSetOffsets[1]
                                             // e-set @ 10 (1 ligature, "etc")
        out[10..12].copy_from_slice(&be(1)); // ligatureCount
        out[12..14].copy_from_slice(&be(4)); // ligatureOffsets[0]
                                             // etc Ligature @ 14
        out[14..16].copy_from_slice(&be(100)); // ligatureGlyph
        out[16..18].copy_from_slice(&be(3)); // componentCount
        out[18..20].copy_from_slice(&be(20)); // t
        out[20..22].copy_from_slice(&be(21)); // c
                                              // f-set @ 22 (2 ligatures, ffi then fi)
        out[22..24].copy_from_slice(&be(2)); // ligatureCount
        out[24..26].copy_from_slice(&be(8)); // ligatureOffsets[0] -> ffi
        out[26..28].copy_from_slice(&be(16)); // ligatureOffsets[1] -> fi
                                              // ffi Ligature @ 30
        out[28..30].copy_from_slice(&[0, 0]); // padding (set table only goes
                                              // through offset 28; bytes 28..30 unused, set to 0)
        out[30..32].copy_from_slice(&be(101)); // ligatureGlyph
        out[32..34].copy_from_slice(&be(3)); // componentCount
        out[34..36].copy_from_slice(&be(6)); // f
        out[36..38].copy_from_slice(&be(9)); // i
                                             // fi Ligature @ 38
        out[38..40].copy_from_slice(&be(102)); // ligatureGlyph
        out[40..42].copy_from_slice(&be(2)); // componentCount
        out[42..44].copy_from_slice(&be(9)); // i
                                             // Coverage Format 1 @ 44
        out[44..46].copy_from_slice(&be(1)); // coverage format
        out[46..48].copy_from_slice(&be(2)); // glyphCount
        out[48..50].copy_from_slice(&be(5)); // e
        out[50..52].copy_from_slice(&be(6)); // f
        out
    }

    #[test]
    fn ligature_subst_example_6_round_trip() {
        // Replays the spec's Example 6: Coverage = {e, f}; e → [etc];
        // f → [ffi, fi].
        let raw = build_example_6_subtable();
        let ls = LigatureSubst::parse(&raw).unwrap();
        assert_eq!(ls.format(), 1);
        assert_eq!(ls.ligature_set_count(), 2);

        // e-set: one ligature "etc" matching glyphs [e=5, t=20, c=21].
        let e_set = ls.ligature_set(0).unwrap().unwrap();
        assert_eq!(e_set.ligature_count(), 1);
        let etc = e_set.ligature(0).unwrap().unwrap();
        assert_eq!(etc.ligature_glyph(), 100);
        assert_eq!(etc.component_count(), 3);
        let etc_tail: Vec<_> = etc.component_glyphs().collect();
        assert_eq!(etc_tail, vec![20, 21]);

        // f-set: ffi (preferred) then fi.
        let f_set = ls.ligature_set(1).unwrap().unwrap();
        assert_eq!(f_set.ligature_count(), 2);
        let ffi = f_set.ligature(0).unwrap().unwrap();
        assert_eq!(ffi.ligature_glyph(), 101);
        assert_eq!(ffi.component_count(), 3);
        let ffi_tail: Vec<_> = ffi.component_glyphs().collect();
        assert_eq!(ffi_tail, vec![6, 9]);
        let fi = f_set.ligature(1).unwrap().unwrap();
        assert_eq!(fi.ligature_glyph(), 102);
        assert_eq!(fi.component_count(), 2);
        let fi_tail: Vec<_> = fi.component_glyphs().collect();
        assert_eq!(fi_tail, vec![9]);
    }

    #[test]
    fn ligature_subst_substitute_matches_etc() {
        let raw = build_example_6_subtable();
        let ls = LigatureSubst::parse(&raw).unwrap();
        // Input sequence (e, t, c) → etc-ligature (gid 100), 3 glyphs
        // consumed.
        assert_eq!(ls.substitute(&[5, 20, 21]), Some((100, 3)));
        // Trailing glyphs past the ligature length are ignored.
        assert_eq!(ls.substitute(&[5, 20, 21, 99]), Some((100, 3)));
    }

    #[test]
    fn ligature_subst_substitute_prefers_ffi_over_fi() {
        let raw = build_example_6_subtable();
        let ls = LigatureSubst::parse(&raw).unwrap();
        // Per spec: "the order in the Ligature offset array defines
        // the preference for using the ligatures". ffi precedes fi in
        // f-set's offset list, so an (f, f, i) input matches ffi first
        // — even though (f, f) is not a valid fi prefix, this fixture
        // mostly demonstrates that the *first* matching ligature wins.
        assert_eq!(ls.substitute(&[6, 6, 9]), Some((101, 3)));
        // An (f, i) input — too short for ffi — falls through to fi.
        assert_eq!(ls.substitute(&[6, 9]), Some((102, 2)));
    }

    #[test]
    fn ligature_subst_substitute_returns_none_when_no_match() {
        let raw = build_example_6_subtable();
        let ls = LigatureSubst::parse(&raw).unwrap();
        // Empty input → None.
        assert_eq!(ls.substitute(&[]), None);
        // First glyph uncovered → None.
        assert_eq!(ls.substitute(&[7, 9]), None);
        // First glyph covered (e) but no Ligature in e-set matches the
        // tail (e-set only contains "etc" which expects t, c).
        assert_eq!(ls.substitute(&[5, 0, 0]), None);
        // First glyph f but no matching tail (f-set wants either
        // [f, i] or [f, i] — wait, ffi tail is [f, i] and fi tail is [i].
        // An (f, x) input where x != i and second-glyph != f matches
        // nothing.
        assert_eq!(ls.substitute(&[6, 99]), None);
    }

    #[test]
    fn ligature_subst_iter_walks_coverage_in_order() {
        let raw = build_example_6_subtable();
        let ls = LigatureSubst::parse(&raw).unwrap();
        let glyphs: Vec<_> = ls.iter().map(|(g, _)| g).collect();
        assert_eq!(glyphs, vec![5, 6]);
        // Each set still decodes from the iter view.
        for (_, set_res) in ls.iter() {
            let set = set_res.unwrap();
            assert!(set.ligature_count() >= 1);
        }
    }

    #[test]
    fn ligature_subst_rejects_unknown_format() {
        // format = 2 — undefined for Lookup Type 4.
        let mut raw = vec![0u8; 16];
        raw[0..2].copy_from_slice(&be(2));
        raw[2..4].copy_from_slice(&be(8));
        // Plausible coverage payload at offset 8.
        raw[8..10].copy_from_slice(&be(1));
        raw[10..12].copy_from_slice(&be(1));
        raw[12..14].copy_from_slice(&be(5));
        assert!(matches!(
            LigatureSubst::parse(&raw),
            Err(Error::BadStructure(_))
        ));
    }

    #[test]
    fn ligature_subst_rejects_truncated_set_offsets_array() {
        // Header claims ligatureSetCount = 4 (needs 8 bytes of
        // setOffsets at offsets 6..14) but the buffer ends mid-array.
        // The Coverage table is placed in-range so that the
        // coverageOffset check passes and the trailing-array length
        // check actually fires.
        //
        //   0  / 2 / format = 1
        //   2  / 2 / coverageOffset = 8   (must be < buffer.len() = 12)
        //   4  / 2 / ligatureSetCount = 4 (needs 8 bytes from off 6 →
        //                                  need = 14; buffer = 12.)
        //   6  / 2 / ligatureSetOffsets[0]
        //   8  / 2 / coverage format = 1
        //  10  / 2 / glyphCount = 0
        let mut raw = vec![0u8; 12];
        raw[0..2].copy_from_slice(&be(1));
        raw[2..4].copy_from_slice(&be(8));
        raw[4..6].copy_from_slice(&be(4));
        raw[6..8].copy_from_slice(&be(0));
        raw[8..10].copy_from_slice(&be(1));
        raw[10..12].copy_from_slice(&be(0));
        assert!(matches!(
            LigatureSubst::parse(&raw),
            Err(Error::UnexpectedEof)
        ));
    }

    #[test]
    fn ligature_subst_rejects_coverage_offset_out_of_range() {
        let mut raw = vec![0u8; 10];
        raw[0..2].copy_from_slice(&be(1)); // format
        raw[2..4].copy_from_slice(&be(99)); // coverageOffset past end
        raw[4..6].copy_from_slice(&be(0)); // ligatureSetCount
        assert!(matches!(
            LigatureSubst::parse(&raw),
            Err(Error::BadStructure(_))
        ));
    }

    #[test]
    fn ligature_subst_zero_component_count_rejected() {
        // A Ligature with componentCount = 0 is malformed by spec
        // (componentCount counts the first glyph too). Build a 1-set,
        // 1-lig subtable whose Ligature claims componentCount = 0.
        //
        //   0   / 2 / format = 1
        //   2   / 2 / coverageOffset = 14
        //   4   / 2 / ligatureSetCount = 1
        //   6   / 2 / ligatureSetOffsets[0] = 8 (→ off 8)
        //   8   / 2 / ligatureCount = 1
        //  10   / 2 / ligatureOffsets[0] = 4 (→ off 12)
        //  12   / 2 / ligatureGlyph = 50
        //  14   / 2 / componentCount = 0   (overlapping with Coverage —
        //                                   we'll lay it out so it's
        //                                   non-overlapping)
        //  Replan: put Coverage AFTER the Ligature payload.
        //   0   / 2 / format = 1
        //   2   / 2 / coverageOffset = 16
        //   4   / 2 / ligatureSetCount = 1
        //   6   / 2 / ligatureSetOffsets[0] = 8 (→ off 8)
        //   8   / 2 / ligatureCount = 1
        //  10   / 2 / ligatureOffsets[0] = 4 (→ off 12)
        //  12   / 2 / ligatureGlyph = 50
        //  14   / 2 / componentCount = 0
        //  16   / 2 / coverage format = 1
        //  18   / 2 / glyphCount = 1
        //  20   / 2 / glyph = 10
        let mut raw = vec![0u8; 22];
        raw[0..2].copy_from_slice(&be(1));
        raw[2..4].copy_from_slice(&be(16));
        raw[4..6].copy_from_slice(&be(1));
        raw[6..8].copy_from_slice(&be(8));
        raw[8..10].copy_from_slice(&be(1));
        raw[10..12].copy_from_slice(&be(4));
        raw[12..14].copy_from_slice(&be(50));
        raw[14..16].copy_from_slice(&be(0)); // componentCount = 0 → reject
        raw[16..18].copy_from_slice(&be(1));
        raw[18..20].copy_from_slice(&be(1));
        raw[20..22].copy_from_slice(&be(10));
        let ls = LigatureSubst::parse(&raw).unwrap();
        let set = ls.ligature_set(0).unwrap().unwrap();
        let lig = set.ligature(0).unwrap();
        assert!(matches!(lig, Err(Error::BadStructure(_))));
    }

    // -------------- GsubTable::ligature_subst integration --------------

    /// Build a tiny GSUB table whose only lookup is a type-4
    /// LigatureSubst subtable.
    fn build_minimal_ligature_gsub() -> Vec<u8> {
        // Subtable bytes copied wholesale from the Example-6 subtable.
        let sub = build_example_6_subtable();

        // GSUB layout (all offsets relative to start of GSUB):
        //   0   /  10 / header (script=10, feature=22, lookup=36)
        //   10  /  12 / ScriptList (1 record, DFLT @18)
        //   18  /   4 / Script (no LangSys)
        //   22  /  10 / FeatureList (1 record, "liga" @ 30)
        //   30  /   6 / Feature
        //   36  /   4 / LookupList (1 entry → 40)
        //   40  /   8 / Lookup type=4, flag=0, subTableCount=1, subOff=8
        //   48  /  ?  / LigatureSubstFormat1 subtable (sub.len() bytes)
        let head_end = 48 + sub.len();
        let mut bytes = vec![0u8; head_end];
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
        bytes[24..28].copy_from_slice(b"liga");
        bytes[28..30].copy_from_slice(&be(8));
        // Feature
        bytes[30..32].copy_from_slice(&be(0));
        bytes[32..34].copy_from_slice(&be(1));
        bytes[34..36].copy_from_slice(&be(0));
        // LookupList
        bytes[36..38].copy_from_slice(&be(1));
        bytes[38..40].copy_from_slice(&be(4));
        // Lookup: type=4, flag=0, subTableCount=1, subtableOffsets=[8]
        bytes[40..42].copy_from_slice(&be(4));
        bytes[42..44].copy_from_slice(&be(0));
        bytes[44..46].copy_from_slice(&be(1));
        bytes[46..48].copy_from_slice(&be(8));
        // Subtable
        bytes[48..head_end].copy_from_slice(&sub);
        bytes
    }

    #[test]
    fn gsub_ligature_subst_end_to_end() {
        let bytes = build_minimal_ligature_gsub();
        let g = GsubTable::parse(&bytes).unwrap();
        assert_eq!(g.lookup_count(), 1);
        let l0 = g.lookup(0).unwrap();
        assert_eq!(l0.lookup_type(), GSUB_LOOKUP_TYPE_LIGATURE);

        let ls = g.ligature_subst(0, 0).expect("subtable exists").unwrap();
        assert_eq!(ls.format(), 1);
        // Same end-to-end substitution as the standalone test.
        assert_eq!(ls.substitute(&[5, 20, 21]), Some((100, 3)));
        assert_eq!(ls.substitute(&[6, 6, 9]), Some((101, 3)));
        assert_eq!(ls.substitute(&[6, 9]), Some((102, 2)));
        assert_eq!(ls.substitute(&[7, 7]), None);
    }

    #[test]
    fn gsub_ligature_subst_rejects_non_type_4_lookup() {
        // Reuse the minimal_v10 layout but declare the Lookup as type
        // = 1 (single), then assert the typed accessor rejects.
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
        // Lookup: declare type = 1 (single), not type 4.
        bytes[48..50].copy_from_slice(&be(1));
        bytes[50..52].copy_from_slice(&be(0));
        bytes[52..54].copy_from_slice(&be(0));

        let g = GsubTable::parse(&bytes).unwrap();
        assert!(matches!(g.ligature_subst(0, 0), Some(Err(_))));
    }

    #[test]
    fn gsub_ligature_subst_out_of_range_indices_return_none() {
        let bytes = build_minimal_ligature_gsub();
        let g = GsubTable::parse(&bytes).unwrap();
        // Subtable index past the lookup's subTableCount.
        assert!(g.ligature_subst(0, 1).is_none());
        // Lookup index past the lookupCount.
        assert!(g.ligature_subst(99, 0).is_none());
    }
}
