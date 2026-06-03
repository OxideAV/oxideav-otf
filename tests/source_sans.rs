//! Integration test against Adobe Source Sans 3 Regular (CFF /
//! Type 2 charstrings, SIL OFL v1.1, ~335 KB).
//!
//! This is a coarse "does it actually parse a real OTF" test —
//! we don't compare pixel-perfect outlines (no clean-room rasterizer
//! oracle is in scope for this round), just check that parsing
//! completes, metadata is sensible, and several common glyphs
//! produce non-empty outlines with at least one cubic curve.

use oxideav_otf::{CubicSegment, EmbeddingPermission, Font, GlyphClass, NameId};

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

#[test]
fn os2_decodes_for_source_sans_3() {
    let f = Font::from_bytes(FIXTURE).unwrap();
    let os2 = f.os2().expect("OS/2 present on Source Sans 3");

    // Source Sans 3 Regular ships an `OS/2` v3 table, exactly 96
    // bytes long (the v2/v3/v4 layout — see the spec's "OS/2 Table
    // Formats" preamble).
    assert_eq!(os2.version(), 3);
    assert_eq!(os2.table_len(), 96);
    let raw = f.table_data(b"OS/2").expect("OS/2 via table_data");
    assert_eq!(raw.len(), 96);

    // Common-values mapping (spec §usWeightClass / §usWidthClass):
    // 400 = Regular, 5 = Medium (normal width).
    assert_eq!(f.weight_class(), Some(400));
    assert_eq!(f.width_class(), Some(5));
    assert_eq!(f.width_class_percent(), Some(100.0));

    // fsType = 0 → Installable embedding (no licensing restriction).
    assert_eq!(f.fs_type(), Some(0));
    assert_eq!(
        f.embedding_permission(),
        Some(EmbeddingPermission::Installable)
    );

    // Style bits: regular (bit 6 set), not italic, not bold.
    // USE_TYPO_METRICS (bit 7) was not asserted by the version of
    // Source Sans 3 we ship in fixtures/; the assertion below is the
    // observed value, not the spec-recommendation.
    assert_eq!(f.is_regular(), Some(true));
    assert_eq!(f.is_italic(), Some(false));
    assert_eq!(f.is_bold(), Some(false));
    assert_eq!(f.is_oblique(), Some(false));

    // achVendID = "ADBO" (Adobe's registered vendor tag).
    assert_eq!(f.vendor_id(), Some("ADBO"));

    // PANOSE: bFamilyType = 2 (Latin Text), bSerifStyle = 11 (Normal
    // Sans). Source Sans 3 Regular's PANOSE is `[2, 11, 5, 3, 3, 4,
    // 3, 2, 2, 4]`.
    let panose = f.panose().expect("panose");
    assert_eq!(panose[0], 2);
    assert_eq!(panose[1], 11);

    // Unicode-range bit 0 (Basic Latin, spec table) should be set
    // for any general-purpose Latin font.
    assert!(os2.has_unicode_range_bit(0));

    // Typo metrics: v3+ → all four fields populated, ascender > 0,
    // descender < 0, line gap >= 0, win clipping >= typo metrics.
    let ta = f.typo_ascender().expect("typo asc");
    let td = f.typo_descender().expect("typo desc");
    let lg = f.typo_line_gap().expect("typo gap");
    let wa = f.win_ascent().expect("win asc");
    let wd = f.win_descent().expect("win desc");
    assert!(ta > 0, "typo asc {ta}");
    assert!(td < 0, "typo desc {td}");
    assert!(lg >= 0, "typo gap {lg}");
    assert!(wa as i32 >= ta as i32, "win asc {wa} < typo asc {ta}");
    assert!(
        wd as i32 >= -td as i32,
        "win desc {wd} < -typo desc {}",
        -td
    );

    // First / last char index in cmap-platform-3-encoding-1: 0x20
    // (space) and 0xFFFF (supplementary-plane sentinel).
    assert_eq!(os2.first_char_index(), 0x0020);
    assert_eq!(os2.last_char_index(), 0xFFFF);

    // Code-page range bit 0 = 1252 Latin 1 (spec ulCodePageRange
    // table) — required for any Latin font.
    assert!(os2.has_code_page_bit(0));

    // v2+ extension: sxHeight, sCapHeight, usDefaultChar, usBreakChar,
    // usMaxContext.
    let xh = f.x_height().expect("x-height");
    let ch = f.cap_height().expect("cap-height");
    let upem = f.units_per_em() as i32;
    assert!(xh > 0 && (xh as i32) < upem, "x-height {xh} vs upem {upem}");
    assert!(ch > xh, "cap-height {ch} should exceed x-height {xh}");
    assert_eq!(f.break_char(), Some(0x0020), "break char should be U+0020");
    // usMaxContext: 5 for Source Sans 3 (matches the GSUB/GPOS depth
    // the font carries; bounded above by 64 per recommendation).
    let mc = f.max_context().expect("max context");
    assert!((1..64).contains(&mc), "max context {mc}");

    // v5 optical-size fields absent on this v3 table.
    assert!(!os2.has_optical_size());
    assert_eq!(os2.lower_optical_point_size_twips(), None);
}

#[test]
fn name_table_surfaces_for_source_sans_3() {
    // Source Sans 3 Regular ships standard `name` records (the OFL
    // copyright + family/subfamily + full name + PostScript name +
    // version + manufacturer + designer + URLs + license + license
    // URL + typographic family/subfamily, per Adobe's `nam` build).
    // The name table is the OpenType-canonical metadata source; we
    // assert the spec-required happy-path records resolve, that the
    // round-204 record-iteration API enumerates them in spec-sorted
    // order, and that the version-0 fallback rejects every lang-tag
    // ID per spec ("there are no language-tag records on version 0").
    let f = Font::from_bytes(FIXTURE).unwrap();

    // family_name() should be identical to NameId::FontFamily lookup.
    let family = f.family_name().expect("family name");
    assert!(family.contains("Source Sans"));
    assert_eq!(f.name_string(NameId::FontFamily), Some(family));

    // Subfamily is "Regular" for the Regular weight.
    let subfamily = f.name_string(NameId::FontSubfamily).expect("subfamily");
    assert_eq!(subfamily, "Regular");

    // Full name combines family + subfamily.
    let full = f.full_name().expect("full name");
    assert!(full.contains("Source Sans"));

    // Source Sans ships a Version string starting with "Version".
    let version = f.name_string(NameId::Version).expect("version");
    assert!(
        version.starts_with("Version") || version.starts_with("version"),
        "unexpected version string: {version:?}"
    );

    // PostScript name is ASCII per spec restrictions (codes 33..126).
    let ps = f.name_string(NameId::PostScript).expect("ps name");
    assert!(
        ps.chars().all(|c| c as u32 >= 33 && c as u32 <= 126),
        "PostScript name {ps:?} violates ASCII restriction"
    );

    // Designer / Manufacturer should both be Adobe-style strings.
    let designer = f.designer().expect("designer");
    let manufacturer = f.manufacturer().expect("manufacturer");
    assert!(
        designer.to_lowercase().contains("adobe") || !designer.is_empty(),
        "designer: {designer:?}"
    );
    assert!(!manufacturer.is_empty(), "manufacturer: {manufacturer:?}");

    // Trademark / license / license URL / vendor URL — exercise the
    // accessors without baking exact strings (Adobe may re-tune them).
    let _ = f.trademark();
    let _ = f.license();
    let _ = f.license_url();
    let _ = f.vendor_url();
    let _ = f.designer_url();
    let _ = f.description();

    // Source Sans 3 emits at least one of the typographic-family
    // names (it is part of the Source family).
    let _ = f.typographic_family();
    let _ = f.typographic_subfamily();

    // unique_font_id (name ID 3) is distinct from the CFF Top DICT's
    // UniqueID integer; both accessors must be reachable without
    // collision.
    let _ = f.unique_font_id();
    let _ = f.unique_id();

    // Source Sans 3 is not a variable font; the variations PS prefix
    // is expected to be absent. The accessor must still return None
    // without panicking.
    assert!(f.variations_ps_name_prefix().is_none());

    // Records iteration. The spec mandates that records are sorted by
    // (platformID, encodingID, languageID, nameID) ascending; our
    // iterator surfaces them in disk order, so the same sort must
    // hold here.
    let name = f.name();
    let recs: Vec<_> = name.records().collect();
    assert!(!recs.is_empty(), "name table has zero records");
    assert_eq!(recs.len() as u16, name.record_count());
    for window in recs.windows(2) {
        let a = window[0];
        let b = window[1];
        let ka = (a.platform_id, a.encoding_id, a.language_id, a.name_id_raw);
        let kb = (b.platform_id, b.encoding_id, b.language_id, b.name_id_raw);
        assert!(
            ka <= kb,
            "name records out of spec sort order: {ka:?} > {kb:?}"
        );
    }

    // Every standard name record (raw < 26) must decode to a NameId
    // variant.
    let mut hit_family = false;
    for r in &recs {
        if let Some(nid) = NameId::from_raw(r.name_id_raw) {
            assert_eq!(nid.to_raw(), r.name_id_raw);
            if nid == NameId::FontFamily {
                hit_family = true;
            }
        }
    }
    assert!(hit_family, "no FontFamily record in Source Sans 3 name");

    // Source Sans 3 ships a version-0 name table (the v1 lang-tag
    // mechanism is uncommon outside multi-script CJK fonts). The
    // version-0 lang_tag accessor must always return None, regardless
    // of the queried ID.
    if f.name_version() == 0 {
        assert_eq!(f.name_lang_tag(0x8000), None);
        assert_eq!(f.name_lang_tag(0xFFFF), None);
        assert_eq!(name.lang_tag_count(), 0);
    }
}

#[test]
fn agl_round_trip_on_real_font() {
    // Adobe Glyph List integration on a real Adobe CFF font. Source
    // Sans 3's CFF charset stores PostScript glyph names directly,
    // so any AGL name reachable via `cmap` must round-trip through
    // both `glyph_id_from_agl_name` and the AGL reverse path.
    let f = Font::from_bytes(FIXTURE).unwrap();

    // Basic Latin uppercase / lowercase letters all have AGL names
    // that match their character, and Source Sans 3 supports them
    // all.
    for (name, expected_char) in [("A", 'A'), ("Z", 'Z'), ("a", 'a'), ("z", 'z')] {
        let gid_via_agl = f
            .glyph_id_from_agl_name(name)
            .unwrap_or_else(|| panic!("AGL→GID failed for {name}"));
        let gid_via_cmap = f.glyph_index(expected_char).unwrap();
        assert_eq!(
            gid_via_agl, gid_via_cmap,
            "AGL gid for {name} ({gid_via_agl}) != cmap gid ({gid_via_cmap})"
        );
        // Reverse: AGL name for the glyph (Source Sans 3 CFF charset
        // stores the same PostScript names verbatim).
        let back = f.agl_glyph_name(gid_via_agl).expect("agl_glyph_name");
        assert_eq!(back, name);
    }
}

#[test]
fn agl_glyph_name_prefers_cff_charset() {
    // Source Sans 3 is a CFF1 font, so `agl_glyph_name` must surface
    // the CFF charset → Strings name (the font's authored
    // PostScript name) for every glyph in range, not the AGL
    // fallback.
    let f = Font::from_bytes(FIXTURE).unwrap();
    for ch in "ABCabc012".chars() {
        let gid = f.glyph_index(ch).unwrap();
        let cff_name = f.glyph_name(gid).unwrap();
        let agl_name = f.agl_glyph_name(gid).unwrap();
        assert_eq!(
            cff_name, agl_name,
            "agl_glyph_name must prefer CFF charset over AGL fallback for {ch}"
        );
    }
}

#[test]
fn agl_lookup_missing_name_returns_none() {
    let f = Font::from_bytes(FIXTURE).unwrap();
    // A name that's not in the AGL must surface None at the
    // glyph_id_from_agl_name path (no codepoint to translate).
    assert!(f.glyph_id_from_agl_name("not_a_real_glyph_xyz").is_none());
    // AGL names that exist but aren't encoded in the font's cmap
    // also surface None. CJK ideograph names in AGL aren't covered
    // by Source Sans 3.
    assert!(f.glyph_id_from_agl_name("ahiragana").is_none());
}

#[test]
fn agl_glyph_name_for_out_of_range_glyph_is_none() {
    let f = Font::from_bytes(FIXTURE).unwrap();
    let max = f.glyph_count();
    assert_eq!(f.agl_glyph_name(max), None);
    assert_eq!(f.agl_glyph_name(u16::MAX), None);
}

#[test]
fn gdef_table_parses_and_classifies_glyphs() {
    let f = Font::from_bytes(FIXTURE).unwrap();
    let gdef = f.gdef().expect("Source Sans 3 ships a GDEF table");
    // Source Sans 3 Regular emits a v1.0 GDEF.
    assert_eq!(gdef.version(), (1, 0));
    assert!(!gdef.has_mark_glyph_sets());
    assert!(!gdef.has_item_var_store());

    // The font ships a GlyphClassDef sub-table; AttachList and
    // LigCaretList are absent (the v1.0 minor=0 header has all four
    // optional Offset16 fields, but only GlyphClassDef and
    // MarkAttachClassDef are populated in this font).
    assert!(gdef.glyph_class_def().is_some());
    assert!(gdef.attach_list().is_none());
    assert!(gdef.lig_caret_list().is_none());
    assert!(gdef.mark_attach_class_def().is_some());

    // Every ASCII letter is a single-character spacing base glyph in
    // any well-formed Latin font.
    for ch in "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz".chars() {
        let gid = f
            .glyph_index(ch)
            .unwrap_or_else(|| panic!("missing {ch:?}"));
        assert_eq!(
            f.glyph_class(gid),
            Some(GlyphClass::Base),
            "expected Base for {ch:?} (gid {gid})"
        );
        // Mark-attach class is always 0 for base glyphs.
        assert_eq!(f.mark_attach_class(gid), 0);
    }

    // Spec count guarantee: every glyph in the font surfaces a class
    // value (either an assigned 1..=4 or the implicit 0). Walk the
    // whole font and tally each class to confirm every spec class
    // (Base / Ligature / Mark / Component) is represented at least
    // once — Source Sans 3 carries Latin diacritics (Mark), `fi`-style
    // ligatures (Ligature), and CFF seac component parts (Component).
    let mut bases = 0u32;
    let mut ligatures = 0u32;
    let mut marks = 0u32;
    let mut components = 0u32;
    for gid in 0..f.glyph_count() {
        match f.glyph_class(gid) {
            Some(GlyphClass::Base) => bases += 1,
            Some(GlyphClass::Ligature) => ligatures += 1,
            Some(GlyphClass::Mark) => marks += 1,
            Some(GlyphClass::Component) => components += 1,
            None => {}
        }
    }
    assert!(bases > 0, "expected at least one base glyph");
    assert!(ligatures > 0, "expected at least one ligature glyph");
    assert!(marks > 0, "expected at least one mark glyph");
    assert!(components > 0, "expected at least one component glyph");
}

#[test]
fn gdef_coverage_index_is_dense_when_set() {
    use oxideav_otf::Coverage;

    // Build a synthetic Coverage table and confirm the public API
    // routes through the same parser the GDEF accessors use.
    let mut raw = Vec::new();
    raw.extend_from_slice(&1u16.to_be_bytes());
    raw.extend_from_slice(&3u16.to_be_bytes());
    raw.extend_from_slice(&5u16.to_be_bytes());
    raw.extend_from_slice(&8u16.to_be_bytes());
    raw.extend_from_slice(&13u16.to_be_bytes());
    let cov = Coverage::parse(&raw).unwrap();
    assert_eq!(cov.index_of(8), Some(1));
    assert!(cov.contains(13));
    assert!(!cov.contains(7));
    let items: Vec<_> = cov.iter().collect();
    assert_eq!(items, vec![(5, 0), (8, 1), (13, 2)]);

    // And confirm the round-trip works on Source Sans 3's actual
    // GlyphClassDef structure: the table is format 2 (range-encoded)
    // and surfaces ASCII letters as class 1 (= Base).
    let f = Font::from_bytes(FIXTURE).unwrap();
    let cd = f.gdef().unwrap().glyph_class_def().unwrap();
    assert_eq!(cd.format(), 2);
    // The class number returned for any uncovered glyph is 0.
    assert_eq!(cd.class_of(u16::MAX), 0);
    // 'A' is covered and is class 1 (Base).
    let a_gid = f.glyph_index('A').unwrap();
    assert_eq!(cd.class_of(a_gid), 1);
}
