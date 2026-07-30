//! COLR × CPAL color resolution: version-0 layers and version-1 paint
//! palette indices resolved to concrete sRGB colors, with the paint
//! alpha × (CPAL alpha / 255) multiplication and the 0xFFFF
//! foreground sentinel (§5.7.11 "Relationship to COLR and SVG Tables"
//! + the COLR paint-graph alpha rule).

use oxideav_otf::tables::colr::{ColrTable, PremultipliedLinearColor, ResolvedColor};
use oxideav_otf::tables::cpal::{ColorRecord, CpalTable};
use oxideav_otf::{resolve_paint_color, Paint, COLR_FOREGROUND_PALETTE_INDEX};

fn u16b(v: u16) -> [u8; 2] {
    v.to_be_bytes()
}
fn u32b(v: u32) -> [u8; 4] {
    v.to_be_bytes()
}
fn f2(v: f32) -> [u8; 2] {
    ((v * 16384.0).round() as i16).to_be_bytes()
}

/// CPAL v0 with 2 palettes x 2 entries (4 records, RGBA given here,
/// BGRA on disk).
fn cpal_bytes() -> Vec<u8> {
    let records: [[u8; 4]; 4] = [
        [0xFF, 0x00, 0x00, 0xFF], // p0e0 opaque red
        [0x00, 0xFF, 0x00, 0x80], // p0e1 half-alpha green
        [0x00, 0x00, 0xFF, 0xFF], // p1e0 opaque blue
        [0x20, 0x30, 0x40, 0x40], // p1e1 quarter-alpha slate
    ];
    let mut b = Vec::new();
    b.extend_from_slice(&u16b(0)); // version
    b.extend_from_slice(&u16b(2)); // numPaletteEntries
    b.extend_from_slice(&u16b(2)); // numPalettes
    b.extend_from_slice(&u16b(4)); // numColorRecords
    b.extend_from_slice(&u32b(16)); // offsetFirstColorRecord
    b.extend_from_slice(&u16b(0));
    b.extend_from_slice(&u16b(2));
    for [r, g, bl, a] in records {
        b.extend_from_slice(&[bl, g, r, a]);
    }
    b
}

/// COLR v0: base glyph 7 = two layers (glyph 20 / entry 0, glyph 21 /
/// entry 0xFFFF foreground).
fn colr_v0_bytes() -> Vec<u8> {
    let mut b = Vec::new();
    b.extend_from_slice(&u16b(0)); // version
    b.extend_from_slice(&u16b(1)); // numBaseGlyphRecords
    b.extend_from_slice(&u32b(14)); // baseGlyphRecordsOffset
    b.extend_from_slice(&u32b(20)); // layerRecordsOffset
    b.extend_from_slice(&u16b(2)); // numLayerRecords
    b.extend_from_slice(&u16b(7)); // BaseGlyph.glyphID
    b.extend_from_slice(&u16b(0)); // firstLayerIndex
    b.extend_from_slice(&u16b(2)); // numLayers
    b.extend_from_slice(&u16b(20)); // layer 0 glyph
    b.extend_from_slice(&u16b(0)); // layer 0 palette entry
    b.extend_from_slice(&u16b(21)); // layer 1 glyph
    b.extend_from_slice(&u16b(0xFFFF)); // layer 1 = foreground
    b
}

/// COLR v1 with one base glyph (gid 5) whose root is a PaintSolid, and
/// a second root (gid 6) that is a linear gradient with two stops.
fn colr_v1_bytes(solid_entry: u16, solid_alpha: f32) -> Vec<u8> {
    // Paint blobs.
    let mut solid = vec![2u8];
    solid.extend_from_slice(&u16b(solid_entry));
    solid.extend_from_slice(&f2(solid_alpha));

    // Linear gradient (format 4): colorLineOffset(3) + 6 x FWORD.
    let mut grad = vec![4u8];
    grad.extend_from_slice(&[0, 0, 16]); // Offset24 to color line (= header 16 bytes)
    for v in [0i16, 0, 100, 0, 0, 50] {
        grad.extend_from_slice(&v.to_be_bytes());
    }
    // ColorLine: extend pad, 2 stops.
    grad.push(0);
    grad.extend_from_slice(&u16b(2));
    // stop 0: offset 0.0, entry 1, alpha 1.0
    grad.extend_from_slice(&f2(0.0));
    grad.extend_from_slice(&u16b(1));
    grad.extend_from_slice(&f2(1.0));
    // stop 1: offset 1.0, foreground, alpha 0.5
    grad.extend_from_slice(&f2(1.0));
    grad.extend_from_slice(&u16b(0xFFFF));
    grad.extend_from_slice(&f2(0.5));

    const HDR: usize = 34;
    let bgl_at = HDR;
    let bgl_len = 4 + 6 * 2;
    let paints_at = bgl_at + bgl_len;
    let solid_at = paints_at;
    let grad_at = solid_at + solid.len();

    let mut b = Vec::new();
    b.extend_from_slice(&u16b(1)); // version
    b.extend_from_slice(&u16b(0)); // numBaseGlyphRecords
    b.extend_from_slice(&u32b(0));
    b.extend_from_slice(&u32b(0));
    b.extend_from_slice(&u16b(0)); // numLayerRecords
    b.extend_from_slice(&u32b(bgl_at as u32)); // baseGlyphListOffset
    b.extend_from_slice(&u32b(0)); // layerListOffset
    b.extend_from_slice(&u32b(0)); // clipListOffset
    b.extend_from_slice(&u32b(0)); // varIndexMapOffset
    b.extend_from_slice(&u32b(0)); // itemVariationStoreOffset
    assert_eq!(b.len(), HDR);
    // BaseGlyphList: 2 records.
    b.extend_from_slice(&u32b(2));
    b.extend_from_slice(&u16b(5));
    b.extend_from_slice(&u32b((solid_at - bgl_at) as u32));
    b.extend_from_slice(&u16b(6));
    b.extend_from_slice(&u32b((grad_at - bgl_at) as u32));
    b.extend_from_slice(&solid);
    b.extend_from_slice(&grad);
    b
}

const FG: ColorRecord = ColorRecord {
    red: 0x11,
    green: 0x22,
    blue: 0x33,
    alpha: 0xFF,
};

#[test]
fn v0_layers_resolve_to_rgba() {
    let cpal_b = cpal_bytes();
    let colr_b = colr_v0_bytes();
    let cpal = CpalTable::parse(&cpal_b).unwrap();
    let colr = ColrTable::parse(&colr_b).unwrap();

    let layers = colr.v0_layers(7).unwrap();
    // Layer 0, palette 0: opaque red.
    let c = layers[0].resolve(&cpal, 0, FG).unwrap();
    assert_eq!(c.rgba8(), [0xFF, 0x00, 0x00, 0xFF]);
    assert_eq!(c.alpha, 1.0);
    // Layer 0, palette 1: opaque blue (same entry, other palette).
    let c = layers[0].resolve(&cpal, 1, FG).unwrap();
    assert_eq!(c.rgba8(), [0x00, 0x00, 0xFF, 0xFF]);
    // Layer 1 is the foreground sentinel in every palette.
    for palette in 0..2 {
        let c = layers[1].resolve(&cpal, palette, FG).unwrap();
        assert_eq!(c.rgba8(), [0x11, 0x22, 0x33, 0xFF]);
    }
    // Out-of-range palette: layer 0 fails, the foreground layer still
    // resolves (it never consults CPAL).
    assert_eq!(layers[0].resolve(&cpal, 9, FG), None);
    assert!(layers[1].resolve(&cpal, 9, FG).is_some());
}

#[test]
fn v1_solid_multiplies_paint_alpha_with_cpal_alpha() {
    let cpal_b = cpal_bytes();
    let cpal = CpalTable::parse(&cpal_b).unwrap();
    // Solid on entry 1 (alpha 0x80) with paint alpha 0.5.
    let colr_b = colr_v1_bytes(1, 0.5);
    let colr = ColrTable::parse(&colr_b).unwrap();
    let root = colr.base_glyph_paint(5).unwrap();
    let Paint::Solid {
        palette_index,
        alpha,
        ..
    } = colr.paint(root, None).unwrap()
    else {
        panic!("expected PaintSolid");
    };
    // Palette 0 entry 1 = green, CPAL alpha 0x80.
    let c = resolve_paint_color(&cpal, 0, palette_index, alpha, FG).unwrap();
    assert_eq!([c.red, c.green, c.blue], [0x00, 0xFF, 0x00]);
    let expected = 0.5 * (0x80 as f32 / 255.0);
    assert!((c.alpha - expected).abs() < 1e-4);
    assert_eq!(c.rgba8()[3], (expected * 255.0).round() as u8);
    // Palette 1 entry 1 has CPAL alpha 0x40.
    let c = resolve_paint_color(&cpal, 1, palette_index, alpha, FG).unwrap();
    assert_eq!([c.red, c.green, c.blue], [0x20, 0x30, 0x40]);
    assert!((c.alpha - 0.5 * (0x40 as f32 / 255.0)).abs() < 1e-4);
}

#[test]
fn v1_gradient_stops_resolve_against_palette_and_foreground() {
    let cpal_b = cpal_bytes();
    let cpal = CpalTable::parse(&cpal_b).unwrap();
    let colr_b = colr_v1_bytes(0, 1.0);
    let colr = ColrTable::parse(&colr_b).unwrap();
    let root = colr.base_glyph_paint(6).unwrap();
    let Paint::LinearGradient { color_line, .. } = colr.paint(root, None).unwrap() else {
        panic!("expected PaintLinearGradient");
    };
    let resolved = color_line.resolve(&cpal, 0, FG).unwrap();
    assert_eq!(resolved.len(), 2);
    // Stop 0: entry 1 (green, CPAL alpha 0x80) x paint alpha 1.0.
    assert_eq!(resolved[0].0, 0.0);
    let c = resolved[0].1;
    assert_eq!([c.red, c.green, c.blue], [0x00, 0xFF, 0x00]);
    assert!((c.alpha - 0x80 as f32 / 255.0).abs() < 1e-4);
    // Stop 1: foreground x alpha 0.5.
    assert_eq!(resolved[1].0, 1.0);
    let c = resolved[1].1;
    assert_eq!([c.red, c.green, c.blue], [0x11, 0x22, 0x33]);
    assert!((c.alpha - 0.5).abs() < 1e-4);
    // Per-stop resolution agrees with the line-level helper.
    assert_eq!(
        color_line.stops[1].resolve(&cpal, 0, FG).unwrap(),
        resolved[1].1
    );
}

#[test]
fn out_of_range_palette_entry_is_a_malformed_color_glyph() {
    let cpal_b = cpal_bytes();
    let cpal = CpalTable::parse(&cpal_b).unwrap();
    // Solid referencing entry 2 of a 2-entry palette.
    let colr_b = colr_v1_bytes(2, 1.0);
    let colr = ColrTable::parse(&colr_b).unwrap();
    let root = colr.base_glyph_paint(5).unwrap();
    let Paint::Solid {
        palette_index,
        alpha,
        ..
    } = colr.paint(root, None).unwrap()
    else {
        panic!("expected PaintSolid");
    };
    assert_eq!(
        resolve_paint_color(&cpal, 0, palette_index, alpha, FG),
        None
    );
}

/// A stand-in invertible transfer function for testing the
/// linearize/premultiply pipeline (the real sRGB transfer function is
/// defined in IEC 61966-2-1 / CSS Color 4 §10.2, outside the staged
/// chapter): linear = v².
fn square(v: f32) -> f32 {
    v * v
}

#[test]
fn interpolation_weights_follow_the_chapter_example() {
    // Chapter worked example: adjacent stops at 0.5 and 0.9, query at
    // 0.8 — the interpolation weight is (0.8-0.5)/(0.9-0.5) = 0.75
    // toward the second stop (linear interpolation is continuous at
    // the stops). Identity transfer, both stops opaque, so channels
    // interpolate directly.
    let a = ResolvedColor {
        red: 200,
        green: 0,
        blue: 100,
        alpha: 1.0,
    };
    let b = ResolvedColor {
        red: 40,
        green: 255,
        blue: 100,
        alpha: 1.0,
    };
    let pa = a.premultiply_linear(|v| v);
    let pb = b.premultiply_linear(|v| v);
    let mid = pa.lerp(&pb, 0.75);
    let want = |x: u8, y: u8| (x as f32 / 255.0) * 0.25 + (y as f32 / 255.0) * 0.75;
    assert!((mid.red - want(200, 40)).abs() < 1e-6);
    assert!((mid.green - want(0, 255)).abs() < 1e-6);
    assert!((mid.blue - want(100, 100)).abs() < 1e-6);
    assert!((mid.alpha - 1.0).abs() < 1e-6);
    // Endpoint continuity: t = 0 is the first stop, t = 1 the second.
    assert_eq!(pa.lerp(&pb, 0.0), pa);
    let end = pa.lerp(&pb, 1.0);
    for (got, want) in [
        (end.red, pb.red),
        (end.green, pb.green),
        (end.blue, pb.blue),
        (end.alpha, pb.alpha),
    ] {
        assert!((got - want).abs() < 1e-6);
    }
}

#[test]
fn premultiplied_interpolation_weights_channels_by_alpha() {
    // Fully transparent red toward opaque blue at t = 0.5: alpha
    // premultiplication zeroes the transparent stop's channels, so the
    // midpoint is pure blue at half strength; un-premultiplying by the
    // interpolated alpha (0.5) recovers full blue.
    let a = ResolvedColor {
        red: 255,
        green: 0,
        blue: 0,
        alpha: 0.0,
    };
    let b = ResolvedColor {
        red: 0,
        green: 0,
        blue: 255,
        alpha: 1.0,
    };
    let mid = a
        .premultiply_linear(|v| v)
        .lerp(&b.premultiply_linear(|v| v), 0.5);
    assert!((mid.alpha - 0.5).abs() < 1e-6);
    assert!((mid.red - 0.0).abs() < 1e-6);
    assert!((mid.blue - 0.5).abs() < 1e-6);
    let un = mid.unpremultiplied_linear();
    assert!((un[2] - 1.0).abs() < 1e-6);
    assert!((un[0] - 0.0).abs() < 1e-6);
    // alpha == 0 un-premultiplies to all-zero, not NaN.
    let zero = a.premultiply_linear(|v| v);
    assert_eq!(zero.unpremultiplied_linear(), [0.0, 0.0, 0.0]);
}

#[test]
fn linearization_applies_before_premultiplication() {
    let c = ResolvedColor {
        red: 128,
        green: 0,
        blue: 255,
        alpha: 0.5,
    };
    let p = c.premultiply_linear(square);
    let frac = 128.0 / 255.0;
    assert!((p.red - square(frac) * 0.5).abs() < 1e-6);
    assert!((p.blue - 1.0 * 0.5).abs() < 1e-6);
    // Alpha is carried on a linear scale, untouched by the transfer
    // function.
    assert!((p.alpha - 0.5).abs() < 1e-6);
}

#[test]
fn color_line_interpolate_at_walks_the_stops() {
    let cpal_b = cpal_bytes();
    let cpal = CpalTable::parse(&cpal_b).unwrap();
    let colr_b = colr_v1_bytes(0, 1.0);
    let colr = ColrTable::parse(&colr_b).unwrap();
    let root = colr.base_glyph_paint(6).unwrap();
    let Paint::LinearGradient { color_line, .. } = colr.paint(root, None).unwrap() else {
        panic!("expected PaintLinearGradient");
    };
    // Stops: 0.0 = entry 1 (green, CPAL alpha 0x80), 1.0 = foreground
    // (0x11 0x22 0x33) x paint alpha 0.5.
    let at = |pos: f32| -> PremultipliedLinearColor {
        color_line.interpolate_at(pos, &cpal, 0, FG, |v| v).unwrap()
    };
    let a0 = 0x80 as f32 / 255.0;
    // At (and before) the first stop: the first stop's premultiplied
    // color.
    for pos in [-0.5, 0.0] {
        let c = at(pos);
        assert!((c.green - a0).abs() < 1e-4);
        assert!((c.alpha - a0).abs() < 1e-4);
        assert!((c.red - 0.0).abs() < 1e-6);
    }
    // At (and past) the last stop: the foreground stop at alpha 0.5.
    for pos in [1.0, 2.0] {
        let c = at(pos);
        assert!((c.alpha - 0.5).abs() < 1e-4);
        assert!((c.red - (0x11 as f32 / 255.0) * 0.5).abs() < 1e-4);
    }
    // Halfway: each component the mean of the premultiplied endpoints.
    let mid = at(0.5);
    let g0 = a0; // green channel of stop 0, premultiplied (255/255 * a0)
    let g1 = (0x22 as f32 / 255.0) * 0.5;
    assert!((mid.green - (g0 + g1) / 2.0).abs() < 1e-4);
    assert!((mid.alpha - (a0 + 0.5) / 2.0).abs() < 1e-4);
    // Out-of-range palette: None.
    assert!(color_line
        .interpolate_at(0.5, &cpal, 9, FG, |v| v)
        .is_none());
}

#[test]
fn foreground_index_ignores_palette_and_multiplies_foreground_alpha() {
    let cpal_b = cpal_bytes();
    let cpal = CpalTable::parse(&cpal_b).unwrap();
    let translucent_fg = ColorRecord {
        red: 0xAA,
        green: 0xBB,
        blue: 0xCC,
        alpha: 0x80,
    };
    let c =
        resolve_paint_color(&cpal, 1, COLR_FOREGROUND_PALETTE_INDEX, 0.5, translucent_fg).unwrap();
    assert_eq!([c.red, c.green, c.blue], [0xAA, 0xBB, 0xCC]);
    assert!((c.alpha - 0.5 * (0x80 as f32 / 255.0)).abs() < 1e-4);
}
