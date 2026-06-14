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

#[test]
fn gsub_header_and_lookup_count() {
    let f = Font::from_bytes(FIXTURE).unwrap();
    let g = f.gsub().expect("Source Sans 3 carries a GSUB table");
    // Source Sans 3's GSUB table is version 1.0 (no FeatureVariations).
    assert_eq!(g.version(), (1, 0));
    assert!(!g.has_feature_variations());
    assert_eq!(g.feature_variations_offset(), 0);
    // Sanity: at least one script, one feature, one lookup.
    assert!(g.script_count() >= 1);
    assert!(g.feature_count() >= 1);
    assert!(g.lookup_count() >= 1);
}

#[test]
fn gsub_default_script_resolves() {
    let f = Font::from_bytes(FIXTURE).unwrap();
    let g = f.gsub().unwrap();
    let scripts = g.script_list().unwrap();
    // Every shaper expects a DFLT script to exist for fallback.
    let mut found_dflt = false;
    let mut found_latn = false;
    for (tag, _) in scripts.iter() {
        if &tag == b"DFLT" {
            found_dflt = true;
        }
        if &tag == b"latn" {
            found_latn = true;
        }
    }
    assert!(found_dflt, "Source Sans 3 should ship a DFLT script");
    assert!(found_latn, "Source Sans 3 should ship a latn script");

    let dflt = g.find_script(b"DFLT").unwrap();
    assert!(dflt.has_default_lang_sys());
    let default_lang = dflt.default_lang_sys().unwrap().unwrap();
    // Every well-formed font hooks at least one feature off DFLT/dflt.
    assert!(default_lang.feature_count() >= 1);
}

#[test]
fn gsub_features_are_tagged() {
    let f = Font::from_bytes(FIXTURE).unwrap();
    let g = f.gsub().unwrap();
    let feats = g.feature_list().unwrap();
    // Walk the whole list and confirm every record yields a 4-byte
    // tag that maps to a parseable Feature record.
    for (tag, feat_res) in feats.iter() {
        let feat = feat_res.expect("FeatureList record must parse");
        let _ = tag;
        // Every feature points at one or more lookups; an empty
        // Feature is legal but unusual.
        for idx in feat.lookup_indices() {
            assert!(idx < g.lookup_count());
        }
    }
}

#[test]
fn gsub_single_subst_decodes_every_type_1_lookup() {
    // Walk every GSUB lookup; for the type-1 lookups (single
    // substitution), decode the subtable through the typed
    // [`SingleSubst`] view and verify the spec's invariants on the
    // result. Source Sans 3 ships 57 type-1 lookups split between
    // SingleSubstFormat1 (12) and SingleSubstFormat2 (45), which
    // exercises both decode paths against real-font byte windows.
    use oxideav_otf::SingleSubst;

    let f = Font::from_bytes(FIXTURE).unwrap();
    let g = f.gsub().expect("Source Sans 3 carries a GSUB table");
    let n = f.glyph_count();

    let mut total_subtables = 0;
    let mut fmt1_subtables = 0;
    let mut fmt2_subtables = 0;
    let mut total_pairs = 0usize;

    for i in 0..g.lookup_count() {
        let l = g.lookup(i).unwrap();
        if l.lookup_type() != 1 {
            continue;
        }
        for s in 0..l.subtable_count() {
            let ss: SingleSubst<'_> = g
                .single_subst(i, s)
                .unwrap_or_else(|| panic!("lookup {i} sub {s} missing"))
                .unwrap_or_else(|e| panic!("lookup {i} sub {s} decode: {e:?}"));
            total_subtables += 1;
            match ss.format() {
                1 => fmt1_subtables += 1,
                2 => fmt2_subtables += 1,
                f => panic!("lookup {i} sub {s} unknown format {f}"),
            }
            // Walk the iterator. The spec requires every covered glyph
            // to map to *some* output glyph; every output must be a
            // valid glyph ID for this font (< maxp.numGlyphs).
            let mut last_input: Option<u32> = None;
            for (input, output) in ss.iter() {
                // Coverage iteration is sorted ascending.
                if let Some(prev) = last_input {
                    assert!(
                        (input as u32) > prev,
                        "Coverage iter not ascending at lookup {i} sub {s}: {prev} -> {input}",
                    );
                }
                last_input = Some(input as u32);
                // Both input and output index real glyph rows.
                assert!(
                    (input as u32) < n as u32,
                    "input glyph {input} >= numGlyphs {n} at lookup {i} sub {s}",
                );
                assert!(
                    (output as u32) < n as u32,
                    "output glyph {output} >= numGlyphs {n} at lookup {i} sub {s}",
                );
                // The point-lookup must agree with the iterator.
                assert_eq!(
                    ss.substitute(input),
                    Some(output),
                    "substitute(input) disagrees with iter at lookup {i} sub {s}",
                );
                total_pairs += 1;
            }
            // A glyph guaranteed NOT to be in this subtable: the
            // synthetic glyph ID `n` is past the end of the font.
            assert_eq!(ss.substitute(n), None);
        }
    }

    // Source Sans 3 historically ships at least 50 type-1 lookups and
    // both formats appear; loosen the bounds slightly so a future
    // Adobe re-cut does not regress the test.
    assert!(
        total_subtables >= 50,
        "expected >=50 type-1 subtables, got {total_subtables}",
    );
    assert!(
        fmt1_subtables >= 5,
        "expected >=5 SingleSubstFormat1 subtables, got {fmt1_subtables}",
    );
    assert!(
        fmt2_subtables >= 30,
        "expected >=30 SingleSubstFormat2 subtables, got {fmt2_subtables}",
    );
    // Every type-1 lookup contributes at least one substitution pair.
    assert!(total_pairs >= total_subtables);
}

#[test]
fn gsub_multiple_subst_decodes_every_type_2_lookup() {
    // Walk every GSUB lookup; for the type-2 lookups (multiple
    // substitution), decode the subtable through the typed
    // [`MultipleSubst`] view and verify the spec's invariants:
    //
    // * format == 1 (the only defined MultipleSubst format)
    // * sequenceCount == coverage.len() (parser invariant, but
    //   re-checked here through the public accessors)
    // * every Sequence's glyphCount >= 1 (spec prohibits deletion)
    // * every substitute glyph fits inside maxp.numGlyphs
    // * iter() walks Coverage in ascending order
    //
    // Source Sans 3 historically ships 2 type-2 lookups: one large
    // subtable (~407 sequences, mark-decomposition for combining
    // diacritics) and one small subtable (~11 sequences, a small
    // ligature decomposition). The bounds below are loose enough to
    // tolerate vendor adjustments across releases.
    use oxideav_otf::MultipleSubst;

    let f = Font::from_bytes(FIXTURE).unwrap();
    let g = f.gsub().expect("Source Sans 3 carries a GSUB table");
    let n = f.glyph_count();

    let mut total_subtables = 0;
    let mut total_sequences = 0usize;
    let mut total_substitute_glyphs = 0usize;

    for i in 0..g.lookup_count() {
        let l = g.lookup(i).unwrap();
        if l.lookup_type() != 2 {
            continue;
        }
        for s in 0..l.subtable_count() {
            let ms: MultipleSubst<'_> = g
                .multiple_subst(i, s)
                .unwrap_or_else(|| panic!("lookup {i} sub {s} missing"))
                .unwrap_or_else(|e| panic!("lookup {i} sub {s} decode: {e:?}"));
            assert_eq!(ms.format(), 1, "lookup {i} sub {s} format != 1");
            total_subtables += 1;

            // Coverage iter is sorted ascending; each Coverage glyph
            // maps to its own Sequence.
            let mut last_input: Option<u32> = None;
            for (input, seq_res) in ms.iter() {
                if let Some(prev) = last_input {
                    assert!(
                        (input as u32) > prev,
                        "Coverage iter not ascending at lookup {i} sub {s}: {prev} -> {input}",
                    );
                }
                last_input = Some(input as u32);
                assert!(
                    (input as u32) < n as u32,
                    "input glyph {input} >= numGlyphs {n} at lookup {i} sub {s}",
                );
                let seq = seq_res.unwrap_or_else(|e| {
                    panic!("Sequence decode at lookup {i} sub {s} input {input}: {e:?}")
                });
                let gc = seq.glyph_count();
                assert!(
                    gc >= 1,
                    "glyphCount = 0 (deletion) at lookup {i} sub {s} input {input}",
                );
                let outs: Vec<u16> = seq.glyphs().collect();
                assert_eq!(outs.len(), gc as usize);
                for (k, &out) in outs.iter().enumerate() {
                    assert!(
                        (out as u32) < n as u32,
                        "substitute glyph {out} >= numGlyphs {n} \
                         at lookup {i} sub {s} input {input} pos {k}",
                    );
                    // glyph(k) point-lookup agrees with the iterator.
                    assert_eq!(seq.glyph(k as u16), Some(out));
                }
                // substitute() returns the same byte window as the
                // sequence() / iter() path.
                let via_sub = ms.substitute(input).expect("covered input must substitute");
                let via_outs: Vec<u16> = via_sub.glyphs().collect();
                assert_eq!(via_outs, outs);
                total_substitute_glyphs += outs.len();
                total_sequences += 1;
            }
            // A glyph guaranteed NOT to be in this subtable: the
            // synthetic glyph ID `n` is past the end of the font.
            assert!(ms.substitute(n).is_none());
        }
    }

    // Source Sans 3 ships at least one type-2 subtable. Loosen the
    // bounds slightly so a future Adobe re-cut does not regress the
    // test.
    assert!(
        total_subtables >= 1,
        "expected >=1 type-2 subtables, got {total_subtables}",
    );
    // Every Sequence emits at least one glyph; deletion is spec-
    // prohibited, so the substitute-glyph total must be at least the
    // sequence count.
    assert!(
        total_substitute_glyphs >= total_sequences,
        "substitute-glyph total {total_substitute_glyphs} \
         < sequence count {total_sequences}",
    );
}

#[test]
fn gsub_alternate_subst_decodes_every_type_3_lookup() {
    // Walk every GSUB lookup; for the type-3 lookups (alternate
    // substitution), decode the subtable through the typed
    // [`AlternateSubst`] view and verify the spec's invariants:
    //
    // * format == 1 (the only defined AlternateSubst format)
    // * alternateSetCount == coverage.len() (parser invariant, but
    //   re-checked here through the public accessors)
    // * every alternate glyph fits inside maxp.numGlyphs
    // * iter() walks Coverage in ascending order
    // * glyph(k) point-lookup agrees with the glyphs() iterator
    // * substitute(input) agrees with the iter/set path
    //
    // Source Sans 3 ships a single type-3 lookup carrying one subtable
    // with ~210 AlternateSet tables (its `aalt` access-all-alternates
    // feature). The bounds below are loose enough to tolerate vendor
    // adjustments across releases.
    use oxideav_otf::AlternateSubst;

    let f = Font::from_bytes(FIXTURE).unwrap();
    let g = f.gsub().expect("Source Sans 3 carries a GSUB table");
    let n = f.glyph_count();

    let mut total_subtables = 0;
    let mut total_sets = 0usize;
    let mut total_alternate_glyphs = 0usize;

    for i in 0..g.lookup_count() {
        let l = g.lookup(i).unwrap();
        if l.lookup_type() != 3 {
            continue;
        }
        for s in 0..l.subtable_count() {
            let alt: AlternateSubst<'_> = g
                .alternate_subst(i, s)
                .unwrap_or_else(|| panic!("lookup {i} sub {s} missing"))
                .unwrap_or_else(|e| panic!("lookup {i} sub {s} decode: {e:?}"));
            assert_eq!(alt.format(), 1, "lookup {i} sub {s} format != 1");
            total_subtables += 1;

            // Coverage iter is sorted ascending; each Coverage glyph
            // maps to its own AlternateSet.
            let mut last_input: Option<u32> = None;
            for (input, set_res) in alt.iter() {
                if let Some(prev) = last_input {
                    assert!(
                        (input as u32) > prev,
                        "Coverage iter not ascending at lookup {i} sub {s}: {prev} -> {input}",
                    );
                }
                last_input = Some(input as u32);
                assert!(
                    (input as u32) < n as u32,
                    "input glyph {input} >= numGlyphs {n} at lookup {i} sub {s}",
                );
                let set = set_res.unwrap_or_else(|e| {
                    panic!("AlternateSet decode at lookup {i} sub {s} input {input}: {e:?}")
                });
                let gc = set.glyph_count();
                let outs: Vec<u16> = set.glyphs().collect();
                assert_eq!(outs.len(), gc as usize);
                for (k, &out) in outs.iter().enumerate() {
                    assert!(
                        (out as u32) < n as u32,
                        "alternate glyph {out} >= numGlyphs {n} \
                         at lookup {i} sub {s} input {input} pos {k}",
                    );
                    // glyph(k) point-lookup agrees with the iterator.
                    assert_eq!(set.glyph(k as u16), Some(out));
                }
                // substitute() returns the same byte window as the
                // alternate_set() / iter() path.
                let via_sub = alt
                    .substitute(input)
                    .expect("covered input must substitute");
                let via_outs: Vec<u16> = via_sub.glyphs().collect();
                assert_eq!(via_outs, outs);
                total_alternate_glyphs += outs.len();
                total_sets += 1;
            }
            // A glyph guaranteed NOT to be in this subtable: the
            // synthetic glyph ID `n` is past the end of the font.
            assert!(alt.substitute(n).is_none());
        }
    }

    // Source Sans 3 ships at least one type-3 subtable.
    assert!(
        total_subtables >= 1,
        "expected >=1 type-3 subtables, got {total_subtables}",
    );
    // Every covered glyph carries an AlternateSet; in this font each
    // set is non-empty, so the alternate-glyph total exceeds the set
    // count.
    assert!(
        total_alternate_glyphs >= total_sets,
        "alternate-glyph total {total_alternate_glyphs} < set count {total_sets}",
    );
}

#[test]
fn gsub_ligature_subst_decodes_every_type_4_lookup() {
    // Walk every GSUB lookup; for the type-4 lookups (ligature
    // substitution), decode the subtable through the typed
    // [`LigatureSubst`] view and verify the spec's invariants:
    //
    // * format == 1 (the only defined LigatureSubst format)
    // * every LigatureSet's count and per-Ligature offsets are
    //   reachable
    // * every Ligature has componentCount >= 1
    // * every component glyph and every ligature glyph fits inside
    //   maxp.numGlyphs
    // * iter() walks Coverage in ascending order
    use oxideav_otf::LigatureSubst;

    let f = Font::from_bytes(FIXTURE).unwrap();
    let g = f.gsub().expect("Source Sans 3 carries a GSUB table");
    let n = f.glyph_count();

    let mut total_subtables = 0;
    let mut total_sets = 0usize;
    let mut total_ligatures = 0usize;
    let mut total_components = 0usize;

    for i in 0..g.lookup_count() {
        let l = g.lookup(i).unwrap();
        if l.lookup_type() != 4 {
            continue;
        }
        for s in 0..l.subtable_count() {
            let ls: LigatureSubst<'_> = g
                .ligature_subst(i, s)
                .unwrap_or_else(|| panic!("lookup {i} sub {s} missing"))
                .unwrap_or_else(|e| panic!("lookup {i} sub {s} decode: {e:?}"));
            assert_eq!(ls.format(), 1, "lookup {i} sub {s} format != 1");
            total_subtables += 1;

            // Coverage iter is sorted ascending.
            let mut last_first: Option<u32> = None;
            for (first_glyph, set_res) in ls.iter() {
                if let Some(prev) = last_first {
                    assert!(
                        (first_glyph as u32) > prev,
                        "Coverage iter not ascending at lookup {i} sub {s}: {prev} -> {first_glyph}",
                    );
                }
                last_first = Some(first_glyph as u32);
                assert!(
                    (first_glyph as u32) < n as u32,
                    "first-component glyph {first_glyph} >= numGlyphs {n} \
                     at lookup {i} sub {s}",
                );
                let set = set_res.unwrap_or_else(|e| {
                    panic!("LigatureSet decode at lookup {i} sub {s} glyph {first_glyph}: {e:?}")
                });
                total_sets += 1;
                let lig_count = set.ligature_count();
                assert!(
                    lig_count >= 1,
                    "empty LigatureSet at lookup {i} sub {s} glyph {first_glyph}",
                );
                for j in 0..lig_count {
                    let lig = set
                        .ligature(j)
                        .unwrap_or_else(|| {
                            panic!(
                                "Ligature index {j} missing at lookup {i} sub {s} \
                                 glyph {first_glyph}",
                            )
                        })
                        .unwrap_or_else(|e| {
                            panic!(
                                "Ligature decode at lookup {i} sub {s} glyph {first_glyph} \
                                 lig {j}: {e:?}",
                            )
                        });
                    let comp = lig.component_count();
                    assert!(
                        comp >= 1,
                        "componentCount = 0 at lookup {i} sub {s} glyph {first_glyph} lig {j}",
                    );
                    let lig_glyph = lig.ligature_glyph();
                    assert!(
                        (lig_glyph as u32) < n as u32,
                        "ligature glyph {lig_glyph} >= numGlyphs {n} \
                         at lookup {i} sub {s} glyph {first_glyph} lig {j}",
                    );
                    // Every tail component glyph also fits inside the
                    // font's glyph table.
                    let tail: Vec<u16> = lig.component_glyphs().collect();
                    assert_eq!(tail.len(), (comp - 1) as usize);
                    for &c in &tail {
                        assert!(
                            (c as u32) < n as u32,
                            "component glyph {c} >= numGlyphs {n} \
                             at lookup {i} sub {s} glyph {first_glyph} lig {j}",
                        );
                    }
                    total_components += comp as usize;
                    total_ligatures += 1;

                    // The substitute() shaper-path returns this
                    // Ligature for the canonical input
                    // [first, tail[0], tail[1], …] — provided no
                    // earlier Ligature in the set matched first. We
                    // check the simplest case: the first Ligature in
                    // the set must match its own canonical input.
                    if j == 0 {
                        let mut input = Vec::with_capacity(comp as usize);
                        input.push(first_glyph);
                        input.extend_from_slice(&tail);
                        let got = ls.substitute(&input);
                        assert_eq!(
                            got,
                            Some((lig_glyph, comp)),
                            "substitute() didn't return Ligature 0 for its own input \
                             at lookup {i} sub {s} glyph {first_glyph}",
                        );
                    }
                }
            }
        }
    }

    // Source Sans 3 ships at least one type-4 subtable (the standard
    // 'liga' feature). Loosen the bounds slightly so a future Adobe
    // re-cut doesn't regress the test.
    assert!(
        total_subtables >= 1,
        "expected >=1 type-4 subtables, got {total_subtables}",
    );
    assert!(
        total_ligatures >= total_sets,
        "ligature count {total_ligatures} < set count {total_sets}",
    );
    assert!(
        total_components >= 2 * total_ligatures,
        "expected at least 2-component ligatures on average, got \
         {total_components} components across {total_ligatures} ligatures",
    );
}

#[test]
fn gsub_extension_subst_decodes_every_type_7_lookup() {
    // Walk every GSUB lookup; for any type-7 lookups (substitution
    // extension), decode the subtable through the typed
    // [`ExtensionSubst`] view and verify the spec's invariants:
    //
    // * format == 1 (the only defined SubstExtensionFormat1 format)
    // * extensionLookupType is in 1..=8 and never 7
    // * within one Lookup, every extension subtable carries the SAME
    //   extensionLookupType (spec: "If a lookup table uses extension
    //   subtables, then all of the extension subtables must have the
    //   same extensionLookupType")
    // * for wrapped types this crate already decodes (1..=4), the
    //   wrapped subtable resolves through the matching typed view
    //
    // Source Sans 3 is small enough that its GSUB does not need the
    // 32-bit indirection (extension subtables exist for fonts whose
    // accumulated subtable sizes exceed 16-bit offsets), so the loop
    // body may not run — the accessor semantics below are exercised
    // either way.
    use oxideav_otf::{
        ExtensionSubst, GSUB_LOOKUP_TYPE_ALTERNATE, GSUB_LOOKUP_TYPE_EXTENSION,
        GSUB_LOOKUP_TYPE_LIGATURE, GSUB_LOOKUP_TYPE_MULTIPLE, GSUB_LOOKUP_TYPE_SINGLE,
    };

    let f = Font::from_bytes(FIXTURE).unwrap();
    let g = f.gsub().expect("Source Sans 3 carries a GSUB table");

    let mut first_non_ext_lookup = None;
    for i in 0..g.lookup_count() {
        let l = g.lookup(i).unwrap();
        if l.lookup_type() != GSUB_LOOKUP_TYPE_EXTENSION {
            first_non_ext_lookup.get_or_insert(i);
            continue;
        }
        let mut lookup_ext_type: Option<u16> = None;
        for s in 0..l.subtable_count() {
            let ext: ExtensionSubst<'_> = g
                .extension_subst(i, s)
                .unwrap_or_else(|| panic!("lookup {i} sub {s} missing"))
                .unwrap_or_else(|e| panic!("lookup {i} sub {s} decode: {e:?}"));
            assert_eq!(ext.format(), 1, "lookup {i} sub {s} format != 1");
            let t = ext.extension_lookup_type();
            assert!(
                (1..=8).contains(&t) && t != GSUB_LOOKUP_TYPE_EXTENSION,
                "lookup {i} sub {s} extensionLookupType {t} out of vocabulary",
            );
            // All extension subtables of one Lookup share a type.
            if let Some(prev) = lookup_ext_type {
                assert_eq!(
                    prev, t,
                    "lookup {i} mixes extensionLookupTypes {prev} and {t}",
                );
            }
            lookup_ext_type = Some(t);
            // Resolve the indirection for the wrapped types this crate
            // already decodes as typed views.
            match t {
                GSUB_LOOKUP_TYPE_SINGLE => {
                    ext.as_single_subst()
                        .unwrap_or_else(|e| panic!("lookup {i} sub {s} wrapped type 1: {e:?}"));
                }
                GSUB_LOOKUP_TYPE_MULTIPLE => {
                    ext.as_multiple_subst()
                        .unwrap_or_else(|e| panic!("lookup {i} sub {s} wrapped type 2: {e:?}"));
                }
                GSUB_LOOKUP_TYPE_ALTERNATE => {
                    ext.as_alternate_subst()
                        .unwrap_or_else(|e| panic!("lookup {i} sub {s} wrapped type 3: {e:?}"));
                }
                GSUB_LOOKUP_TYPE_LIGATURE => {
                    ext.as_ligature_subst()
                        .unwrap_or_else(|e| panic!("lookup {i} sub {s} wrapped type 4: {e:?}"));
                }
                _ => {
                    // Types 5 / 6 / 8 stay raw; the window must at
                    // least be non-empty (parse() guarantees this).
                    assert!(!ext.extension_subtable_bytes().is_empty());
                }
            }
        }
    }

    // Accessor semantics on a real (non-type-7) lookup: the typed
    // accessor must reject with BadStructure rather than None so a
    // caller can distinguish "missing" from "wrong type".
    let i = first_non_ext_lookup.expect("font ships at least one non-extension lookup");
    assert!(matches!(g.extension_subst(i, 0), Some(Err(_))));
    // Out-of-range lookup index -> None.
    assert!(g.extension_subst(g.lookup_count(), 0).is_none());
}

#[test]
fn gpos_header_and_lookups_walk() {
    let f = Font::from_bytes(FIXTURE).unwrap();
    let g = f.gpos().expect("Source Sans 3 carries a GPOS table");
    assert_eq!(g.version(), (1, 0));
    assert!(!g.has_feature_variations());

    // Source Sans 3 ships several latn-tagged features, including
    // kern. We don't hardcode counts (vendor adjustments happen
    // across releases) but every lookup must parse cleanly.
    let lookups = g.lookup_list().unwrap();
    assert!(lookups.count() >= 1);
    for (i, l_res) in lookups.iter().enumerate() {
        let l = l_res.unwrap_or_else(|e| panic!("GPOS lookup {i} parse error: {e:?}"));
        // GPOS lookup types are 1..=9 per the spec; an unknown type
        // here would point at our parser, not the font.
        let t = l.lookup_type();
        assert!((1..=9).contains(&t), "lookup {i} has unknown type {t}");
        // markFilteringSet presence agrees with the flag bit.
        let want = l.flag().use_mark_filtering_set();
        assert_eq!(l.mark_filtering_set().is_some(), want);
    }
}

#[test]
fn gpos_single_pos_subtables_decode() {
    use oxideav_otf::GPOS_LOOKUP_TYPE_SINGLE;
    let f = Font::from_bytes(FIXTURE).unwrap();
    let g = f.gpos().unwrap();

    let mut type1_lookups = 0usize;
    let mut covered_glyphs = 0usize;
    for i in 0..g.lookup_count() {
        let l = g.lookup(i).unwrap();
        if l.lookup_type() != GPOS_LOOKUP_TYPE_SINGLE {
            // The wrong-type accessor must reject a non-type-1 lookup.
            assert!(matches!(g.single_pos(i, 0), Some(Err(_)) | None));
            continue;
        }
        type1_lookups += 1;
        for s in 0..l.subtable_count() {
            let sp = g
                .single_pos(i, s)
                .expect("type-1 subtable in range")
                .expect("SinglePos parses");
            // Format is one of the two defined values.
            assert!(matches!(sp.format(), 1 | 2));
            // valueFormat declares only defined bits.
            assert!(sp.value_format().is_valid());
            // Every covered glyph yields a parseable ValueRecord, and
            // the record's declared fields agree with the value format.
            let vf = sp.value_format();
            for (glyph, rec_res) in sp.iter() {
                let rec = rec_res.expect("ValueRecord parses");
                // Undeclared placement/advance fields must be zero.
                if !vf.has_x_placement() {
                    assert_eq!(rec.x_placement, 0);
                }
                if !vf.has_y_advance() {
                    assert_eq!(rec.y_advance, 0);
                }
                // The same glyph queried directly matches the iterator.
                let direct = sp.value(glyph).unwrap().unwrap();
                assert_eq!(direct, rec);
                covered_glyphs += 1;
            }
        }
    }
    // Source Sans 3's GPOS uses pair-adjustment (type 2) kerning and
    // mark attachment rather than single-adjustment positioning, so the
    // fixture carries no type-1 lookups: this walk documents that the
    // wrong-type accessor rejects every non-type-1 lookup and that the
    // absence is legitimate. The SinglePos decode path itself is
    // exercised by the synthetic byte-tower unit tests in the gpos
    // module (both formats, every error path).
    assert_eq!(type1_lookups, 0);
    assert_eq!(covered_glyphs, 0);
}

#[test]
fn gpos_pair_pos_subtables_decode() {
    use oxideav_otf::GPOS_LOOKUP_TYPE_PAIR;
    let f = Font::from_bytes(FIXTURE).unwrap();
    let g = f.gpos().unwrap();
    let num_glyphs = f.glyph_count();

    let mut type2_lookups = 0usize;
    let mut decoded_subtables = 0usize;
    let mut format1_pairs = 0usize;
    let mut format2_subtables = 0usize;

    for i in 0..g.lookup_count() {
        let l = g.lookup(i).unwrap();
        if l.lookup_type() != GPOS_LOOKUP_TYPE_PAIR {
            // The wrong-type accessor must reject a non-type-2 lookup.
            assert!(matches!(g.pair_pos(i, 0), Some(Err(_)) | None));
            continue;
        }
        type2_lookups += 1;
        for s in 0..l.subtable_count() {
            let pp = g
                .pair_pos(i, s)
                .expect("type-2 subtable in range")
                .expect("PairPos parses");
            assert!(matches!(pp.format(), 1 | 2));
            assert!(pp.value_format1().is_valid());
            assert!(pp.value_format2().is_valid());
            decoded_subtables += 1;

            // Coverage iterates in ascending glyph order and every first
            // glyph fits inside the glyph repertoire.
            let mut prev: Option<u16> = None;
            for (gid, _idx) in pp.coverage().iter() {
                if let Some(p) = prev {
                    assert!(gid > p, "coverage must be strictly ascending");
                }
                prev = Some(gid);
                assert!((gid as u32) < num_glyphs as u32);
            }

            match pp.format() {
                1 => {
                    // Walk every explicit (first, second, value) triple;
                    // confirm both glyphs are in range, the iterator is
                    // ordered, and a direct `pair()` query agrees with it.
                    let mut last: Option<(u16, u16)> = None;
                    for (first, second, val_res) in pp.iter() {
                        let val = val_res.expect("PairValue parses");
                        assert!((first as u32) < num_glyphs as u32);
                        assert!((second as u32) < num_glyphs as u32);
                        if let Some((lf, ls)) = last {
                            assert!(
                                (first, second) > (lf, ls),
                                "format-1 pairs ascend by (first, second)"
                            );
                        }
                        last = Some((first, second));
                        let direct = pp.pair(first, second).unwrap().unwrap();
                        assert_eq!(direct, val);
                        format1_pairs += 1;
                    }
                }
                2 => {
                    // Class-matrix form: spot-check a covered first glyph
                    // against an arbitrary second glyph resolves without
                    // panicking and that `class_pair` agrees with `pair`.
                    format2_subtables += 1;
                    if let Some((first, _idx)) = pp.coverage().iter().next() {
                        // glyph 0 (.notdef) is a valid second-glyph probe.
                        let via_pair = pp.pair(first, 0);
                        // A covered first glyph always yields a cell in
                        // format 2 (possibly the all-zero default).
                        if let Some(res) = via_pair {
                            let _ = res.expect("format-2 cell parses");
                        }
                    }
                }
                _ => unreachable!(),
            }
        }
    }

    // Source Sans 3's GPOS carries no *direct* type-2 lookups — its
    // kerning is reached through a type-9 (extension positioning) lookup
    // (see `gpos_pair_pos_via_extension` for that path). This walk
    // therefore documents that the direct accessor rejects every
    // non-type-2 lookup and that no type-2 lookup is present directly;
    // the synthetic byte-tower unit tests in the gpos module carry the
    // format-1 / format-2 decode coverage.
    let _ = (format1_pairs, format2_subtables, decoded_subtables);
    assert_eq!(type2_lookups, 0);
}

/// Source Sans 3 reaches its pair-adjustment kerning through a type-9
/// (extension) GPOS lookup. GPOS extension subtables share the GSUB
/// extension layout (`format = 1`, `extensionLookupType`,
/// `Offset32 extensionOffset` from the start of the extension subtable);
/// this test resolves that indirection by hand and decodes the wrapped
/// PairPos directly, exercising the real-font format-1 / format-2 path.
#[test]
fn gpos_pair_pos_via_extension() {
    use oxideav_otf::{GPOS_LOOKUP_TYPE_EXTENSION, GPOS_LOOKUP_TYPE_PAIR};
    let f = Font::from_bytes(FIXTURE).unwrap();
    let g = f.gpos().unwrap();
    let num_glyphs = f.glyph_count();

    let mut wrapped_pair_subtables = 0usize;
    let mut format1 = 0usize;
    let mut format2 = 0usize;

    for i in 0..g.lookup_count() {
        let l = g.lookup(i).unwrap();
        if l.lookup_type() != GPOS_LOOKUP_TYPE_EXTENSION {
            continue;
        }
        for s in 0..l.subtable_count() {
            let raw = l.subtable_bytes(s).expect("extension subtable bytes");
            // ExtensionPosFormat1: format(2) + extLookupType(2) + Offset32.
            assert!(raw.len() >= 8);
            let format = u16::from_be_bytes([raw[0], raw[1]]);
            assert_eq!(format, 1, "GPOS extension format must be 1");
            let ext_type = u16::from_be_bytes([raw[2], raw[3]]);
            let ext_off = u32::from_be_bytes([raw[4], raw[5], raw[6], raw[7]]) as usize;
            assert!(ext_off != 0 && ext_off < raw.len());
            if ext_type != GPOS_LOOKUP_TYPE_PAIR {
                continue;
            }
            // The wrapped bytes ARE a PairPos subtable.
            let pp = oxideav_otf::PairPos::parse(&raw[ext_off..]).expect("wrapped PairPos parses");
            assert!(pp.value_format1().is_valid());
            assert!(pp.value_format2().is_valid());
            wrapped_pair_subtables += 1;

            // Coverage strictly ascending, all first glyphs in range.
            let mut prev: Option<u16> = None;
            for (gid, _idx) in pp.coverage().iter() {
                if let Some(p) = prev {
                    assert!(gid > p);
                }
                prev = Some(gid);
                assert!((gid as u32) < num_glyphs as u32);
            }

            match pp.format() {
                1 => {
                    format1 += 1;
                    let mut last: Option<(u16, u16)> = None;
                    let mut seen = 0usize;
                    for (first, second, val_res) in pp.iter() {
                        let val = val_res.expect("PairValue parses");
                        assert!((first as u32) < num_glyphs as u32);
                        assert!((second as u32) < num_glyphs as u32);
                        if let Some(prev) = last {
                            assert!((first, second) > prev);
                        }
                        last = Some((first, second));
                        // Direct lookup agrees with the iterator.
                        assert_eq!(pp.pair(first, second).unwrap().unwrap(), val);
                        seen += 1;
                        if seen > 4000 {
                            break; // keep the test bounded
                        }
                    }
                    assert!(seen > 0, "a format-1 PairPos lists at least one pair");
                }
                2 => {
                    format2 += 1;
                    // Probe each covered first glyph against .notdef; a
                    // covered first glyph always yields a class cell.
                    let mut probed = 0usize;
                    for (first, _idx) in pp.coverage().iter() {
                        if let Some(res) = pp.pair(first, 0) {
                            let _ = res.expect("format-2 cell parses");
                        }
                        probed += 1;
                        if probed > 256 {
                            break;
                        }
                    }
                    // class_pair(0, 0) is always a valid cell.
                    let _ = pp.class_pair(0, 0).unwrap().unwrap();
                }
                _ => unreachable!(),
            }
        }
    }

    // The fixture's kerning is a type-2 PairPos wrapped in a type-9
    // extension, so at least one wrapped PairPos must have decoded.
    assert!(
        wrapped_pair_subtables >= 1,
        "fixture exposes pair kerning behind a type-9 extension"
    );
    assert!(format1 + format2 >= 1);
}

#[test]
fn gpos_finds_latin_script() {
    let f = Font::from_bytes(FIXTURE).unwrap();
    let g = f.gpos().unwrap();
    let latn = g.find_script(b"latn").expect("latn script in GPOS");
    assert!(latn.has_default_lang_sys());
    let default_lang = latn.default_lang_sys().unwrap().unwrap();
    assert!(default_lang.feature_count() >= 1);
    // No required feature is set on Source Sans 3's latn/dflt.
    assert!(default_lang.required_feature_index().is_none());
}

#[test]
fn layout_table_version_accessors() {
    let f = Font::from_bytes(FIXTURE).unwrap();
    assert_eq!(f.gsub_version(), Some((1, 0)));
    assert_eq!(f.gpos_version(), Some((1, 0)));
}
