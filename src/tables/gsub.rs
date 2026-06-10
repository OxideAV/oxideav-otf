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
//! * **GsubLookupType 2 — Multiple substitution** (one glyph replaced
//!   by a sequence of glyphs) via [`MultipleSubst`]. The one on-disk
//!   format is decoded
//!   ([`docs/text/opentype/otspec-gsub.html` §"Lookup type 2 subtable:
//!   multiple substitution"]):
//!   * Format 1 — `(format, coverageOffset, sequenceCount,
//!     sequenceOffsets[sequenceCount])`. Each Sequence table is
//!     `(glyphCount, substituteGlyphIDs[glyphCount])`. The covered
//!     input glyph at Coverage Index `i` is replaced by the
//!     `substituteGlyphIDs[]` of the Sequence at offset `i`. Per spec,
//!     `sequenceCount` must equal the Coverage table's glyph count, and
//!     every Sequence's `glyphCount` must be greater than zero (the
//!     spec explicitly prohibits using Multiple substitution as a
//!     deletion).
//! * **GsubLookupType 3 — Alternate substitution** (one glyph replaced
//!   by one of a set of aesthetic alternatives, the choice being a
//!   higher-layer decision) via [`AlternateSubst`]. The one on-disk
//!   format is decoded
//!   ([`docs/text/opentype/otspec-gsub.html` §"Lookup type 3 subtable:
//!   alternate substitution"]):
//!   * Format 1 — `(format, coverageOffset, alternateSetCount,
//!     alternateSetOffsets[alternateSetCount])`. Each AlternateSet is
//!     `(glyphCount, alternateGlyphIDs[glyphCount])`, in arbitrary
//!     order per spec.
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
//! * **GsubLookupType 7 — Substitution extension** (a 32-bit-offset
//!   indirection wrapping a subtable of any other lookup type) via
//!   [`ExtensionSubst`]. The one on-disk format is decoded
//!   ([`docs/text/opentype/otspec-gsub.html` §"Lookup type 7 subtable:
//!   substitution subtable extension"]):
//!   * Format 1 — `(format, extensionLookupType, extensionOffset)`.
//!     The `Offset32 extensionOffset` (relative to the start of the
//!     ExtensionSubstFormat1 subtable) reaches the real subtable, whose
//!     type is `extensionLookupType` (any GsubLookupType other than 7).
//!     Typed resolvers wrap the types this crate already decodes
//!     (1 / 2 / 3 / 4); the rest are reachable as raw bytes.
//!
//! Other GSUB subtable types (5 Contextual, 6 Chained-context, 8
//! Reverse-chained-single) remain raw sub-slices via
//! [`super::layout::Lookup::subtable_bytes`]; decoding their interiors
//! is deferred to a future round.

use crate::parser::{read_i16, read_u16, read_u32};
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
// Lookup type 2: Multiple substitution (otspec-gsub.html §"Lookup type 2
// subtable: multiple substitution")
// ---------------------------------------------------------------------------

/// Parsed `MultipleSubst` subtable — the GSUB `lookupType = 2` payload.
///
/// Spec: `docs/text/opentype/otspec-gsub.html` §"Lookup type 2 subtable:
/// multiple substitution".
///
/// One on-disk format is defined. The Coverage table names the input
/// glyphs; each covered glyph maps (by Coverage Index) to a
/// per-input-glyph `Sequence` table that carries the output glyph
/// sequence.
///
/// ```text
/// MultipleSubstFormat1 subtable
///   0 / 2 / format = 1
///   2 / 2 / coverageOffset       (Offset16, from start of subtable)
///   4 / 2 / sequenceCount        (== Coverage.len())
///   6 / 2 * n / sequenceOffsets[sequenceCount]
///                                 (Offset16, from start of subtable,
///                                  ordered by Coverage index)
///
/// Sequence table
///   0 / 2 / glyphCount           (must be > 0; the spec prohibits
///                                 using MultipleSubst as a deletion)
///   2 / 2 * n / substituteGlyphIDs[glyphCount]
/// ```
///
/// [`Self::substitute`] returns the replacement glyph sequence (if any)
/// for an input glyph as a borrowed slice of `u16`-valued bytes; use
/// [`Self::substitute_iter`] for an iterator over the typed `u16`
/// outputs.
#[derive(Debug, Clone, Copy)]
pub struct MultipleSubst<'a> {
    /// Raw subtable bytes (offsets in the on-disk records are relative
    /// to this buffer's start).
    bytes: &'a [u8],
    coverage: Coverage<'a>,
    /// Slice of the `sequenceOffsets[]` array (2 bytes per offset).
    seq_offsets: &'a [u8],
}

impl<'a> MultipleSubst<'a> {
    /// Parse a MultipleSubst subtable from a buffer whose first two
    /// bytes are the `format` identifier.
    ///
    /// Validates the format discriminant (only `1` is defined), the
    /// `coverageOffset` window, that `sequenceCount` equals the
    /// Coverage length (spec: "the sequenceOffsets array … must contain
    /// the same number of offsets as the Coverage table"), and that the
    /// trailing `sequenceOffsets[]` array fits inside the supplied
    /// slice. Per-Sequence payloads are validated lazily —
    /// [`Self::sequence`] re-validates on each call so a malformed
    /// inner record can't poison the top-level view.
    pub fn parse(bytes: &'a [u8]) -> Result<Self, Error> {
        let format = read_u16(bytes, 0)?;
        if format != 1 {
            return Err(Error::BadStructure(
                "GSUB/MultipleSubst: unknown subtable format",
            ));
        }
        let cov_off = read_u16(bytes, 2)? as usize;
        if cov_off == 0 || cov_off >= bytes.len() {
            return Err(Error::BadStructure(
                "GSUB/MultipleSubst: coverageOffset out of range",
            ));
        }
        let coverage = Coverage::parse(&bytes[cov_off..])?;
        let seq_count = read_u16(bytes, 4)? as usize;
        // Spec: "the sequenceOffsets array … must contain the same
        // number of offsets as the Coverage table."
        if seq_count != coverage.len() {
            return Err(Error::BadStructure(
                "GSUB/MultipleSubst: sequenceCount != coverage.len()",
            ));
        }
        let array_start = 6usize;
        let need = array_start
            .checked_add(
                seq_count
                    .checked_mul(2)
                    .ok_or(Error::BadStructure("GSUB/MultipleSubst length overflow"))?,
            )
            .ok_or(Error::BadStructure("GSUB/MultipleSubst length overflow"))?;
        if bytes.len() < need {
            return Err(Error::UnexpectedEof);
        }
        Ok(Self {
            bytes,
            coverage,
            seq_offsets: &bytes[array_start..need],
        })
    }

    /// Subtable format discriminant (always `1`).
    pub fn format(&self) -> u16 {
        1
    }

    /// The input-side [`Coverage`] table. Each covered glyph maps (by
    /// Coverage Index) to a per-input-glyph [`Sequence`].
    pub fn coverage(&self) -> Coverage<'a> {
        self.coverage
    }

    /// `sequenceCount` — number of Sequence tables. Equal to
    /// `coverage().len()` by spec invariant.
    pub fn sequence_count(&self) -> u16 {
        (self.seq_offsets.len() / 2) as u16
    }

    /// Borrow the [`Sequence`] at the given Coverage index. Returns
    /// `None` for an out-of-range index, `Some(Err(...))` when the
    /// referenced bytes are malformed or violate the spec's
    /// "glyphCount > 0" rule.
    pub fn sequence(&self, seq_i: u16) -> Option<Result<Sequence<'a>, Error>> {
        let off2 = (seq_i as usize).checked_mul(2)?;
        if off2 + 2 > self.seq_offsets.len() {
            return None;
        }
        let off = u16::from_be_bytes([self.seq_offsets[off2], self.seq_offsets[off2 + 1]]) as usize;
        if off == 0 || off >= self.bytes.len() {
            return Some(Err(Error::BadStructure(
                "GSUB/MultipleSubst: sequenceOffset out of range",
            )));
        }
        Some(Sequence::parse(&self.bytes[off..]))
    }

    /// Apply this subtable as a shaper would.
    ///
    /// Returns `Some(sequence)` when `input` is in [`Self::coverage`];
    /// the returned [`Sequence`] carries the substitute glyph sequence.
    /// Returns `None` when `input` is uncovered or the per-input
    /// Sequence bytes are unreachable / malformed (the typed accessor
    /// [`Self::sequence`] is the diagnosis path for the latter).
    pub fn substitute(&self, input: u16) -> Option<Sequence<'a>> {
        let i = self.coverage.index_of(input)?;
        self.sequence(i)?.ok()
    }

    /// Iterate the `(input_glyph, Sequence)` pairs in this subtable in
    /// ascending Coverage order. Malformed Sequence references are
    /// surfaced as `Err`.
    pub fn iter(&self) -> MultipleSubstIter<'a> {
        MultipleSubstIter {
            cov: self.coverage.iter(),
            outer: *self,
        }
    }
}

/// Iterator yielded by [`MultipleSubst::iter`].
#[derive(Debug, Clone)]
pub struct MultipleSubstIter<'a> {
    cov: crate::tables::gdef::CoverageIter<'a>,
    outer: MultipleSubst<'a>,
}

impl<'a> Iterator for MultipleSubstIter<'a> {
    type Item = (u16, Result<Sequence<'a>, Error>);
    fn next(&mut self) -> Option<Self::Item> {
        let (g, idx) = self.cov.next()?;
        let seq = self.outer.sequence(idx)?;
        Some((g, seq))
    }
}

/// Parsed `Sequence` table — a count + an array of output glyph IDs
/// (the substitute glyph sequence for one covered input glyph).
///
/// The on-disk record is `(glyphCount,
/// substituteGlyphIDs[glyphCount])`. Per spec, `glyphCount` "must
/// always be greater than 0" — the prohibition against using
/// MultipleSubst as a deletion. [`Sequence::parse`] enforces the rule;
/// a zero `glyphCount` surfaces as `Error::BadStructure`.
#[derive(Debug, Clone, Copy)]
pub struct Sequence<'a> {
    /// Raw `substituteGlyphIDs[]` payload — `2 * glyphCount` bytes,
    /// big-endian `u16` per entry.
    glyphs: &'a [u8],
}

impl<'a> Sequence<'a> {
    /// Parse a Sequence table from a buffer whose first two bytes are
    /// `glyphCount`.
    pub fn parse(bytes: &'a [u8]) -> Result<Self, Error> {
        let count = read_u16(bytes, 0)? as usize;
        if count == 0 {
            // Spec: "The use of multiple substitution for deletion of
            // an input glyph is prohibited. The glyphCount value must
            // always be greater than 0."
            return Err(Error::BadStructure(
                "GSUB/Sequence: glyphCount must be >= 1",
            ));
        }
        let array_start = 2usize;
        let need = array_start
            .checked_add(
                count
                    .checked_mul(2)
                    .ok_or(Error::BadStructure("GSUB/Sequence length overflow"))?,
            )
            .ok_or(Error::BadStructure("GSUB/Sequence length overflow"))?;
        if bytes.len() < need {
            return Err(Error::UnexpectedEof);
        }
        Ok(Self {
            glyphs: &bytes[array_start..need],
        })
    }

    /// `glyphCount` — number of glyph IDs in the substitute sequence.
    /// Always >= 1 per spec.
    pub fn glyph_count(&self) -> u16 {
        (self.glyphs.len() / 2) as u16
    }

    /// The substitute glyph at output index `i` (`0 .. glyphCount`).
    pub fn glyph(&self, i: u16) -> Option<u16> {
        let off = (i as usize).checked_mul(2)?;
        if off + 2 > self.glyphs.len() {
            return None;
        }
        Some(u16::from_be_bytes([self.glyphs[off], self.glyphs[off + 1]]))
    }

    /// Iterator over every substitute glyph in output order.
    pub fn glyphs(&self) -> SequenceGlyphIter<'a> {
        SequenceGlyphIter {
            bytes: self.glyphs,
            pos: 0,
        }
    }
}

/// Iterator over a [`Sequence`]'s `substituteGlyphIDs[]` in output
/// order.
#[derive(Debug, Clone)]
pub struct SequenceGlyphIter<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Iterator for SequenceGlyphIter<'a> {
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

// ---------------------------------------------------------------------------
// Lookup type 3: Alternate substitution (otspec-gsub.html §"Lookup type 3
// subtable: alternate substitution")
// ---------------------------------------------------------------------------

/// Parsed `AlternateSubst` subtable — the GSUB `lookupType = 3` payload.
///
/// Spec: `docs/text/opentype/otspec-gsub.html` §"Lookup type 3 subtable:
/// alternate substitution".
///
/// An alternate substitution identifies any number of functionally
/// equivalent but different-looking forms (aesthetic alternatives) of a
/// glyph. The Coverage table names the input glyphs; each covered glyph
/// maps (by Coverage Index) to an `AlternateSet` table that lists the
/// alternative glyph IDs the client may choose from. Per spec, the
/// alternatives "can be in any order in the array" — the choice of which
/// alternate to use is a higher-level (feature / UI) decision and is not
/// encoded here.
///
/// One on-disk format is defined.
///
/// ```text
/// AlternateSubstFormat1 subtable
///   0 / 2 / format = 1
///   2 / 2 / coverageOffset       (Offset16, from start of subtable)
///   4 / 2 / alternateSetCount    (== Coverage.len())
///   6 / 2 * n / alternateSetOffsets[alternateSetCount]
///                                 (Offset16, from start of subtable,
///                                  ordered by Coverage index)
///
/// AlternateSet table
///   0 / 2 / glyphCount
///   2 / 2 * n / alternateGlyphIDs[glyphCount]  (arbitrary order)
/// ```
#[derive(Debug, Clone, Copy)]
pub struct AlternateSubst<'a> {
    /// Raw subtable bytes (offsets in the on-disk records are relative
    /// to this buffer's start).
    bytes: &'a [u8],
    coverage: Coverage<'a>,
    /// Slice of the `alternateSetOffsets[]` array (2 bytes per offset).
    set_offsets: &'a [u8],
}

impl<'a> AlternateSubst<'a> {
    /// Parse an AlternateSubst subtable from a buffer whose first two
    /// bytes are the `format` identifier.
    ///
    /// Validates the format discriminant (only `1` is defined), the
    /// `coverageOffset` window, that `alternateSetCount` equals the
    /// Coverage length (the `alternateSetOffsets` array is "ordered by
    /// Coverage index", so the two counts must agree), and that the
    /// trailing `alternateSetOffsets[]` array fits inside the supplied
    /// slice. Per-AlternateSet payloads are validated lazily —
    /// [`Self::alternate_set`] re-validates on each call so a malformed
    /// inner record can't poison the top-level view.
    pub fn parse(bytes: &'a [u8]) -> Result<Self, Error> {
        let format = read_u16(bytes, 0)?;
        if format != 1 {
            return Err(Error::BadStructure(
                "GSUB/AlternateSubst: unknown subtable format",
            ));
        }
        let cov_off = read_u16(bytes, 2)? as usize;
        if cov_off == 0 || cov_off >= bytes.len() {
            return Err(Error::BadStructure(
                "GSUB/AlternateSubst: coverageOffset out of range",
            ));
        }
        let coverage = Coverage::parse(&bytes[cov_off..])?;
        let set_count = read_u16(bytes, 4)? as usize;
        // Spec: alternateSetOffsets are "ordered by Coverage index", so
        // the array must carry exactly one offset per covered glyph.
        if set_count != coverage.len() {
            return Err(Error::BadStructure(
                "GSUB/AlternateSubst: alternateSetCount != coverage.len()",
            ));
        }
        let array_start = 6usize;
        let need = array_start
            .checked_add(
                set_count
                    .checked_mul(2)
                    .ok_or(Error::BadStructure("GSUB/AlternateSubst length overflow"))?,
            )
            .ok_or(Error::BadStructure("GSUB/AlternateSubst length overflow"))?;
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

    /// The input-side [`Coverage`] table. Each covered glyph maps (by
    /// Coverage Index) to a per-input-glyph [`AlternateSet`].
    pub fn coverage(&self) -> Coverage<'a> {
        self.coverage
    }

    /// `alternateSetCount` — number of AlternateSet tables. Equal to
    /// `coverage().len()` by spec invariant.
    pub fn alternate_set_count(&self) -> u16 {
        (self.set_offsets.len() / 2) as u16
    }

    /// Borrow the [`AlternateSet`] at the given Coverage index. Returns
    /// `None` for an out-of-range index, `Some(Err(...))` when the
    /// referenced bytes are malformed.
    pub fn alternate_set(&self, set_i: u16) -> Option<Result<AlternateSet<'a>, Error>> {
        let off2 = (set_i as usize).checked_mul(2)?;
        if off2 + 2 > self.set_offsets.len() {
            return None;
        }
        let off = u16::from_be_bytes([self.set_offsets[off2], self.set_offsets[off2 + 1]]) as usize;
        if off == 0 || off >= self.bytes.len() {
            return Some(Err(Error::BadStructure(
                "GSUB/AlternateSubst: alternateSetOffset out of range",
            )));
        }
        Some(AlternateSet::parse(&self.bytes[off..]))
    }

    /// Apply this subtable as a shaper would.
    ///
    /// Returns `Some(alternate_set)` when `input` is in
    /// [`Self::coverage`]; the returned [`AlternateSet`] carries the
    /// alternative glyph IDs from which a client picks. Returns `None`
    /// when `input` is uncovered or the per-input AlternateSet bytes are
    /// unreachable / malformed (the typed accessor
    /// [`Self::alternate_set`] is the diagnosis path for the latter).
    ///
    /// Note: this does **not** itself choose an alternate — the spec
    /// leaves alternate selection to a higher layer ("the client could
    /// use the default glyph or substitute any of the alternatives").
    pub fn substitute(&self, input: u16) -> Option<AlternateSet<'a>> {
        let i = self.coverage.index_of(input)?;
        self.alternate_set(i)?.ok()
    }

    /// Iterate the `(input_glyph, AlternateSet)` pairs in this subtable
    /// in ascending Coverage order. Malformed AlternateSet references are
    /// surfaced as `Err`.
    pub fn iter(&self) -> AlternateSubstIter<'a> {
        AlternateSubstIter {
            cov: self.coverage.iter(),
            outer: *self,
        }
    }
}

/// Iterator yielded by [`AlternateSubst::iter`].
#[derive(Debug, Clone)]
pub struct AlternateSubstIter<'a> {
    cov: crate::tables::gdef::CoverageIter<'a>,
    outer: AlternateSubst<'a>,
}

impl<'a> Iterator for AlternateSubstIter<'a> {
    type Item = (u16, Result<AlternateSet<'a>, Error>);
    fn next(&mut self) -> Option<Self::Item> {
        let (g, idx) = self.cov.next()?;
        let set = self.outer.alternate_set(idx)?;
        Some((g, set))
    }
}

/// Parsed `AlternateSet` table — a count + an array of alternate glyph
/// IDs (the aesthetic alternatives for one covered input glyph).
///
/// The on-disk record is `(glyphCount, alternateGlyphIDs[glyphCount])`.
/// Per spec the alternates are "in arbitrary order"; the spec sets no
/// lower bound on `glyphCount`, so an empty AlternateSet (no alternates)
/// is accepted rather than rejected — it simply yields zero choices.
#[derive(Debug, Clone, Copy)]
pub struct AlternateSet<'a> {
    /// Raw `alternateGlyphIDs[]` payload — `2 * glyphCount` bytes,
    /// big-endian `u16` per entry.
    glyphs: &'a [u8],
}

impl<'a> AlternateSet<'a> {
    /// Parse an AlternateSet table from a buffer whose first two bytes
    /// are `glyphCount`.
    pub fn parse(bytes: &'a [u8]) -> Result<Self, Error> {
        let count = read_u16(bytes, 0)? as usize;
        let array_start = 2usize;
        let need = array_start
            .checked_add(
                count
                    .checked_mul(2)
                    .ok_or(Error::BadStructure("GSUB/AlternateSet length overflow"))?,
            )
            .ok_or(Error::BadStructure("GSUB/AlternateSet length overflow"))?;
        if bytes.len() < need {
            return Err(Error::UnexpectedEof);
        }
        Ok(Self {
            glyphs: &bytes[array_start..need],
        })
    }

    /// `glyphCount` — number of alternate glyph IDs.
    pub fn glyph_count(&self) -> u16 {
        (self.glyphs.len() / 2) as u16
    }

    /// The alternate glyph at index `i` (`0 .. glyphCount`). The order is
    /// arbitrary per spec — index `0` is not privileged.
    pub fn glyph(&self, i: u16) -> Option<u16> {
        let off = (i as usize).checked_mul(2)?;
        if off + 2 > self.glyphs.len() {
            return None;
        }
        Some(u16::from_be_bytes([self.glyphs[off], self.glyphs[off + 1]]))
    }

    /// Iterator over every alternate glyph in on-disk (arbitrary) order.
    pub fn glyphs(&self) -> AlternateGlyphIter<'a> {
        AlternateGlyphIter {
            bytes: self.glyphs,
            pos: 0,
        }
    }
}

/// Iterator over an [`AlternateSet`]'s `alternateGlyphIDs[]` in on-disk
/// (arbitrary) order.
#[derive(Debug, Clone)]
pub struct AlternateGlyphIter<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Iterator for AlternateGlyphIter<'a> {
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

// ---------------------------------------------------------------------------
// Lookup type 7: Substitution extension (otspec-gsub.html §"Lookup type 7
// subtable: substitution subtable extension")
// ---------------------------------------------------------------------------

/// Parsed `ExtensionSubst` subtable — the GSUB `lookupType = 7` payload.
///
/// Spec: `docs/text/opentype/otspec-gsub.html` §"Lookup type 7 subtable:
/// substitution subtable extension".
///
/// This lookup type is a *format extension mechanism*, not a
/// substitution action: it lets a Lookup reach its real subtable
/// through a 32-bit offset, for fonts whose accumulated subtable sizes
/// exceed what the usual 16-bit offsets can address. The spec's
/// processing model: proceed as though the Lookup's `lookupType` were
/// the `extensionLookupType` of the subtables, and as though each
/// extension subtable referenced by `extensionOffset` replaced the
/// type 7 subtable that referenced it.
///
/// One on-disk format is defined.
///
/// ```text
/// SubstExtensionFormat1 subtable (8 bytes)
///   0 / 2 / format = 1
///   2 / 2 / extensionLookupType   (any GsubLookupType other than 7)
///   4 / 4 / extensionOffset       (Offset32, from start of this
///                                  ExtensionSubstFormat1 subtable)
/// ```
///
/// Parse-time validation: `format == 1`; `extensionLookupType` must be
/// a defined GsubLookupType (`1..=8`) **other than 7** (the spec
/// forbids an extension pointing at another extension); and
/// `extensionOffset` must land inside the supplied byte window. The
/// wrapped subtable is surfaced both raw
/// ([`Self::extension_subtable_bytes`]) and through typed resolvers for
/// the lookup types this crate already decodes
/// ([`Self::as_single_subst`] / [`Self::as_multiple_subst`] /
/// [`Self::as_alternate_subst`] / [`Self::as_ligature_subst`]).
#[derive(Debug, Clone, Copy)]
pub struct ExtensionSubst<'a> {
    /// Raw subtable bytes (`extensionOffset` is relative to this
    /// buffer's start).
    bytes: &'a [u8],
    ext_lookup_type: u16,
    ext_offset: u32,
}

impl<'a> ExtensionSubst<'a> {
    /// Parse an ExtensionSubst subtable from a buffer whose first two
    /// bytes are the `format` identifier.
    pub fn parse(bytes: &'a [u8]) -> Result<Self, Error> {
        let format = read_u16(bytes, 0)?;
        if format != 1 {
            return Err(Error::BadStructure(
                "GSUB/ExtensionSubst: unknown subtable format",
            ));
        }
        let ext_lookup_type = read_u16(bytes, 2)?;
        // Spec: "The extensionLookupType field must be set to any
        // lookup type other than 7." The GsubLookupType vocabulary is
        // 1..=8, so anything outside that range is equally undefined.
        if ext_lookup_type == GSUB_LOOKUP_TYPE_EXTENSION {
            return Err(Error::BadStructure(
                "GSUB/ExtensionSubst: extensionLookupType must not be 7",
            ));
        }
        if !(GSUB_LOOKUP_TYPE_SINGLE..=GSUB_LOOKUP_TYPE_REVERSE_CHAINED_SINGLE)
            .contains(&ext_lookup_type)
        {
            return Err(Error::BadStructure(
                "GSUB/ExtensionSubst: extensionLookupType out of range",
            ));
        }
        let ext_offset = read_u32(bytes, 4)?;
        // "All offsets to extension subtables are set in the usual
        // way—that is, relative to start of the ExtensionSubstFormat1
        // subtable." A NULL offset has no defined meaning here, and the
        // wrapped subtable must start inside the byte window.
        if ext_offset == 0 || (ext_offset as usize) >= bytes.len() {
            return Err(Error::BadStructure(
                "GSUB/ExtensionSubst: extensionOffset out of range",
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
    /// Guaranteed to be in `1..=8` and never `7`.
    pub fn extension_lookup_type(&self) -> u16 {
        self.ext_lookup_type
    }

    /// `extensionOffset` — byte offset of the wrapped subtable,
    /// relative to the start of this ExtensionSubstFormat1 subtable.
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

    /// Resolve the wrapped subtable as a [`SingleSubst`]
    /// (`extensionLookupType = 1`). `Err(BadStructure)` when the
    /// declared type disagrees or the wrapped bytes are malformed.
    pub fn as_single_subst(&self) -> Result<SingleSubst<'a>, Error> {
        if self.ext_lookup_type != GSUB_LOOKUP_TYPE_SINGLE {
            return Err(Error::BadStructure(
                "GSUB/ExtensionSubst: extensionLookupType is not 1",
            ));
        }
        SingleSubst::parse(self.extension_subtable_bytes())
    }

    /// Resolve the wrapped subtable as a [`MultipleSubst`]
    /// (`extensionLookupType = 2`). `Err(BadStructure)` when the
    /// declared type disagrees or the wrapped bytes are malformed.
    pub fn as_multiple_subst(&self) -> Result<MultipleSubst<'a>, Error> {
        if self.ext_lookup_type != GSUB_LOOKUP_TYPE_MULTIPLE {
            return Err(Error::BadStructure(
                "GSUB/ExtensionSubst: extensionLookupType is not 2",
            ));
        }
        MultipleSubst::parse(self.extension_subtable_bytes())
    }

    /// Resolve the wrapped subtable as an [`AlternateSubst`]
    /// (`extensionLookupType = 3`). `Err(BadStructure)` when the
    /// declared type disagrees or the wrapped bytes are malformed.
    pub fn as_alternate_subst(&self) -> Result<AlternateSubst<'a>, Error> {
        if self.ext_lookup_type != GSUB_LOOKUP_TYPE_ALTERNATE {
            return Err(Error::BadStructure(
                "GSUB/ExtensionSubst: extensionLookupType is not 3",
            ));
        }
        AlternateSubst::parse(self.extension_subtable_bytes())
    }

    /// Resolve the wrapped subtable as a [`LigatureSubst`]
    /// (`extensionLookupType = 4`). `Err(BadStructure)` when the
    /// declared type disagrees or the wrapped bytes are malformed.
    pub fn as_ligature_subst(&self) -> Result<LigatureSubst<'a>, Error> {
        if self.ext_lookup_type != GSUB_LOOKUP_TYPE_LIGATURE {
            return Err(Error::BadStructure(
                "GSUB/ExtensionSubst: extensionLookupType is not 4",
            ));
        }
        LigatureSubst::parse(self.extension_subtable_bytes())
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
    /// [`MultipleSubst`] (`GsubLookupType = 2`).
    ///
    /// Returns:
    /// * `None` — `lookup_i` or `sub_i` is out of range, or the
    ///   referenced subtable bytes are unreachable.
    /// * `Some(Err(Error::BadStructure))` — the lookup is not declared
    ///   as `GSUB_LOOKUP_TYPE_MULTIPLE`, or the subtable bytes are
    ///   malformed.
    /// * `Some(Ok(MultipleSubst))` — the typed subtable view.
    pub fn multiple_subst(
        &self,
        lookup_i: u16,
        sub_i: u16,
    ) -> Option<Result<MultipleSubst<'a>, Error>> {
        let lk = self.lookup(lookup_i)?;
        if lk.lookup_type() != GSUB_LOOKUP_TYPE_MULTIPLE {
            return Some(Err(Error::BadStructure(
                "GSUB/MultipleSubst: lookup is not type 2",
            )));
        }
        let bytes = lk.subtable_bytes(sub_i)?;
        Some(MultipleSubst::parse(bytes))
    }

    /// Decode subtable `sub_i` of lookup `lookup_i` as an
    /// [`AlternateSubst`] (`GsubLookupType = 3`).
    ///
    /// Returns:
    /// * `None` — `lookup_i` or `sub_i` is out of range, or the
    ///   referenced subtable bytes are unreachable.
    /// * `Some(Err(Error::BadStructure))` — the lookup is not declared
    ///   as `GSUB_LOOKUP_TYPE_ALTERNATE`, or the subtable bytes are
    ///   malformed.
    /// * `Some(Ok(AlternateSubst))` — the typed subtable view.
    pub fn alternate_subst(
        &self,
        lookup_i: u16,
        sub_i: u16,
    ) -> Option<Result<AlternateSubst<'a>, Error>> {
        let lk = self.lookup(lookup_i)?;
        if lk.lookup_type() != GSUB_LOOKUP_TYPE_ALTERNATE {
            return Some(Err(Error::BadStructure(
                "GSUB/AlternateSubst: lookup is not type 3",
            )));
        }
        let bytes = lk.subtable_bytes(sub_i)?;
        Some(AlternateSubst::parse(bytes))
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

    /// Decode subtable `sub_i` of lookup `lookup_i` as an
    /// [`ExtensionSubst`] (`GsubLookupType = 7`).
    ///
    /// Returns:
    /// * `None` — `lookup_i` or `sub_i` is out of range, or the
    ///   referenced subtable bytes are unreachable.
    /// * `Some(Err(Error::BadStructure))` — the lookup is not
    ///   declared as `GSUB_LOOKUP_TYPE_EXTENSION`, or the subtable
    ///   bytes are malformed.
    /// * `Some(Ok(ExtensionSubst))` — the typed subtable view; resolve
    ///   the wrapped subtable through
    ///   [`ExtensionSubst::extension_subtable_bytes`] or one of the
    ///   typed `as_*` resolvers.
    pub fn extension_subst(
        &self,
        lookup_i: u16,
        sub_i: u16,
    ) -> Option<Result<ExtensionSubst<'a>, Error>> {
        let lk = self.lookup(lookup_i)?;
        if lk.lookup_type() != GSUB_LOOKUP_TYPE_EXTENSION {
            return Some(Err(Error::BadStructure(
                "GSUB/ExtensionSubst: lookup is not type 7",
            )));
        }
        let bytes = lk.subtable_bytes(sub_i)?;
        Some(ExtensionSubst::parse(bytes))
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

    // -------------- MultipleSubst Format 1 -----------------------------

    /// Build the spec's worked Example 4: replace the "ffi" ligature
    /// glyph (`0x00F1` = 241) with the three-glyph sequence
    /// `[f=0x1A=26, f=0x1A=26, i=0x1D=29]` (the bytes from §"Example 4:
    /// MultipleSubstFormat1 subtable"). The layout below uses hand-set
    /// offsets so the resulting bytes match the spec's hex listing
    /// exactly through the header + Coverage + Sequence rows.
    fn build_example_4_subtable() -> Vec<u8> {
        // Layout plan (matches the spec's Example 4 byte listing):
        //   off  0 .. 2  / format = 1
        //   off  2 .. 4  / coverageOffset = 8
        //   off  4 .. 6  / sequenceCount = 1
        //   off  6 .. 8  / sequenceOffsets[0] = 14   (→ Sequence @ off 14)
        //   off  8 .. 10 / coverage format = 1
        //   off 10 .. 12 / glyphCount = 1
        //   off 12 .. 14 / glyphArray[0] = 0x00F1 (ffi)
        //   off 14 .. 16 / glyphCount = 3
        //   off 16 .. 18 / substituteGlyphIDs[0] = 0x001A (f)
        //   off 18 .. 20 / substituteGlyphIDs[1] = 0x001A (f)
        //   off 20 .. 22 / substituteGlyphIDs[2] = 0x001D (i)
        let mut out = vec![0u8; 22];
        out[0..2].copy_from_slice(&be(1));
        out[2..4].copy_from_slice(&be(8));
        out[4..6].copy_from_slice(&be(1));
        out[6..8].copy_from_slice(&be(14));
        // Coverage Format 1
        out[8..10].copy_from_slice(&be(1));
        out[10..12].copy_from_slice(&be(1));
        out[12..14].copy_from_slice(&be(0x00F1));
        // Sequence
        out[14..16].copy_from_slice(&be(3));
        out[16..18].copy_from_slice(&be(0x001A));
        out[18..20].copy_from_slice(&be(0x001A));
        out[20..22].copy_from_slice(&be(0x001D));
        out
    }

    #[test]
    fn multiple_subst_example_4_round_trip() {
        // Replays the spec's Example 4: the "ffi" ligature (glyph 241)
        // decomposes into [f=26, f=26, i=29].
        let raw = build_example_4_subtable();
        let ms = MultipleSubst::parse(&raw).unwrap();
        assert_eq!(ms.format(), 1);
        assert_eq!(ms.sequence_count(), 1);

        let seq = ms.sequence(0).unwrap().unwrap();
        assert_eq!(seq.glyph_count(), 3);
        assert_eq!(seq.glyph(0), Some(0x001A));
        assert_eq!(seq.glyph(1), Some(0x001A));
        assert_eq!(seq.glyph(2), Some(0x001D));
        assert_eq!(seq.glyph(3), None);
        let collected: Vec<_> = seq.glyphs().collect();
        assert_eq!(collected, vec![0x001A, 0x001A, 0x001D]);

        // substitute() routes Coverage → Sequence and yields the same
        // bytes for the covered input glyph.
        let out_seq = ms.substitute(0x00F1).expect("ffi is covered");
        let outputs: Vec<_> = out_seq.glyphs().collect();
        assert_eq!(outputs, vec![0x001A, 0x001A, 0x001D]);
        // Uncovered glyphs return None.
        assert_eq!(
            ms.substitute(0x00F2).map(|s| s.glyph_count()),
            None,
            "uncovered glyph must not substitute",
        );
    }

    #[test]
    fn multiple_subst_iter_walks_coverage_in_order() {
        // Two covered glyphs `{5, 9}`; their sequences are
        // `[100, 101]` and `[200]` respectively.
        //
        // Layout plan:
        //   off  0 .. 2  / format = 1
        //   off  2 .. 4  / coverageOffset = 22
        //   off  4 .. 6  / sequenceCount = 2
        //   off  6 .. 8  / sequenceOffsets[0] = 10  (→ seq0 @ off 10)
        //   off  8 .. 10 / sequenceOffsets[1] = 16  (→ seq1 @ off 16)
        //   off 10 .. 12 / glyphCount = 2
        //   off 12 .. 14 / substituteGlyphIDs[0] = 100
        //   off 14 .. 16 / substituteGlyphIDs[1] = 101
        //   off 16 .. 18 / glyphCount = 1
        //   off 18 .. 20 / substituteGlyphIDs[0] = 200
        //   off 20 .. 22 / padding (Coverage starts at 22)
        //   off 22 .. 24 / coverage format = 1
        //   off 24 .. 26 / glyphCount = 2
        //   off 26 .. 28 / glyphArray[0] = 5
        //   off 28 .. 30 / glyphArray[1] = 9
        let mut raw = vec![0u8; 30];
        raw[0..2].copy_from_slice(&be(1));
        raw[2..4].copy_from_slice(&be(22));
        raw[4..6].copy_from_slice(&be(2));
        raw[6..8].copy_from_slice(&be(10));
        raw[8..10].copy_from_slice(&be(16));
        raw[10..12].copy_from_slice(&be(2));
        raw[12..14].copy_from_slice(&be(100));
        raw[14..16].copy_from_slice(&be(101));
        raw[16..18].copy_from_slice(&be(1));
        raw[18..20].copy_from_slice(&be(200));
        raw[22..24].copy_from_slice(&be(1));
        raw[24..26].copy_from_slice(&be(2));
        raw[26..28].copy_from_slice(&be(5));
        raw[28..30].copy_from_slice(&be(9));

        let ms = MultipleSubst::parse(&raw).unwrap();
        assert_eq!(ms.sequence_count(), 2);

        let pairs: Vec<(u16, Vec<u16>)> = ms
            .iter()
            .map(|(g, s)| (g, s.unwrap().glyphs().collect()))
            .collect();
        assert_eq!(pairs, vec![(5, vec![100, 101]), (9, vec![200])],);
    }

    #[test]
    fn multiple_subst_rejects_unknown_format() {
        // format = 2 is undefined for Lookup Type 2.
        let mut raw = vec![0u8; 14];
        raw[0..2].copy_from_slice(&be(2));
        raw[2..4].copy_from_slice(&be(8));
        // Plausible coverage payload at offset 8 so the format check
        // fires before the coverage walk.
        raw[8..10].copy_from_slice(&be(1));
        raw[10..12].copy_from_slice(&be(1));
        raw[12..14].copy_from_slice(&be(5));
        assert!(matches!(
            MultipleSubst::parse(&raw),
            Err(Error::BadStructure(_))
        ));
    }

    #[test]
    fn multiple_subst_rejects_coverage_offset_out_of_range() {
        let mut raw = vec![0u8; 10];
        raw[0..2].copy_from_slice(&be(1));
        raw[2..4].copy_from_slice(&be(99));
        raw[4..6].copy_from_slice(&be(0));
        assert!(matches!(
            MultipleSubst::parse(&raw),
            Err(Error::BadStructure(_))
        ));
    }

    #[test]
    fn multiple_subst_rejects_sequence_count_mismatch() {
        // Build the Example-4 subtable, then poke sequenceCount to a
        // value that disagrees with the Coverage's glyphCount = 1.
        let mut raw = build_example_4_subtable();
        // sequenceCount lives at offset 4..6. We patch to 7 and the
        // parser should refuse rather than fall through to a stale
        // offset.
        raw[4..6].copy_from_slice(&be(7));
        assert!(matches!(
            MultipleSubst::parse(&raw),
            Err(Error::BadStructure(_) | Error::UnexpectedEof),
        ));
    }

    #[test]
    fn multiple_subst_rejects_truncated_sequence_offsets_array() {
        // Header claims sequenceCount = 4 (needs 8 bytes of offsets at
        // offsets 6..14) but the buffer ends mid-array. Coverage is
        // placed in-range so the coverageOffset check passes; we then
        // need Coverage.len() to also be 4 so the sequenceCount /
        // Coverage.len() check passes and the trailing-array length
        // check actually fires.
        //
        //   0  / 2 / format = 1
        //   2  / 2 / coverageOffset = 8
        //   4  / 2 / sequenceCount = 4 (needs 8 bytes from off 6 →
        //                              need = 14; buffer = 12.)
        //   6  / 2 / sequenceOffsets[0]
        //   8  / 2 / coverage format = 1
        //  10  / 2 / glyphCount = 4 (so sequenceCount == coverage.len())
        let mut raw = vec![0u8; 12];
        raw[0..2].copy_from_slice(&be(1));
        raw[2..4].copy_from_slice(&be(8));
        raw[4..6].copy_from_slice(&be(4));
        raw[6..8].copy_from_slice(&be(0));
        raw[8..10].copy_from_slice(&be(1));
        raw[10..12].copy_from_slice(&be(4));
        // The Coverage parser also wants room for the 4-glyph array,
        // which the buffer doesn't have. Either rejection (truncated
        // sequenceOffsets[] or truncated Coverage) is spec-correct.
        assert!(matches!(
            MultipleSubst::parse(&raw),
            Err(Error::UnexpectedEof) | Err(Error::BadStructure(_))
        ));
    }

    #[test]
    fn multiple_subst_rejects_zero_glyph_count_sequence() {
        // A Sequence with glyphCount = 0 is spec-prohibited
        // ("The use of multiple substitution for deletion of an input
        // glyph is prohibited."). The top-level subtable parses, but
        // walking into the bad Sequence surfaces BadStructure.
        //
        // Layout:
        //   off  0 .. 2 / format = 1
        //   off  2 .. 4 / coverageOffset = 12
        //   off  4 .. 6 / sequenceCount = 1
        //   off  6 .. 8 / sequenceOffsets[0] = 10 (→ Sequence @ off 10)
        //   off  8 .. 10 / padding (Coverage starts at 12)
        //   off 10 .. 12 / glyphCount = 0 (Sequence)
        //   off 12 .. 14 / coverage format = 1
        //   off 14 .. 16 / glyphCount = 1
        //   off 16 .. 18 / glyphArray[0] = 7
        let mut raw = vec![0u8; 18];
        raw[0..2].copy_from_slice(&be(1));
        raw[2..4].copy_from_slice(&be(12));
        raw[4..6].copy_from_slice(&be(1));
        raw[6..8].copy_from_slice(&be(10));
        raw[10..12].copy_from_slice(&be(0));
        raw[12..14].copy_from_slice(&be(1));
        raw[14..16].copy_from_slice(&be(1));
        raw[16..18].copy_from_slice(&be(7));

        let ms = MultipleSubst::parse(&raw).unwrap();
        let bad = ms.sequence(0).unwrap();
        assert!(matches!(bad, Err(Error::BadStructure(_))));
        // substitute() returns None when the inner Sequence is bad —
        // shaper-path callers can't act on a malformed subtable.
        assert!(ms.substitute(7).is_none());
    }

    // -------------- GsubTable::multiple_subst integration --------------

    /// Build a minimal v1.0 GSUB table whose only Lookup is a type-2
    /// MultipleSubst subtable (the spec's Example 4).
    fn build_minimal_multiple_gsub() -> Vec<u8> {
        let sub = build_example_4_subtable();

        // GSUB layout (mirrors the LigatureSubst end-to-end fixture):
        //   0   /  10 / header (script=10, feature=22, lookup=36)
        //   10  /  12 / ScriptList
        //   18  /   4 / Script
        //   22  /  10 / FeatureList ("ccmp" — the canonical Multiple
        //                            user, though the tag is purely
        //                            informational here)
        //   30  /   6 / Feature
        //   36  /   4 / LookupList
        //   40  /   8 / Lookup type=2, flag=0, subTableCount=1, subOff=8
        //   48  /  ?  / MultipleSubstFormat1 subtable (sub.len() bytes)
        let head_end = 48 + sub.len();
        let mut bytes = vec![0u8; head_end];
        bytes[0..2].copy_from_slice(&be(1));
        bytes[2..4].copy_from_slice(&be(0));
        bytes[4..6].copy_from_slice(&be(10));
        bytes[6..8].copy_from_slice(&be(22));
        bytes[8..10].copy_from_slice(&be(36));
        bytes[10..12].copy_from_slice(&be(1));
        bytes[12..16].copy_from_slice(b"DFLT");
        bytes[16..18].copy_from_slice(&be(8));
        bytes[18..20].copy_from_slice(&be(0));
        bytes[20..22].copy_from_slice(&be(0));
        bytes[22..24].copy_from_slice(&be(1));
        bytes[24..28].copy_from_slice(b"ccmp");
        bytes[28..30].copy_from_slice(&be(8));
        bytes[30..32].copy_from_slice(&be(0));
        bytes[32..34].copy_from_slice(&be(1));
        bytes[34..36].copy_from_slice(&be(0));
        bytes[36..38].copy_from_slice(&be(1));
        bytes[38..40].copy_from_slice(&be(4));
        // Lookup: type=2, flag=0, subTableCount=1, subtableOffsets=[8]
        bytes[40..42].copy_from_slice(&be(2));
        bytes[42..44].copy_from_slice(&be(0));
        bytes[44..46].copy_from_slice(&be(1));
        bytes[46..48].copy_from_slice(&be(8));
        // Subtable
        bytes[48..head_end].copy_from_slice(&sub);
        bytes
    }

    #[test]
    fn gsub_multiple_subst_end_to_end() {
        let bytes = build_minimal_multiple_gsub();
        let g = GsubTable::parse(&bytes).unwrap();
        assert_eq!(g.lookup_count(), 1);
        let l0 = g.lookup(0).unwrap();
        assert_eq!(l0.lookup_type(), GSUB_LOOKUP_TYPE_MULTIPLE);

        let ms = g.multiple_subst(0, 0).expect("subtable exists").unwrap();
        let seq = ms.substitute(0x00F1).expect("ffi is covered");
        let outputs: Vec<_> = seq.glyphs().collect();
        assert_eq!(outputs, vec![0x001A, 0x001A, 0x001D]);

        // Wrong subtable index -> None.
        assert!(g.multiple_subst(0, 1).is_none());
        // Wrong lookup index -> None.
        assert!(g.multiple_subst(99, 0).is_none());
    }

    #[test]
    fn gsub_multiple_subst_rejects_non_type_2_lookup() {
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
        // Lookup: declare type = 4 (ligature), not type 2.
        bytes[48..50].copy_from_slice(&be(4));
        bytes[50..52].copy_from_slice(&be(0));
        bytes[52..54].copy_from_slice(&be(0));

        let g = GsubTable::parse(&bytes).unwrap();
        assert!(matches!(g.multiple_subst(0, 0), Some(Err(_))));
    }

    // -------------- AlternateSubst Format 1 ----------------------------

    /// Build the spec's worked Example 5: the default ampersand glyph
    /// (`0x003A` = 58) maps to an AlternateSet of two alternative
    /// ampersand glyphs `[0x00C9 = 201, 0x00CA = 202]`. The offsets are
    /// hand-set to match §"Example 5: AlternateSubstFormat 1 subtable"
    /// exactly (coverageOffset = 8, alternateSetOffsets[0] = 14).
    fn build_example_5_subtable() -> Vec<u8> {
        // Layout plan (matches the spec's Example 5 byte listing):
        //   off  0 .. 2  / format = 1
        //   off  2 .. 4  / coverageOffset = 8
        //   off  4 .. 6  / alternateSetCount = 1
        //   off  6 .. 8  / alternateSetOffsets[0] = 14 (→ AltSet @ off 14)
        //   off  8 .. 10 / coverage format = 1
        //   off 10 .. 12 / glyphCount = 1
        //   off 12 .. 14 / glyphArray[0] = 0x003A (default ampersand)
        //   off 14 .. 16 / glyphCount = 2 (AlternateSet)
        //   off 16 .. 18 / alternateGlyphIDs[0] = 0x00C9
        //   off 18 .. 20 / alternateGlyphIDs[1] = 0x00CA
        let mut out = vec![0u8; 20];
        out[0..2].copy_from_slice(&be(1));
        out[2..4].copy_from_slice(&be(8));
        out[4..6].copy_from_slice(&be(1));
        out[6..8].copy_from_slice(&be(14));
        // Coverage Format 1
        out[8..10].copy_from_slice(&be(1));
        out[10..12].copy_from_slice(&be(1));
        out[12..14].copy_from_slice(&be(0x003A));
        // AlternateSet
        out[14..16].copy_from_slice(&be(2));
        out[16..18].copy_from_slice(&be(0x00C9));
        out[18..20].copy_from_slice(&be(0x00CA));
        out
    }

    #[test]
    fn alternate_subst_example_5_round_trip() {
        let raw = build_example_5_subtable();
        let alt = AlternateSubst::parse(&raw).unwrap();
        assert_eq!(alt.format(), 1);
        assert_eq!(alt.alternate_set_count(), 1);

        let set = alt.alternate_set(0).unwrap().unwrap();
        assert_eq!(set.glyph_count(), 2);
        assert_eq!(set.glyph(0), Some(0x00C9));
        assert_eq!(set.glyph(1), Some(0x00CA));
        assert_eq!(set.glyph(2), None);
        let collected: Vec<_> = set.glyphs().collect();
        assert_eq!(collected, vec![0x00C9, 0x00CA]);

        // substitute() routes Coverage → AlternateSet for the covered
        // ampersand and yields the same alternatives.
        let out_set = alt.substitute(0x003A).expect("ampersand is covered");
        let outputs: Vec<_> = out_set.glyphs().collect();
        assert_eq!(outputs, vec![0x00C9, 0x00CA]);
        // Uncovered glyphs return None.
        assert_eq!(
            alt.substitute(0x003B).map(|s| s.glyph_count()),
            None,
            "uncovered glyph must not substitute",
        );
    }

    #[test]
    fn alternate_subst_iter_walks_coverage_in_order() {
        // Two covered glyphs `{5, 9}`; their AlternateSets are
        // `[100, 101]` and `[200]` respectively.
        //
        // Layout plan:
        //   off  0 .. 2  / format = 1
        //   off  2 .. 4  / coverageOffset = 22
        //   off  4 .. 6  / alternateSetCount = 2
        //   off  6 .. 8  / alternateSetOffsets[0] = 10  (→ set0 @ off 10)
        //   off  8 .. 10 / alternateSetOffsets[1] = 16  (→ set1 @ off 16)
        //   off 10 .. 12 / glyphCount = 2
        //   off 12 .. 14 / alternateGlyphIDs[0] = 100
        //   off 14 .. 16 / alternateGlyphIDs[1] = 101
        //   off 16 .. 18 / glyphCount = 1
        //   off 18 .. 20 / alternateGlyphIDs[0] = 200
        //   off 20 .. 22 / padding (Coverage starts at 22)
        //   off 22 .. 24 / coverage format = 1
        //   off 24 .. 26 / glyphCount = 2
        //   off 26 .. 28 / glyphArray[0] = 5
        //   off 28 .. 30 / glyphArray[1] = 9
        let mut raw = vec![0u8; 30];
        raw[0..2].copy_from_slice(&be(1));
        raw[2..4].copy_from_slice(&be(22));
        raw[4..6].copy_from_slice(&be(2));
        raw[6..8].copy_from_slice(&be(10));
        raw[8..10].copy_from_slice(&be(16));
        raw[10..12].copy_from_slice(&be(2));
        raw[12..14].copy_from_slice(&be(100));
        raw[14..16].copy_from_slice(&be(101));
        raw[16..18].copy_from_slice(&be(1));
        raw[18..20].copy_from_slice(&be(200));
        raw[22..24].copy_from_slice(&be(1));
        raw[24..26].copy_from_slice(&be(2));
        raw[26..28].copy_from_slice(&be(5));
        raw[28..30].copy_from_slice(&be(9));

        let alt = AlternateSubst::parse(&raw).unwrap();
        assert_eq!(alt.alternate_set_count(), 2);

        let pairs: Vec<(u16, Vec<u16>)> = alt
            .iter()
            .map(|(g, s)| (g, s.unwrap().glyphs().collect()))
            .collect();
        assert_eq!(pairs, vec![(5, vec![100, 101]), (9, vec![200])],);
    }

    #[test]
    fn alternate_subst_rejects_unknown_format() {
        // format = 2 is undefined for Lookup Type 3.
        let mut raw = vec![0u8; 14];
        raw[0..2].copy_from_slice(&be(2));
        raw[2..4].copy_from_slice(&be(8));
        // Plausible coverage payload at offset 8 so the format check
        // fires before the coverage walk.
        raw[8..10].copy_from_slice(&be(1));
        raw[10..12].copy_from_slice(&be(1));
        raw[12..14].copy_from_slice(&be(5));
        assert!(matches!(
            AlternateSubst::parse(&raw),
            Err(Error::BadStructure(_))
        ));
    }

    #[test]
    fn alternate_subst_rejects_coverage_offset_out_of_range() {
        let mut raw = vec![0u8; 10];
        raw[0..2].copy_from_slice(&be(1));
        raw[2..4].copy_from_slice(&be(99));
        raw[4..6].copy_from_slice(&be(0));
        assert!(matches!(
            AlternateSubst::parse(&raw),
            Err(Error::BadStructure(_))
        ));
    }

    #[test]
    fn alternate_subst_rejects_set_count_mismatch() {
        // Build the Example-5 subtable, then poke alternateSetCount to a
        // value that disagrees with the Coverage's glyphCount = 1.
        let mut raw = build_example_5_subtable();
        raw[4..6].copy_from_slice(&be(7));
        assert!(matches!(
            AlternateSubst::parse(&raw),
            Err(Error::BadStructure(_) | Error::UnexpectedEof),
        ));
    }

    #[test]
    fn alternate_subst_rejects_truncated_set_offsets_array() {
        // Header claims alternateSetCount = 4 (needs 8 bytes of offsets
        // at offsets 6..14) but the buffer ends mid-array. Coverage is
        // placed in-range with a matching glyphCount = 4 so the count
        // invariant passes and the trailing-array length check fires.
        //
        //   0  / 2 / format = 1
        //   2  / 2 / coverageOffset = 8
        //   4  / 2 / alternateSetCount = 4
        //   6  / 2 / alternateSetOffsets[0]
        //   8  / 2 / coverage format = 1
        //  10  / 2 / glyphCount = 4
        let mut raw = vec![0u8; 12];
        raw[0..2].copy_from_slice(&be(1));
        raw[2..4].copy_from_slice(&be(8));
        raw[4..6].copy_from_slice(&be(4));
        raw[6..8].copy_from_slice(&be(0));
        raw[8..10].copy_from_slice(&be(1));
        raw[10..12].copy_from_slice(&be(4));
        assert!(matches!(
            AlternateSubst::parse(&raw),
            Err(Error::UnexpectedEof) | Err(Error::BadStructure(_))
        ));
    }

    #[test]
    fn alternate_subst_accepts_empty_alternate_set() {
        // The spec sets no lower bound on AlternateSet.glyphCount, so an
        // empty AlternateSet (no alternatives) is accepted — it simply
        // yields zero choices. Contrast MultipleSubst, which prohibits a
        // zero-length Sequence.
        //
        // Layout:
        //   off  0 .. 2 / format = 1
        //   off  2 .. 4 / coverageOffset = 12
        //   off  4 .. 6 / alternateSetCount = 1
        //   off  6 .. 8 / alternateSetOffsets[0] = 10 (→ AltSet @ off 10)
        //   off  8 .. 10 / padding (Coverage starts at 12)
        //   off 10 .. 12 / glyphCount = 0 (AlternateSet)
        //   off 12 .. 14 / coverage format = 1
        //   off 14 .. 16 / glyphCount = 1
        //   off 16 .. 18 / glyphArray[0] = 7
        let mut raw = vec![0u8; 18];
        raw[0..2].copy_from_slice(&be(1));
        raw[2..4].copy_from_slice(&be(12));
        raw[4..6].copy_from_slice(&be(1));
        raw[6..8].copy_from_slice(&be(10));
        raw[10..12].copy_from_slice(&be(0));
        raw[12..14].copy_from_slice(&be(1));
        raw[14..16].copy_from_slice(&be(1));
        raw[16..18].copy_from_slice(&be(7));

        let alt = AlternateSubst::parse(&raw).unwrap();
        let set = alt.alternate_set(0).unwrap().unwrap();
        assert_eq!(set.glyph_count(), 0);
        assert_eq!(set.glyph(0), None);
        assert_eq!(set.glyphs().count(), 0);
        // substitute() of the covered glyph yields the (empty) set.
        let out = alt.substitute(7).expect("covered");
        assert_eq!(out.glyph_count(), 0);
    }

    // -------------- GsubTable::alternate_subst integration -------------

    /// Build a minimal v1.0 GSUB table whose only Lookup is a type-3
    /// AlternateSubst subtable (the spec's Example 5).
    fn build_minimal_alternate_gsub() -> Vec<u8> {
        let sub = build_example_5_subtable();

        // GSUB layout (mirrors the MultipleSubst end-to-end fixture):
        //   0   /  10 / header (script=10, feature=22, lookup=36)
        //   10  /  12 / ScriptList
        //   22  /  10 / FeatureList ("aalt" — the canonical Alternate
        //                            user; the tag is informational here)
        //   36  /   4 / LookupList
        //   40  /   8 / Lookup type=3, flag=0, subTableCount=1, subOff=8
        //   48  /  ?  / AlternateSubstFormat1 subtable (sub.len() bytes)
        let head_end = 48 + sub.len();
        let mut bytes = vec![0u8; head_end];
        bytes[0..2].copy_from_slice(&be(1));
        bytes[2..4].copy_from_slice(&be(0));
        bytes[4..6].copy_from_slice(&be(10));
        bytes[6..8].copy_from_slice(&be(22));
        bytes[8..10].copy_from_slice(&be(36));
        bytes[10..12].copy_from_slice(&be(1));
        bytes[12..16].copy_from_slice(b"DFLT");
        bytes[16..18].copy_from_slice(&be(8));
        bytes[18..20].copy_from_slice(&be(0));
        bytes[20..22].copy_from_slice(&be(0));
        bytes[22..24].copy_from_slice(&be(1));
        bytes[24..28].copy_from_slice(b"aalt");
        bytes[28..30].copy_from_slice(&be(8));
        bytes[30..32].copy_from_slice(&be(0));
        bytes[32..34].copy_from_slice(&be(1));
        bytes[34..36].copy_from_slice(&be(0));
        bytes[36..38].copy_from_slice(&be(1));
        bytes[38..40].copy_from_slice(&be(4));
        // Lookup: type=3, flag=0, subTableCount=1, subtableOffsets=[8]
        bytes[40..42].copy_from_slice(&be(3));
        bytes[42..44].copy_from_slice(&be(0));
        bytes[44..46].copy_from_slice(&be(1));
        bytes[46..48].copy_from_slice(&be(8));
        // Subtable
        bytes[48..head_end].copy_from_slice(&sub);
        bytes
    }

    #[test]
    fn gsub_alternate_subst_end_to_end() {
        let bytes = build_minimal_alternate_gsub();
        let g = GsubTable::parse(&bytes).unwrap();
        assert_eq!(g.lookup_count(), 1);
        let l0 = g.lookup(0).unwrap();
        assert_eq!(l0.lookup_type(), GSUB_LOOKUP_TYPE_ALTERNATE);

        let alt = g.alternate_subst(0, 0).expect("subtable exists").unwrap();
        let set = alt.substitute(0x003A).expect("ampersand is covered");
        let outputs: Vec<_> = set.glyphs().collect();
        assert_eq!(outputs, vec![0x00C9, 0x00CA]);

        // Wrong subtable index -> None.
        assert!(g.alternate_subst(0, 1).is_none());
        // Wrong lookup index -> None.
        assert!(g.alternate_subst(99, 0).is_none());
    }

    #[test]
    fn gsub_alternate_subst_rejects_non_type_3_lookup() {
        // Reuse the minimal layout but declare the Lookup as type = 4
        // (ligature), then assert the typed accessor rejects.
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
        // Lookup: declare type = 4 (ligature), not type 3.
        bytes[48..50].copy_from_slice(&be(4));
        bytes[50..52].copy_from_slice(&be(0));
        bytes[52..54].copy_from_slice(&be(0));

        let g = GsubTable::parse(&bytes).unwrap();
        assert!(matches!(g.alternate_subst(0, 0), Some(Err(_))));
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

    // -------------- ExtensionSubst (lookup type 7) ----------------------

    /// Build a SubstExtensionFormat1 subtable wrapping `inner` at the
    /// minimal `extensionOffset = 8` (immediately after the 8-byte
    /// extension header).
    fn build_extension_subst(ext_type: u16, inner: &[u8]) -> Vec<u8> {
        // Layout:
        //   0 / 2 / format = 1
        //   2 / 2 / extensionLookupType
        //   4 / 4 / extensionOffset = 8 (Offset32)
        //   8 / n / wrapped subtable
        let mut out = Vec::new();
        out.extend_from_slice(&be(1)); // format
        out.extend_from_slice(&be(ext_type)); // extensionLookupType
        out.extend_from_slice(&8u32.to_be_bytes()); // extensionOffset
        out.extend_from_slice(inner);
        out
    }

    #[test]
    fn extension_subst_round_trip_wrapping_single_subst() {
        // Wrap a SingleSubstFormat1 (delta = 100 over {20, 21, 22})
        // behind a type-7 extension and resolve it through the typed
        // path. Per spec, the engine proceeds "as though each extension
        // subtable referenced by extensionOffset replaced the type 7
        // subtable that referenced it".
        let inner = build_single_subst_fmt1(100, &[20, 21, 22]);
        let raw = build_extension_subst(GSUB_LOOKUP_TYPE_SINGLE, &inner);
        let ext = ExtensionSubst::parse(&raw).unwrap();
        assert_eq!(ext.format(), 1);
        assert_eq!(ext.extension_lookup_type(), GSUB_LOOKUP_TYPE_SINGLE);
        assert_eq!(ext.extension_offset(), 8);
        // The raw window starts exactly at the wrapped subtable.
        assert_eq!(ext.extension_subtable_bytes(), &inner[..]);

        let ss = ext.as_single_subst().unwrap();
        assert_eq!(ss.format(), 1);
        assert_eq!(ss.substitute(20), Some(120));
        assert_eq!(ss.substitute(23), None);

        // The declared type gates the other resolvers.
        assert!(matches!(
            ext.as_multiple_subst(),
            Err(Error::BadStructure(_))
        ));
        assert!(matches!(
            ext.as_alternate_subst(),
            Err(Error::BadStructure(_))
        ));
        assert!(matches!(
            ext.as_ligature_subst(),
            Err(Error::BadStructure(_))
        ));
    }

    #[test]
    fn extension_subst_round_trip_wrapping_ligature_subst() {
        // Same indirection over the spec's Example-6 ligature subtable.
        let inner = build_example_6_subtable();
        let raw = build_extension_subst(GSUB_LOOKUP_TYPE_LIGATURE, &inner);
        let ext = ExtensionSubst::parse(&raw).unwrap();
        assert_eq!(ext.extension_lookup_type(), GSUB_LOOKUP_TYPE_LIGATURE);
        let ls = ext.as_ligature_subst().unwrap();
        assert_eq!(ls.substitute(&[5, 20, 21]), Some((100, 3)));
        assert!(matches!(ext.as_single_subst(), Err(Error::BadStructure(_))));
    }

    #[test]
    fn extension_subst_undecoded_type_exposes_raw_bytes() {
        // extensionLookupType = 8 (reverse chained single) has no typed
        // view yet; the parse must still validate the header and expose
        // the wrapped bytes raw.
        let inner = [0xAAu8, 0xBB, 0xCC, 0xDD];
        let raw = build_extension_subst(GSUB_LOOKUP_TYPE_REVERSE_CHAINED_SINGLE, &inner);
        let ext = ExtensionSubst::parse(&raw).unwrap();
        assert_eq!(
            ext.extension_lookup_type(),
            GSUB_LOOKUP_TYPE_REVERSE_CHAINED_SINGLE
        );
        assert_eq!(ext.extension_subtable_bytes(), &inner[..]);
        // No typed resolver matches type 8.
        assert!(matches!(ext.as_single_subst(), Err(Error::BadStructure(_))));
        assert!(matches!(
            ext.as_ligature_subst(),
            Err(Error::BadStructure(_))
        ));
    }

    #[test]
    fn extension_subst_rejects_unknown_format() {
        let inner = build_single_subst_fmt1(1, &[10]);
        let mut raw = build_extension_subst(GSUB_LOOKUP_TYPE_SINGLE, &inner);
        raw[0..2].copy_from_slice(&be(2)); // format = 2 is undefined
        assert!(matches!(
            ExtensionSubst::parse(&raw),
            Err(Error::BadStructure(_))
        ));
    }

    #[test]
    fn extension_subst_rejects_nested_extension_type() {
        // Spec: "The extensionLookupType field must be set to any
        // lookup type other than 7."
        let inner = build_single_subst_fmt1(1, &[10]);
        let raw = build_extension_subst(GSUB_LOOKUP_TYPE_EXTENSION, &inner);
        assert!(matches!(
            ExtensionSubst::parse(&raw),
            Err(Error::BadStructure(_))
        ));
    }

    #[test]
    fn extension_subst_rejects_out_of_vocabulary_lookup_type() {
        // The GsubLookupType vocabulary is 1..=8; 0 and 9 are undefined.
        let inner = build_single_subst_fmt1(1, &[10]);
        for bad in [0u16, 9, 0xFFFF] {
            let raw = build_extension_subst(bad, &inner);
            assert!(
                matches!(ExtensionSubst::parse(&raw), Err(Error::BadStructure(_))),
                "extensionLookupType = {bad} must be rejected"
            );
        }
    }

    #[test]
    fn extension_subst_rejects_offset_out_of_range() {
        let inner = build_single_subst_fmt1(1, &[10]);
        // NULL offset: no defined meaning for an extension subtable.
        let mut raw = build_extension_subst(GSUB_LOOKUP_TYPE_SINGLE, &inner);
        raw[4..8].copy_from_slice(&0u32.to_be_bytes());
        assert!(matches!(
            ExtensionSubst::parse(&raw),
            Err(Error::BadStructure(_))
        ));
        // Offset == buffer length: the wrapped subtable would start
        // past the end of the byte window.
        let mut raw = build_extension_subst(GSUB_LOOKUP_TYPE_SINGLE, &inner);
        let len = raw.len() as u32;
        raw[4..8].copy_from_slice(&len.to_be_bytes());
        assert!(matches!(
            ExtensionSubst::parse(&raw),
            Err(Error::BadStructure(_))
        ));
    }

    #[test]
    fn extension_subst_rejects_truncated_header() {
        // The header is 8 bytes (format + extensionLookupType +
        // Offset32); chopping the Offset32 must surface as EOF.
        let inner = build_single_subst_fmt1(1, &[10]);
        let raw = build_extension_subst(GSUB_LOOKUP_TYPE_SINGLE, &inner);
        assert!(matches!(
            ExtensionSubst::parse(&raw[..6]),
            Err(Error::UnexpectedEof)
        ));
        assert!(matches!(
            ExtensionSubst::parse(&raw[..2]),
            Err(Error::UnexpectedEof)
        ));
    }

    // -------------- GsubTable::extension_subst integration --------------

    /// Build a tiny GSUB table whose only lookup is a type-7 extension
    /// wrapping a SingleSubstFormat1 subtable.
    fn build_minimal_extension_gsub() -> Vec<u8> {
        let inner = build_single_subst_fmt1(200, &[50, 51]);
        let sub = build_extension_subst(GSUB_LOOKUP_TYPE_SINGLE, &inner);

        // GSUB layout (all offsets relative to start of GSUB):
        //   0   /  10 / header (script=10, feature=22, lookup=36)
        //   10  /  12 / ScriptList (1 record, DFLT @18)
        //   18  /   4 / Script (no LangSys)
        //   22  /  10 / FeatureList (1 record, "calt" @ 30)
        //   30  /   6 / Feature
        //   36  /   4 / LookupList (1 entry → 40)
        //   40  /   8 / Lookup type=7, flag=0, subTableCount=1, subOff=8
        //   48  /  ?  / SubstExtensionFormat1 subtable (sub.len() bytes)
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
        bytes[24..28].copy_from_slice(b"calt");
        bytes[28..30].copy_from_slice(&be(8));
        // Feature
        bytes[30..32].copy_from_slice(&be(0));
        bytes[32..34].copy_from_slice(&be(1));
        bytes[34..36].copy_from_slice(&be(0));
        // LookupList
        bytes[36..38].copy_from_slice(&be(1));
        bytes[38..40].copy_from_slice(&be(4));
        // Lookup: type=7, flag=0, subTableCount=1, subtableOffsets=[8]
        bytes[40..42].copy_from_slice(&be(7));
        bytes[42..44].copy_from_slice(&be(0));
        bytes[44..46].copy_from_slice(&be(1));
        bytes[46..48].copy_from_slice(&be(8));
        // Subtable
        bytes[48..head_end].copy_from_slice(&sub);
        bytes
    }

    #[test]
    fn gsub_extension_subst_end_to_end() {
        let bytes = build_minimal_extension_gsub();
        let g = GsubTable::parse(&bytes).unwrap();
        assert_eq!(g.lookup_count(), 1);
        let l0 = g.lookup(0).unwrap();
        assert_eq!(l0.lookup_type(), GSUB_LOOKUP_TYPE_EXTENSION);

        let ext = g.extension_subst(0, 0).expect("subtable exists").unwrap();
        assert_eq!(ext.format(), 1);
        assert_eq!(ext.extension_lookup_type(), GSUB_LOOKUP_TYPE_SINGLE);
        // Resolve the indirection and apply the wrapped substitution.
        let ss = ext.as_single_subst().unwrap();
        assert_eq!(ss.substitute(50), Some(250));
        assert_eq!(ss.substitute(51), Some(251));
        assert_eq!(ss.substitute(52), None);

        // The type-1 accessor must NOT bypass the declared lookup type:
        // the Lookup says 7, so single_subst() rejects it.
        assert!(matches!(g.single_subst(0, 0), Some(Err(_))));
    }

    #[test]
    fn gsub_extension_subst_rejects_non_type_7_lookup() {
        // The ligature GSUB declares its lookup as type 4.
        let bytes = build_minimal_ligature_gsub();
        let g = GsubTable::parse(&bytes).unwrap();
        assert!(matches!(g.extension_subst(0, 0), Some(Err(_))));
    }

    #[test]
    fn gsub_extension_subst_out_of_range_indices_return_none() {
        let bytes = build_minimal_extension_gsub();
        let g = GsubTable::parse(&bytes).unwrap();
        // Subtable index past the lookup's subTableCount.
        assert!(g.extension_subst(0, 1).is_none());
        // Lookup index past the lookupCount.
        assert!(g.extension_subst(99, 0).is_none());
    }
}
