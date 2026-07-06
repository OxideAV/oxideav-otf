//! GSUB lookup application: the substitution half of the shaper.
//!
//! Spec: `docs/text/opentype/otspec-gsub.html` (per-lookup-type
//! actions) + `docs/text/opentype/otspec-chapter2-common-layout-tables.html`
//! (processing model, LookupFlag filtering, contextual nesting).
//!
//! Processing model implemented here (chapter 2 §"Lookup table" and
//! §"Contextual lookups"):
//!
//! * A lookup walks the glyph run left to right. Glyphs skipped by the
//!   lookup's flags are processed "as though not present": they are
//!   never the current glyph and are stepped over while matching
//!   multi-glyph patterns.
//! * At each candidate glyph the lookup's subtables are tried in
//!   order; the first subtable whose pattern matches is applied and
//!   the cursor moves past the affected glyphs.
//! * Contextual subtables (types 5/6) don't mutate glyphs themselves:
//!   each match carries `SequenceLookupRecord`s naming nested lookups
//!   applied at input-sequence positions, in record order, each acting
//!   on the result of the previous. Nested lookups use their *own*
//!   flags, not the outer lookup's.
//! * Reverse-chaining contextual single substitution (type 8)
//!   processes the run from end to start, substituting in place.

use super::buffer::{
    match_backtrack, match_input, match_lookahead, GlyphInfo, SkipFilter, LIG_COMPONENT_NONE,
};
use super::{PlannedLookup, MAX_NESTING_DEPTH};
use crate::tables::context::{ChainedSequenceContext, SequenceContext, SequenceLookupRecord};
use crate::tables::gdef::GdefTable;
use crate::tables::gsub::{
    GsubTable, GSUB_LOOKUP_TYPE_ALTERNATE, GSUB_LOOKUP_TYPE_CHAINED_CONTEXT,
    GSUB_LOOKUP_TYPE_CONTEXT, GSUB_LOOKUP_TYPE_EXTENSION, GSUB_LOOKUP_TYPE_LIGATURE,
    GSUB_LOOKUP_TYPE_MULTIPLE, GSUB_LOOKUP_TYPE_REVERSE_CHAINED_SINGLE, GSUB_LOOKUP_TYPE_SINGLE,
};

/// Result of one successful subtable application at a position.
struct Applied {
    /// Buffer index to continue processing from.
    next: usize,
    /// Signed change in buffer length caused by the application.
    delta: isize,
}

/// Apply GSUB lookup `planned.lookup_index` over the whole buffer.
pub(crate) fn apply_lookup(
    gsub: &GsubTable<'_>,
    gdef: Option<&GdefTable<'_>>,
    planned: &PlannedLookup,
    buffer: &mut Vec<GlyphInfo>,
) {
    let Some(lookup) = gsub.lookup(planned.lookup_index) else {
        return;
    };
    let filter = SkipFilter::new(lookup.flag(), lookup.mark_filtering_set(), gdef);

    if lookup.lookup_type() == GSUB_LOOKUP_TYPE_REVERSE_CHAINED_SINGLE {
        apply_reverse_chain(gsub, planned.lookup_index, &filter, buffer);
        return;
    }

    let mut pos = 0usize;
    while pos < buffer.len() {
        if filter.skips(buffer[pos].glyph) {
            pos += 1;
            continue;
        }
        match try_apply_at(
            gsub,
            gdef,
            planned.lookup_index,
            planned.feature_value,
            &filter,
            buffer,
            pos,
            0,
        ) {
            Some(applied) => pos = applied.next,
            None => pos += 1,
        }
    }
}

/// Try every subtable of the lookup at buffer position `pos`; the
/// first that matches is applied (chapter 2: "the subtables are
/// evaluated in the order the offsets are listed").
#[allow(clippy::too_many_arguments)]
fn try_apply_at(
    gsub: &GsubTable<'_>,
    gdef: Option<&GdefTable<'_>>,
    lookup_index: u16,
    value: u32,
    filter: &SkipFilter<'_, '_>,
    buffer: &mut Vec<GlyphInfo>,
    pos: usize,
    depth: usize,
) -> Option<Applied> {
    let lookup = gsub.lookup(lookup_index)?;
    let lookup_type = lookup.lookup_type();
    for s in 0..lookup.subtable_count() {
        let applied = match lookup_type {
            GSUB_LOOKUP_TYPE_SINGLE => {
                let ss = gsub.single_subst(lookup_index, s)?.ok()?;
                try_single(&ss, value, buffer, pos)
            }
            GSUB_LOOKUP_TYPE_MULTIPLE => {
                let ms = gsub.multiple_subst(lookup_index, s)?.ok()?;
                try_multiple(&ms, value, buffer, pos)
            }
            GSUB_LOOKUP_TYPE_ALTERNATE => {
                let alt = gsub.alternate_subst(lookup_index, s)?.ok()?;
                try_alternate(&alt, value, buffer, pos)
            }
            GSUB_LOOKUP_TYPE_LIGATURE => {
                let ls = gsub.ligature_subst(lookup_index, s)?.ok()?;
                try_ligature(&ls, value, filter, buffer, pos)
            }
            GSUB_LOOKUP_TYPE_CONTEXT => {
                let ctx = gsub.context_subst(lookup_index, s)?.ok()?;
                try_context(gsub, gdef, &ctx, filter, buffer, pos, depth)
            }
            GSUB_LOOKUP_TYPE_CHAINED_CONTEXT => {
                let ctx = gsub.chained_context_subst(lookup_index, s)?.ok()?;
                try_chained_context(gsub, gdef, &ctx, filter, buffer, pos, depth)
            }
            GSUB_LOOKUP_TYPE_EXTENSION => {
                let ext = gsub.extension_subst(lookup_index, s)?.ok()?;
                match ext.extension_lookup_type() {
                    GSUB_LOOKUP_TYPE_SINGLE => {
                        try_single(&ext.as_single_subst().ok()?, value, buffer, pos)
                    }
                    GSUB_LOOKUP_TYPE_MULTIPLE => {
                        try_multiple(&ext.as_multiple_subst().ok()?, value, buffer, pos)
                    }
                    GSUB_LOOKUP_TYPE_ALTERNATE => {
                        try_alternate(&ext.as_alternate_subst().ok()?, value, buffer, pos)
                    }
                    GSUB_LOOKUP_TYPE_LIGATURE => {
                        try_ligature(&ext.as_ligature_subst().ok()?, value, filter, buffer, pos)
                    }
                    GSUB_LOOKUP_TYPE_CONTEXT => try_context(
                        gsub,
                        gdef,
                        &ext.as_context_subst().ok()?,
                        filter,
                        buffer,
                        pos,
                        depth,
                    ),
                    GSUB_LOOKUP_TYPE_CHAINED_CONTEXT => try_chained_context(
                        gsub,
                        gdef,
                        &ext.as_chained_context_subst().ok()?,
                        filter,
                        buffer,
                        pos,
                        depth,
                    ),
                    _ => None,
                }
            }
            _ => None,
        };
        if applied.is_some() {
            return applied;
        }
    }
    None
}

/// Apply a *nested* lookup (from a `SequenceLookupRecord`) at exactly
/// one buffer position. The nested lookup's own flags are used
/// (chapter 2: "lookup flags in the nested lookups are considered").
fn apply_nested(
    gsub: &GsubTable<'_>,
    gdef: Option<&GdefTable<'_>>,
    lookup_index: u16,
    buffer: &mut Vec<GlyphInfo>,
    pos: usize,
    depth: usize,
) -> Option<isize> {
    if depth > MAX_NESTING_DEPTH {
        return None;
    }
    let lookup = gsub.lookup(lookup_index)?;
    if lookup.lookup_type() == GSUB_LOOKUP_TYPE_REVERSE_CHAINED_SINGLE {
        // Type 8 is not usable as a nested lookup (it defines its own
        // whole-run reverse processing order).
        return None;
    }
    let filter = SkipFilter::new(lookup.flag(), lookup.mark_filtering_set(), gdef);
    if filter.skips(buffer[pos].glyph) {
        return None;
    }
    try_apply_at(gsub, gdef, lookup_index, 1, &filter, buffer, pos, depth)
        .map(|applied| applied.delta)
}

// ---------------------------------------------------------------------------
// Per-type appliers
// ---------------------------------------------------------------------------

fn try_single(
    ss: &crate::tables::gsub::SingleSubst<'_>,
    value: u32,
    buffer: &mut [GlyphInfo],
    pos: usize,
) -> Option<Applied> {
    // Alternate-index feature values (>= 2) target AlternateSets, not
    // plain single substitutions (see `FeatureSetting`).
    if value != 1 {
        return None;
    }
    let out = ss.substitute(buffer[pos].glyph)?;
    buffer[pos].glyph = out;
    Some(Applied {
        next: pos + 1,
        delta: 0,
    })
}

fn try_multiple(
    ms: &crate::tables::gsub::MultipleSubst<'_>,
    value: u32,
    buffer: &mut Vec<GlyphInfo>,
    pos: usize,
) -> Option<Applied> {
    if value != 1 {
        return None;
    }
    let seq = ms.substitute(buffer[pos].glyph)?;
    let outputs: Vec<u16> = seq.glyphs().collect();
    if outputs.is_empty() {
        // The spec prohibits empty Sequences (deletion); ignore one.
        return None;
    }
    let cluster = buffer[pos].cluster;
    buffer[pos] = GlyphInfo::new(outputs[0], cluster);
    for (k, &g) in outputs[1..].iter().enumerate() {
        buffer.insert(pos + 1 + k, GlyphInfo::new(g, cluster));
    }
    Some(Applied {
        next: pos + outputs.len(),
        delta: outputs.len() as isize - 1,
    })
}

fn try_alternate(
    alt: &crate::tables::gsub::AlternateSubst<'_>,
    value: u32,
    buffer: &mut [GlyphInfo],
    pos: usize,
) -> Option<Applied> {
    let set = alt.substitute(buffer[pos].glyph)?;
    // The spec leaves the choice of alternate to the client; our
    // client policy takes the feature value as a 1-based selector.
    let index = if value >= 1 { value as usize - 1 } else { 0 };
    let out = set.glyphs().nth(index)?;
    buffer[pos].glyph = out;
    Some(Applied {
        next: pos + 1,
        delta: 0,
    })
}

fn try_ligature(
    ls: &crate::tables::gsub::LigatureSubst<'_>,
    value: u32,
    filter: &SkipFilter<'_, '_>,
    buffer: &mut Vec<GlyphInfo>,
    pos: usize,
) -> Option<Applied> {
    if value != 1 {
        return None;
    }
    let cov = ls.coverage().index_of(buffer[pos].glyph)?;
    let set = ls.ligature_set(cov)?.ok()?;
    // "array order = preference order": first matching Ligature wins.
    for j in 0..set.ligature_count() {
        let Some(Ok(lig)) = set.ligature(j) else {
            continue;
        };
        let comp_count = lig.component_count() as usize;
        if comp_count == 0 {
            continue;
        }
        let comps: Vec<u16> = lig.component_glyphs().collect();
        if comps.len() != comp_count - 1 {
            continue;
        }
        let Some(positions) =
            match_input(buffer, filter, pos, comp_count, |k, g| g == comps[k - 1])
        else {
            continue;
        };

        // Mark glyphs skipped between component k and k+1 associate
        // with component k — GPOS mark-to-ligature attachment selects
        // its anchor by this component index (the spec: the layout
        // client "must keep track of associations of marks to
        // particular ligature-glyph components").
        for (ci, win) in positions.windows(2).enumerate() {
            for skipped in buffer.iter_mut().take(win[1]).skip(win[0] + 1) {
                skipped.lig_component = ci as u16;
            }
        }

        let cluster = positions
            .iter()
            .map(|&p| buffer[p].cluster)
            .min()
            .unwrap_or(buffer[pos].cluster);
        buffer[pos].glyph = lig.ligature_glyph();
        buffer[pos].cluster = cluster;
        buffer[pos].lig_num_comps = comp_count as u16;
        buffer[pos].lig_component = LIG_COMPONENT_NONE;
        // Remove the consumed component glyphs (not the skipped
        // glyphs in between — those remain, now after the ligature).
        for &p in positions[1..].iter().rev() {
            buffer.remove(p);
        }
        return Some(Applied {
            next: pos + 1,
            delta: -(comp_count as isize - 1),
        });
    }
    None
}

/// Run the nested-lookup records of a matched (chained) context.
///
/// `positions` maps input-sequence indices to buffer positions.
/// Records apply in array order, "each acting on the result of the
/// previous"; when a nested application changes the buffer length,
/// later matched positions shift accordingly. (The spec does not
/// define the outcome of a record that targets a glyph consumed by an
/// earlier record's ligature; such positions simply shift with the
/// delta here.)
fn apply_context_records(
    gsub: &GsubTable<'_>,
    gdef: Option<&GdefTable<'_>>,
    mut positions: Vec<usize>,
    records: &[SequenceLookupRecord],
    buffer: &mut Vec<GlyphInfo>,
    depth: usize,
) -> Applied {
    let end_before = positions.last().copied().unwrap_or(0) + 1;
    let mut total_delta = 0isize;
    for rec in records {
        let k = rec.sequence_index as usize;
        if k >= positions.len() {
            continue;
        }
        let at = positions[k];
        if at >= buffer.len() {
            continue;
        }
        if let Some(delta) = apply_nested(gsub, gdef, rec.lookup_list_index, buffer, at, depth + 1)
        {
            total_delta += delta;
            for p in positions.iter_mut() {
                if *p > at {
                    *p = (*p as isize + delta).max(0) as usize;
                }
            }
        }
    }
    Applied {
        next: (end_before as isize + total_delta).max(0) as usize,
        delta: total_delta,
    }
}

fn try_context(
    gsub: &GsubTable<'_>,
    gdef: Option<&GdefTable<'_>>,
    ctx: &SequenceContext<'_>,
    filter: &SkipFilter<'_, '_>,
    buffer: &mut Vec<GlyphInfo>,
    pos: usize,
    depth: usize,
) -> Option<Applied> {
    let g = buffer[pos].glyph;
    match ctx {
        SequenceContext::Format1 {
            coverage,
            rule_sets,
        } => {
            let cov = coverage.index_of(g)? as usize;
            let rules = rule_sets.get(cov)?;
            for rule in rules {
                if let Some(positions) =
                    match_input(buffer, filter, pos, 1 + rule.input.len(), |k, gl| {
                        gl == rule.input[k - 1]
                    })
                {
                    return Some(apply_context_records(
                        gsub,
                        gdef,
                        positions,
                        &rule.lookups,
                        buffer,
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
            let rules = rule_sets.get(class)?;
            for rule in rules {
                if let Some(positions) =
                    match_input(buffer, filter, pos, 1 + rule.input.len(), |k, gl| {
                        class_def.class_of(gl) == rule.input[k - 1]
                    })
                {
                    return Some(apply_context_records(
                        gsub,
                        gdef,
                        positions,
                        &rule.lookups,
                        buffer,
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
            let positions = match_input(buffer, filter, pos, coverages.len(), |k, gl| {
                coverages[k].contains(gl)
            })?;
            Some(apply_context_records(
                gsub, gdef, positions, lookups, buffer, depth,
            ))
        }
    }
}

fn try_chained_context(
    gsub: &GsubTable<'_>,
    gdef: Option<&GdefTable<'_>>,
    ctx: &ChainedSequenceContext<'_>,
    filter: &SkipFilter<'_, '_>,
    buffer: &mut Vec<GlyphInfo>,
    pos: usize,
    depth: usize,
) -> Option<Applied> {
    let g = buffer[pos].glyph;
    match ctx {
        ChainedSequenceContext::Format1 {
            coverage,
            rule_sets,
        } => {
            let cov = coverage.index_of(g)? as usize;
            let rules = rule_sets.get(cov)?;
            for rule in rules {
                let Some(positions) =
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
                let last = *positions.last().unwrap_or(&pos);
                if !match_lookahead(buffer, filter, last, rule.lookahead.len(), |k, gl| {
                    gl == rule.lookahead[k]
                }) {
                    continue;
                }
                return Some(apply_context_records(
                    gsub,
                    gdef,
                    positions,
                    &rule.lookups,
                    buffer,
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
            let rules = rule_sets.get(class)?;
            for rule in rules {
                let Some(positions) =
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
                let last = *positions.last().unwrap_or(&pos);
                if !match_lookahead(buffer, filter, last, rule.lookahead.len(), |k, gl| {
                    lookahead_class_def.class_of(gl) == rule.lookahead[k]
                }) {
                    continue;
                }
                return Some(apply_context_records(
                    gsub,
                    gdef,
                    positions,
                    &rule.lookups,
                    buffer,
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
            let positions = match_input(buffer, filter, pos, input.len(), |k, gl| {
                input[k].contains(gl)
            })?;
            if !match_backtrack(buffer, filter, pos, backtrack.len(), |k, gl| {
                backtrack[k].contains(gl)
            }) {
                return None;
            }
            let last = *positions.last().unwrap_or(&pos);
            if !match_lookahead(buffer, filter, last, lookahead.len(), |k, gl| {
                lookahead[k].contains(gl)
            }) {
                return None;
            }
            Some(apply_context_records(
                gsub, gdef, positions, lookups, buffer, depth,
            ))
        }
    }
}

/// GSUB type 8: reverse-chaining contextual single substitution.
///
/// Spec (GSUB §"Lookup type 8"): "processing of input glyph sequence
/// goes from end to start"; the input is a single covered glyph, the
/// backtrack/lookahead Coverage sequences gate the match, and the
/// substitution replaces the glyph in place.
fn apply_reverse_chain(
    gsub: &GsubTable<'_>,
    lookup_index: u16,
    filter: &SkipFilter<'_, '_>,
    buffer: &mut [GlyphInfo],
) {
    let Some(lookup) = gsub.lookup(lookup_index) else {
        return;
    };
    let mut i = buffer.len();
    while i > 0 {
        i -= 1;
        if filter.skips(buffer[i].glyph) {
            continue;
        }
        for s in 0..lookup.subtable_count() {
            let rc = match gsub.reverse_chain_single_subst(lookup_index, s) {
                Some(Ok(rc)) => rc,
                _ => {
                    // A type-7 wrapper around a type-8 subtable.
                    match gsub.extension_subst(lookup_index, s) {
                        Some(Ok(ext)) => match ext.as_reverse_chain_single_subst() {
                            Ok(rc) => rc,
                            Err(_) => continue,
                        },
                        _ => continue,
                    }
                }
            };
            let Some(out) = rc.substitute(buffer[i].glyph) else {
                continue;
            };
            let bt = rc.backtrack_glyph_count() as usize;
            if !match_backtrack(buffer, filter, i, bt, |k, gl| {
                rc.backtrack_coverage(k as u16)
                    .map(|c| c.contains(gl))
                    .unwrap_or(false)
            }) {
                continue;
            }
            let la = rc.lookahead_glyph_count() as usize;
            if !match_lookahead(buffer, filter, i, la, |k, gl| {
                rc.lookahead_coverage(k as u16)
                    .map(|c| c.contains(gl))
                    .unwrap_or(false)
            }) {
                continue;
            }
            buffer[i].glyph = out;
            break;
        }
    }
}
