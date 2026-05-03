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
