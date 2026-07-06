//! GPOS lookup application: the positioning half of the shaper.
//!
//! Spec: `docs/text/opentype/otspec-gpos.html`. Positioning starts
//! from the hmtx advances (§"Basic glyph positioning": placement
//! describes the glyph's position with respect to the current pen
//! point, advance describes where the pen moves next) and each GPOS
//! lookup adjusts placements (`x_offset` / `y_offset`) and advances
//! in design units. Value semantics per the spec: "to lower the
//! dieresis over an 'o' by 10 units, set the YPlacement = -10";
//! positive X advance adjustments widen the pen step.
//!
//! Variable fonts: when the caller supplies axis coordinates, hmtx
//! advances take their HVAR deltas, and `ValueRecord` / Anchor
//! format-3 `VariationIndex` tables resolve against the GDEF
//! `ItemVariationStore` (GPOS chapter §"GPOS table and OpenType Font
//! Variations"). Plain `Device` tables carry per-*ppem* pixel
//! corrections; shaping here stays in design units, so they are not
//! applied.

use super::buffer::{match_backtrack, match_input, match_lookahead, LIG_COMPONENT_NONE};
use super::buffer::{GlyphInfo, SkipFilter};
use super::MAX_NESTING_DEPTH;
use super::{PlannedLookup, ShapeOptions};
use crate::tables::context::{ChainedSequenceContext, SequenceContext, SequenceLookupRecord};
use crate::tables::gdef::GdefTable;
use crate::tables::gdef::GlyphClass;
use crate::tables::gpos::{
    CursivePos, GposTable, MarkBasePos, MarkLigPos, MarkMarkPos, PairPos, SinglePos, ValueRecord,
    GPOS_LOOKUP_TYPE_CHAINED_CONTEXT, GPOS_LOOKUP_TYPE_CONTEXT, GPOS_LOOKUP_TYPE_CURSIVE,
    GPOS_LOOKUP_TYPE_EXTENSION, GPOS_LOOKUP_TYPE_MARK_TO_BASE, GPOS_LOOKUP_TYPE_MARK_TO_LIGATURE,
    GPOS_LOOKUP_TYPE_MARK_TO_MARK, GPOS_LOOKUP_TYPE_PAIR, GPOS_LOOKUP_TYPE_SINGLE,
};
use crate::Font;

/// Mutable position slot, parallel to the glyph buffer.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Pos {
    pub x_advance: i32,
    pub y_advance: i32,
    pub x_offset: i32,
    pub y_offset: i32,
}

/// Initialize positions from `hmtx` advances (plus HVAR deltas when
/// shaping a variable-font instance; `normalized` is the fvar/avar
/// normalized coordinate tuple, empty for the default instance).
pub(crate) fn init_positions(
    font: &Font<'_>,
    buffer: &[GlyphInfo],
    normalized: &[f32],
) -> Vec<Pos> {
    buffer
        .iter()
        .map(|g| {
            let mut adv = font.glyph_advance(g.glyph) as i32;
            if !normalized.is_empty() {
                if let Some(hvar) = font.hvar() {
                    adv += hvar.advance(g.glyph, normalized).round() as i32;
                }
            }
            Pos {
                x_advance: adv,
                ..Pos::default()
            }
        })
        .collect()
}

/// Add a `ValueRecord`'s adjustments to a position slot.
///
/// `subtable` is the raw subtable slice the record's device offsets
/// are relative to; `normalized` enables VariationIndex resolution
/// against the GDEF ItemVariationStore (`ivs`).
pub(crate) fn apply_value_record(
    pos: &mut Pos,
    rec: &ValueRecord,
    subtable: &[u8],
    gdef: Option<&GdefTable<'_>>,
    normalized: &[f32],
) {
    pos.x_offset += rec.x_placement as i32;
    pos.y_offset += rec.y_placement as i32;
    pos.x_advance += rec.x_advance as i32;
    pos.y_advance += rec.y_advance as i32;

    if normalized.is_empty() {
        return;
    }
    let deltas = [
        (rec.x_placement_device(subtable), &mut pos.x_offset),
        (rec.y_placement_device(subtable), &mut pos.y_offset),
        (rec.x_advance_device(subtable), &mut pos.x_advance),
        (rec.y_advance_device(subtable), &mut pos.y_advance),
    ];
    for (dev, target) in deltas {
        if let Some(Ok(dev)) = dev {
            if let Some(vi) = dev.as_variation_index() {
                *target += variation_delta(gdef, vi.outer_index, vi.inner_index, normalized);
            }
        }
    }
}

/// Resolve a `(outer, inner)` delta-set index against the GDEF
/// `ItemVariationStore`, rounding to integer design units.
pub(crate) fn variation_delta(
    gdef: Option<&GdefTable<'_>>,
    outer: u16,
    inner: u16,
    normalized: &[f32],
) -> i32 {
    let Some(gdef) = gdef else { return 0 };
    let Some(Ok(store)) = gdef.item_variation_store() else {
        return 0;
    };
    store.delta(outer, inner, normalized).round() as i32
}

/// Apply GPOS lookup `planned.lookup_index` over the whole run.
pub(crate) fn apply_lookup(
    gpos: &GposTable<'_>,
    gdef: Option<&GdefTable<'_>>,
    planned: &PlannedLookup,
    buffer: &[GlyphInfo],
    positions: &mut [Pos],
    normalized: &[f32],
) {
    let Some(lookup) = gpos.lookup(planned.lookup_index) else {
        return;
    };
    let filter = SkipFilter::new(lookup.flag(), lookup.mark_filtering_set(), gdef);

    let mut pos = 0usize;
    while pos < buffer.len() {
        if filter.skips(buffer[pos].glyph) {
            pos += 1;
            continue;
        }
        match try_apply_at(
            gpos,
            gdef,
            planned.lookup_index,
            &filter,
            buffer,
            positions,
            pos,
            normalized,
            0,
        ) {
            Some(next) => pos = next,
            None => pos += 1,
        }
    }
}

/// Try every subtable of the lookup at position `pos`; the first that
/// matches is applied. Returns the next cursor position on a match.
#[allow(clippy::too_many_arguments)]
pub(crate) fn try_apply_at(
    gpos: &GposTable<'_>,
    gdef: Option<&GdefTable<'_>>,
    lookup_index: u16,
    filter: &SkipFilter<'_, '_>,
    buffer: &[GlyphInfo],
    positions: &mut [Pos],
    pos: usize,
    normalized: &[f32],
    depth: usize,
) -> Option<usize> {
    let lookup = gpos.lookup(lookup_index)?;
    let lookup_type = lookup.lookup_type();
    for s in 0..lookup.subtable_count() {
        let next = match lookup_type {
            GPOS_LOOKUP_TYPE_SINGLE => {
                let sp = gpos.single_pos(lookup_index, s)?.ok()?;
                try_single(&sp, gdef, positions, buffer, pos, normalized)
            }
            GPOS_LOOKUP_TYPE_PAIR => {
                let pp = gpos.pair_pos(lookup_index, s)?.ok()?;
                try_pair(&pp, gdef, filter, buffer, positions, pos, normalized)
            }
            GPOS_LOOKUP_TYPE_CURSIVE => {
                let cp = gpos.cursive_pos(lookup_index, s)?.ok()?;
                let rtl = lookup.flag().right_to_left();
                try_cursive(&cp, rtl, filter, buffer, positions, pos)
            }
            GPOS_LOOKUP_TYPE_MARK_TO_BASE => {
                let mb = gpos.mark_base_pos(lookup_index, s)?.ok()?;
                try_mark_base(&mb, gdef, filter, buffer, positions, pos)
            }
            GPOS_LOOKUP_TYPE_MARK_TO_LIGATURE => {
                let ml = gpos.mark_lig_pos(lookup_index, s)?.ok()?;
                try_mark_lig(&ml, gdef, filter, buffer, positions, pos)
            }
            GPOS_LOOKUP_TYPE_MARK_TO_MARK => {
                let mm = gpos.mark_mark_pos(lookup_index, s)?.ok()?;
                try_mark_mark(&mm, filter, buffer, positions, pos)
            }
            GPOS_LOOKUP_TYPE_CONTEXT => {
                let ctx = gpos.context_pos(lookup_index, s)?.ok()?;
                try_context(
                    gpos, gdef, &ctx, filter, buffer, positions, pos, normalized, depth,
                )
            }
            GPOS_LOOKUP_TYPE_CHAINED_CONTEXT => {
                let ctx = gpos.chained_context_pos(lookup_index, s)?.ok()?;
                try_chained_context(
                    gpos, gdef, &ctx, filter, buffer, positions, pos, normalized, depth,
                )
            }
            GPOS_LOOKUP_TYPE_EXTENSION => {
                let ext = gpos.extension_pos(lookup_index, s)?.ok()?;
                let sub_bytes = ext.extension_subtable_bytes();
                match ext.extension_lookup_type() {
                    GPOS_LOOKUP_TYPE_SINGLE => {
                        let sp = ext.as_single_pos().ok()?;
                        let _ = sub_bytes;
                        try_single(&sp, gdef, positions, buffer, pos, normalized)
                    }
                    GPOS_LOOKUP_TYPE_PAIR => {
                        let pp = ext.as_pair_pos().ok()?;
                        try_pair(&pp, gdef, filter, buffer, positions, pos, normalized)
                    }
                    GPOS_LOOKUP_TYPE_CURSIVE => {
                        let cp = ext.as_cursive_pos().ok()?;
                        let rtl = lookup.flag().right_to_left();
                        try_cursive(&cp, rtl, filter, buffer, positions, pos)
                    }
                    GPOS_LOOKUP_TYPE_MARK_TO_BASE => {
                        let mb = ext.as_mark_base_pos().ok()?;
                        try_mark_base(&mb, gdef, filter, buffer, positions, pos)
                    }
                    GPOS_LOOKUP_TYPE_MARK_TO_LIGATURE => {
                        let ml = ext.as_mark_lig_pos().ok()?;
                        try_mark_lig(&ml, gdef, filter, buffer, positions, pos)
                    }
                    GPOS_LOOKUP_TYPE_MARK_TO_MARK => {
                        let mm = ext.as_mark_mark_pos().ok()?;
                        try_mark_mark(&mm, filter, buffer, positions, pos)
                    }
                    GPOS_LOOKUP_TYPE_CONTEXT => {
                        let ctx = ext.as_context_pos().ok()?;
                        try_context(
                            gpos, gdef, &ctx, filter, buffer, positions, pos, normalized, depth,
                        )
                    }
                    GPOS_LOOKUP_TYPE_CHAINED_CONTEXT => {
                        let ctx = ext.as_chained_context_pos().ok()?;
                        try_chained_context(
                            gpos, gdef, &ctx, filter, buffer, positions, pos, normalized, depth,
                        )
                    }
                    _ => None,
                }
            }
            _ => None,
        };
        if next.is_some() {
            return next;
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Type 1 — single adjustment
// ---------------------------------------------------------------------------

fn try_single(
    sp: &SinglePos<'_>,
    gdef: Option<&GdefTable<'_>>,
    positions: &mut [Pos],
    buffer: &[GlyphInfo],
    pos: usize,
    normalized: &[f32],
) -> Option<usize> {
    let rec = sp.value(buffer[pos].glyph)?.ok()?;
    apply_value_record(&mut positions[pos], &rec, sp.raw(), gdef, normalized);
    Some(pos + 1)
}

// ---------------------------------------------------------------------------
// Type 2 — pair adjustment (kerning)
// ---------------------------------------------------------------------------

fn try_pair(
    pp: &PairPos<'_>,
    gdef: Option<&GdefTable<'_>>,
    filter: &SkipFilter<'_, '_>,
    buffer: &[GlyphInfo],
    positions: &mut [Pos],
    pos: usize,
    normalized: &[f32],
) -> Option<usize> {
    let second = super::buffer::next_unskipped(buffer, filter, pos)?;
    let pv = pp.pair(buffer[pos].glyph, buffer[second].glyph)?.ok()?;
    // Device / VariationIndex offsets in pair ValueRecords are
    // measured from the PairSet table (format 1) or the subtable
    // (format 2) — the ValueRecord definition's "immediate parent".
    let base = pp.value_device_base(buffer[pos].glyph).unwrap_or(0);
    let dev = &pp.raw()[base.min(pp.raw().len())..];
    apply_value_record(&mut positions[pos], &pv.first, dev, gdef, normalized);
    apply_value_record(&mut positions[second], &pv.second, dev, gdef, normalized);
    // Spec: "If valueFormat2 is set to 0, then the second glyph of the
    // pair is the 'next' glyph for which a lookup should be performed";
    // otherwise processing continues after the second glyph.
    if pp.value_format2().bits() == 0 {
        Some(second)
    } else {
        Some(second + 1)
    }
}

// ---------------------------------------------------------------------------
// Types 4 / 6 — mark attachment
// ---------------------------------------------------------------------------

/// Displace the glyph at `mark_pos` so its `mark_anchor` coincides
/// with `base_anchor` on the glyph at `base_pos`.
///
/// Anchor coordinates are in the design space of their own glyph
/// (origin at the glyph origin); the run positions glyphs by pen
/// advances, so the mark offset removes the advances accumulated
/// between the base origin and the mark origin and inherits the
/// base's own placement offsets. "Placement of the base glyph and
/// advances of both glyphs are not affected" (GPOS §"Lookup type 4").
fn attach(
    positions: &mut [Pos],
    base_pos: usize,
    mark_pos: usize,
    base_anchor: (i32, i32),
    mark_anchor: (i32, i32),
) {
    let mut adv_x = 0i32;
    let mut adv_y = 0i32;
    for p in positions.iter().take(mark_pos).skip(base_pos) {
        adv_x += p.x_advance;
        adv_y += p.y_advance;
    }
    positions[mark_pos].x_offset =
        positions[base_pos].x_offset + base_anchor.0 - mark_anchor.0 - adv_x;
    positions[mark_pos].y_offset =
        positions[base_pos].y_offset + base_anchor.1 - mark_anchor.1 - adv_y;
}

/// GPOS type 4: attach a combining mark to its base glyph.
///
/// Spec: "To identify the base glyph that combines with a mark, the
/// text-processing client must look backward in the glyph string from
/// the mark to the preceding base glyph" — the backward search steps
/// over other mark glyphs (GDEF class 3) and anything the lookup's
/// own flags skip.
fn try_mark_base(
    mb: &MarkBasePos<'_>,
    gdef: Option<&GdefTable<'_>>,
    filter: &SkipFilter<'_, '_>,
    buffer: &[GlyphInfo],
    positions: &mut [Pos],
    pos: usize,
) -> Option<usize> {
    let mark = buffer[pos].glyph;
    mb.mark_coverage().index_of(mark)?;

    // Backward search for the base: skip marks and filtered glyphs.
    let mut base_pos = None;
    let mut p = pos;
    while p > 0 {
        p -= 1;
        let g = buffer[p].glyph;
        if filter.skips(g) {
            continue;
        }
        if let Some(gdef) = gdef {
            if gdef.glyph_class(g) == Some(GlyphClass::Mark) {
                continue;
            }
        }
        base_pos = Some(p);
        break;
    }
    let base_pos = base_pos?;
    let att = mb.attachment(mark, buffer[base_pos].glyph)?.ok()?;
    attach(
        positions,
        base_pos,
        pos,
        (att.base_anchor.x as i32, att.base_anchor.y as i32),
        (att.mark_anchor.x as i32, att.mark_anchor.y as i32),
    );
    Some(pos + 1)
}

/// GPOS type 6: attach a mark (`mark1`) to a preceding mark (`mark2`).
///
/// Spec: "The mark2 glyph that combines with a mark1 glyph is the
/// glyph preceding the mark1 glyph in glyph string order (skipping
/// glyphs according to LookupFlags)" — mark-to-mark lookups typically
/// carry a mark-attachment-class or mark-filtering-set flag so the
/// backward step lands on the relevant mark.
fn try_mark_mark(
    mm: &MarkMarkPos<'_>,
    filter: &SkipFilter<'_, '_>,
    buffer: &[GlyphInfo],
    positions: &mut [Pos],
    pos: usize,
) -> Option<usize> {
    let mark1 = buffer[pos].glyph;
    mm.mark1_coverage().index_of(mark1)?;
    let mark2_pos = super::buffer::prev_unskipped(buffer, filter, pos)?;
    let att = mm.attachment(mark1, buffer[mark2_pos].glyph)?.ok()?;
    attach(
        positions,
        mark2_pos,
        pos,
        (att.mark2_anchor.x as i32, att.mark2_anchor.y as i32),
        (att.mark1_anchor.x as i32, att.mark1_anchor.y as i32),
    );
    Some(pos + 1)
}

// ---------------------------------------------------------------------------
// Type 3 — cursive attachment
// ---------------------------------------------------------------------------

/// GPOS type 3: join two adjacent covered glyphs by aligning the exit
/// anchor of the first with the entry anchor of the second (GPOS
/// §"Lookup type 3": "a text-processing client aligns the exit anchor
/// point of a glyph with the entry anchor point of the following
/// glyph").
///
/// In horizontal LTR layout the in-stream alignment lands on the
/// first glyph's advance (the pen must sit on the exit anchor when
/// the second glyph starts, whose entry anchor then pins the ink):
/// `advance₁ = offset₁ + exit.x − entry.x − offset₂`. The
/// cross-stream (Y) alignment shifts a placement offset: with the
/// `RIGHT_TO_LEFT` lookup flag clear the *following* glyph moves to
/// the leading glyph's exit height; with it set "the last glyph in a
/// matched input sequence keeps its initial position ... and the
/// cross-stream positions of the preceding, connected glyphs are
/// adjusted", so the *leading* glyph moves instead.
fn try_cursive(
    cp: &CursivePos<'_>,
    rtl_flag: bool,
    filter: &SkipFilter<'_, '_>,
    buffer: &[GlyphInfo],
    positions: &mut [Pos],
    pos: usize,
) -> Option<usize> {
    let next = super::buffer::next_unskipped(buffer, filter, pos)?;
    let att = cp.attachment(buffer[pos].glyph, buffer[next].glyph)?.ok()?;
    let exit = (att.exit_anchor.x as i32, att.exit_anchor.y as i32);
    let entry = (att.entry_anchor.x as i32, att.entry_anchor.y as i32);

    positions[pos].x_advance =
        positions[pos].x_offset + exit.0 - entry.0 - positions[next].x_offset;
    if rtl_flag {
        positions[pos].y_offset = positions[next].y_offset + entry.1 - exit.1;
    } else {
        positions[next].y_offset = positions[pos].y_offset + exit.1 - entry.1;
    }
    // The joined glyph is the next candidate (its own exit may join
    // the glyph after it).
    Some(next)
}

// ---------------------------------------------------------------------------
// Type 5 — mark-to-ligature attachment
// ---------------------------------------------------------------------------

/// GPOS type 5: attach a combining mark to a component of a preceding
/// ligature glyph. The component index comes from the ligature
/// bookkeeping recorded during GSUB ligature substitution (a mark
/// consumed *inside* the ligature pattern remembers the component
/// that preceded it); a mark following the whole ligature associates
/// with the last component.
fn try_mark_lig(
    ml: &MarkLigPos<'_>,
    gdef: Option<&GdefTable<'_>>,
    filter: &SkipFilter<'_, '_>,
    buffer: &[GlyphInfo],
    positions: &mut [Pos],
    pos: usize,
) -> Option<usize> {
    let mark = buffer[pos].glyph;
    ml.mark_coverage().index_of(mark)?;

    // Backward search for the ligature glyph: skip marks and
    // flag-filtered glyphs (same walk as mark-to-base).
    let mut lig_pos = None;
    let mut p = pos;
    while p > 0 {
        p -= 1;
        let g = buffer[p].glyph;
        if filter.skips(g) {
            continue;
        }
        if let Some(gdef) = gdef {
            if gdef.glyph_class(g) == Some(GlyphClass::Mark) {
                continue;
            }
        }
        lig_pos = Some(p);
        break;
    }
    let lig_pos = lig_pos?;
    let lig_glyph = buffer[lig_pos].glyph;

    // Component selection: the recorded in-pattern component, else
    // the ligature's last component.
    let comp_count = match ml.component_count(lig_glyph)? {
        Ok(c) => c,
        Err(_) => return None,
    };
    if comp_count == 0 {
        return None;
    }
    let component = if buffer[pos].lig_component != LIG_COMPONENT_NONE {
        buffer[pos].lig_component.min(comp_count - 1)
    } else {
        comp_count - 1
    };
    let att = ml.attachment(mark, lig_glyph, component)?.ok()?;
    attach(
        positions,
        lig_pos,
        pos,
        (att.ligature_anchor.x as i32, att.ligature_anchor.y as i32),
        (att.mark_anchor.x as i32, att.mark_anchor.y as i32),
    );
    Some(pos + 1)
}

// ---------------------------------------------------------------------------
// Types 7 / 8 — contextual positioning
// ---------------------------------------------------------------------------

/// Apply a nested lookup (from a `SequenceLookupRecord`) at one
/// position, with the nested lookup's own flags.
fn apply_nested(
    gpos: &GposTable<'_>,
    gdef: Option<&GdefTable<'_>>,
    lookup_index: u16,
    buffer: &[GlyphInfo],
    positions: &mut [Pos],
    pos: usize,
    normalized: &[f32],
    depth: usize,
) {
    if depth > MAX_NESTING_DEPTH {
        return;
    }
    let Some(lookup) = gpos.lookup(lookup_index) else {
        return;
    };
    let filter = SkipFilter::new(lookup.flag(), lookup.mark_filtering_set(), gdef);
    if filter.skips(buffer[pos].glyph) {
        return;
    }
    let _ = try_apply_at(
        gpos,
        gdef,
        lookup_index,
        &filter,
        buffer,
        positions,
        pos,
        normalized,
        depth,
    );
}

/// Run the nested-lookup records of a matched (chained) context.
/// GPOS never changes the buffer length, so matched positions stay
/// valid throughout.
#[allow(clippy::too_many_arguments)]
fn apply_context_records(
    gpos: &GposTable<'_>,
    gdef: Option<&GdefTable<'_>>,
    positions_matched: &[usize],
    records: &[SequenceLookupRecord],
    buffer: &[GlyphInfo],
    positions: &mut [Pos],
    normalized: &[f32],
    depth: usize,
) -> usize {
    for rec in records {
        let k = rec.sequence_index as usize;
        if k >= positions_matched.len() {
            continue;
        }
        apply_nested(
            gpos,
            gdef,
            rec.lookup_list_index,
            buffer,
            positions,
            positions_matched[k],
            normalized,
            depth + 1,
        );
    }
    positions_matched.last().copied().unwrap_or(0) + 1
}

#[allow(clippy::too_many_arguments)]
fn try_context(
    gpos: &GposTable<'_>,
    gdef: Option<&GdefTable<'_>>,
    ctx: &SequenceContext<'_>,
    filter: &SkipFilter<'_, '_>,
    buffer: &[GlyphInfo],
    positions: &mut [Pos],
    pos: usize,
    normalized: &[f32],
    depth: usize,
) -> Option<usize> {
    let g = buffer[pos].glyph;
    match ctx {
        SequenceContext::Format1 {
            coverage,
            rule_sets,
        } => {
            let cov = coverage.index_of(g)? as usize;
            for rule in rule_sets.get(cov)? {
                if let Some(matched) =
                    match_input(buffer, filter, pos, 1 + rule.input.len(), |k, gl| {
                        gl == rule.input[k - 1]
                    })
                {
                    return Some(apply_context_records(
                        gpos,
                        gdef,
                        &matched,
                        &rule.lookups,
                        buffer,
                        positions,
                        normalized,
                        depth,
                    ));
                }
            }
            None
        }
        SequenceContext::Format2 {
            coverage,
            class_def,
            rule_sets,
        } => {
            coverage.index_of(g)?;
            let class = class_def.class_of(g) as usize;
            for rule in rule_sets.get(class)? {
                if let Some(matched) =
                    match_input(buffer, filter, pos, 1 + rule.input.len(), |k, gl| {
                        class_def.class_of(gl) == rule.input[k - 1]
                    })
                {
                    return Some(apply_context_records(
                        gpos,
                        gdef,
                        &matched,
                        &rule.lookups,
                        buffer,
                        positions,
                        normalized,
                        depth,
                    ));
                }
            }
            None
        }
        SequenceContext::Format3 { coverages, lookups } => {
            if coverages.is_empty() || !coverages[0].contains(g) {
                return None;
            }
            let matched = match_input(buffer, filter, pos, coverages.len(), |k, gl| {
                coverages[k].contains(gl)
            })?;
            Some(apply_context_records(
                gpos, gdef, &matched, lookups, buffer, positions, normalized, depth,
            ))
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn try_chained_context(
    gpos: &GposTable<'_>,
    gdef: Option<&GdefTable<'_>>,
    ctx: &ChainedSequenceContext<'_>,
    filter: &SkipFilter<'_, '_>,
    buffer: &[GlyphInfo],
    positions: &mut [Pos],
    pos: usize,
    normalized: &[f32],
    depth: usize,
) -> Option<usize> {
    let g = buffer[pos].glyph;
    match ctx {
        ChainedSequenceContext::Format1 {
            coverage,
            rule_sets,
        } => {
            let cov = coverage.index_of(g)? as usize;
            for rule in rule_sets.get(cov)? {
                let Some(matched) =
                    match_input(buffer, filter, pos, 1 + rule.input.len(), |k, gl| {
                        gl == rule.input[k - 1]
                    })
                else {
                    continue;
                };
                if !match_backtrack(buffer, filter, pos, rule.backtrack.len(), |k, gl| {
                    gl == rule.backtrack[k]
                }) {
                    continue;
                }
                let last = *matched.last().unwrap_or(&pos);
                if !match_lookahead(buffer, filter, last, rule.lookahead.len(), |k, gl| {
                    gl == rule.lookahead[k]
                }) {
                    continue;
                }
                return Some(apply_context_records(
                    gpos,
                    gdef,
                    &matched,
                    &rule.lookups,
                    buffer,
                    positions,
                    normalized,
                    depth,
                ));
            }
            None
        }
        ChainedSequenceContext::Format2 {
            coverage,
            backtrack_class_def,
            input_class_def,
            lookahead_class_def,
            rule_sets,
        } => {
            coverage.index_of(g)?;
            let class = input_class_def.class_of(g) as usize;
            for rule in rule_sets.get(class)? {
                let Some(matched) =
                    match_input(buffer, filter, pos, 1 + rule.input.len(), |k, gl| {
                        input_class_def.class_of(gl) == rule.input[k - 1]
                    })
                else {
                    continue;
                };
                if !match_backtrack(buffer, filter, pos, rule.backtrack.len(), |k, gl| {
                    backtrack_class_def.class_of(gl) == rule.backtrack[k]
                }) {
                    continue;
                }
                let last = *matched.last().unwrap_or(&pos);
                if !match_lookahead(buffer, filter, last, rule.lookahead.len(), |k, gl| {
                    lookahead_class_def.class_of(gl) == rule.lookahead[k]
                }) {
                    continue;
                }
                return Some(apply_context_records(
                    gpos,
                    gdef,
                    &matched,
                    &rule.lookups,
                    buffer,
                    positions,
                    normalized,
                    depth,
                ));
            }
            None
        }
        ChainedSequenceContext::Format3 {
            backtrack,
            input,
            lookahead,
            lookups,
        } => {
            if input.is_empty() || !input[0].contains(g) {
                return None;
            }
            let matched = match_input(buffer, filter, pos, input.len(), |k, gl| {
                input[k].contains(gl)
            })?;
            if !match_backtrack(buffer, filter, pos, backtrack.len(), |k, gl| {
                backtrack[k].contains(gl)
            }) {
                return None;
            }
            let last = *matched.last().unwrap_or(&pos);
            if !match_lookahead(buffer, filter, last, lookahead.len(), |k, gl| {
                lookahead[k].contains(gl)
            }) {
                return None;
            }
            Some(apply_context_records(
                gpos, gdef, &matched, lookups, buffer, positions, normalized, depth,
            ))
        }
    }
}

// ---------------------------------------------------------------------------
// Legacy `kern` table fallback
// ---------------------------------------------------------------------------

/// Apply the legacy `kern` table's horizontal pair kerning to the
/// advance of the first glyph of each adjacent pair. Used only when
/// the font offers no GPOS `kern` feature for the resolved script
/// (§5.7.5; modern fonts use GPOS pair adjustment).
pub(crate) fn apply_legacy_kern(
    font: &Font<'_>,
    buffer: &[GlyphInfo],
    positions: &mut [Pos],
    options: &ShapeOptions,
) {
    if font.kern().is_none() {
        return;
    }
    // Respect an explicit `kern` disable; otherwise kerning is a
    // default-enabled feature.
    let enabled = options
        .features
        .iter()
        .find(|f| f.tag == *b"kern")
        .map(|f| f.value != 0)
        .unwrap_or(true);
    if !enabled {
        return;
    }
    for i in 1..buffer.len() {
        let v = font.kern_pair(buffer[i - 1].glyph, buffer[i].glyph);
        positions[i - 1].x_advance += v as i32;
    }
}
