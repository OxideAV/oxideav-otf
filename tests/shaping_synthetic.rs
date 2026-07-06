//! Shaping tests for the GPOS lookup types the Source Sans fixture
//! does not exercise: cursive attachment (type 3), mark-to-ligature
//! attachment (type 5), and (chained) contextual positioning
//! (types 7/8) — driven end-to-end through `Font::shape` over a
//! synthetic byte-built font.
//!
//! Spec: `docs/text/opentype/otspec-gpos.html` (lookup semantics),
//! `docs/text/opentype/otspec-gsub.html` (the liga lookup used to
//! form the test ligature), `docs/text/opentype/otspec-otff.html`
//! (sfnt assembly). Expected positions are computed by hand from the
//! anchor coordinates written into the tables.
//!
//! Glyph inventory:
//!   gid 1 = 'a' (base, adv 500)     gid 2 = 'b' (base, adv 600)
//!   gid 3 = 'c' (mark, adv 0)       gid 4 = a+b ligature (adv 900)
//!   gid 5 = 'd' (base, adv 550)

use oxideav_otf::{Font, ShapeOptions};

fn be16(v: u16) -> [u8; 2] {
    v.to_be_bytes()
}

/// Coverage format 1 (glyph IDs must be sorted).
fn coverage(glyphs: &[u16]) -> Vec<u8> {
    let mut b = Vec::new();
    b.extend_from_slice(&be16(1));
    b.extend_from_slice(&be16(glyphs.len() as u16));
    for &g in glyphs {
        b.extend_from_slice(&be16(g));
    }
    b
}

/// Anchor format 1.
fn anchor(x: i16, y: i16) -> Vec<u8> {
    let mut b = Vec::new();
    b.extend_from_slice(&be16(1));
    b.extend_from_slice(&x.to_be_bytes());
    b.extend_from_slice(&y.to_be_bytes());
    b
}

/// ClassDef format 2 from `(start, end, class)` ranges.
fn classdef(ranges: &[(u16, u16, u16)]) -> Vec<u8> {
    let mut b = Vec::new();
    b.extend_from_slice(&be16(2));
    b.extend_from_slice(&be16(ranges.len() as u16));
    for &(s, e, c) in ranges {
        b.extend_from_slice(&be16(s));
        b.extend_from_slice(&be16(e));
        b.extend_from_slice(&be16(c));
    }
    b
}

/// Script/feature/lookup scaffolding shared by GSUB and GPOS: a DFLT
/// script whose default LangSys enables features `tags[i]`, each
/// referencing exactly one lookup (lookup index = i); the lookups
/// themselves are `(lookup_type, lookup_flag, subtable_bytes)`.
fn layout_table(tags: &[[u8; 4]], lookups: &[(u16, u16, Vec<u8>)]) -> Vec<u8> {
    let mut b = Vec::new();
    b.extend_from_slice(&be16(1)); // majorVersion
    b.extend_from_slice(&be16(0)); // minorVersion
    b.extend_from_slice(&be16(0)); // scriptListOffset (patch @4)
    b.extend_from_slice(&be16(0)); // featureListOffset (patch @6)
    b.extend_from_slice(&be16(0)); // lookupListOffset (patch @8)

    // ScriptList: 1 record "DFLT".
    let script_list = b.len();
    b[4..6].copy_from_slice(&be16(script_list as u16));
    b.extend_from_slice(&be16(1));
    b.extend_from_slice(b"DFLT");
    b.extend_from_slice(&be16(8)); // Script follows the 8-byte list
                                   // Script: defaultLangSys at 4, no LangSysRecords.
    b.extend_from_slice(&be16(4));
    b.extend_from_slice(&be16(0));
    // LangSys: lookupOrder 0, no required feature, features 0..n.
    b.extend_from_slice(&be16(0));
    b.extend_from_slice(&be16(0xFFFF));
    b.extend_from_slice(&be16(tags.len() as u16));
    for i in 0..tags.len() {
        b.extend_from_slice(&be16(i as u16));
    }

    // FeatureList: feature i = tags[i] → lookup i.
    let feature_list = b.len();
    b[6..8].copy_from_slice(&be16(feature_list as u16));
    b.extend_from_slice(&be16(tags.len() as u16));
    let rec_base = b.len();
    for tag in tags {
        b.extend_from_slice(tag);
        b.extend_from_slice(&be16(0)); // patch below
    }
    for (i, _) in tags.iter().enumerate() {
        let feat_off = b.len() - feature_list;
        let patch = rec_base + i * 6 + 4;
        b[patch..patch + 2].copy_from_slice(&be16(feat_off as u16));
        b.extend_from_slice(&be16(0)); // featureParams
        b.extend_from_slice(&be16(1)); // lookupIndexCount
        b.extend_from_slice(&be16(i as u16)); // lookup i
    }

    // LookupList.
    let lookup_list = b.len();
    b[8..10].copy_from_slice(&be16(lookup_list as u16));
    b.extend_from_slice(&be16(lookups.len() as u16));
    let off_base = b.len();
    for _ in lookups {
        b.extend_from_slice(&be16(0)); // patch below
    }
    for (i, (ltype, flag, sub)) in lookups.iter().enumerate() {
        let lookup_off = b.len() - lookup_list;
        let patch = off_base + i * 2;
        b[patch..patch + 2].copy_from_slice(&be16(lookup_off as u16));
        b.extend_from_slice(&be16(*ltype));
        b.extend_from_slice(&be16(*flag));
        b.extend_from_slice(&be16(1)); // subTableCount
        b.extend_from_slice(&be16(8)); // subtable follows the 8-byte header
        b.extend_from_slice(sub);
    }
    b
}

/// GSUB LigatureSubst format 1: 'a' + 'b' → gid 4.
fn gsub_liga_subtable() -> Vec<u8> {
    let mut b = Vec::new();
    b.extend_from_slice(&be16(1)); // format
    b.extend_from_slice(&be16(0)); // coverageOffset (patch @2)
    b.extend_from_slice(&be16(1)); // ligatureSetCount
    b.extend_from_slice(&be16(0)); // ligatureSetOffset[0] (patch @6)
    let cov = b.len();
    b[2..4].copy_from_slice(&be16(cov as u16));
    b.extend_from_slice(&coverage(&[1]));
    let set = b.len();
    b[6..8].copy_from_slice(&be16(set as u16));
    b.extend_from_slice(&be16(1)); // ligatureCount
    b.extend_from_slice(&be16(4)); // ligatureOffset (rel to set)
    b.extend_from_slice(&be16(4)); // ligatureGlyph
    b.extend_from_slice(&be16(2)); // componentCount
    b.extend_from_slice(&be16(2)); // component[1] = gid 2
    b
}

/// GPOS CursivePosFormat1: gid 5 exits at (600, 100), gid 2 enters at
/// (0, -50).
fn gpos_cursive_subtable() -> Vec<u8> {
    let mut b = Vec::new();
    b.extend_from_slice(&be16(1)); // format
    b.extend_from_slice(&be16(0)); // coverageOffset (patch @2)
    b.extend_from_slice(&be16(2)); // entryExitCount
                                   // EntryExit[0] = gid 2: entry, no exit.
    b.extend_from_slice(&be16(0)); // entryAnchorOffset (patch @6)
    b.extend_from_slice(&be16(0)); // exitAnchorOffset = NULL
                                   // EntryExit[1] = gid 5: exit, no entry.
    b.extend_from_slice(&be16(0)); // entryAnchorOffset = NULL
    b.extend_from_slice(&be16(0)); // exitAnchorOffset (patch @12)
    let cov = b.len();
    b[2..4].copy_from_slice(&be16(cov as u16));
    b.extend_from_slice(&coverage(&[2, 5]));
    let entry = b.len();
    b[6..8].copy_from_slice(&be16(entry as u16));
    b.extend_from_slice(&anchor(0, -50));
    let exit = b.len();
    b[12..14].copy_from_slice(&be16(exit as u16));
    b.extend_from_slice(&anchor(600, 100));
    b
}

/// GPOS MarkLigPosFormat1: mark gid 3 (class 0, anchor (50, 20)),
/// ligature gid 4 with two components anchored at (100, 300) and
/// (700, 320).
fn gpos_mark_lig_subtable() -> Vec<u8> {
    let mut b = Vec::new();
    b.extend_from_slice(&be16(1)); // format
    b.extend_from_slice(&be16(0)); // markCoverageOffset (patch @2)
    b.extend_from_slice(&be16(0)); // ligatureCoverageOffset (patch @4)
    b.extend_from_slice(&be16(1)); // markClassCount
    b.extend_from_slice(&be16(0)); // markArrayOffset (patch @8)
    b.extend_from_slice(&be16(0)); // ligatureArrayOffset (patch @10)
    let mark_cov = b.len();
    b[2..4].copy_from_slice(&be16(mark_cov as u16));
    b.extend_from_slice(&coverage(&[3]));
    let lig_cov = b.len();
    b[4..6].copy_from_slice(&be16(lig_cov as u16));
    b.extend_from_slice(&coverage(&[4]));
    // MarkArray: 1 MarkRecord { class 0, anchorOffset }.
    let mark_array = b.len();
    b[8..10].copy_from_slice(&be16(mark_array as u16));
    b.extend_from_slice(&be16(1)); // markCount
    b.extend_from_slice(&be16(0)); // class
    b.extend_from_slice(&be16(6)); // anchorOffset (rel to MarkArray: count + 1 record)
    b.extend_from_slice(&anchor(50, 20));
    // LigatureArray → LigatureAttach (2 components × 1 class).
    let lig_array = b.len();
    b[10..12].copy_from_slice(&be16(lig_array as u16));
    b.extend_from_slice(&be16(1)); // ligatureCount
    b.extend_from_slice(&be16(4)); // ligatureAttachOffset (rel to array)
                                   // LigatureAttach: componentCount 2, anchors per class.
    b.extend_from_slice(&be16(2)); // componentCount
    b.extend_from_slice(&be16(6)); // component[0] class-0 anchor (rel to attach)
    b.extend_from_slice(&be16(12)); // component[1] class-0 anchor
    b.extend_from_slice(&anchor(100, 300));
    b.extend_from_slice(&anchor(700, 320));
    b
}

/// GPOS chained contextual (type 8) format 3: input = gid 5 preceded
/// by gid 2 → nested lookup 3 at input position 0.
fn gpos_chained_context_subtable() -> Vec<u8> {
    let mut b = Vec::new();
    b.extend_from_slice(&be16(3)); // format
    b.extend_from_slice(&be16(1)); // backtrackGlyphCount
    b.extend_from_slice(&be16(0)); // backtrackCoverage[0] (patch @4)
    b.extend_from_slice(&be16(1)); // inputGlyphCount
    b.extend_from_slice(&be16(0)); // inputCoverage[0] (patch @8)
    b.extend_from_slice(&be16(0)); // lookaheadGlyphCount
    b.extend_from_slice(&be16(1)); // seqLookupCount
    b.extend_from_slice(&be16(0)); // sequenceIndex
    b.extend_from_slice(&be16(3)); // lookupListIndex = 3
    let bt = b.len();
    b[4..6].copy_from_slice(&be16(bt as u16));
    b.extend_from_slice(&coverage(&[2]));
    let inp = b.len();
    b[8..10].copy_from_slice(&be16(inp as u16));
    b.extend_from_slice(&coverage(&[5]));
    b
}

/// GPOS SinglePos format 1: gid 5 gains xAdvance +25 (the nested
/// action of the chained context above).
fn gpos_single_subtable() -> Vec<u8> {
    let mut b = Vec::new();
    b.extend_from_slice(&be16(1)); // format
    b.extend_from_slice(&be16(8)); // coverageOffset (fixed: header is 8 bytes)
    b.extend_from_slice(&be16(0x0004)); // valueFormat = X_ADVANCE
    b.extend_from_slice(&25i16.to_be_bytes()); // xAdvance
    b.extend_from_slice(&coverage(&[5]));
    b
}

/// GDEF with a GlyphClassDef: gids 1/2/5 base, 3 mark, 4 ligature.
fn gdef_table() -> Vec<u8> {
    let mut b = Vec::new();
    b.extend_from_slice(&be16(1)); // majorVersion
    b.extend_from_slice(&be16(0)); // minorVersion
    b.extend_from_slice(&be16(12)); // glyphClassDefOffset
    b.extend_from_slice(&be16(0)); // attachListOffset
    b.extend_from_slice(&be16(0)); // ligCaretListOffset
    b.extend_from_slice(&be16(0)); // markAttachClassDefOffset
    b.extend_from_slice(&classdef(&[(1, 2, 1), (3, 3, 3), (4, 4, 2), (5, 5, 1)]));
    b
}

/// Assemble a minimal OTF (CFF2-flavoured, per the sfnt spec) with
/// the shaping tables above. `cursive_rtl` sets the RIGHT_TO_LEFT
/// flag on the cursive lookup.
fn build_font(cursive_rtl: bool) -> Vec<u8> {
    let num_glyphs = 6u16;
    let advances: [u16; 6] = [0, 500, 600, 0, 900, 550];

    let mut head = vec![0u8; 54];
    head[0..4].copy_from_slice(&0x00010000u32.to_be_bytes());
    head[12..16].copy_from_slice(&0x5F0F3CF5u32.to_be_bytes());
    head[18..20].copy_from_slice(&1000u16.to_be_bytes());

    let mut hhea = vec![0u8; 36];
    hhea[0..4].copy_from_slice(&0x00010000u32.to_be_bytes());
    hhea[34..36].copy_from_slice(&num_glyphs.to_be_bytes());

    let mut maxp = vec![0u8; 6];
    maxp[0..4].copy_from_slice(&0x00005000u32.to_be_bytes());
    maxp[4..6].copy_from_slice(&num_glyphs.to_be_bytes());

    let mut hmtx = Vec::new();
    for adv in advances {
        hmtx.extend_from_slice(&adv.to_be_bytes());
        hmtx.extend_from_slice(&0i16.to_be_bytes());
    }

    // cmap format 0: a→1, b→2, c→3, d→5.
    let mut cmap = Vec::new();
    cmap.extend_from_slice(&be16(0));
    cmap.extend_from_slice(&be16(1));
    cmap.extend_from_slice(&be16(0));
    cmap.extend_from_slice(&be16(0));
    cmap.extend_from_slice(&12u32.to_be_bytes());
    cmap.extend_from_slice(&be16(0));
    cmap.extend_from_slice(&be16(262));
    cmap.extend_from_slice(&be16(0));
    let mut ids = [0u8; 256];
    ids[b'a' as usize] = 1;
    ids[b'b' as usize] = 2;
    ids[b'c' as usize] = 3;
    ids[b'd' as usize] = 5;
    cmap.extend_from_slice(&ids);

    let mut name = vec![0u8; 6];
    name[4..6].copy_from_slice(&be16(6));

    // Minimal CFF2 (same shape as tests/cff2_synthetic.rs): shaping
    // never decodes outlines, the table only has to parse.
    let cff2: Vec<u8> = {
        let mut v = Vec::new();
        v.extend_from_slice(&[2, 0, 5, 0, 5]);
        v.push((14 + 139) as u8);
        v.push(17);
        v.push((22 + 139) as u8);
        v.extend_from_slice(&[12, 36]);
        v.extend_from_slice(&[0, 0, 0, 0]);
        v.extend_from_slice(&[0, 0, 0, 1]);
        v.push(1);
        v.extend_from_slice(&[1, 2]);
        v.push(0x01);
        v.extend_from_slice(&[0, 0, 0, 1]);
        v.push(1);
        v.extend_from_slice(&[1, 1]);
        v
    };

    let gsub = layout_table(
        &[*b"liga"],
        &[(4, 0x0008, gsub_liga_subtable())], // IGNORE_MARKS
    );
    let cursive_flag = if cursive_rtl { 0x0001 } else { 0x0000 };
    let gpos = layout_table(
        &[*b"curs", *b"mark", *b"kern"],
        &[
            (3, cursive_flag, gpos_cursive_subtable()),
            (5, 0x0000, gpos_mark_lig_subtable()),
            (8, 0x0000, gpos_chained_context_subtable()),
            (1, 0x0000, gpos_single_subtable()), // nested only
        ],
    );
    let gdef = gdef_table();

    let mut tables: Vec<(&[u8; 4], Vec<u8>)> = vec![
        (b"CFF2", cff2),
        (b"GDEF", gdef),
        (b"GPOS", gpos),
        (b"GSUB", gsub),
        (b"cmap", cmap),
        (b"head", head),
        (b"hhea", hhea),
        (b"hmtx", hmtx),
        (b"maxp", maxp),
        (b"name", name),
    ];
    tables.sort_by(|a, b| a.0.cmp(b.0));

    let n = tables.len() as u16;
    let header_size = 12 + 16 * n as usize;
    let mut offsets = Vec::new();
    let mut cursor = header_size;
    for (_t, payload) in &tables {
        offsets.push(cursor);
        cursor += payload.len();
        while cursor % 4 != 0 {
            cursor += 1;
        }
    }
    let mut out = Vec::with_capacity(cursor);
    out.extend_from_slice(&0x4F54544Fu32.to_be_bytes());
    out.extend_from_slice(&n.to_be_bytes());
    out.extend_from_slice(&[0u8; 6]);
    for (i, (tag, payload)) in tables.iter().enumerate() {
        out.extend_from_slice(*tag);
        out.extend_from_slice(&0u32.to_be_bytes());
        out.extend_from_slice(&(offsets[i] as u32).to_be_bytes());
        out.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    }
    for (i, (_tag, payload)) in tables.iter().enumerate() {
        while out.len() < offsets[i] {
            out.push(0);
        }
        out.extend_from_slice(payload);
    }
    out
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
fn cursive_attachment_joins_exit_to_entry() {
    // "db": d exits at (600, 100), b enters at (0, -50). The advance
    // of d must land the pen on the exit anchor relative to b's entry
    // (600), and b's cross-stream offset lifts to the exit height
    // (100 - (-50) = 150).
    let bytes = build_font(false);
    let font = Font::from_bytes(&bytes).unwrap();
    let run = font.shape("db", &ShapeOptions::default()).unwrap();
    assert_eq!(run[0].glyph, 5);
    assert_eq!(run[1].glyph, 2);
    assert_eq!(run[0].x_advance, 600);
    assert_eq!(run[0].y_offset, 0);
    assert_eq!(run[1].y_offset, 150);
}

#[test]
fn cursive_rtl_flag_adjusts_leading_glyph() {
    // With RIGHT_TO_LEFT set, "the last glyph in a matched input
    // sequence will be positioned on the baseline": b keeps y 0 and
    // d shifts down instead (entry.y - exit.y = -150).
    let bytes = build_font(true);
    let font = Font::from_bytes(&bytes).unwrap();
    let run = font.shape("db", &ShapeOptions::default()).unwrap();
    assert_eq!(run[0].x_advance, 600);
    assert_eq!(run[0].y_offset, -150);
    assert_eq!(run[1].y_offset, 0);
}

#[test]
fn cursive_disabled_leaves_positions() {
    let bytes = build_font(false);
    let font = Font::from_bytes(&bytes).unwrap();
    let opts = ShapeOptions {
        features: vec![oxideav_otf::FeatureSetting::new(*b"curs", 0)],
        ..ShapeOptions::default()
    };
    let run = font.shape("db", &opts).unwrap();
    assert_eq!(run[0].x_advance, 550);
    assert_eq!(run[1].y_offset, 0);
}

#[test]
fn mark_inside_ligature_attaches_to_its_component() {
    // "acb": the liga lookup IGNORE_MARKS-skips the mark, forming the
    // a+b ligature around it; the mark remembers it followed
    // component 0 and takes the component-0 anchor:
    // x = 100 - 50 - adv(lig 900) = -850, y = 300 - 20 = 280.
    let bytes = build_font(false);
    let font = Font::from_bytes(&bytes).unwrap();
    let run = font.shape("acb", &ShapeOptions::default()).unwrap();
    assert_eq!(run.iter().map(|g| g.glyph).collect::<Vec<_>>(), vec![4, 3]);
    // Ligature cluster merges to the first component's cluster.
    assert_eq!(run[0].cluster, 0);
    assert_eq!(run[1].cluster, 1);
    assert_eq!(run[1].x_offset, -850);
    assert_eq!(run[1].y_offset, 280);
    assert_eq!(run[1].x_advance, 0);
}

#[test]
fn mark_after_ligature_attaches_to_last_component() {
    // "abc": the ligature forms from a+b, then the trailing mark has
    // no recorded component and associates with the last (second)
    // component: x = 700 - 50 - 900 = -250, y = 320 - 20 = 300.
    let bytes = build_font(false);
    let font = Font::from_bytes(&bytes).unwrap();
    let run = font.shape("abc", &ShapeOptions::default()).unwrap();
    assert_eq!(run.iter().map(|g| g.glyph).collect::<Vec<_>>(), vec![4, 3]);
    assert_eq!(run[1].x_offset, -250);
    assert_eq!(run[1].y_offset, 300);
}

#[test]
fn chained_context_positioning_applies_nested_single() {
    // "bd": d (input, backtrack b) gains +25 advance through the
    // chained-context type-8 lookup's nested type-1 lookup.
    let bytes = build_font(false);
    let font = Font::from_bytes(&bytes).unwrap();
    let run = font.shape("bd", &ShapeOptions::default()).unwrap();
    assert_eq!(run[0].x_advance, 600);
    assert_eq!(run[1].x_advance, 550 + 25);
    // Without the backtrack glyph the context must not match.
    let alone = font.shape("d", &ShapeOptions::default()).unwrap();
    assert_eq!(alone[0].x_advance, 550);
}

#[test]
fn ligature_component_bookkeeping_survives_typed_views() {
    // Cross-check: the MarkLigPos typed view reports the same anchors
    // the shaper used for the in-pattern mark.
    let bytes = build_font(false);
    let font = Font::from_bytes(&bytes).unwrap();
    let gpos = font.gpos().unwrap();
    let ml = gpos.mark_lig_pos(1, 0).unwrap().unwrap();
    let att = ml.attachment(3, 4, 0).unwrap().unwrap();
    assert_eq!((att.ligature_anchor.x, att.ligature_anchor.y), (100, 300));
    assert_eq!((att.mark_anchor.x, att.mark_anchor.y), (50, 20));
    let att1 = ml.attachment(3, 4, 1).unwrap().unwrap();
    assert_eq!((att1.ligature_anchor.x, att1.ligature_anchor.y), (700, 320));
}
