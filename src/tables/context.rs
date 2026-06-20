//! Shared contextual / chained-contextual layout subtables.
//!
//! Spec: `docs/text/opentype/otspec-chapter2-common-layout-tables.html`
//! §"Sequence Contexts and Chained Sequence Contexts". The six on-disk
//! formats decoded here are shared verbatim between GSUB and GPOS:
//!
//! | format                          | GSUB type | GPOS type |
//! |---------------------------------|-----------|-----------|
//! | SequenceContextFormat1/2/3      | 5         | 7         |
//! | ChainedSequenceContextFormat1/2/3 | 6       | 8         |
//!
//! The subtables describe *where* a lookup applies (a glyph-sequence
//! pattern), not *what* it does — each match carries a list of
//! [`SequenceLookupRecord`]s naming nested lookups (by `LookupList`
//! index) to apply at given positions. This module decodes the pattern
//! structure and the lookup-record actions; resolving a nested lookup
//! and mutating the glyph buffer is the shaping client's job.
//!
//! All three "contextual" formats parameterise the input pattern
//! differently:
//!
//! * **Format 1** — explicit glyph IDs ([`SequenceContext::Format1`]).
//! * **Format 2** — glyph *classes* via a [`ClassDef`]
//!   ([`SequenceContext::Format2`]).
//! * **Format 3** — one pattern, each position a [`Coverage`] set
//!   ([`SequenceContext::Format3`]).
//!
//! The chained variants add backtrack and lookahead sequences around the
//! input sequence (the backtrack is stored in reverse logical order, per
//! spec).

use crate::parser::read_u16;
use crate::tables::gdef::{ClassDef, Coverage};
use crate::Error;

/// A `SequenceLookup` record — a nested-lookup action attached to a
/// contextual match.
///
/// Spec §"Sequence lookup record": `{ uint16 sequenceIndex; uint16
/// lookupListIndex }`. `sequence_index` is the zero-based position within
/// the *input* sequence at which the nested lookup applies;
/// `lookup_list_index` selects the nested lookup from the GSUB/GPOS
/// `LookupList`. Records are applied in array order, each acting on the
/// result of the previous.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SequenceLookupRecord {
    /// Zero-based index into the input glyph sequence.
    pub sequence_index: u16,
    /// Zero-based index into the GSUB/GPOS `LookupList`.
    pub lookup_list_index: u16,
}

/// Read a contiguous `count`-long array of [`SequenceLookupRecord`]s
/// starting at `off` within `bytes` (each record is 4 bytes).
fn read_seq_lookups(
    bytes: &[u8],
    off: usize,
    count: u16,
) -> Result<Vec<SequenceLookupRecord>, Error> {
    let mut out = Vec::with_capacity(count as usize);
    for i in 0..count as usize {
        let rec = off
            .checked_add(
                i.checked_mul(4)
                    .ok_or(Error::BadStructure("context: seqLookup index overflow"))?,
            )
            .ok_or(Error::BadStructure("context: seqLookup offset overflow"))?;
        out.push(SequenceLookupRecord {
            sequence_index: read_u16(bytes, rec)?,
            lookup_list_index: read_u16(bytes, rec + 2)?,
        });
    }
    Ok(out)
}

/// One decoded `SequenceRule` (Format 1) or `ClassSequenceRule`
/// (Format 2).
///
/// For Format 1 `input` holds explicit glyph IDs (the `inputSequence`
/// array, i.e. glyphs *2..glyphCount*, the first being implied by the
/// covered glyph). For Format 2 the same array holds glyph *class
/// values* instead. The first input position is never stored — it is the
/// Coverage-covered glyph (Format 1) or its class (Format 2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SequenceRule {
    /// `inputSequence` — the glyph IDs (Format 1) or class values
    /// (Format 2) for input positions `1..glyphCount`.
    pub input: Vec<u16>,
    /// The nested-lookup actions to apply on a match.
    pub lookups: Vec<SequenceLookupRecord>,
}

impl SequenceRule {
    /// Parse a SequenceRule / ClassSequenceRule from `bytes` at `off`.
    ///
    /// Layout: `uint16 glyphCount`, `uint16 seqLookupCount`,
    /// `uint16 inputSequence[glyphCount-1]`, then
    /// `seqLookupCount` SequenceLookup records.
    fn parse(bytes: &[u8], off: usize) -> Result<Self, Error> {
        let glyph_count = read_u16(bytes, off)?;
        let lookup_count = read_u16(bytes, off + 2)?;
        // glyphCount must be >= 1 (it includes the implied first glyph);
        // inputSequence has glyphCount - 1 entries.
        if glyph_count == 0 {
            return Err(Error::BadStructure("context: SequenceRule glyphCount 0"));
        }
        let input_len = (glyph_count - 1) as usize;
        let input_off = off + 4;
        let mut input = Vec::with_capacity(input_len);
        for i in 0..input_len {
            input.push(read_u16(bytes, input_off + i * 2)?);
        }
        let lookups_off = input_off + input_len * 2;
        let lookups = read_seq_lookups(bytes, lookups_off, lookup_count)?;
        Ok(SequenceRule { input, lookups })
    }
}

/// One decoded `ChainedSequenceRule` (Format 1) or
/// `ChainedClassSequenceRule` (Format 2).
///
/// `backtrack` is stored in *reverse logical order* exactly as on disk:
/// `backtrack[0]` is the glyph/class immediately preceding the input
/// (logical position `i-1`). `input` omits the first input position (the
/// covered glyph / its class). `lookahead` follows the input in logical
/// order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChainedSequenceRule {
    /// `backtrackSequence` — preceding glyphs/classes, reverse order.
    pub backtrack: Vec<u16>,
    /// `inputSequence` — input positions `1..inputGlyphCount`.
    pub input: Vec<u16>,
    /// `lookaheadSequence` — following glyphs/classes, logical order.
    pub lookahead: Vec<u16>,
    /// The nested-lookup actions to apply on a match.
    pub lookups: Vec<SequenceLookupRecord>,
}

impl ChainedSequenceRule {
    /// Parse a ChainedSequenceRule / ChainedClassSequenceRule at `off`.
    ///
    /// Layout: `uint16 backtrackGlyphCount`, `backtrackSequence[..]`,
    /// `uint16 inputGlyphCount`, `inputSequence[inputGlyphCount-1]`,
    /// `uint16 lookaheadGlyphCount`, `lookaheadSequence[..]`,
    /// `uint16 seqLookupCount`, then the SequenceLookup records.
    fn parse(bytes: &[u8], off: usize) -> Result<Self, Error> {
        let mut p = off;
        let bt_count = read_u16(bytes, p)?;
        p += 2;
        let mut backtrack = Vec::with_capacity(bt_count as usize);
        for _ in 0..bt_count {
            backtrack.push(read_u16(bytes, p)?);
            p += 2;
        }
        let in_count = read_u16(bytes, p)?;
        p += 2;
        if in_count == 0 {
            return Err(Error::BadStructure(
                "context: ChainedSequenceRule inputGlyphCount 0",
            ));
        }
        let in_len = (in_count - 1) as usize;
        let mut input = Vec::with_capacity(in_len);
        for _ in 0..in_len {
            input.push(read_u16(bytes, p)?);
            p += 2;
        }
        let la_count = read_u16(bytes, p)?;
        p += 2;
        let mut lookahead = Vec::with_capacity(la_count as usize);
        for _ in 0..la_count {
            lookahead.push(read_u16(bytes, p)?);
            p += 2;
        }
        let lookup_count = read_u16(bytes, p)?;
        p += 2;
        let lookups = read_seq_lookups(bytes, p, lookup_count)?;
        Ok(ChainedSequenceRule {
            backtrack,
            input,
            lookahead,
            lookups,
        })
    }
}

/// Read the offset array `offsets[count]` of `Offset16`s starting at
/// `arr_off`, resolving each (NULL → `None`) to a base-relative position.
fn read_offsets(
    bytes: &[u8],
    base: usize,
    arr_off: usize,
    count: u16,
) -> Result<Vec<Option<usize>>, Error> {
    let mut out = Vec::with_capacity(count as usize);
    for i in 0..count as usize {
        let raw = read_u16(bytes, arr_off + i * 2)? as usize;
        if raw == 0 {
            out.push(None);
        } else {
            let abs = base
                .checked_add(raw)
                .ok_or(Error::BadStructure("context: offset overflow"))?;
            if abs >= bytes.len() {
                return Err(Error::BadStructure("context: offset out of range"));
            }
            out.push(Some(abs));
        }
    }
    Ok(out)
}

/// A decoded Sequence Context subtable (GSUB type 5 / GPOS type 7).
///
/// Spec §"Sequence Contexts". The variant indicates the on-disk format.
#[derive(Debug, Clone)]
pub enum SequenceContext<'a> {
    /// Format 1 — glyph-ID based. Each `seqRuleSet` is the rules for one
    /// covered initial glyph.
    Format1 {
        /// Coverage of input-position-0 glyphs.
        coverage: Coverage<'a>,
        /// Per-covered-glyph rule sets (NULL set → empty `Vec`).
        rule_sets: Vec<Vec<SequenceRule>>,
    },
    /// Format 2 — class based. The rule sets are indexed by the input
    /// glyph's class value (from `class_def`).
    Format2 {
        /// Coverage of input-position-0 glyphs.
        coverage: Coverage<'a>,
        /// Class definition mapping glyphs → class values.
        class_def: ClassDef<'a>,
        /// Per-class rule sets (NULL set → empty `Vec`).
        rule_sets: Vec<Vec<SequenceRule>>,
    },
    /// Format 3 — one pattern, each input position a Coverage set.
    Format3 {
        /// One Coverage table per input position, in order.
        coverages: Vec<Coverage<'a>>,
        /// Nested-lookup actions for the single pattern.
        lookups: Vec<SequenceLookupRecord>,
    },
}

impl<'a> SequenceContext<'a> {
    /// Parse a Sequence Context subtable from its raw `bytes`.
    ///
    /// Dispatches on the leading `format` field (1, 2, or 3); an unknown
    /// value is rejected as `BadStructure`.
    pub fn parse(bytes: &'a [u8]) -> Result<Self, Error> {
        let format = read_u16(bytes, 0)?;
        match format {
            1 => {
                let cov_off = read_u16(bytes, 2)? as usize;
                let count = read_u16(bytes, 4)?;
                let coverage = Coverage::parse(&bytes[checked(cov_off, bytes)?..])?;
                let set_offs = read_offsets(bytes, 0, 6, count)?;
                let mut rule_sets = Vec::with_capacity(set_offs.len());
                for set in set_offs {
                    rule_sets.push(parse_rule_set(bytes, set)?);
                }
                Ok(SequenceContext::Format1 {
                    coverage,
                    rule_sets,
                })
            }
            2 => {
                let cov_off = read_u16(bytes, 2)? as usize;
                let class_off = read_u16(bytes, 4)? as usize;
                let count = read_u16(bytes, 6)?;
                let coverage = Coverage::parse(&bytes[checked(cov_off, bytes)?..])?;
                let class_def = ClassDef::parse(&bytes[checked(class_off, bytes)?..])?;
                let set_offs = read_offsets(bytes, 0, 8, count)?;
                let mut rule_sets = Vec::with_capacity(set_offs.len());
                for set in set_offs {
                    rule_sets.push(parse_rule_set(bytes, set)?);
                }
                Ok(SequenceContext::Format2 {
                    coverage,
                    class_def,
                    rule_sets,
                })
            }
            3 => {
                let glyph_count = read_u16(bytes, 2)?;
                let lookup_count = read_u16(bytes, 4)?;
                let cov_arr = 6;
                let mut coverages = Vec::with_capacity(glyph_count as usize);
                for i in 0..glyph_count as usize {
                    let off = read_u16(bytes, cov_arr + i * 2)? as usize;
                    coverages.push(Coverage::parse(&bytes[checked(off, bytes)?..])?);
                }
                let lookups_off = cov_arr + glyph_count as usize * 2;
                let lookups = read_seq_lookups(bytes, lookups_off, lookup_count)?;
                Ok(SequenceContext::Format3 { coverages, lookups })
            }
            _ => Err(Error::BadStructure(
                "context: unknown SequenceContext format",
            )),
        }
    }

    /// The subtable's on-disk format discriminant (1, 2, or 3).
    pub fn format(&self) -> u16 {
        match self {
            SequenceContext::Format1 { .. } => 1,
            SequenceContext::Format2 { .. } => 2,
            SequenceContext::Format3 { .. } => 3,
        }
    }
}

/// A decoded Chained Sequence Context subtable (GSUB type 6 / GPOS
/// type 8).
///
/// Mirrors [`SequenceContext`] but every rule additionally carries
/// backtrack and lookahead sequences; Format 2 carries three separate
/// [`ClassDef`]s (backtrack / input / lookahead) and Format 3 three
/// Coverage arrays.
#[derive(Debug, Clone)]
pub enum ChainedSequenceContext<'a> {
    /// Format 1 — glyph-ID based, per-covered-glyph rule sets.
    Format1 {
        /// Coverage of input-position-0 glyphs.
        coverage: Coverage<'a>,
        /// Per-covered-glyph chained rule sets.
        rule_sets: Vec<Vec<ChainedSequenceRule>>,
    },
    /// Format 2 — class based, with separate backtrack / input /
    /// lookahead class definitions.
    Format2 {
        /// Coverage of input-position-0 glyphs.
        coverage: Coverage<'a>,
        /// Class definition for backtrack positions.
        backtrack_class_def: ClassDef<'a>,
        /// Class definition for input positions.
        input_class_def: ClassDef<'a>,
        /// Class definition for lookahead positions.
        lookahead_class_def: ClassDef<'a>,
        /// Per-input-class chained rule sets.
        rule_sets: Vec<Vec<ChainedSequenceRule>>,
    },
    /// Format 3 — one pattern, each position (backtrack / input /
    /// lookahead) a Coverage set.
    Format3 {
        /// Backtrack Coverage tables (reverse logical order on disk).
        backtrack: Vec<Coverage<'a>>,
        /// Input Coverage tables, in logical order.
        input: Vec<Coverage<'a>>,
        /// Lookahead Coverage tables, in logical order.
        lookahead: Vec<Coverage<'a>>,
        /// Nested-lookup actions for the single pattern.
        lookups: Vec<SequenceLookupRecord>,
    },
}

impl<'a> ChainedSequenceContext<'a> {
    /// Parse a Chained Sequence Context subtable from its raw `bytes`.
    pub fn parse(bytes: &'a [u8]) -> Result<Self, Error> {
        let format = read_u16(bytes, 0)?;
        match format {
            1 => {
                let cov_off = read_u16(bytes, 2)? as usize;
                let count = read_u16(bytes, 4)?;
                let coverage = Coverage::parse(&bytes[checked(cov_off, bytes)?..])?;
                let set_offs = read_offsets(bytes, 0, 6, count)?;
                let mut rule_sets = Vec::with_capacity(set_offs.len());
                for set in set_offs {
                    rule_sets.push(parse_chained_rule_set(bytes, set)?);
                }
                Ok(ChainedSequenceContext::Format1 {
                    coverage,
                    rule_sets,
                })
            }
            2 => {
                let cov_off = read_u16(bytes, 2)? as usize;
                let bt_class_off = read_u16(bytes, 4)? as usize;
                let in_class_off = read_u16(bytes, 6)? as usize;
                let la_class_off = read_u16(bytes, 8)? as usize;
                let count = read_u16(bytes, 10)?;
                let coverage = Coverage::parse(&bytes[checked(cov_off, bytes)?..])?;
                let backtrack_class_def = ClassDef::parse(&bytes[checked(bt_class_off, bytes)?..])?;
                let input_class_def = ClassDef::parse(&bytes[checked(in_class_off, bytes)?..])?;
                let lookahead_class_def = ClassDef::parse(&bytes[checked(la_class_off, bytes)?..])?;
                let set_offs = read_offsets(bytes, 0, 12, count)?;
                let mut rule_sets = Vec::with_capacity(set_offs.len());
                for set in set_offs {
                    rule_sets.push(parse_chained_rule_set(bytes, set)?);
                }
                Ok(ChainedSequenceContext::Format2 {
                    coverage,
                    backtrack_class_def,
                    input_class_def,
                    lookahead_class_def,
                    rule_sets,
                })
            }
            3 => {
                let mut p = 2;
                let bt_count = read_u16(bytes, p)?;
                p += 2;
                let backtrack = parse_coverage_array(bytes, p, bt_count)?;
                p += bt_count as usize * 2;
                let in_count = read_u16(bytes, p)?;
                p += 2;
                let input = parse_coverage_array(bytes, p, in_count)?;
                p += in_count as usize * 2;
                let la_count = read_u16(bytes, p)?;
                p += 2;
                let lookahead = parse_coverage_array(bytes, p, la_count)?;
                p += la_count as usize * 2;
                let lookup_count = read_u16(bytes, p)?;
                p += 2;
                let lookups = read_seq_lookups(bytes, p, lookup_count)?;
                Ok(ChainedSequenceContext::Format3 {
                    backtrack,
                    input,
                    lookahead,
                    lookups,
                })
            }
            _ => Err(Error::BadStructure(
                "context: unknown ChainedSequenceContext format",
            )),
        }
    }

    /// The subtable's on-disk format discriminant (1, 2, or 3).
    pub fn format(&self) -> u16 {
        match self {
            ChainedSequenceContext::Format1 { .. } => 1,
            ChainedSequenceContext::Format2 { .. } => 2,
            ChainedSequenceContext::Format3 { .. } => 3,
        }
    }
}

/// Bounds-check a from-table-start offset, returning it if it lands
/// inside `bytes` (and is non-zero), else `BadStructure`.
fn checked(off: usize, bytes: &[u8]) -> Result<usize, Error> {
    if off == 0 || off >= bytes.len() {
        return Err(Error::BadStructure("context: required offset out of range"));
    }
    Ok(off)
}

/// Parse a SequenceRuleSet / ClassSequenceRuleSet at `set` (NULL → empty).
fn parse_rule_set(bytes: &[u8], set: Option<usize>) -> Result<Vec<SequenceRule>, Error> {
    let Some(set) = set else {
        return Ok(Vec::new());
    };
    let count = read_u16(bytes, set)?;
    let rule_offs = read_offsets(bytes, set, set + 2, count)?;
    let mut rules = Vec::with_capacity(count as usize);
    for off in rule_offs {
        // Rule offsets within a rule set are required to be non-NULL.
        let off = off.ok_or(Error::BadStructure("context: NULL rule offset"))?;
        rules.push(SequenceRule::parse(bytes, off)?);
    }
    Ok(rules)
}

/// Parse a ChainedSequenceRuleSet at `set` (NULL → empty).
fn parse_chained_rule_set(
    bytes: &[u8],
    set: Option<usize>,
) -> Result<Vec<ChainedSequenceRule>, Error> {
    let Some(set) = set else {
        return Ok(Vec::new());
    };
    let count = read_u16(bytes, set)?;
    let rule_offs = read_offsets(bytes, set, set + 2, count)?;
    let mut rules = Vec::with_capacity(count as usize);
    for off in rule_offs {
        let off = off.ok_or(Error::BadStructure("context: NULL chained rule offset"))?;
        rules.push(ChainedSequenceRule::parse(bytes, off)?);
    }
    Ok(rules)
}

/// Parse `count` `Offset16`s at `arr_off` (from table start) into
/// Coverage tables. All offsets must be non-NULL (a Format-3 pattern
/// position always has a Coverage table).
fn parse_coverage_array<'a>(
    bytes: &'a [u8],
    arr_off: usize,
    count: u16,
) -> Result<Vec<Coverage<'a>>, Error> {
    let mut out = Vec::with_capacity(count as usize);
    for i in 0..count as usize {
        let off = read_u16(bytes, arr_off + i * 2)? as usize;
        out.push(Coverage::parse(&bytes[checked(off, bytes)?..])?);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn be(u: u16) -> [u8; 2] {
        u.to_be_bytes()
    }

    /// Coverage format-1 table for the listed glyphs.
    fn coverage(glyphs: &[u16]) -> Vec<u8> {
        let mut b = Vec::new();
        b.extend_from_slice(&be(1));
        b.extend_from_slice(&be(glyphs.len() as u16));
        for &g in glyphs {
            b.extend_from_slice(&be(g));
        }
        b
    }

    /// ClassDef format-2 table from (startGlyph, endGlyph, class) ranges.
    fn classdef(ranges: &[(u16, u16, u16)]) -> Vec<u8> {
        let mut b = Vec::new();
        b.extend_from_slice(&be(2));
        b.extend_from_slice(&be(ranges.len() as u16));
        for &(s, e, c) in ranges {
            b.extend_from_slice(&be(s));
            b.extend_from_slice(&be(e));
            b.extend_from_slice(&be(c));
        }
        b
    }

    // -- SequenceContext Format 1 ----------------------------------------

    /// Format-1 subtable: Coverage {10}; one rule set with one rule whose
    /// input sequence is [20, 30] (so the full pattern is 10,20,30) and
    /// one SequenceLookup record (seqIndex=1, lookupIndex=7).
    fn seqctx_f1() -> Vec<u8> {
        let mut b = Vec::new();
        b.extend_from_slice(&be(1)); // format
        b.extend_from_slice(&be(0)); // coverageOffset (patch)
        b.extend_from_slice(&be(1)); // seqRuleSetCount
        b.extend_from_slice(&be(0)); // seqRuleSetOffset[0] (patch)

        let cov_off = b.len();
        b.extend_from_slice(&coverage(&[10]));

        let set_off = b.len();
        b.extend_from_slice(&be(1)); // seqRuleCount
        b.extend_from_slice(&be(0)); // seqRuleOffset[0] (patch, rel to set)

        let rule_off = b.len();
        b.extend_from_slice(&be(3)); // glyphCount (10,20,30)
        b.extend_from_slice(&be(1)); // seqLookupCount
        b.extend_from_slice(&be(20)); // inputSequence[0]
        b.extend_from_slice(&be(30)); // inputSequence[1]
        b.extend_from_slice(&be(1)); // seqLookup.sequenceIndex
        b.extend_from_slice(&be(7)); // seqLookup.lookupListIndex

        b[2..4].copy_from_slice(&be(cov_off as u16));
        b[6..8].copy_from_slice(&be(set_off as u16));
        let rule_rel = (rule_off - set_off) as u16;
        b[set_off + 2..set_off + 4].copy_from_slice(&be(rule_rel));
        b
    }

    #[test]
    fn seqctx_format1_decodes_rule_and_lookup() {
        let sub = seqctx_f1();
        let ctx = SequenceContext::parse(&sub).unwrap();
        assert_eq!(ctx.format(), 1);
        let SequenceContext::Format1 {
            coverage,
            rule_sets,
        } = &ctx
        else {
            panic!("expected Format1");
        };
        assert!(coverage.contains(10));
        assert_eq!(rule_sets.len(), 1);
        assert_eq!(rule_sets[0].len(), 1);
        let rule = &rule_sets[0][0];
        assert_eq!(rule.input, vec![20, 30]);
        assert_eq!(rule.lookups.len(), 1);
        assert_eq!(rule.lookups[0].sequence_index, 1);
        assert_eq!(rule.lookups[0].lookup_list_index, 7);
    }

    #[test]
    fn seqctx_format1_rejects_bad_format() {
        let mut sub = seqctx_f1();
        sub[0..2].copy_from_slice(&be(9));
        assert!(matches!(
            SequenceContext::parse(&sub),
            Err(Error::BadStructure(_))
        ));
    }

    // -- SequenceContext Format 2 ----------------------------------------

    #[test]
    fn seqctx_format2_decodes_class_rules() {
        // Coverage {10,11}; ClassDef: 10→1, 11→2; rule sets per class.
        // classSeqRuleSetCount = 3 (classes 0,1,2). Class 1 set has one
        // rule input=[class 2] → pattern is (class1, class2); lookup
        // (0, 4).
        let mut b = Vec::new();
        b.extend_from_slice(&be(2)); // format
        b.extend_from_slice(&be(0)); // coverageOffset (patch)
        b.extend_from_slice(&be(0)); // classDefOffset (patch)
        b.extend_from_slice(&be(3)); // classSeqRuleSetCount
        b.extend_from_slice(&be(0)); // offset[0] (class 0) = NULL
        b.extend_from_slice(&be(0)); // offset[1] (class 1) (patch)
        b.extend_from_slice(&be(0)); // offset[2] (class 2) = NULL

        let cov_off = b.len();
        b.extend_from_slice(&coverage(&[10, 11]));
        let class_off = b.len();
        b.extend_from_slice(&classdef(&[(10, 10, 1), (11, 11, 2)]));

        let set_off = b.len();
        b.extend_from_slice(&be(1)); // classSeqRuleCount
        b.extend_from_slice(&be(0)); // ruleOffset (patch, rel to set)
        let rule_off = b.len();
        b.extend_from_slice(&be(2)); // glyphCount (class1, class2)
        b.extend_from_slice(&be(1)); // seqLookupCount
        b.extend_from_slice(&be(2)); // inputSequence[0] = class 2
        b.extend_from_slice(&be(0)); // seqLookup.sequenceIndex
        b.extend_from_slice(&be(4)); // seqLookup.lookupListIndex

        b[2..4].copy_from_slice(&be(cov_off as u16));
        b[4..6].copy_from_slice(&be(class_off as u16));
        // offsets[]: offset[0]@8, offset[1]@10, offset[2]@12. Class 1 set.
        b[10..12].copy_from_slice(&be(set_off as u16));
        b[set_off + 2..set_off + 4].copy_from_slice(&be((rule_off - set_off) as u16));

        let ctx = SequenceContext::parse(&b).unwrap();
        let SequenceContext::Format2 {
            coverage,
            class_def,
            rule_sets,
        } = &ctx
        else {
            panic!("expected Format2");
        };
        assert!(coverage.contains(10));
        assert_eq!(class_def.class_of(10), 1);
        assert_eq!(class_def.class_of(11), 2);
        assert_eq!(rule_sets.len(), 3);
        assert!(rule_sets[0].is_empty()); // NULL class-0 set
        assert_eq!(rule_sets[1].len(), 1);
        assert_eq!(rule_sets[1][0].input, vec![2]); // class value
        assert_eq!(rule_sets[1][0].lookups[0].lookup_list_index, 4);
        assert!(rule_sets[2].is_empty()); // NULL class-2 set
    }

    // -- SequenceContext Format 3 ----------------------------------------

    #[test]
    fn seqctx_format3_decodes_coverage_pattern() {
        // Pattern: pos0 cov{10,11}, pos1 cov{20}; lookups (1, 3).
        let mut b = Vec::new();
        b.extend_from_slice(&be(3)); // format
        b.extend_from_slice(&be(2)); // glyphCount
        b.extend_from_slice(&be(1)); // seqLookupCount
        b.extend_from_slice(&be(0)); // coverageOffset[0] (patch)
        b.extend_from_slice(&be(0)); // coverageOffset[1] (patch)
        b.extend_from_slice(&be(1)); // seqLookup.sequenceIndex
        b.extend_from_slice(&be(3)); // seqLookup.lookupListIndex

        let c0 = b.len();
        b.extend_from_slice(&coverage(&[10, 11]));
        let c1 = b.len();
        b.extend_from_slice(&coverage(&[20]));
        b[6..8].copy_from_slice(&be(c0 as u16));
        b[8..10].copy_from_slice(&be(c1 as u16));

        let ctx = SequenceContext::parse(&b).unwrap();
        let SequenceContext::Format3 { coverages, lookups } = &ctx else {
            panic!("expected Format3");
        };
        assert_eq!(coverages.len(), 2);
        assert!(coverages[0].contains(11));
        assert!(coverages[1].contains(20));
        assert!(!coverages[1].contains(10));
        assert_eq!(lookups[0].sequence_index, 1);
        assert_eq!(lookups[0].lookup_list_index, 3);
    }

    // -- ChainedSequenceContext Format 1 ---------------------------------

    /// Chained Format-1: Coverage {10}; one rule with backtrack=[5],
    /// input=[20] (pattern 10,20), lookahead=[40,41]; lookup (0, 9).
    fn chainctx_f1() -> Vec<u8> {
        let mut b = Vec::new();
        b.extend_from_slice(&be(1)); // format
        b.extend_from_slice(&be(0)); // coverageOffset (patch)
        b.extend_from_slice(&be(1)); // chainedSeqRuleSetCount
        b.extend_from_slice(&be(0)); // offset[0] (patch)

        let cov_off = b.len();
        b.extend_from_slice(&coverage(&[10]));

        let set_off = b.len();
        b.extend_from_slice(&be(1)); // ruleCount
        b.extend_from_slice(&be(0)); // ruleOffset (patch, rel to set)

        let rule_off = b.len();
        b.extend_from_slice(&be(1)); // backtrackGlyphCount
        b.extend_from_slice(&be(5)); // backtrack[0]
        b.extend_from_slice(&be(2)); // inputGlyphCount (10,20)
        b.extend_from_slice(&be(20)); // inputSequence[0]
        b.extend_from_slice(&be(2)); // lookaheadGlyphCount
        b.extend_from_slice(&be(40)); // lookahead[0]
        b.extend_from_slice(&be(41)); // lookahead[1]
        b.extend_from_slice(&be(1)); // seqLookupCount
        b.extend_from_slice(&be(0)); // seqLookup.sequenceIndex
        b.extend_from_slice(&be(9)); // seqLookup.lookupListIndex

        b[2..4].copy_from_slice(&be(cov_off as u16));
        b[4 + 2..4 + 4].copy_from_slice(&be(set_off as u16));
        b[set_off + 2..set_off + 4].copy_from_slice(&be((rule_off - set_off) as u16));
        b
    }

    #[test]
    fn chainctx_format1_decodes_backtrack_input_lookahead() {
        let sub = chainctx_f1();
        let ctx = ChainedSequenceContext::parse(&sub).unwrap();
        assert_eq!(ctx.format(), 1);
        let ChainedSequenceContext::Format1 {
            coverage,
            rule_sets,
        } = &ctx
        else {
            panic!("expected Format1");
        };
        assert!(coverage.contains(10));
        let rule = &rule_sets[0][0];
        assert_eq!(rule.backtrack, vec![5]);
        assert_eq!(rule.input, vec![20]);
        assert_eq!(rule.lookahead, vec![40, 41]);
        assert_eq!(rule.lookups[0].lookup_list_index, 9);
    }

    // -- ChainedSequenceContext Format 2 ---------------------------------

    #[test]
    fn chainctx_format2_decodes_three_classdefs() {
        // Coverage {10}; three ClassDefs (bt/in/la); one input-class-1
        // rule set with a rule bt=[class2], in=[], la=[class3]; lookup
        // (0, 6). The single-glyph input means inputGlyphCount=1 → no
        // stored input entries.
        let mut b = Vec::new();
        b.extend_from_slice(&be(2)); // format
        b.extend_from_slice(&be(0)); // coverageOffset (patch @2)
        b.extend_from_slice(&be(0)); // backtrackClassDefOffset (patch @4)
        b.extend_from_slice(&be(0)); // inputClassDefOffset (patch @6)
        b.extend_from_slice(&be(0)); // lookaheadClassDefOffset (patch @8)
        b.extend_from_slice(&be(2)); // chainedClassSeqRuleSetCount (classes 0,1)
        b.extend_from_slice(&be(0)); // offset[0] = NULL
        b.extend_from_slice(&be(0)); // offset[1] (patch @12)

        let cov_off = b.len();
        b.extend_from_slice(&coverage(&[10]));
        let bt_off = b.len();
        b.extend_from_slice(&classdef(&[(5, 5, 2)]));
        let in_off = b.len();
        b.extend_from_slice(&classdef(&[(10, 10, 1)]));
        let la_off = b.len();
        b.extend_from_slice(&classdef(&[(40, 40, 3)]));

        let set_off = b.len();
        b.extend_from_slice(&be(1)); // ruleCount
        b.extend_from_slice(&be(0)); // ruleOffset (patch, rel to set)
        let rule_off = b.len();
        b.extend_from_slice(&be(1)); // backtrackGlyphCount
        b.extend_from_slice(&be(2)); // backtrack[0] = class 2
        b.extend_from_slice(&be(1)); // inputGlyphCount (just the covered glyph)
        b.extend_from_slice(&be(1)); // lookaheadGlyphCount
        b.extend_from_slice(&be(3)); // lookahead[0] = class 3
        b.extend_from_slice(&be(1)); // seqLookupCount
        b.extend_from_slice(&be(0)); // seqLookup.sequenceIndex
        b.extend_from_slice(&be(6)); // seqLookup.lookupListIndex

        b[2..4].copy_from_slice(&be(cov_off as u16));
        b[4..6].copy_from_slice(&be(bt_off as u16));
        b[6..8].copy_from_slice(&be(in_off as u16));
        b[8..10].copy_from_slice(&be(la_off as u16));
        // offsets[]: offset[0]@12 (class 0, NULL), offset[1]@14 (class 1).
        b[14..16].copy_from_slice(&be(set_off as u16));
        b[set_off + 2..set_off + 4].copy_from_slice(&be((rule_off - set_off) as u16));

        let ctx = ChainedSequenceContext::parse(&b).unwrap();
        let ChainedSequenceContext::Format2 {
            input_class_def,
            backtrack_class_def,
            lookahead_class_def,
            rule_sets,
            ..
        } = &ctx
        else {
            panic!("expected Format2");
        };
        assert_eq!(input_class_def.class_of(10), 1);
        assert_eq!(backtrack_class_def.class_of(5), 2);
        assert_eq!(lookahead_class_def.class_of(40), 3);
        assert_eq!(rule_sets.len(), 2);
        assert!(rule_sets[0].is_empty());
        let rule = &rule_sets[1][0];
        assert_eq!(rule.backtrack, vec![2]);
        assert_eq!(rule.input, Vec::<u16>::new());
        assert_eq!(rule.lookahead, vec![3]);
        assert_eq!(rule.lookups[0].lookup_list_index, 6);
    }

    // -- ChainedSequenceContext Format 3 ---------------------------------

    #[test]
    fn chainctx_format3_decodes_three_coverage_arrays() {
        // backtrack cov{5}; input cov{10}; lookahead cov{40,41}; lookup
        // (0, 8).
        let mut b = Vec::new();
        b.extend_from_slice(&be(3)); // format
        b.extend_from_slice(&be(1)); // backtrackGlyphCount
        b.extend_from_slice(&be(0)); // backtrackCoverageOffset[0] (patch)
        b.extend_from_slice(&be(1)); // inputGlyphCount
        b.extend_from_slice(&be(0)); // inputCoverageOffset[0] (patch)
        b.extend_from_slice(&be(2)); // lookaheadGlyphCount
        b.extend_from_slice(&be(0)); // lookaheadCoverageOffset[0] (patch)
        b.extend_from_slice(&be(0)); // lookaheadCoverageOffset[1] (patch)
        b.extend_from_slice(&be(1)); // seqLookupCount
        b.extend_from_slice(&be(0)); // seqLookup.sequenceIndex
        b.extend_from_slice(&be(8)); // seqLookup.lookupListIndex

        let bt = b.len();
        b.extend_from_slice(&coverage(&[5]));
        let inp = b.len();
        b.extend_from_slice(&coverage(&[10]));
        let la0 = b.len();
        b.extend_from_slice(&coverage(&[40]));
        let la1 = b.len();
        b.extend_from_slice(&coverage(&[41]));

        // Patch offsets: backtrack@4, input@8, lookahead@12,14.
        b[4..6].copy_from_slice(&be(bt as u16));
        b[8..10].copy_from_slice(&be(inp as u16));
        b[12..14].copy_from_slice(&be(la0 as u16));
        b[14..16].copy_from_slice(&be(la1 as u16));

        let ctx = ChainedSequenceContext::parse(&b).unwrap();
        let ChainedSequenceContext::Format3 {
            backtrack,
            input,
            lookahead,
            lookups,
        } = &ctx
        else {
            panic!("expected Format3");
        };
        assert_eq!(backtrack.len(), 1);
        assert!(backtrack[0].contains(5));
        assert!(input[0].contains(10));
        assert_eq!(lookahead.len(), 2);
        assert!(lookahead[0].contains(40));
        assert!(lookahead[1].contains(41));
        assert_eq!(lookups[0].lookup_list_index, 8);
    }

    #[test]
    fn chainctx_rejects_bad_format() {
        let mut sub = chainctx_f1();
        sub[0..2].copy_from_slice(&be(7));
        assert!(matches!(
            ChainedSequenceContext::parse(&sub),
            Err(Error::BadStructure(_))
        ));
    }
}
