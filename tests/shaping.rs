//! End-to-end shaping tests over the Source Sans 3 fixture.
//!
//! Expected glyph IDs / advances / offsets were produced by running an
//! independent system-installed shaping engine as a black-box
//! validator over the same fixture and transcribing its output; the
//! table data itself is cross-checked against the crate's own typed
//! GSUB/GPOS views inside the tests where noted.

use oxideav_otf::{FeatureSetting, Font, ShapeOptions, ShapedGlyph};

fn fixture() -> Vec<u8> {
    std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/SourceSans3-Regular.otf"
    ))
    .expect("fixture font present")
}

fn shape(text: &str, options: &ShapeOptions) -> Vec<ShapedGlyph> {
    let bytes = fixture();
    let font = Font::from_bytes(&bytes).unwrap();
    font.shape(text, options).unwrap()
}

fn glyphs(run: &[ShapedGlyph]) -> Vec<u16> {
    run.iter().map(|g| g.glyph).collect()
}

fn clusters(run: &[ShapedGlyph]) -> Vec<u32> {
    run.iter().map(|g| g.cluster).collect()
}

fn advances(run: &[ShapedGlyph]) -> Vec<i32> {
    run.iter().map(|g| g.x_advance).collect()
}

fn features(list: &[([u8; 4], u32)]) -> ShapeOptions {
    ShapeOptions {
        features: list
            .iter()
            .map(|&(tag, value)| FeatureSetting::new(tag, value))
            .collect(),
        ..ShapeOptions::default()
    }
}

// ---------------------------------------------------------------------------
// GSUB — ligature substitution (liga, on by default)
// ---------------------------------------------------------------------------

#[test]
fn liga_forms_ff_ligature() {
    // "office" → o f_f i c e; the f+f pair ligates, the trailing i
    // stays (Source Sans has no f_f_i ligature under liga — the f_f
    // wins as the longest match in its LigatureSet preference order).
    let run = shape("office", &ShapeOptions::default());
    assert_eq!(glyphs(&run), vec![42, 687, 36, 30, 32]);
    // Ligature cluster = smallest component cluster (chars 1+2).
    assert_eq!(clusters(&run), vec![0, 1, 3, 4, 5]);
    assert_eq!(advances(&run), vec![542, 577, 246, 435, 496]);
}

#[test]
fn liga_disabled_keeps_components() {
    let run = shape("office", &features(&[(*b"liga", 0)]));
    assert_eq!(glyphs(&run), vec![42, 33, 33, 36, 30, 32]);
    assert_eq!(advances(&run), vec![542, 292, 292, 246, 435, 496]);
}

#[test]
fn liga_ffi_prefers_longest_match() {
    // f f i: the f_f ligature forms first (LigatureSet preference
    // order), leaving i.
    let run = shape("ffi", &ShapeOptions::default());
    assert_eq!(glyphs(&run), vec![687, 36]);
    assert_eq!(clusters(&run), vec![0, 2]);
}

#[test]
fn dlig_discretionary_ligature_disabled_by_default() {
    let default = shape("fj", &ShapeOptions::default());
    assert_eq!(glyphs(&default), vec![33, 37]);
}

// ---------------------------------------------------------------------------
// GSUB — single substitution features
// ---------------------------------------------------------------------------

#[test]
fn smcp_small_caps() {
    let run = shape("abc", &features(&[(*b"smcp", 1)]));
    assert_eq!(glyphs(&run), vec![1498, 1499, 1500]);
    assert_eq!(advances(&run), vec![470, 526, 503]);
}

#[test]
fn zero_slashed_zero() {
    let run = shape("0", &features(&[(*b"zero", 1)]));
    assert_eq!(glyphs(&run), vec![1344]);
    assert_eq!(advances(&run), vec![498]);
}

#[test]
fn sups_superscript_digits() {
    let run = shape("123", &features(&[(*b"sups", 1)]));
    assert_eq!(glyphs(&run), vec![1975, 1976, 1977]);
    assert_eq!(advances(&run), vec![367, 367, 367]);
}

#[test]
fn onum_oldstyle_figures() {
    let run = shape("12", &features(&[(*b"onum", 1)]));
    assert_eq!(glyphs(&run), vec![1358, 1359]);
    assert_eq!(advances(&run), vec![497, 497]);
}

// ---------------------------------------------------------------------------
// GSUB — alternate substitution (salt / aalt with alternate index)
// ---------------------------------------------------------------------------

#[test]
fn salt_stylistic_alternate() {
    let run = shape("a", &features(&[(*b"salt", 1)]));
    assert_eq!(glyphs(&run), vec![739]);
    assert_eq!(advances(&run), vec![513]);
}

#[test]
fn aalt_selects_numbered_alternate() {
    // aalt mixes single substitutions (selector 1) and AlternateSets
    // (selector >= 2 picks the n'th alternate).
    let first = shape("a", &features(&[(*b"aalt", 1)]));
    assert_eq!(glyphs(&first), vec![710]);
    let second = shape("a", &features(&[(*b"aalt", 2)]));
    assert_eq!(glyphs(&second), vec![739]);
}

// ---------------------------------------------------------------------------
// GSUB — chained contextual substitution (frac / ordn)
// ---------------------------------------------------------------------------

#[test]
fn frac_builds_fraction_via_chained_context() {
    // frac drives numerator/denominator forms through chained
    // contextual lookups with nested single substitutions.
    let run = shape("1/2", &features(&[(*b"frac", 1)]));
    assert_eq!(glyphs(&run), vec![2023, 2139, 2010]);
    assert_eq!(advances(&run), vec![367, 86, 367]);
}

#[test]
fn ordn_ordinal_context() {
    // "No" with ordn: the o after an uppercase N takes the ordinal
    // form via a contextual substitution.
    let run = shape("No", &features(&[(*b"ordn", 1)]));
    assert_eq!(glyphs(&run), vec![15, 2079]);
    assert_eq!(advances(&run), vec![647, 365]);
}

// ---------------------------------------------------------------------------
// GPOS — pair adjustment (kern, on by default)
// ---------------------------------------------------------------------------

#[test]
fn kern_pair_adjustments() {
    let run = shape("AVATAR To", &ShapeOptions::default());
    assert_eq!(glyphs(&run), vec![2, 23, 2, 21, 2, 19, 1, 21, 42]);
    assert_eq!(
        advances(&run),
        vec![530, 501, 489, 496, 544, 569, 200, 470, 542]
    );
}

#[test]
fn kern_disabled_uses_plain_advances() {
    let run = shape("AVATAR To", &features(&[(*b"kern", 0)]));
    assert_eq!(
        advances(&run),
        vec![544, 515, 544, 536, 544, 569, 200, 536, 542]
    );
}

#[test]
fn kern_to_pair() {
    let kerned = shape("To", &ShapeOptions::default());
    assert_eq!(advances(&kerned), vec![470, 542]);
    let plain = shape("To", &features(&[(*b"kern", 0)]));
    assert_eq!(advances(&plain), vec![536, 542]);
}

// ---------------------------------------------------------------------------
// Cross-checks against the crate's own table views
// ---------------------------------------------------------------------------

#[test]
fn kern_matches_gpos_pair_value() {
    // The shaped advance delta for "To" must equal the GPOS pair
    // adjustment the typed PairPos view reports for (T, o).
    let bytes = fixture();
    let font = Font::from_bytes(&bytes).unwrap();
    let t = font.glyph_index('T').unwrap();
    let o = font.glyph_index('o').unwrap();
    let base = font.glyph_advance(t) as i32;

    let gpos = font.gpos().unwrap();
    let mut delta = None;
    'outer: for i in 0..gpos.lookup_count() {
        let l = gpos.lookup(i).unwrap();
        if l.lookup_type() != oxideav_otf::GPOS_LOOKUP_TYPE_EXTENSION {
            continue;
        }
        for s in 0..l.subtable_count() {
            let ext = gpos.extension_pos(i, s).unwrap().unwrap();
            if ext.extension_lookup_type() != oxideav_otf::GPOS_LOOKUP_TYPE_PAIR {
                continue;
            }
            let pp = ext.as_pair_pos().unwrap();
            if let Some(Ok(pv)) = pp.pair(t, o) {
                delta = Some(pv.first.x_advance as i32);
                break 'outer;
            }
        }
    }
    let delta = delta.expect("fixture kerns T/o");
    let run = shape("To", &ShapeOptions::default());
    assert_eq!(run[0].x_advance, base + delta);
}

#[test]
fn unmapped_character_shapes_to_notdef() {
    // U+0FFF is unmapped in the fixture; it must shape to glyph 0.
    let run = shape("\u{0FFF}", &ShapeOptions::default());
    assert_eq!(glyphs(&run), vec![0]);
}

#[test]
fn empty_text_shapes_to_empty_run() {
    let run = shape("", &ShapeOptions::default());
    assert!(run.is_empty());
}

#[test]
fn explicit_script_language_selection() {
    // Explicit latn + an undefined language tag falls back to the
    // default language system: identical to the default result.
    let explicit = shape(
        "office",
        &ShapeOptions {
            script: Some(*b"latn"),
            language: Some(*b"XXX "),
            ..ShapeOptions::default()
        },
    );
    let default = shape("office", &ShapeOptions::default());
    assert_eq!(explicit, default);
}

// ---------------------------------------------------------------------------
// GPOS — mark attachment (mark / mkmk, on by default)
// ---------------------------------------------------------------------------

#[test]
fn mark_to_base_combining_acute() {
    // T + U+0301 (no precomposed form exists, so the mark survives as
    // its own glyph and GPOS mark-to-base positions it).
    let run = shape("T\u{0301}", &ShapeOptions::default());
    assert_eq!(glyphs(&run), vec![21, 2304]);
    assert_eq!(advances(&run), vec![536, 0]);
    assert_eq!(run[1].x_offset, -269);
    assert_eq!(run[1].y_offset, 0);
}

#[test]
fn mark_attachment_between_ligature_components_blocks_liga() {
    // f + U+0301 + i: the combining mark sits between f and i; the
    // fixture's liga lookup does not set IGNORE_MARKS, so no fi
    // ligature forms, and the mark attaches to the f.
    let run = shape("f\u{0301}i", &ShapeOptions::default());
    assert_eq!(glyphs(&run), vec![33, 2304, 36]);
    assert_eq!(clusters(&run), vec![0, 1, 2]);
    assert_eq!(run[1].x_offset, -74);
    assert_eq!(run[1].y_offset, 50);
    assert_eq!(advances(&run), vec![292, 0, 246]);
}

#[test]
fn mark_stacking_two_acutes() {
    // T + U+0301 + U+0301: both marks position over the T (the
    // fixture's mkmk data yields the same offset for the second).
    let run = shape("T\u{0301}\u{0301}", &ShapeOptions::default());
    assert_eq!(glyphs(&run), vec![21, 2304, 2304]);
    assert_eq!(run[1].x_offset, -269);
    assert_eq!(run[2].x_offset, -269);
    assert_eq!(advances(&run), vec![536, 0, 0]);
}

#[test]
fn mark_after_ligature_attaches_without_reforming() {
    // f + f + U+0301 + i: the f_f ligature forms (both components
    // precede the mark), the mark follows the ligature, i remains.
    let run = shape("ff\u{0301}i", &ShapeOptions::default());
    assert_eq!(glyphs(&run), vec![687, 2304, 36]);
    assert_eq!(advances(&run), vec![577, 0, 246]);
}

#[test]
fn mark_to_base_matches_typed_anchor_views() {
    // Cross-check the shaped offset against the MarkBasePos typed
    // view: offset = base_anchor - mark_anchor - base_advance.
    let bytes = fixture();
    let font = Font::from_bytes(&bytes).unwrap();
    let t = font.glyph_index('T').unwrap();
    let acute = 2304u16; // U+0301 mark glyph (from the shaped run)
    let gpos = font.gpos().unwrap();
    let mut expected = None;
    'outer: for i in 0..gpos.lookup_count() {
        let l = gpos.lookup(i).unwrap();
        if l.lookup_type() != oxideav_otf::GPOS_LOOKUP_TYPE_MARK_TO_BASE {
            continue;
        }
        for s in 0..l.subtable_count() {
            let mb = gpos.mark_base_pos(i, s).unwrap().unwrap();
            if let Some(Ok(att)) = mb.attachment(acute, t) {
                expected = Some((
                    att.base_anchor.x as i32
                        - att.mark_anchor.x as i32
                        - font.glyph_advance(t) as i32,
                    att.base_anchor.y as i32 - att.mark_anchor.y as i32,
                ));
                break 'outer;
            }
        }
    }
    let (ex, ey) = expected.expect("fixture attaches acute to T");
    let run = shape("T\u{0301}", &ShapeOptions::default());
    assert_eq!((run[1].x_offset, run[1].y_offset), (ex, ey));
}

#[test]
fn mark_positioning_disabled() {
    let run = shape("T\u{0301}", &features(&[(*b"mark", 0)]));
    assert_eq!(run[1].x_offset, 0);
    assert_eq!(run[1].y_offset, 0);
}

// ---------------------------------------------------------------------------
// Wholesale paragraph comparison against the black-box validator
// ---------------------------------------------------------------------------

#[test]
fn paragraph_matches_reference_shaper() {
    // Full default-feature shaping of a mixed run (ligatures, kerning
    // around punctuation and capitals, digits, the percent sign);
    // glyph IDs, clusters, and advances all transcribed from the
    // independent shaping binary.
    let run = shape(
        "The quick brown fox flies off with waffles; VAT: 71%",
        &ShapeOptions::default(),
    );
    let expected: &[(u16, u32, i32)] = &[
        (21, 0, 536),
        (35, 1, 544),
        (32, 2, 496),
        (1, 3, 200),
        (44, 4, 555),
        (48, 5, 544),
        (36, 6, 246),
        (30, 7, 456),
        (38, 8, 495),
        (1, 9, 200),
        (29, 10, 553),
        (45, 11, 337),
        (42, 12, 538),
        (50, 13, 719),
        (41, 14, 547),
        (1, 15, 200),
        (33, 16, 282),
        (42, 17, 525),
        (51, 18, 446),
        (1, 19, 200),
        (33, 20, 292),
        (39, 21, 255),
        (36, 22, 246),
        (32, 23, 496),
        (46, 24, 419),
        (1, 25, 200),
        (42, 26, 542),
        (687, 27, 577),
        (1, 29, 200),
        (50, 30, 719),
        (36, 31, 246),
        (47, 32, 338),
        (35, 33, 544),
        (1, 34, 200),
        (50, 35, 709),
        (28, 36, 504),
        (687, 37, 577),
        (39, 39, 255),
        (32, 40, 496),
        (46, 41, 419),
        (1390, 42, 249),
        (1, 43, 200),
        (23, 44, 501),
        (2, 45, 489),
        (21, 46, 516),
        (1389, 47, 249),
        (1, 48, 200),
        (1340, 49, 497),
        (1334, 50, 497),
        (2140, 51, 824),
    ];
    assert_eq!(run.len(), expected.len());
    for (i, (g, e)) in run.iter().zip(expected).enumerate() {
        assert_eq!(
            (g.glyph, g.cluster, g.x_advance),
            *e,
            "glyph {i} diverges from the reference shaper"
        );
    }
}
