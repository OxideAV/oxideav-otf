//! Synthetic byte-level tests for the `COLR` table: version-0 layer
//! records and the version-1 paint graph (all 32 paint formats, color
//! lines, the clip list, the varIndexBase / delta-set variation
//! scheme, cycle detection, and boundedness analysis).

use oxideav_otf::tables::colr::ColrTable;
use oxideav_otf::{CompositeMode, Error, Extend, Paint};

// ---- byte builders ---------------------------------------------------------

fn u16b(v: u16) -> [u8; 2] {
    v.to_be_bytes()
}

fn i16b(v: i16) -> [u8; 2] {
    v.to_be_bytes()
}

fn u24b(v: u32) -> [u8; 3] {
    [(v >> 16) as u8, (v >> 8) as u8, v as u8]
}

fn u32b(v: u32) -> [u8; 4] {
    v.to_be_bytes()
}

fn f2(v: f32) -> [u8; 2] {
    ((v * 16384.0).round() as i16).to_be_bytes()
}

fn fixed(v: f32) -> [u8; 4] {
    ((v * 65536.0).round() as i32).to_be_bytes()
}

/// A ColorLine with `extend` and `(offset, palette, alpha)` stops.
fn color_line(extend: u8, stops: &[(f32, u16, f32)]) -> Vec<u8> {
    let mut b = vec![extend];
    b.extend_from_slice(&u16b(stops.len() as u16));
    for &(off, pal, alpha) in stops {
        b.extend_from_slice(&f2(off));
        b.extend_from_slice(&u16b(pal));
        b.extend_from_slice(&f2(alpha));
    }
    b
}

/// A VarColorLine: stops carry a trailing varIndexBase.
fn var_color_line(extend: u8, stops: &[(f32, u16, f32, u32)]) -> Vec<u8> {
    let mut b = vec![extend];
    b.extend_from_slice(&u16b(stops.len() as u16));
    for &(off, pal, alpha, vib) in stops {
        b.extend_from_slice(&f2(off));
        b.extend_from_slice(&u16b(pal));
        b.extend_from_slice(&f2(alpha));
        b.extend_from_slice(&u32b(vib));
    }
    b
}

fn paint_solid(palette: u16, alpha: f32) -> Vec<u8> {
    let mut b = vec![2u8];
    b.extend_from_slice(&u16b(palette));
    b.extend_from_slice(&f2(alpha));
    b
}

/// Assemble a version-1 COLR table.
///
/// * `base_glyphs` — `(glyphID, paint index)` pairs (must be sorted by
///   glyph ID).
/// * `layers` — LayerList entries as paint indices.
/// * `paints` — paint blobs, appended in order after all list
///   sections; internal Offset24s inside a blob are blob-relative and
///   already correct because children live inside the same blob.
/// * `clip_list` / `var_index_map` / `ivs` — raw section blobs.
struct ColrV1 {
    base_glyphs: Vec<(u16, usize)>,
    layers: Vec<usize>,
    paints: Vec<Vec<u8>>,
    clip_list: Vec<u8>,
    var_index_map: Vec<u8>,
    ivs: Vec<u8>,
}

impl ColrV1 {
    fn new() -> Self {
        Self {
            base_glyphs: Vec::new(),
            layers: Vec::new(),
            paints: Vec::new(),
            clip_list: Vec::new(),
            var_index_map: Vec::new(),
            ivs: Vec::new(),
        }
    }

    fn build(&self) -> Vec<u8> {
        const HDR: usize = 34;
        let bgl_at = HDR;
        let bgl_len = 4 + 6 * self.base_glyphs.len();
        let ll_at = bgl_at + bgl_len;
        let ll_len = 4 + 4 * self.layers.len();
        let clip_at = ll_at + ll_len;
        let vim_at = clip_at + self.clip_list.len();
        let ivs_at = vim_at + self.var_index_map.len();
        let paints_at = ivs_at + self.ivs.len();

        // Absolute offset of each paint blob.
        let mut paint_abs = Vec::with_capacity(self.paints.len());
        let mut cursor = paints_at;
        for p in &self.paints {
            paint_abs.push(cursor);
            cursor += p.len();
        }

        let mut b = Vec::new();
        b.extend_from_slice(&u16b(1)); // version
        b.extend_from_slice(&u16b(0)); // numBaseGlyphRecords
        b.extend_from_slice(&u32b(0)); // baseGlyphRecordsOffset
        b.extend_from_slice(&u32b(0)); // layerRecordsOffset
        b.extend_from_slice(&u16b(0)); // numLayerRecords
        b.extend_from_slice(&u32b(bgl_at as u32));
        b.extend_from_slice(&u32b(if self.layers.is_empty() {
            0
        } else {
            ll_at as u32
        }));
        b.extend_from_slice(&u32b(if self.clip_list.is_empty() {
            0
        } else {
            clip_at as u32
        }));
        b.extend_from_slice(&u32b(if self.var_index_map.is_empty() {
            0
        } else {
            vim_at as u32
        }));
        b.extend_from_slice(&u32b(if self.ivs.is_empty() {
            0
        } else {
            ivs_at as u32
        }));
        assert_eq!(b.len(), HDR);

        b.extend_from_slice(&u32b(self.base_glyphs.len() as u32));
        for &(gid, pi) in &self.base_glyphs {
            b.extend_from_slice(&u16b(gid));
            b.extend_from_slice(&u32b((paint_abs[pi] - bgl_at) as u32));
        }
        b.extend_from_slice(&u32b(self.layers.len() as u32));
        for &pi in &self.layers {
            b.extend_from_slice(&u32b((paint_abs[pi] - ll_at) as u32));
        }
        b.extend_from_slice(&self.clip_list);
        b.extend_from_slice(&self.var_index_map);
        b.extend_from_slice(&self.ivs);
        for p in &self.paints {
            b.extend_from_slice(p);
        }
        b
    }
}

/// Single-axis IVS with one subtable holding one int16 delta per row,
/// all against region (start 0, peak 1, end 1) — so at normalized
/// coordinate `c >= 0` the interpolated delta of row `i` is
/// `c * rows[i]`.
fn ivs_rows(rows: &[i16]) -> Vec<u8> {
    let mut v = Vec::new();
    v.extend_from_slice(&u16b(1)); // format
    v.extend_from_slice(&u32b(12)); // regionListOffset
    v.extend_from_slice(&u16b(1)); // ivdCount
    v.extend_from_slice(&u32b(22)); // ivd[0]
    assert_eq!(v.len(), 12);
    v.extend_from_slice(&u16b(1)); // axisCount
    v.extend_from_slice(&u16b(1)); // regionCount
    v.extend_from_slice(&f2(0.0));
    v.extend_from_slice(&f2(1.0));
    v.extend_from_slice(&f2(1.0));
    assert_eq!(v.len(), 22);
    v.extend_from_slice(&u16b(rows.len() as u16)); // itemCount
    v.extend_from_slice(&u16b(1)); // shortDeltaCount
    v.extend_from_slice(&u16b(1)); // regionIndexCount
    v.extend_from_slice(&u16b(0)); // regionIndex 0
    for &d in rows {
        v.extend_from_slice(&i16b(d));
    }
    v
}

/// Format-0 DeltaSetIndexMap sending index `i` to `(0, i)` for
/// `i < n` (32 inner-index bits… 16 used; 2-byte entries, 8 inner
/// bits).
fn identity_map(n: u16) -> Vec<u8> {
    let mut v = Vec::new();
    v.push(0u8); // format
    v.push(0x17u8); // entryFormat: entry size 2, inner bits 8
    v.extend_from_slice(&u16b(n));
    for i in 0..n {
        v.extend_from_slice(&u16b(i)); // outer 0, inner i
    }
    v
}

// ---- version 0 -------------------------------------------------------------

#[test]
fn v0_layer_lookup() {
    let mut b = Vec::new();
    b.extend_from_slice(&u16b(0)); // version
    b.extend_from_slice(&u16b(2)); // numBaseGlyphRecords
    b.extend_from_slice(&u32b(14)); // baseGlyphRecordsOffset
    b.extend_from_slice(&u32b(26)); // layerRecordsOffset
    b.extend_from_slice(&u16b(3)); // numLayerRecords
                                   // base glyph records (sorted by gid): gid 4 -> layers[0..2],
                                   // gid 9 -> layers[2..3].
    b.extend_from_slice(&u16b(4));
    b.extend_from_slice(&u16b(0));
    b.extend_from_slice(&u16b(2));
    b.extend_from_slice(&u16b(9));
    b.extend_from_slice(&u16b(2));
    b.extend_from_slice(&u16b(1));
    // layer records.
    for &(g, p) in &[(11u16, 0u16), (12, 1), (13, 0xFFFF)] {
        b.extend_from_slice(&u16b(g));
        b.extend_from_slice(&u16b(p));
    }

    let colr = ColrTable::parse(&b).unwrap();
    assert_eq!(colr.version(), 0);
    let l4 = colr.v0_layers(4).unwrap();
    assert_eq!(l4.len(), 2);
    assert_eq!((l4[0].glyph_id, l4[0].palette_index), (11, 0));
    assert_eq!((l4[1].glyph_id, l4[1].palette_index), (12, 1));
    let l9 = colr.v0_layers(9).unwrap();
    assert_eq!(l9.len(), 1);
    // 0xFFFF palette index = text foreground.
    assert_eq!(
        l9[0].palette_index,
        oxideav_otf::COLR_FOREGROUND_PALETTE_INDEX
    );
    assert!(colr.v0_layers(5).is_none());
    // No version-1 content.
    assert!(colr.base_glyph_paint(4).is_none());
}

#[test]
fn v0_layer_run_out_of_range_rejected() {
    let mut b = Vec::new();
    b.extend_from_slice(&u16b(0));
    b.extend_from_slice(&u16b(1));
    b.extend_from_slice(&u32b(14));
    b.extend_from_slice(&u32b(20));
    b.extend_from_slice(&u16b(1));
    // gid 4 -> layers[1..3] but only 1 layer record exists.
    b.extend_from_slice(&u16b(4));
    b.extend_from_slice(&u16b(1));
    b.extend_from_slice(&u16b(2));
    b.extend_from_slice(&u16b(11));
    b.extend_from_slice(&u16b(0));
    assert!(matches!(ColrTable::parse(&b), Err(Error::BadStructure(_))));
}

// ---- version 1: static paint formats ---------------------------------------

/// Build a table whose LayerList holds one paint of every static
/// format (and the root PaintColrLayers over all of them).
#[test]
fn v1_all_static_paint_formats() {
    let mut t = ColrV1::new();

    // 0: PaintSolid.
    t.paints.push(paint_solid(7, 0.5));
    // 1: PaintLinearGradient + 2-stop pad color line at offset 16.
    {
        let mut p = vec![4u8];
        p.extend_from_slice(&u24b(16));
        for v in [10i16, 20, 30, 40, 50, 60] {
            p.extend_from_slice(&i16b(v));
        }
        p.extend_from_slice(&color_line(0, &[(0.0, 1, 1.0), (1.0, 2, 0.25)]));
        t.paints.push(p);
    }
    // 2: PaintRadialGradient, repeat extend.
    {
        let mut p = vec![6u8];
        p.extend_from_slice(&u24b(16));
        p.extend_from_slice(&i16b(-5)); // x0
        p.extend_from_slice(&i16b(-6)); // y0
        p.extend_from_slice(&u16b(7)); // radius0
        p.extend_from_slice(&i16b(8)); // x1
        p.extend_from_slice(&i16b(9)); // y1
        p.extend_from_slice(&u16b(10)); // radius1
        p.extend_from_slice(&color_line(1, &[(0.0, 3, 1.0)]));
        t.paints.push(p);
    }
    // 3: PaintSweepGradient, reflect extend; F2DOT14 -2.0 -> -180°,
    // 0.0 -> +180°.
    {
        let mut p = vec![8u8];
        p.extend_from_slice(&u24b(12));
        p.extend_from_slice(&i16b(100));
        p.extend_from_slice(&i16b(200));
        p.extend_from_slice(&f2(-2.0));
        p.extend_from_slice(&f2(0.0));
        p.extend_from_slice(&color_line(2, &[(0.5, 4, 1.0)]));
        t.paints.push(p);
    }
    // 4: PaintGlyph wrapping a solid (child at blob offset 6).
    {
        let mut p = vec![10u8];
        p.extend_from_slice(&u24b(6));
        p.extend_from_slice(&u16b(42));
        p.extend_from_slice(&paint_solid(1, 1.0));
        t.paints.push(p);
    }
    // 5: PaintColrGlyph referencing base glyph 5 (added below).
    {
        let mut p = vec![11u8];
        p.extend_from_slice(&u16b(5));
        t.paints.push(p);
    }
    // 6: PaintTransform (affine at 7, child at 31).
    {
        let mut p = vec![12u8];
        p.extend_from_slice(&u24b(31));
        p.extend_from_slice(&u24b(7));
        for v in [1.5f32, 0.25, -0.5, 2.0, 10.0, -20.0] {
            p.extend_from_slice(&fixed(v));
        }
        p.extend_from_slice(&paint_solid(1, 1.0));
        t.paints.push(p);
    }
    // 7: PaintTranslate (child at 8).
    {
        let mut p = vec![14u8];
        p.extend_from_slice(&u24b(8));
        p.extend_from_slice(&i16b(100));
        p.extend_from_slice(&i16b(-200));
        p.extend_from_slice(&paint_solid(1, 1.0));
        t.paints.push(p);
    }
    // 8: PaintScale.
    {
        let mut p = vec![16u8];
        p.extend_from_slice(&u24b(8));
        p.extend_from_slice(&f2(0.5));
        p.extend_from_slice(&f2(1.5));
        p.extend_from_slice(&paint_solid(1, 1.0));
        t.paints.push(p);
    }
    // 9: PaintScaleAroundCenter.
    {
        let mut p = vec![18u8];
        p.extend_from_slice(&u24b(12));
        p.extend_from_slice(&f2(0.5));
        p.extend_from_slice(&f2(1.5));
        p.extend_from_slice(&i16b(11));
        p.extend_from_slice(&i16b(22));
        p.extend_from_slice(&paint_solid(1, 1.0));
        t.paints.push(p);
    }
    // 10: PaintScaleUniform.
    {
        let mut p = vec![20u8];
        p.extend_from_slice(&u24b(6));
        p.extend_from_slice(&f2(1.25));
        p.extend_from_slice(&paint_solid(1, 1.0));
        t.paints.push(p);
    }
    // 11: PaintScaleUniformAroundCenter.
    {
        let mut p = vec![22u8];
        p.extend_from_slice(&u24b(10));
        p.extend_from_slice(&f2(1.25));
        p.extend_from_slice(&i16b(-3));
        p.extend_from_slice(&i16b(4));
        p.extend_from_slice(&paint_solid(1, 1.0));
        t.paints.push(p);
    }
    // 12: PaintRotate — 0.5 -> 90° (no bias).
    {
        let mut p = vec![24u8];
        p.extend_from_slice(&u24b(6));
        p.extend_from_slice(&f2(0.5));
        p.extend_from_slice(&paint_solid(1, 1.0));
        t.paints.push(p);
    }
    // 13: PaintRotateAroundCenter.
    {
        let mut p = vec![26u8];
        p.extend_from_slice(&u24b(10));
        p.extend_from_slice(&f2(-0.25));
        p.extend_from_slice(&i16b(6));
        p.extend_from_slice(&i16b(7));
        p.extend_from_slice(&paint_solid(1, 1.0));
        t.paints.push(p);
    }
    // 14: PaintSkew.
    {
        let mut p = vec![28u8];
        p.extend_from_slice(&u24b(8));
        p.extend_from_slice(&f2(0.1));
        p.extend_from_slice(&f2(-0.1));
        p.extend_from_slice(&paint_solid(1, 1.0));
        t.paints.push(p);
    }
    // 15: PaintSkewAroundCenter.
    {
        let mut p = vec![30u8];
        p.extend_from_slice(&u24b(12));
        p.extend_from_slice(&f2(0.1));
        p.extend_from_slice(&f2(-0.1));
        p.extend_from_slice(&i16b(1));
        p.extend_from_slice(&i16b(2));
        p.extend_from_slice(&paint_solid(1, 1.0));
        t.paints.push(p);
    }
    // 16: PaintComposite (source solid at 8, backdrop solid at 13),
    // with an unrecognized mode byte (falls back to Clear).
    {
        let mut p = vec![32u8];
        p.extend_from_slice(&u24b(8));
        p.push(200u8);
        p.extend_from_slice(&u24b(13));
        p.extend_from_slice(&paint_solid(1, 1.0));
        p.extend_from_slice(&paint_solid(2, 1.0));
        t.paints.push(p);
    }
    // 17: an unrecognized future paint format.
    t.paints.push(vec![33u8, 0, 0, 0]);

    // Root: PaintColrLayers over all 18 layers.
    let n = t.paints.len();
    {
        let mut p = vec![1u8];
        p.push(n as u8);
        p.extend_from_slice(&u32b(0));
        t.paints.push(p);
    }
    t.layers = (0..n).collect();
    t.base_glyphs = vec![(5, n)];

    let bytes = t.build();
    let colr = ColrTable::parse(&bytes).unwrap();
    assert_eq!(colr.version(), 1);
    assert_eq!(colr.layer_list_len(), 18);

    let root = colr.base_glyph_paint(5).unwrap();
    let Paint::ColrLayers {
        num_layers,
        first_layer_index,
    } = colr.paint(root, None).unwrap()
    else {
        panic!("root must be ColrLayers");
    };
    assert_eq!((num_layers, first_layer_index), (18, 0));
    let layers = colr.layers(first_layer_index, num_layers).unwrap();

    // Layer 0: solid.
    match colr.paint(layers[0], None).unwrap() {
        Paint::Solid {
            palette_index,
            alpha,
            var_index_base,
        } => {
            assert_eq!(palette_index, 7);
            assert!((alpha - 0.5).abs() < 1e-4);
            assert_eq!(var_index_base, None);
        }
        p => panic!("layer 0: {p:?}"),
    }
    // Layer 1: linear gradient.
    match colr.paint(layers[1], None).unwrap() {
        Paint::LinearGradient {
            color_line,
            x0,
            y0,
            x1,
            y1,
            x2,
            y2,
            ..
        } => {
            assert_eq!(color_line.extend, Extend::Pad);
            assert_eq!(color_line.stops.len(), 2);
            assert!((color_line.stops[0].stop_offset - 0.0).abs() < 1e-4);
            assert_eq!(color_line.stops[1].palette_index, 2);
            assert!((color_line.stops[1].alpha - 0.25).abs() < 1e-4);
            assert_eq!(
                [x0, y0, x1, y1, x2, y2],
                [10.0, 20.0, 30.0, 40.0, 50.0, 60.0]
            );
        }
        p => panic!("layer 1: {p:?}"),
    }
    // Layer 2: radial gradient.
    match colr.paint(layers[2], None).unwrap() {
        Paint::RadialGradient {
            color_line,
            x0,
            y0,
            radius0,
            x1,
            y1,
            radius1,
            ..
        } => {
            assert_eq!(color_line.extend, Extend::Repeat);
            assert_eq!(
                [x0, y0, radius0, x1, y1, radius1],
                [-5.0, -6.0, 7.0, 8.0, 9.0, 10.0]
            );
        }
        p => panic!("layer 2: {p:?}"),
    }
    // Layer 3: sweep gradient with angle bias.
    match colr.paint(layers[3], None).unwrap() {
        Paint::SweepGradient {
            color_line,
            center_x,
            center_y,
            start_angle,
            end_angle,
            ..
        } => {
            assert_eq!(color_line.extend, Extend::Reflect);
            assert_eq!([center_x, center_y], [100.0, 200.0]);
            assert!((start_angle - (-180.0)).abs() < 1e-3);
            assert!((end_angle - 180.0).abs() < 1e-3);
        }
        p => panic!("layer 3: {p:?}"),
    }
    // Layer 4: glyph clip.
    let Paint::Glyph { paint, glyph_id } = colr.paint(layers[4], None).unwrap() else {
        panic!("layer 4");
    };
    assert_eq!(glyph_id, 42);
    assert!(matches!(
        colr.paint(paint, None).unwrap(),
        Paint::Solid {
            palette_index: 1,
            ..
        }
    ));
    // Layer 5: colr glyph reuse.
    assert!(matches!(
        colr.paint(layers[5], None).unwrap(),
        Paint::ColrGlyph { glyph_id: 5 }
    ));
    // Layer 6: transform matrix.
    match colr.paint(layers[6], None).unwrap() {
        Paint::Transform { transform: m, .. } => {
            assert!((m.xx - 1.5).abs() < 1e-4);
            assert!((m.yx - 0.25).abs() < 1e-4);
            assert!((m.xy - (-0.5)).abs() < 1e-4);
            assert!((m.yy - 2.0).abs() < 1e-4);
            assert!((m.dx - 10.0).abs() < 1e-4);
            assert!((m.dy - (-20.0)).abs() < 1e-4);
            // x' = xx·x + xy·y + dx.
            let (x, y) = m.apply(2.0, 4.0);
            assert!((x - (1.5 * 2.0 - 0.5 * 4.0 + 10.0)).abs() < 1e-3);
            assert!((y - (0.25 * 2.0 + 2.0 * 4.0 - 20.0)).abs() < 1e-3);
        }
        p => panic!("layer 6: {p:?}"),
    }
    // Layer 7: translate.
    assert!(matches!(
        colr.paint(layers[7], None).unwrap(),
        Paint::Translate { dx, dy, .. } if dx == 100.0 && dy == -200.0
    ));
    // Layers 8-11: the scale family.
    assert!(matches!(
        colr.paint(layers[8], None).unwrap(),
        Paint::Scale { scale_x, scale_y, .. }
            if (scale_x - 0.5).abs() < 1e-4 && (scale_y - 1.5).abs() < 1e-4
    ));
    assert!(matches!(
        colr.paint(layers[9], None).unwrap(),
        Paint::ScaleAroundCenter {
            center_x: 11.0,
            center_y: 22.0,
            ..
        }
    ));
    assert!(matches!(
        colr.paint(layers[10], None).unwrap(),
        Paint::ScaleUniform { scale, .. } if (scale - 1.25).abs() < 1e-4
    ));
    assert!(matches!(
        colr.paint(layers[11], None).unwrap(),
        Paint::ScaleUniformAroundCenter {
            center_x: -3.0,
            center_y: 4.0,
            ..
        }
    ));
    // Layers 12-13: rotate (0.5 -> 90°, unbiased).
    assert!(matches!(
        colr.paint(layers[12], None).unwrap(),
        Paint::Rotate { angle, .. } if (angle - 90.0).abs() < 1e-3
    ));
    assert!(matches!(
        colr.paint(layers[13], None).unwrap(),
        Paint::RotateAroundCenter { angle, center_x: 6.0, center_y: 7.0, .. }
            if (angle - (-45.0)).abs() < 1e-3
    ));
    // Layers 14-15: skew (0.1 -> 18°).
    assert!(matches!(
        colr.paint(layers[14], None).unwrap(),
        Paint::Skew { x_skew_angle, y_skew_angle, .. }
            if (x_skew_angle - 18.0).abs() < 0.02 && (y_skew_angle + 18.0).abs() < 0.02
    ));
    assert!(matches!(
        colr.paint(layers[15], None).unwrap(),
        Paint::SkewAroundCenter {
            center_x: 1.0,
            center_y: 2.0,
            ..
        }
    ));
    // Layer 16: composite; unknown mode byte falls back to Clear.
    let Paint::Composite {
        source,
        mode,
        backdrop,
    } = colr.paint(layers[16], None).unwrap()
    else {
        panic!("layer 16");
    };
    assert_eq!(mode, CompositeMode::Clear);
    assert!(matches!(
        colr.paint(source, None).unwrap(),
        Paint::Solid {
            palette_index: 1,
            ..
        }
    ));
    assert!(matches!(
        colr.paint(backdrop, None).unwrap(),
        Paint::Solid {
            palette_index: 2,
            ..
        }
    ));
    // Layer 17: unrecognized format is surfaced, not an error.
    assert!(matches!(
        colr.paint(layers[17], None).unwrap(),
        Paint::Unknown { format: 33 }
    ));

    // The whole graph validates (the PaintColrGlyph self-reference at
    // layer 5 makes the root graph cyclic, though!).
    assert!(matches!(
        colr.validate_color_glyph(5),
        Err(Error::BadStructure(_))
    ));
}

// ---- version 1: variations -------------------------------------------------

/// Build the variable test table: PaintVarSolid, a VarColorLine,
/// PaintVarTranslate, a VarAffine2x3, and a format-2 clip box, driven
/// by a single-axis IVS.
fn variable_table() -> Vec<u8> {
    let mut t = ColrV1::new();

    // Delta rows (int16, region (0,1,1) — scaled by the coordinate):
    //   0: solid alpha +0.25 (F2DOT14 4096)
    //   1: color stop offset -0.5 (F2DOT14 -8192)
    //   2: translate dx +50    3: translate dy -30
    //   4..7: clip box deltas +25, +0, +25, +10
    //   8: affine xx +32767/65536 (~0.5 in Fixed units)
    //   9..13: zero rows so the affine's yx/xy/yy/dx/dy sequence
    //          indices resolve to no adjustment (indices past the map
    //          would otherwise clamp to the last entry, per spec).
    t.ivs = ivs_rows(&[4096, -8192, 50, -30, 25, 0, 25, 10, 0x7FFF, 0, 0, 0, 0, 0]);
    t.var_index_map = identity_map(14);

    // 0: PaintVarSolid, raw alpha 0.25, vib 0.
    {
        let mut p = vec![3u8];
        p.extend_from_slice(&u16b(9));
        p.extend_from_slice(&f2(0.25));
        p.extend_from_slice(&u32b(0));
        t.paints.push(p);
    }
    // 1: PaintVarLinearGradient with a VarColorLine: stop A raw 0.6
    // (vib 1 → delta row 1), stop B raw 0.5 (no-data sentinel).
    {
        let mut p = vec![5u8];
        p.extend_from_slice(&u24b(20));
        for v in [0i16, 0, 100, 0, 0, 100] {
            p.extend_from_slice(&i16b(v));
        }
        p.extend_from_slice(&u32b(0xFFFF_FFFF)); // geometry: no variation
        p.extend_from_slice(&var_color_line(
            0,
            &[(0.6, 1, 1.0, 1), (0.5, 2, 1.0, 0xFFFF_FFFF)],
        ));
        t.paints.push(p);
    }
    // 2: PaintVarTranslate raw (100, 200), vib 2.
    {
        let mut p = vec![15u8];
        p.extend_from_slice(&u24b(12));
        p.extend_from_slice(&i16b(100));
        p.extend_from_slice(&i16b(200));
        p.extend_from_slice(&u32b(2));
        p.extend_from_slice(&paint_solid(1, 1.0));
        t.paints.push(p);
    }
    // 3: PaintVarTransform with VarAffine2x3 (vib 8 → only xx moves…
    // sequence starts at xx, so row 8 hits xx). Give dx the +0.5
    // delta instead by pointing vib at 8-4… keep it simple: vib 8,
    // so xx += 0.5 at full coordinate.
    {
        let mut p = vec![13u8];
        p.extend_from_slice(&u24b(35));
        p.extend_from_slice(&u24b(7));
        for v in [1.0f32, 0.0, 0.0, 1.0, 0.0, 0.0] {
            p.extend_from_slice(&fixed(v));
        }
        p.extend_from_slice(&u32b(8));
        p.extend_from_slice(&paint_solid(1, 1.0));
        t.paints.push(p);
    }
    // Root over the 4 layers.
    {
        let mut p = vec![1u8];
        p.push(4u8);
        p.extend_from_slice(&u32b(0));
        t.paints.push(p);
    }
    t.layers = vec![0, 1, 2, 3];
    t.base_glyphs = vec![(7, 4)];

    // ClipList: gid range 7..=8, format-2 box, vib 4.
    {
        let mut c = vec![1u8]; // ClipList format
        c.extend_from_slice(&u32b(1)); // numClips
        c.extend_from_slice(&u16b(7));
        c.extend_from_slice(&u16b(8));
        c.extend_from_slice(&u24b(12)); // box at clipList+12
        assert_eq!(c.len(), 12);
        c.push(2u8); // ClipBox format 2
        c.extend_from_slice(&i16b(10));
        c.extend_from_slice(&i16b(-20));
        c.extend_from_slice(&i16b(100));
        c.extend_from_slice(&i16b(200));
        c.extend_from_slice(&u32b(4));
        t.clip_list = c;
    }

    t.build()
}

#[test]
fn v1_variable_paints() {
    let bytes = variable_table();
    let colr = ColrTable::parse(&bytes).unwrap();
    assert!(colr.var_index_map().is_some());
    assert!(colr.item_variation_store().is_some());

    let root = colr.base_glyph_paint(7).unwrap();
    let layers = match colr.paint(root, None).unwrap() {
        Paint::ColrLayers {
            num_layers,
            first_layer_index,
        } => colr.layers(first_layer_index, num_layers).unwrap(),
        p => panic!("{p:?}"),
    };
    let full = [1.0f32];
    let half = [0.5f32];

    // Solid alpha: default 0.25; +0.25 at full, +0.125 at half.
    for (coords, want) in [
        (None, 0.25f32),
        (Some(&full[..]), 0.5),
        (Some(&half[..]), 0.375),
    ] {
        match colr.paint(layers[0], coords).unwrap() {
            Paint::Solid { alpha, .. } => assert!((alpha - want).abs() < 1e-4, "{want}"),
            p => panic!("{p:?}"),
        }
    }

    // VarColorLine: static order sorts raw offsets [0.5, 0.6]; at the
    // full instance stop A moves 0.6→0.1 and sorts first.
    match colr.paint(layers[1], None).unwrap() {
        Paint::LinearGradient { color_line, .. } => {
            assert_eq!(color_line.stops[0].palette_index, 2);
            assert!((color_line.stops[1].stop_offset - 0.6).abs() < 1e-4);
        }
        p => panic!("{p:?}"),
    }
    match colr.paint(layers[1], Some(&full)).unwrap() {
        Paint::LinearGradient { color_line, .. } => {
            assert_eq!(color_line.stops[0].palette_index, 1);
            assert!((color_line.stops[0].stop_offset - 0.1).abs() < 1e-4);
            assert!((color_line.stops[1].stop_offset - 0.5).abs() < 1e-4);
        }
        p => panic!("{p:?}"),
    }

    // Translate deltas: FWORD units.
    match colr.paint(layers[2], Some(&full)).unwrap() {
        Paint::Translate { dx, dy, .. } => {
            assert!((dx - 150.0).abs() < 1e-3);
            assert!((dy - 170.0).abs() < 1e-3);
        }
        p => panic!("{p:?}"),
    }

    // VarAffine2x3: xx += 32767/65536 at the full instance.
    match colr.paint(layers[3], Some(&full)).unwrap() {
        Paint::Transform { transform, .. } => {
            assert!((transform.xx - (1.0 + 32767.0 / 65536.0)).abs() < 1e-4);
            assert!((transform.yy - 1.0).abs() < 1e-6);
        }
        p => panic!("{p:?}"),
    }

    // Clip box: static values pass through; at the half instance the
    // deltas are 12.5/0/12.5/5 and the box expands (floor mins, ceil
    // maxes).
    let stat = colr.clip_box(7, None).unwrap().unwrap();
    assert_eq!(
        (stat.x_min, stat.y_min, stat.x_max, stat.y_max),
        (10.0, -20.0, 100.0, 200.0)
    );
    let var = colr.clip_box(8, Some(&half)).unwrap().unwrap();
    assert_eq!(var.format, 2);
    assert_eq!(
        (var.x_min, var.y_min, var.x_max, var.y_max),
        (22.0, -20.0, 113.0, 205.0)
    );
    // Outside the range: no clip box.
    assert!(colr.clip_box(6, None).unwrap().is_none());
    assert!(colr.clip_box(9, None).unwrap().is_none());

    // The graph is acyclic and validates.
    colr.validate_color_glyph(7).unwrap();
}

/// Without a varIndexMap, the varIndexBase-derived value is used
/// directly as the delta-set index (outer = high 16 bits, inner =
/// low 16 bits).
#[test]
fn v1_implicit_identity_mapping() {
    let mut t = ColrV1::new();
    t.ivs = ivs_rows(&[4096, -4096]);
    // PaintVarSolid vib 1 → inner index 1 → delta -0.25.
    {
        let mut p = vec![3u8];
        p.extend_from_slice(&u16b(0));
        p.extend_from_slice(&f2(0.5));
        p.extend_from_slice(&u32b(1));
        t.paints.push(p);
    }
    t.base_glyphs = vec![(1, 0)];
    let bytes = t.build();
    let colr = ColrTable::parse(&bytes).unwrap();
    assert!(colr.var_index_map().is_none());
    let root = colr.base_glyph_paint(1).unwrap();
    match colr.paint(root, Some(&[1.0])).unwrap() {
        Paint::Solid { alpha, .. } => assert!((alpha - 0.25).abs() < 1e-4),
        p => panic!("{p:?}"),
    }
}

/// Single-axis IVS whose one subtable uses the LONG_WORDS delta form:
/// one int32 ("word") delta per row against region (0, 1, 1). The
/// wordDeltaCount field is `0x8000 | 1`.
fn ivs_rows_long(rows: &[i32]) -> Vec<u8> {
    let mut v = Vec::new();
    v.extend_from_slice(&u16b(1)); // format
    v.extend_from_slice(&u32b(12)); // regionListOffset
    v.extend_from_slice(&u16b(1)); // ivdCount
    v.extend_from_slice(&u32b(22)); // ivd[0]
    assert_eq!(v.len(), 12);
    v.extend_from_slice(&u16b(1)); // axisCount
    v.extend_from_slice(&u16b(1)); // regionCount
    v.extend_from_slice(&f2(0.0));
    v.extend_from_slice(&f2(1.0));
    v.extend_from_slice(&f2(1.0));
    assert_eq!(v.len(), 22);
    v.extend_from_slice(&u16b(rows.len() as u16)); // itemCount
    v.extend_from_slice(&u16b(0x8000 | 1)); // LONG_WORDS | wordDeltaCount 1
    v.extend_from_slice(&u16b(1)); // regionIndexCount
    v.extend_from_slice(&u16b(0)); // regionIndex 0
    for &d in rows {
        v.extend_from_slice(&d.to_be_bytes());
    }
    v
}

/// A 32-bit LONG_WORDS delta beyond int16 range applied to a `Fixed`
/// item: per the variations common-formats chapter, the Fixed value is
/// treated like a 32-bit integer (delta in 1/65536 units), and Fixed
/// deltas in general need the LONG_WORDS ItemVariationData form.
#[test]
fn v1_long_words_fixed_delta() {
    let mut t = ColrV1::new();
    // Row 0: +131072 = exactly +2.0 on a Fixed (16.16) value — far
    // outside what an int16 delta could carry.
    t.ivs = ivs_rows_long(&[131_072]);
    // PaintVarTransform, identity affine, vib 0 → sequence index 0
    // targets xx; the remaining sequence indices (1..=5) have no rows
    // and resolve to zero adjustment.
    {
        let mut p = vec![13u8];
        p.extend_from_slice(&u24b(35));
        p.extend_from_slice(&u24b(7));
        for v in [1.0f32, 0.0, 0.0, 1.0, 0.0, 0.0] {
            p.extend_from_slice(&fixed(v));
        }
        p.extend_from_slice(&u32b(0));
        p.extend_from_slice(&paint_solid(1, 1.0));
        t.paints.push(p);
    }
    t.base_glyphs = vec![(3, 0)];
    let bytes = t.build();
    let colr = ColrTable::parse(&bytes).unwrap();
    let root = colr.base_glyph_paint(3).unwrap();
    // Default instance: untouched.
    match colr.paint(root, None).unwrap() {
        Paint::Transform { transform, .. } => assert!((transform.xx - 1.0).abs() < 1e-6),
        p => panic!("{p:?}"),
    }
    // Full instance: xx = 1.0 + 131072/65536 = 3.0 exactly.
    match colr.paint(root, Some(&[1.0])).unwrap() {
        Paint::Transform { transform, .. } => {
            assert!((transform.xx - 3.0).abs() < 1e-5);
            assert!((transform.yy - 1.0).abs() < 1e-6);
            assert!((transform.dx - 0.0).abs() < 1e-6);
        }
        p => panic!("{p:?}"),
    }
    // Half instance: xx = 2.0.
    match colr.paint(root, Some(&[0.5])).unwrap() {
        Paint::Transform { transform, .. } => assert!((transform.xx - 2.0).abs() < 1e-5),
        p => panic!("{p:?}"),
    }
}

// ---- graph analysis --------------------------------------------------------

#[test]
fn cycle_detection_across_colr_glyphs() {
    // gid 1 -> PaintColrGlyph(2), gid 2 -> PaintColrGlyph(1).
    let mut t = ColrV1::new();
    for gid in [2u16, 1] {
        let mut p = vec![11u8];
        p.extend_from_slice(&u16b(gid));
        t.paints.push(p);
    }
    t.base_glyphs = vec![(1, 0), (2, 1)];
    let bytes = t.build();
    let colr = ColrTable::parse(&bytes).unwrap();
    assert!(matches!(
        colr.validate_color_glyph(1),
        Err(Error::BadStructure(_))
    ));
    assert!(matches!(
        colr.is_bounded(colr.base_glyph_paint(2).unwrap()),
        Err(Error::BadStructure(_))
    ));
}

#[test]
fn colr_glyph_to_missing_base_is_invalid() {
    let mut t = ColrV1::new();
    let mut p = vec![11u8];
    p.extend_from_slice(&u16b(99)); // no BaseGlyphPaintRecord for 99
    t.paints.push(p);
    t.base_glyphs = vec![(1, 0)];
    let colr_bytes = t.build();
    let colr = ColrTable::parse(&colr_bytes).unwrap();
    assert!(matches!(
        colr.validate_color_glyph(1),
        Err(Error::BadStructure(_))
    ));
}

#[test]
fn boundedness_rules() {
    let mut t = ColrV1::new();
    // 0: bare solid (unbounded).
    t.paints.push(paint_solid(0, 1.0));
    // 1: glyph-clipped solid (bounded).
    {
        let mut p = vec![10u8];
        p.extend_from_slice(&u24b(6));
        p.extend_from_slice(&u16b(3));
        p.extend_from_slice(&paint_solid(0, 1.0));
        t.paints.push(p);
    }
    // 2: rotate of bounded child (bounded).
    {
        let mut p = vec![24u8];
        p.extend_from_slice(&u24b(6));
        p.extend_from_slice(&f2(0.5));
        // child: glyph-clipped solid.
        p.extend_from_slice(&{
            let mut q = vec![10u8];
            q.extend_from_slice(&u24b(6));
            q.extend_from_slice(&u16b(3));
            q.extend_from_slice(&paint_solid(0, 1.0));
            q
        });
        t.paints.push(p);
    }
    // 3: composite SRC_OVER of bounded source over unbounded backdrop
    // (unbounded: both must be bounded).
    {
        let mut p = vec![32u8];
        p.extend_from_slice(&u24b(8)); // source: bounded glyph at 8
        p.push(3u8); // SRC_OVER
        p.extend_from_slice(&u24b(19)); // backdrop: bare solid at 19
        let mut g = vec![10u8];
        g.extend_from_slice(&u24b(6));
        g.extend_from_slice(&u16b(3));
        g.extend_from_slice(&paint_solid(0, 1.0)); // 11 bytes total
        p.extend_from_slice(&g);
        p.extend_from_slice(&paint_solid(0, 1.0));
        t.paints.push(p);
    }
    // 4: same composite but SRC_IN (bounded: either suffices).
    {
        let mut p = vec![32u8];
        p.extend_from_slice(&u24b(8));
        p.push(5u8); // SRC_IN
        p.extend_from_slice(&u24b(19));
        let mut g = vec![10u8];
        g.extend_from_slice(&u24b(6));
        g.extend_from_slice(&u16b(3));
        g.extend_from_slice(&paint_solid(0, 1.0));
        p.extend_from_slice(&g);
        p.extend_from_slice(&paint_solid(0, 1.0));
        t.paints.push(p);
    }
    // 5: layers over [bounded glyph, bare solid] (unbounded).
    {
        let mut p = vec![1u8, 2u8];
        p.extend_from_slice(&u32b(0));
        t.paints.push(p);
    }
    // 6: layers over [bounded glyph] only (bounded).
    {
        let mut p = vec![1u8, 1u8];
        p.extend_from_slice(&u32b(1));
        t.paints.push(p);
    }
    t.layers = vec![0, 1];
    t.base_glyphs = vec![(1, 0), (2, 1), (3, 2), (4, 3), (5, 4), (6, 5), (7, 6)];
    let bytes = t.build();
    let colr = ColrTable::parse(&bytes).unwrap();
    let bounded = |gid: u16| {
        colr.is_bounded(colr.base_glyph_paint(gid).unwrap())
            .unwrap()
    };
    assert!(!bounded(1), "bare solid");
    assert!(bounded(2), "glyph clip");
    assert!(bounded(3), "transform of bounded");
    assert!(!bounded(4), "src-over needs both");
    assert!(bounded(5), "src-in needs either");
    assert!(!bounded(6), "layers with an unbounded layer");
    assert!(bounded(7), "layers, all bounded");
}

#[test]
fn layer_slice_out_of_range_is_error() {
    let mut t = ColrV1::new();
    t.paints.push(paint_solid(0, 1.0));
    // Root asks for 2 layers but the LayerList has 1.
    {
        let mut p = vec![1u8, 2u8];
        p.extend_from_slice(&u32b(0));
        t.paints.push(p);
    }
    t.layers = vec![0];
    t.base_glyphs = vec![(1, 1)];
    let bytes = t.build();
    let colr = ColrTable::parse(&bytes).unwrap();
    assert!(matches!(colr.layers(0, 2), Err(Error::BadStructure(_))));
    assert!(matches!(
        colr.validate_color_glyph(1),
        Err(Error::BadStructure(_))
    ));
}

/// Exhaustive single-byte mutation + truncation robustness: every
/// mutant must either fail to parse or survive full-surface queries
/// (paint decoding at two instances, graph validation, boundedness,
/// clip-box lookup) with `Result` errors only — never a panic, hang,
/// or runaway allocation.
#[test]
fn mutation_robustness() {
    let base = variable_table();

    let exercise = |bytes: &[u8]| {
        let Ok(colr) = ColrTable::parse(bytes) else {
            return;
        };
        let roots: Vec<_> = colr.base_glyph_paints().collect();
        for &(gid, root) in &roots {
            let _ = colr.paint(root, None);
            let _ = colr.paint(root, Some(&[1.0]));
            let _ = colr.validate_color_glyph(gid);
            let _ = colr.is_bounded(root);
            let _ = colr.clip_box(gid, Some(&[0.5]));
        }
        for gid in [0u16, 7, 8, 0xFFFF] {
            let _ = colr.v0_layers(gid);
            let _ = colr.clip_box(gid, None);
        }
    };

    // Single-byte mutations: three interesting values per position.
    for i in 0..base.len() {
        for v in [0x00u8, 0xFF, base[i].wrapping_add(1)] {
            let mut m = base.clone();
            m[i] = v;
            exercise(&m);
        }
    }
    // Every truncation length.
    for len in 0..base.len() {
        exercise(&base[..len]);
    }
}

#[test]
fn paint_format_zero_is_error() {
    let mut t = ColrV1::new();
    t.paints.push(vec![0u8, 0, 0, 0]);
    t.base_glyphs = vec![(1, 0)];
    let bytes = t.build();
    let colr = ColrTable::parse(&bytes).unwrap();
    let root = colr.base_glyph_paint(1).unwrap();
    assert!(matches!(
        colr.paint(root, None),
        Err(Error::BadStructure(_))
    ));
}
