//! Integration test against Adobe Source Sans 3 Regular (CFF /
//! Type 2 charstrings, SIL OFL v1.1, ~335 KB).
//!
//! This is a coarse "does it actually parse a real OTF" test —
//! we don't compare pixel-perfect outlines (no clean-room rasterizer
//! oracle is in scope for this round), just check that parsing
//! completes, metadata is sensible, and several common glyphs
//! produce non-empty outlines with at least one cubic curve.

use oxideav_otf::{CubicSegment, Font};

const FIXTURE: &[u8] = include_bytes!("fixtures/SourceSans3-Regular.otf");

#[test]
fn parses_source_sans_metadata() {
    let f = Font::from_bytes(FIXTURE).expect("Source Sans 3 parse");
    let family = f.family_name().expect("family");
    assert!(
        family.contains("Source Sans"),
        "unexpected family name: {family:?}"
    );
    // Adobe ships Source Sans 3 with units_per_em = 1000 (the CFF
    // default).
    assert_eq!(f.units_per_em(), 1000);
    assert!(f.glyph_count() > 1500, "got {} glyphs", f.glyph_count());
    assert!(f.ascent() > 0, "ascent {}", f.ascent());
    assert!(f.descent() < 0, "descent {}", f.descent());
}

#[test]
fn glyph_lookup_basic_set() {
    let f = Font::from_bytes(FIXTURE).unwrap();
    for ch in "ABCMagWZ012!".chars() {
        let gid = f
            .glyph_index(ch)
            .unwrap_or_else(|| panic!("missing glyph for {ch:?}"));
        assert!(gid > 0, "got gid 0 for {ch:?}");
        assert!(
            f.glyph_advance(gid) > 0,
            "non-positive advance for {ch:?}: {}",
            f.glyph_advance(gid)
        );
    }
}

#[test]
fn outlines_decode_with_cubic_segments() {
    let f = Font::from_bytes(FIXTURE).unwrap();
    let mut total_curves = 0usize;
    let mut decoded = 0usize;
    // Pick three glyphs known to have curves: 'O' (mostly curves),
    // 'a' (mix of curves + lines), '8' (multiple closed curves).
    for ch in ['O', 'a', '8'] {
        let gid = f.glyph_index(ch).expect("glyph");
        let o = f.glyph_outline(gid).expect("outline decode");
        assert!(!o.is_empty(), "{ch:?} outline empty");
        decoded += 1;
        let curves = o
            .contours
            .iter()
            .flat_map(|c| c.segments.iter())
            .filter(|s| matches!(s, CubicSegment::CurveTo { .. }))
            .count();
        total_curves += curves;
        // Bounds should reflect non-trivial extent.
        assert!(o.bounds.width() > 0.0, "{ch:?} bounds width zero");
        assert!(o.bounds.height() > 0.0, "{ch:?} bounds height zero");
    }
    assert_eq!(decoded, 3);
    assert!(total_curves > 5, "expected curves, got {total_curves}");
}

#[test]
fn many_glyphs_decode_without_panicking() {
    // Walks the ASCII printable range plus a handful of Latin
    // extended codepoints, decoding the outline for every glyph
    // that has a cmap entry. Catches any opcode-coverage gap that
    // a single hand-picked glyph might miss.
    let f = Font::from_bytes(FIXTURE).unwrap();
    let mut decoded = 0;
    for cp in 0x20u32..0x7Fu32 {
        if let Some(gid) = f.glyph_index(char::from_u32(cp).unwrap()) {
            f.glyph_outline(gid).expect("outline decode");
            decoded += 1;
        }
    }
    for cp in [0xC0u32, 0xC1, 0xE9, 0xF1, 0x153] {
        if let Some(gid) = f.glyph_index(char::from_u32(cp).unwrap()) {
            f.glyph_outline(gid).expect("outline decode");
            decoded += 1;
        }
    }
    assert!(
        decoded >= 80,
        "expected >= 80 glyphs decoded, got {decoded}"
    );
}

#[test]
fn glyph_name_for_a() {
    let f = Font::from_bytes(FIXTURE).unwrap();
    let gid = f.glyph_index('A').unwrap();
    // Source Sans uses standard glyph names; A's name should be
    // either the standard "A" SID or a custom one — both cases
    // decode to "A" as a string.
    let name = f.glyph_name(gid).expect("glyph name");
    assert_eq!(name, "A");
}

#[test]
fn ps_name_present() {
    let f = Font::from_bytes(FIXTURE).unwrap();
    let ps = f.ps_name().expect("ps_name");
    assert!(ps.contains("Source") || ps.contains("source"));
}

#[test]
fn table_directory_enumerates_required_tables() {
    let f = Font::from_bytes(FIXTURE).unwrap();
    let tags: Vec<[u8; 4]> = f.table_tags().map(|(t, _)| t).collect();
    for required in [b"head", b"hhea", b"maxp", b"hmtx", b"cmap", b"name"] {
        assert!(
            tags.contains(required),
            "missing required table {:?}",
            std::str::from_utf8(required).unwrap()
        );
    }
    // Source Sans 3 ships with `CFF ` (with the trailing space).
    assert!(tags.contains(b"CFF "), "missing CFF table");
    // And lengths are sane — every reported length should let us slice.
    for (tag, len) in f.table_tags() {
        let data = f.table_data(&tag).expect("table data");
        assert_eq!(data.len() as u32, len, "tag {tag:?} length mismatch");
    }
    assert!(f.has_table(b"CFF "));
    assert!(!f.has_table(b"ZZZZ"));
}

#[test]
fn cff_top_metadata_surfaced() {
    let f = Font::from_bytes(FIXTURE).unwrap();
    let bbox = f.font_bbox();
    // Source Sans 3 Top DICT carries a real FontBBox covering the
    // whole repertoire — width and height should both be positive.
    let width = bbox[2] - bbox[0];
    let height = bbox[3] - bbox[1];
    assert!(width > 0.0, "FontBBox width {width}: {bbox:?}");
    assert!(height > 0.0, "FontBBox height {height}: {bbox:?}");

    // Source Sans 3 Regular is upright, monoline, not fixed pitch.
    assert_eq!(f.italic_angle(), 0.0);
    assert!(!f.is_fixed_pitch());

    // Underline metrics are conventionally negative position, small
    // positive thickness — at minimum, thickness must be positive.
    assert!(
        f.underline_thickness() > 0.0,
        "thickness {}",
        f.underline_thickness()
    );
    assert!(
        f.underline_position() < 0.0,
        "position {}",
        f.underline_position()
    );
}

#[test]
fn glyph_bbox_for_real_glyph_is_non_empty() {
    let f = Font::from_bytes(FIXTURE).unwrap();
    for ch in ['A', 'O', '8', 'g'] {
        let gid = f.glyph_index(ch).expect("glyph");
        let bb = f
            .glyph_bbox(gid)
            .expect("decode ok")
            .unwrap_or_else(|| panic!("{ch:?} bbox unexpectedly None"));
        assert!(bb.width() > 0.0, "{ch:?} width 0: {bb:?}");
        assert!(bb.height() > 0.0, "{ch:?} height 0: {bb:?}");
    }
}

#[test]
fn cff_font_matrix_and_paint_metadata_surfaced() {
    let f = Font::from_bytes(FIXTURE).unwrap();

    // Source Sans 3 is a regular filled OpenType-CFF font (PaintType
    // 0, CharstringType 2). StrokeWidth is only meaningful when
    // PaintType is 2, but the operator may still be present or absent
    // — either way it must surface as a finite f64.
    assert_eq!(f.paint_type(), 0, "Source Sans is a filled font");
    assert_eq!(
        f.charstring_type(),
        2,
        "OpenType CFF always carries Type 2 charstrings"
    );
    assert!(f.stroke_width().is_finite(), "StrokeWidth must be finite");

    // The FontMatrix is conventionally the 1/upem identity. Source
    // Sans 3 has upem == 1000, so the spec-default matrix
    // [0.001, 0, 0, 0.001, 0, 0] applies if no override is present.
    // Whether the font emits an explicit FontMatrix is a font-author
    // choice; either way the surfaced matrix's scale must be
    // approximately 1/upem so that glyph-unit coordinates scale to a
    // 1.0-em user-space square.
    let m = f.font_matrix();
    let upem = f.units_per_em() as f64;
    let scale_x = m[0].abs();
    let scale_y = m[3].abs();
    let expected = 1.0 / upem;
    assert!(
        (scale_x - expected).abs() < 1e-6,
        "FontMatrix[0]={scale_x} should be ~1/upem={expected}"
    );
    assert!(
        (scale_y - expected).abs() < 1e-6,
        "FontMatrix[3]={scale_y} should be ~1/upem={expected}"
    );
    // Off-diagonal shear is conventionally 0 for non-oblique fonts.
    assert!(m[1].abs() < 1e-9, "FontMatrix b shear: {}", m[1]);
    assert!(m[2].abs() < 1e-9, "FontMatrix c shear: {}", m[2]);
}

#[test]
fn cff_metadata_strings_resolve_when_present() {
    // Source Sans 3 has at least a notice in the CFF Top DICT; even
    // if specific strings are absent the lookup must not panic.
    let f = Font::from_bytes(FIXTURE).unwrap();
    // None / Some both fine — just exercise the path.
    let _ = f.weight_name();
    let _ = f.notice();
    let _ = f.copyright();
    let _ = f.version_string();
}

#[test]
fn cff_r176_identity_and_synthetic_operators_do_not_panic() {
    // Source Sans 3 is a modern OpenType-CFF font shipped with a
    // single-font CFF (not a synthetic, not multiple-master), so the
    // synthetic-font operators (SyntheticBase / BaseFontName /
    // BaseFontBlend) are expected to be absent. UniqueID / XUID are
    // legacy PostScript identifiers and are also expected to be
    // omitted in Source Sans 3 per Adobe TN5176 4 Dec 03 Appendix H
    // (XUID deprecated in OpenType-CFF). The point of this test is
    // simply to exercise the accessors against a real font and confirm
    // the parser doesn't panic on either presence or absence.
    let f = Font::from_bytes(FIXTURE).unwrap();
    // None / Some both fine.
    let _ = f.unique_id();
    let xuid = f.xuid();
    // Whatever XUID surfaces (empty or populated), each entry must be
    // a valid i32 (the type guarantee — but exercise the slice to make
    // sure the borrow path is sound).
    for &v in xuid {
        let _ = v;
    }
    assert!(
        f.synthetic_base().is_none(),
        "Source Sans 3 is not a synthetic font, SyntheticBase should be absent"
    );
    let _ = f.postscript();
    assert!(
        f.base_font_name().is_none(),
        "Source Sans 3 has no multiple-master base, BaseFontName should be absent"
    );
    assert!(
        f.base_font_blend().is_empty(),
        "Source Sans 3 has no multiple-master base, BaseFontBlend should be empty"
    );
}

#[test]
fn cff_private_hint_zones_decode_for_real_font() {
    // Source Sans 3 ships hand-tuned hint zones in its CFF Private
    // DICT. We do not bake the exact values into the test (Adobe may
    // re-tune them in a point release) but we assert the qualitative
    // properties every well-formed Latin font must satisfy and the
    // round-183 hint-zone surface must therefore round-trip:
    //
    //   - BlueValues is populated and has an even count (pairs of
    //     bottom-top y).
    //   - All BlueValues are integral and non-decreasing — both
    //     properties hold for the *undeltified* values per TN5176 §15.
    //   - StdHW / StdVW are positive (dominant stem widths).
    //   - BlueScale / BlueShift / BlueFuzz retain plausible Latin-font
    //     values (BlueShift typically 7, BlueFuzz typically 0 or 1,
    //     BlueScale typically near 0.04).
    //   - LanguageGroup is 0 (Latin), ExpansionFactor is the default
    //     0.06, ForceBold is false (Source Sans 3 Regular is not bold).
    let f = Font::from_bytes(FIXTURE).unwrap();
    let h = f.private_hints();

    assert!(
        !h.blue_values.is_empty(),
        "BlueValues empty — Source Sans 3 Regular ships an alignment zone table"
    );
    assert_eq!(
        h.blue_values.len() % 2,
        0,
        "BlueValues must come in (bottom, top) pairs; got {} entries",
        h.blue_values.len()
    );
    // Undeltified values are non-decreasing (deltas are signed but the
    // running sum across a real font's alignment zones is monotone
    // bottom-to-top, since Adobe orders the zones ascending).
    for window in h.blue_values.windows(2) {
        assert!(
            window[0] <= window[1],
            "BlueValues should be monotone after undeltification: {} > {}",
            window[0],
            window[1]
        );
    }
    // Each entry should be at a font-unit granularity (no fractional
    // part).
    for v in &h.blue_values {
        assert!(
            (v - v.round()).abs() < 1e-9,
            "BlueValue {v} is not integral after undeltification"
        );
    }

    let std_hw = h.std_hw.expect("StdHW present");
    let std_vw = h.std_vw.expect("StdVW present");
    assert!(std_hw > 0.0, "StdHW {std_hw} should be positive");
    assert!(std_vw > 0.0, "StdVW {std_vw} should be positive");

    // BlueShift is conventionally 7 unless the font author overrides
    // it; either way it must be a small positive integer.
    assert!(
        h.blue_shift >= 0.0 && h.blue_shift < 256.0,
        "BlueShift {} is implausible",
        h.blue_shift
    );
    assert!(
        h.blue_fuzz >= 0.0 && h.blue_fuzz < 256.0,
        "BlueFuzz {} is implausible",
        h.blue_fuzz
    );
    assert!(
        h.blue_scale > 0.0 && h.blue_scale < 1.0,
        "BlueScale {} should be a positive sub-unit fraction",
        h.blue_scale
    );

    assert_eq!(
        h.language_group, 0,
        "Source Sans 3 is a Latin font; LanguageGroup must be 0"
    );
    assert!(
        !h.force_bold,
        "Source Sans 3 Regular is upright-non-bold; ForceBold must be false"
    );

    // glyph_private_hints on any in-range glyph routes to the same
    // single Private DICT for a non-CID font, so it must return Some
    // and match `private_hints` exactly.
    let gid_a = f.glyph_index('A').unwrap();
    let per_glyph = f.glyph_private_hints(gid_a).expect("per-glyph hints");
    assert_eq!(
        per_glyph, h,
        "non-CID per-glyph hints must equal font-wide hints"
    );
    // Past-end gid returns None (FDSelect would have no entry).
    assert!(f.glyph_private_hints(f.glyph_count()).is_none());
}

#[test]
fn post_table_surfaces_on_real_font() {
    use oxideav_otf::PostFormat;

    let f = Font::from_bytes(FIXTURE).unwrap();
    // OpenType-CFF1 mandates `post` version 3.0 per the spec's
    // "Versions" preamble; Source Sans 3 is OpenType-CFF1.
    let post_fmt = f.post_format().expect("post table present");
    assert_eq!(
        post_fmt,
        PostFormat::V3_0,
        "OpenType-CFF1 must use post version 3.0; got {post_fmt:?}"
    );

    let post = f.post().expect("post borrow");
    assert_eq!(post.raw_version(), 0x0003_0000);
    // Source Sans 3 Regular is upright → italic angle should be 0.
    assert!(
        f.post_italic_angle().unwrap().abs() < 1e-6,
        "Source Sans 3 Regular italic angle: {}",
        f.post_italic_angle().unwrap()
    );
    // Source Sans 3 Regular is proportional → not fixed pitch.
    assert_eq!(f.post_is_fixed_pitch(), Some(false));

    // Underline metrics: position is below baseline (negative),
    // thickness is a small positive value. Both are font-author
    // choices, so we only check the typographic conventions.
    let up = f.post_underline_position().unwrap();
    let ut = f.post_underline_thickness().unwrap();
    assert!(up < 0, "post.underlinePosition {up} should be negative");
    assert!(ut > 0, "post.underlineThickness {ut} should be positive");
    assert!(ut < f.units_per_em() as i16, "thickness > UPEM?");

    // Format 3.0 has no per-glyph name array; `post_glyph_name`
    // returns None for every glyph.
    let gid_a = f.glyph_index('A').unwrap();
    assert!(
        f.post_glyph_name(gid_a).is_none(),
        "post v3.0 must not carry per-glyph names"
    );

    // The `post` table is also reachable through the generic
    // table-directory enumeration; cross-check the two paths agree
    // on size.
    let raw = f.table_data(b"post").expect("post via table_data");
    assert_eq!(raw.len(), 32, "OpenType-CFF1 post 3.0 is exactly 32 bytes");
}
