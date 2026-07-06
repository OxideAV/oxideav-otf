//! Glyph buffer and lookup-flag skip filtering for the shaping engine.
//!
//! Spec: `docs/text/opentype/otspec-chapter2-common-layout-tables.html`
//! §"Lookup table" (LookupFlag bit enumeration) — the source of the
//! skip rules implemented by [`SkipFilter`]:
//!
//! * `IGNORE_BASE_GLYPHS` / `IGNORE_LIGATURES` / `IGNORE_MARKS` refer
//!   to the glyph classes of the GDEF `GlyphClassDef`; if set, "lookups
//!   must ignore glyphs of the respective type; that is, the other
//!   glyphs must be processed just as though these glyphs were not
//!   present in the glyph sequence".
//! * `MARK_ATTACHMENT_CLASS_FILTER` (high byte, non-zero): "a lookup
//!   must ignore any mark glyphs that are not in the specified mark
//!   attachment class" (GDEF `MarkAttachClassDef`).
//! * `USE_MARK_FILTERING_SET`: "the lookup must ignore any mark glyphs
//!   that are not in the specified mark glyph set". Per spec, a mark
//!   filtering set *supersedes* any mark attachment class indication,
//!   and `IGNORE_MARKS` supersedes both.

use crate::tables::gdef::{GdefTable, GlyphClass};
use crate::tables::layout::LookupFlag;

/// One glyph slot in the shaping buffer.
///
/// `cluster` is the index (in Unicode scalar values) of the character
/// that produced this glyph; glyphs merged by a ligature substitution
/// take the smallest cluster of their components, and glyphs produced
/// by a multiple substitution all inherit the input glyph's cluster.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct GlyphInfo {
    /// Current glyph ID.
    pub glyph: u16,
    /// Character index this glyph maps back to.
    pub cluster: u32,
    /// For a mark glyph that was skipped *inside* a matched ligature
    /// pattern: the zero-based index of the ligature component that
    /// preceded it. [`LIG_COMPONENT_NONE`] when not assigned. GPOS
    /// mark-to-ligature attachment (lookup type 5) consumes this to
    /// select the component anchor; an unassigned mark following a
    /// ligature associates with the ligature's last component (the
    /// spec leaves the association to "the original character string
    /// and subsequent character- or glyph-sequence processing").
    pub lig_component: u16,
    /// For a glyph produced by a ligature substitution: the number of
    /// components that formed it (`Ligature.componentCount`). `0` for
    /// ordinary glyphs.
    pub lig_num_comps: u16,
}

/// Sentinel for [`GlyphInfo::lig_component`]: not associated with any
/// ligature component.
pub(crate) const LIG_COMPONENT_NONE: u16 = 0xFFFF;

impl GlyphInfo {
    /// A plain glyph slot with no ligature bookkeeping.
    pub(crate) fn new(glyph: u16, cluster: u32) -> Self {
        GlyphInfo {
            glyph,
            cluster,
            lig_component: LIG_COMPONENT_NONE,
            lig_num_comps: 0,
        }
    }
}

/// The per-lookup glyph skip filter derived from the lookup's
/// [`LookupFlag`] (and `markFilteringSet`, when present) plus the
/// font's GDEF class definitions.
#[derive(Debug, Clone, Copy)]
pub(crate) struct SkipFilter<'g, 'a> {
    flag: LookupFlag,
    /// `Some(set)` iff `USE_MARK_FILTERING_SET` is set on the lookup.
    mark_filtering_set: Option<u16>,
    gdef: Option<&'g GdefTable<'a>>,
}

impl<'g, 'a> SkipFilter<'g, 'a> {
    pub(crate) fn new(
        flag: LookupFlag,
        mark_filtering_set: Option<u16>,
        gdef: Option<&'g GdefTable<'a>>,
    ) -> Self {
        SkipFilter {
            flag,
            mark_filtering_set,
            gdef,
        }
    }

    /// `true` when the lookup must process the glyph sequence "as
    /// though this glyph were not present".
    ///
    /// Without a GDEF `GlyphClassDef` the class-driven ignore bits
    /// cannot classify anything, so nothing is skipped (the spec makes
    /// a GlyphClassDef a requirement *for fonts* that set those bits).
    pub(crate) fn skips(&self, glyph: u16) -> bool {
        let Some(gdef) = self.gdef else {
            return false;
        };
        let class = gdef.glyph_class(glyph);
        match class {
            Some(GlyphClass::Base) if self.flag.ignore_base_glyphs() => return true,
            Some(GlyphClass::Ligature) if self.flag.ignore_ligatures() => return true,
            Some(GlyphClass::Mark) => {
                if self.flag.ignore_marks() {
                    return true;
                }
                // Mark filtering set supersedes the attachment-class
                // filter (spec §"LookupFlag bit enumeration").
                if let Some(set) = self.mark_filtering_set {
                    let in_set = gdef
                        .mark_glyph_sets()
                        .map(|sets| sets.contains(set as usize, glyph))
                        .unwrap_or(false);
                    return !in_set;
                }
                let attach_filter = self.flag.mark_attachment_type();
                if attach_filter != 0 {
                    return gdef.mark_attach_class(glyph) != attach_filter as u16;
                }
            }
            _ => {}
        }
        false
    }
}

/// Skip-aware cursor helpers over a `&[GlyphInfo]` buffer.
///
/// "Non-skipped" positions are the glyphs the active lookup sees; all
/// matching of input / backtrack / lookahead sequences walks these.
pub(crate) fn next_unskipped(
    buffer: &[GlyphInfo],
    filter: &SkipFilter<'_, '_>,
    mut pos: usize,
) -> Option<usize> {
    pos += 1;
    while pos < buffer.len() {
        if !filter.skips(buffer[pos].glyph) {
            return Some(pos);
        }
        pos += 1;
    }
    None
}

/// The nearest non-skipped position strictly before `pos`.
pub(crate) fn prev_unskipped(
    buffer: &[GlyphInfo],
    filter: &SkipFilter<'_, '_>,
    pos: usize,
) -> Option<usize> {
    let mut p = pos;
    while p > 0 {
        p -= 1;
        if !filter.skips(buffer[p].glyph) {
            return Some(p);
        }
    }
    None
}

/// Collect the buffer positions of a match of `count` glyphs starting
/// at the (non-skipped) position `start`, where the glyph at relative
/// input index `k` (1-based; `0` is `start` itself) must satisfy
/// `accept(k, glyph)`. Returns the matched positions (including
/// `start`) or `None`.
pub(crate) fn match_input<F>(
    buffer: &[GlyphInfo],
    filter: &SkipFilter<'_, '_>,
    start: usize,
    count: usize,
    mut accept: F,
) -> Option<Vec<usize>>
where
    F: FnMut(usize, u16) -> bool,
{
    let mut positions = Vec::with_capacity(count);
    positions.push(start);
    let mut pos = start;
    for k in 1..count {
        pos = next_unskipped(buffer, filter, pos)?;
        if !accept(k, buffer[pos].glyph) {
            return None;
        }
        positions.push(pos);
    }
    Some(positions)
}

/// Match a backtrack sequence before position `pos`. `seq_len` glyphs
/// are tested in reverse logical order (the on-disk backtrack order):
/// `accept(0, g)` sees the glyph immediately preceding `pos`.
pub(crate) fn match_backtrack<F>(
    buffer: &[GlyphInfo],
    filter: &SkipFilter<'_, '_>,
    pos: usize,
    seq_len: usize,
    mut accept: F,
) -> bool
where
    F: FnMut(usize, u16) -> bool,
{
    let mut p = pos;
    for k in 0..seq_len {
        let Some(prev) = prev_unskipped(buffer, filter, p) else {
            return false;
        };
        if !accept(k, buffer[prev].glyph) {
            return false;
        }
        p = prev;
    }
    true
}

/// Match a lookahead sequence after position `last` (the last matched
/// input position). `accept(0, g)` sees the first glyph after `last`.
pub(crate) fn match_lookahead<F>(
    buffer: &[GlyphInfo],
    filter: &SkipFilter<'_, '_>,
    last: usize,
    seq_len: usize,
    mut accept: F,
) -> bool
where
    F: FnMut(usize, u16) -> bool,
{
    let mut p = last;
    for k in 0..seq_len {
        let Some(next) = next_unskipped(buffer, filter, p) else {
            return false;
        };
        if !accept(k, buffer[next].glyph) {
            return false;
        }
        p = next;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn buf(glyphs: &[u16]) -> Vec<GlyphInfo> {
        glyphs
            .iter()
            .enumerate()
            .map(|(i, &g)| GlyphInfo::new(g, i as u32))
            .collect()
    }

    #[test]
    fn no_gdef_skips_nothing() {
        let f = SkipFilter::new(LookupFlag(LookupFlag::IGNORE_MARKS), None, None);
        assert!(!f.skips(42));
    }

    #[test]
    fn match_input_contiguous() {
        let b = buf(&[1, 2, 3, 4]);
        let f = SkipFilter::new(LookupFlag(0), None, None);
        let m = match_input(&b, &f, 1, 3, |k, g| g == [0, 3, 4][k]).unwrap();
        assert_eq!(m, vec![1, 2, 3]);
        assert!(match_input(&b, &f, 1, 3, |_, g| g == 9).is_none());
        // Running off the end of the buffer fails the match.
        assert!(match_input(&b, &f, 3, 2, |_, _| true).is_none());
    }

    #[test]
    fn backtrack_and_lookahead() {
        let b = buf(&[7, 8, 9, 10]);
        let f = SkipFilter::new(LookupFlag(0), None, None);
        // Backtrack from position 2 is [8, 7] in reverse logical order.
        assert!(match_backtrack(&b, &f, 2, 2, |k, g| g == [8, 7][k]));
        assert!(!match_backtrack(&b, &f, 2, 3, |_, _| true));
        // Lookahead after position 2 is [10].
        assert!(match_lookahead(&b, &f, 2, 1, |_, g| g == 10));
        assert!(!match_lookahead(&b, &f, 2, 2, |_, _| true));
    }
}
