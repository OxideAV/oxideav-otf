//! `COLR` — Color Table (OpenType 1.9.1), versions 0 and 1.
//!
//! Version 0 defines a color glyph as a flat, bottom-up z-ordered run
//! of `(glyphID, paletteIndex)` layer records. Version 1 additionally
//! defines color glyphs as a **directed acyclic graph of Paint
//! tables** (formats 1–32): solid fills, linear / radial / sweep
//! gradients, affine transforms, compositing / blending, glyph-outline
//! clipping, and cross-glyph reuse — all optionally variable through
//! the COLR-embedded `ItemVariationStore` + `DeltaSetIndexMap` and the
//! per-table `varIndexBase` base/sequence scheme.
//!
//! This module decodes both versions into a typed surface:
//!
//! - [`ColrTable::v0_layers`] — the version-0 layer run for a base
//!   glyph.
//! - [`ColrTable::base_glyph_paint`] — the version-1 paint-graph root
//!   for a base glyph, as a [`PaintRef`].
//! - [`ColrTable::paint`] — decode one Paint table into the [`Paint`]
//!   enum. Child paints stay as [`PaintRef`] handles so arbitrarily
//!   shared DAGs decode in linear space; pass instance coordinates to
//!   resolve the `PaintVar*` delta sets in the same call.
//! - [`ColrTable::layers`] — the `LayerList` slice referenced by a
//!   [`Paint::ColrLayers`].
//! - [`ColrTable::clip_box`] — the precomputed (optionally variable)
//!   clip box for a base glyph.
//! - [`ColrTable::validate_color_glyph`] / [`ColrTable::is_bounded`] —
//!   whole-graph well-formedness: acyclicity, decodability of every
//!   reachable node, and the spec's boundedness rules.
//!
//! Geometry fields are converted to `f32`: `FWORD` / `UFWORD` design
//! units pass through numerically, `F2DOT14` scale factors and alphas
//! are divided by 16384, `Fixed` affine components by 65536, and
//! angles are returned in **degrees** (sweep-gradient angles carry the
//! spec's +1.0 bias before the ×180° conversion; rotation / skew
//! angles are unbiased).

use std::collections::{HashMap, HashSet};

use crate::parser::{read_f2dot14, read_fixed, read_i16, read_u16, read_u24, read_u32, read_u8};
use crate::tables::cpal::{ColorRecord, CpalTable};
use crate::tables::ivs::{DeltaSetIndexMap, ItemVariationStore};
use crate::Error;

/// `paletteIndex` value that selects the application-determined text
/// foreground color instead of a `CPAL` entry.
pub const COLR_FOREGROUND_PALETTE_INDEX: u16 = 0xFFFF;

/// `varIndexBase` sentinel: the table/record has no variation data.
const NO_VARIATION_INDEX: u32 = 0xFFFF_FFFF;

/// A version-0 `BaseGlyph` record: a `(firstLayerIndex, numLayers)`
/// run into the layer-records array.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BaseGlyphRecord {
    /// Glyph ID of the base glyph.
    pub glyph_id: u16,
    /// Index (base 0) into the layer-records array.
    pub first_layer_index: u16,
    /// Number of color layers associated with this glyph.
    pub num_layers: u16,
}

/// A version-0 `Layer` record: one glyph outline filled with one
/// palette color.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LayerRecord {
    /// Glyph ID of the glyph used for this layer.
    pub glyph_id: u16,
    /// `CPAL` palette-entry index; [`COLR_FOREGROUND_PALETTE_INDEX`]
    /// selects the text foreground color.
    pub palette_index: u16,
}

/// An opaque handle to one Paint table inside the `COLR` data:
/// decode it with [`ColrTable::paint`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PaintRef(u32);

/// A concrete, render-ready sRGB color: a `CPAL` record's channels
/// with the `COLR` paint alpha multiplied into the record's own alpha.
///
/// Channels are **not** premultiplied by the alpha.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ResolvedColor {
    /// Red channel (sRGB).
    pub red: u8,
    /// Green channel (sRGB).
    pub green: u8,
    /// Blue channel (sRGB).
    pub blue: u8,
    /// Effective alpha in `0.0..=1.0`:
    /// `paint alpha × (CPAL record alpha / 255)`.
    pub alpha: f32,
}

impl ResolvedColor {
    /// The color as an 8-bit `[r, g, b, a]` quadruple (alpha rounded
    /// to the nearest of 256 levels).
    pub fn rgba8(&self) -> [u8; 4] {
        [
            self.red,
            self.green,
            self.blue,
            (self.alpha.clamp(0.0, 1.0) * 255.0).round() as u8,
        ]
    }
}

/// Resolve a `COLR` `(paletteIndex, alpha)` pair to a concrete color
/// against one `CPAL` palette.
///
/// `palette_index == 0xFFFF` ([`COLR_FOREGROUND_PALETTE_INDEX`])
/// selects `foreground` — the application-determined text foreground
/// color, identical across palettes — instead of a `CPAL` entry.
/// Otherwise the entry comes from `cpal.color(palette, palette_index)`
/// and `None` means the index is out of the palette's range (a
/// malformed color glyph). Per the spec the paint `alpha` (clamped to
/// `[0, 1]`) is multiplied with the selected record's own alpha
/// (`record alpha / 255`).
pub fn resolve_paint_color(
    cpal: &CpalTable<'_>,
    palette: u16,
    palette_index: u16,
    alpha: f32,
    foreground: ColorRecord,
) -> Option<ResolvedColor> {
    let record = if palette_index == COLR_FOREGROUND_PALETTE_INDEX {
        foreground
    } else {
        cpal.color(palette, palette_index)?
    };
    Some(ResolvedColor {
        red: record.red,
        green: record.green,
        blue: record.blue,
        alpha: (alpha.clamp(0.0, 1.0) * record.alpha_f32()).clamp(0.0, 1.0),
    })
}

impl LayerRecord {
    /// The layer's concrete fill color against one `CPAL` palette. A
    /// version-0 layer carries no alpha of its own, so the result is
    /// the record's color (or `foreground` for the 0xFFFF sentinel)
    /// with the record's own alpha; `None` when `palette_index` is out
    /// of the palette's range.
    pub fn resolve(
        &self,
        cpal: &CpalTable<'_>,
        palette: u16,
        foreground: ColorRecord,
    ) -> Option<ResolvedColor> {
        resolve_paint_color(cpal, palette, self.palette_index, 1.0, foreground)
    }
}

/// `Extend` — color-line behavior outside the `[0, 1]` stop range.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Extend {
    /// Use the nearest color stop.
    Pad,
    /// Repeat from the farthest color stop.
    Repeat,
    /// Mirror the color line from the nearest end.
    Reflect,
}

impl Extend {
    /// Decode an `extend` byte. Unrecognized values fall back to
    /// [`Extend::Pad`], as the spec requires.
    pub fn from_u8(v: u8) -> Self {
        match v {
            1 => Extend::Repeat,
            2 => Extend::Reflect,
            _ => Extend::Pad,
        }
    }
}

/// One color stop on a color line. In a `VarColorLine`,
/// `var_index_base` carries the record's delta-set base (sequence:
/// +0 `stopOffset`, +1 `alpha`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColorStop {
    /// Position on the color line (instance-resolved when the paint
    /// was decoded with coordinates).
    pub stop_offset: f32,
    /// `CPAL` palette-entry index; [`COLR_FOREGROUND_PALETTE_INDEX`]
    /// selects the text foreground color.
    pub palette_index: u16,
    /// Alpha, clamped to `[0, 1]`; multiplied with the `CPAL` entry's
    /// own alpha when rendering.
    pub alpha: f32,
    /// The record's `varIndexBase` (`None` in a non-variable
    /// `ColorLine`, or when the record carries the no-data sentinel).
    pub var_index_base: Option<u32>,
}

impl ColorStop {
    /// The stop's concrete color against one `CPAL` palette: the
    /// record selected by `palette_index` (or `foreground` for the
    /// 0xFFFF sentinel) with the stop's alpha multiplied in; `None`
    /// when `palette_index` is out of the palette's range.
    pub fn resolve(
        &self,
        cpal: &CpalTable<'_>,
        palette: u16,
        foreground: ColorRecord,
    ) -> Option<ResolvedColor> {
        resolve_paint_color(cpal, palette, self.palette_index, self.alpha, foreground)
    }
}

/// A color line: an `extend` mode plus its color stops, sorted by
/// ascending (instance-resolved) `stop_offset` as the spec's rendering
/// order requires.
#[derive(Debug, Clone, PartialEq)]
pub struct ColorLine {
    /// Behavior outside the stop range.
    pub extend: Extend,
    /// Color stops in ascending `stop_offset` order (stable-sorted, so
    /// same-offset stops keep their file order).
    pub stops: Vec<ColorStop>,
}

impl ColorLine {
    /// Every stop resolved to `(stop_offset, color)` against one
    /// `CPAL` palette, in the line's ascending-offset order. `None`
    /// when any stop's `palette_index` is out of the palette's range.
    pub fn resolve(
        &self,
        cpal: &CpalTable<'_>,
        palette: u16,
        foreground: ColorRecord,
    ) -> Option<Vec<(f32, ResolvedColor)>> {
        self.stops
            .iter()
            .map(|s| Some((s.stop_offset, s.resolve(cpal, palette, foreground)?)))
            .collect()
    }
}

/// A 2×3 affine transformation matrix (`Affine2x3` / `VarAffine2x3`).
///
/// Maps `(x, y)` to `(xx·x + xy·y + dx, yx·x + yy·y + dy)`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Affine2x3 {
    /// x-component of the transformed x-basis vector.
    pub xx: f32,
    /// y-component of the transformed x-basis vector.
    pub yx: f32,
    /// x-component of the transformed y-basis vector.
    pub xy: f32,
    /// y-component of the transformed y-basis vector.
    pub yy: f32,
    /// Translation in the x direction.
    pub dx: f32,
    /// Translation in the y direction.
    pub dy: f32,
}

impl Affine2x3 {
    /// Apply the matrix to a point.
    pub fn apply(&self, x: f32, y: f32) -> (f32, f32) {
        (
            self.xx * x + self.xy * y + self.dx,
            self.yx * x + self.yy * y + self.dy,
        )
    }
}

/// `CompositeMode` — how a [`Paint::Composite`] source combines with
/// its backdrop (Porter-Duff modes 0–12, separable blend modes 13–23,
/// non-separable HSL blend modes 24–27).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(missing_docs)]
pub enum CompositeMode {
    Clear,
    Src,
    Dest,
    SrcOver,
    DestOver,
    SrcIn,
    DestIn,
    SrcOut,
    DestOut,
    SrcAtop,
    DestAtop,
    Xor,
    Plus,
    Screen,
    Overlay,
    Darken,
    Lighten,
    ColorDodge,
    ColorBurn,
    HardLight,
    SoftLight,
    Difference,
    Exclusion,
    Multiply,
    HslHue,
    HslSaturation,
    HslColor,
    HslLuminosity,
}

impl CompositeMode {
    /// Decode a `compositeMode` byte. Unrecognized values fall back to
    /// [`CompositeMode::Clear`], as the spec requires.
    pub fn from_u8(v: u8) -> Self {
        use CompositeMode::*;
        match v {
            0 => Clear,
            1 => Src,
            2 => Dest,
            3 => SrcOver,
            4 => DestOver,
            5 => SrcIn,
            6 => DestIn,
            7 => SrcOut,
            8 => DestOut,
            9 => SrcAtop,
            10 => DestAtop,
            11 => Xor,
            12 => Plus,
            13 => Screen,
            14 => Overlay,
            15 => Darken,
            16 => Lighten,
            17 => ColorDodge,
            18 => ColorBurn,
            19 => HardLight,
            20 => SoftLight,
            21 => Difference,
            22 => Exclusion,
            23 => Multiply,
            24 => HslHue,
            25 => HslSaturation,
            26 => HslColor,
            27 => HslLuminosity,
            _ => Clear,
        }
    }
}

/// A decoded Paint table. The 14 `PaintVar*` on-disk formats decode to
/// the same variant as their static counterpart, with `var_index_base`
/// set to the table's `varIndexBase` (and, when the paint was decoded
/// with instance coordinates, the geometry already delta-adjusted).
#[derive(Debug, Clone, PartialEq)]
pub enum Paint {
    /// Format 1 — a slice of the `LayerList`, composited bottom-up
    /// with source-over. Resolve the child paints with
    /// [`ColrTable::layers`].
    ColrLayers {
        /// Number of layer offsets to read from the `LayerList`.
        num_layers: u8,
        /// Index (base 0) into the `LayerList`.
        first_layer_index: u32,
    },
    /// Formats 2 / 3 — a solid palette-color fill.
    Solid {
        /// `CPAL` palette-entry index.
        palette_index: u16,
        /// Alpha, clamped to `[0, 1]`.
        alpha: f32,
        /// `varIndexBase` (sequence: +0 `alpha`).
        var_index_base: Option<u32>,
    },
    /// Formats 4 / 5 — a linear gradient along `p₀→p₁` with rotation
    /// point `p₂`.
    LinearGradient {
        /// The gradient's color line.
        color_line: ColorLine,
        /// Start point (p₀) x, in design units.
        x0: f32,
        /// Start point (p₀) y.
        y0: f32,
        /// End point (p₁) x.
        x1: f32,
        /// End point (p₁) y.
        y1: f32,
        /// Rotation point (p₂) x.
        x2: f32,
        /// Rotation point (p₂) y.
        y2: f32,
        /// `varIndexBase` (sequence: x0, y0, x1, y1, x2, y2).
        var_index_base: Option<u32>,
    },
    /// Formats 6 / 7 — a radial gradient between two circles. Radii
    /// are unsigned in storage but deltas may drive them negative in
    /// variable fonts.
    RadialGradient {
        /// The gradient's color line.
        color_line: ColorLine,
        /// Start circle center x, in design units.
        x0: f32,
        /// Start circle center y.
        y0: f32,
        /// Start circle radius.
        radius0: f32,
        /// End circle center x.
        x1: f32,
        /// End circle center y.
        y1: f32,
        /// End circle radius.
        radius1: f32,
        /// `varIndexBase` (sequence: x0, y0, radius0, x1, y1, radius1).
        var_index_base: Option<u32>,
    },
    /// Formats 8 / 9 — a sweep gradient around a center point. Angles
    /// are in counter-clockwise **degrees** with the spec's +1.0 bias
    /// already applied (stored F2DOT14 −2.0 → −180°, 0.0 → +180°,
    /// 1.0 → +360°).
    SweepGradient {
        /// The gradient's color line.
        color_line: ColorLine,
        /// Center x, in design units.
        center_x: f32,
        /// Center y.
        center_y: f32,
        /// Start angle in degrees (bias applied).
        start_angle: f32,
        /// End angle in degrees (bias applied).
        end_angle: f32,
        /// `varIndexBase` (sequence: centerX, centerY, startAngle,
        /// endAngle).
        var_index_base: Option<u32>,
    },
    /// Format 10 — clip the child paint to a glyph outline.
    Glyph {
        /// The fill sub-graph.
        paint: PaintRef,
        /// Glyph ID of the source outline (a `glyf` / `CFF ` / `CFF2`
        /// glyph; any COLR data for it is ignored here).
        glyph_id: u16,
    },
    /// Format 11 — reuse another base glyph's entire paint graph.
    ColrGlyph {
        /// Glyph ID of a `BaseGlyphList` base glyph.
        glyph_id: u16,
    },
    /// Formats 12 / 13 — an arbitrary affine transform of the child.
    Transform {
        /// The transformed sub-graph.
        paint: PaintRef,
        /// The 2×3 matrix (instance-resolved for `VarAffine2x3` when
        /// decoded with coordinates).
        transform: Affine2x3,
        /// The `VarAffine2x3`'s `varIndexBase` (sequence: xx, yx, xy,
        /// yy, dx, dy).
        var_index_base: Option<u32>,
    },
    /// Formats 14 / 15 — translation.
    Translate {
        /// The transformed sub-graph.
        paint: PaintRef,
        /// Translation in x, in design units.
        dx: f32,
        /// Translation in y.
        dy: f32,
        /// `varIndexBase` (sequence: dx, dy).
        var_index_base: Option<u32>,
    },
    /// Formats 16 / 17 — scale about the origin.
    Scale {
        /// The transformed sub-graph.
        paint: PaintRef,
        /// Scale factor in x.
        scale_x: f32,
        /// Scale factor in y.
        scale_y: f32,
        /// `varIndexBase` (sequence: scaleX, scaleY).
        var_index_base: Option<u32>,
    },
    /// Formats 18 / 19 — scale about a center point.
    ScaleAroundCenter {
        /// The transformed sub-graph.
        paint: PaintRef,
        /// Scale factor in x.
        scale_x: f32,
        /// Scale factor in y.
        scale_y: f32,
        /// Center of scaling, x (design units).
        center_x: f32,
        /// Center of scaling, y.
        center_y: f32,
        /// `varIndexBase` (sequence: scaleX, scaleY, centerX, centerY).
        var_index_base: Option<u32>,
    },
    /// Formats 20 / 21 — uniform scale about the origin.
    ScaleUniform {
        /// The transformed sub-graph.
        paint: PaintRef,
        /// Scale factor in x and y.
        scale: f32,
        /// `varIndexBase` (sequence: scale).
        var_index_base: Option<u32>,
    },
    /// Formats 22 / 23 — uniform scale about a center point.
    ScaleUniformAroundCenter {
        /// The transformed sub-graph.
        paint: PaintRef,
        /// Scale factor in x and y.
        scale: f32,
        /// Center of scaling, x (design units).
        center_x: f32,
        /// Center of scaling, y.
        center_y: f32,
        /// `varIndexBase` (sequence: scale, centerX, centerY).
        var_index_base: Option<u32>,
    },
    /// Formats 24 / 25 — rotation about the origin. The angle is in
    /// counter-clockwise degrees (stored F2DOT14 × 180°, no bias).
    Rotate {
        /// The transformed sub-graph.
        paint: PaintRef,
        /// Rotation angle in degrees.
        angle: f32,
        /// `varIndexBase` (sequence: angle).
        var_index_base: Option<u32>,
    },
    /// Formats 26 / 27 — rotation about a center point.
    RotateAroundCenter {
        /// The transformed sub-graph.
        paint: PaintRef,
        /// Rotation angle in degrees.
        angle: f32,
        /// Center of rotation, x (design units).
        center_x: f32,
        /// Center of rotation, y.
        center_y: f32,
        /// `varIndexBase` (sequence: angle, centerX, centerY).
        var_index_base: Option<u32>,
    },
    /// Formats 28 / 29 — skew about the origin. Angles are in degrees
    /// (stored F2DOT14 × 180°, no bias).
    Skew {
        /// The transformed sub-graph.
        paint: PaintRef,
        /// Skew angle in the x-axis direction, degrees.
        x_skew_angle: f32,
        /// Skew angle in the y-axis direction, degrees.
        y_skew_angle: f32,
        /// `varIndexBase` (sequence: xSkewAngle, ySkewAngle).
        var_index_base: Option<u32>,
    },
    /// Formats 30 / 31 — skew about a center point.
    SkewAroundCenter {
        /// The transformed sub-graph.
        paint: PaintRef,
        /// Skew angle in the x-axis direction, degrees.
        x_skew_angle: f32,
        /// Skew angle in the y-axis direction, degrees.
        y_skew_angle: f32,
        /// Center of skew, x (design units).
        center_x: f32,
        /// Center of skew, y.
        center_y: f32,
        /// `varIndexBase` (sequence: xSkewAngle, ySkewAngle, centerX,
        /// centerY).
        var_index_base: Option<u32>,
    },
    /// Format 32 — render the backdrop, render the source, combine
    /// them with `mode`.
    Composite {
        /// The source sub-graph.
        source: PaintRef,
        /// How the source combines into the backdrop.
        mode: CompositeMode,
        /// The backdrop (destination) sub-graph.
        backdrop: PaintRef,
    },
    /// A paint format this implementation does not recognize (formats
    /// above 32, reserved for future minor versions). The spec directs
    /// implementations to ignore such paints.
    Unknown {
        /// The unrecognized `format` byte.
        format: u8,
    },
}

/// A (possibly variable) clip box from the `ClipList`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClipBox {
    /// The on-disk `ClipBox` format: 1 (static) or 2 (variable).
    pub format: u8,
    /// Minimum x of the clip box, design units.
    pub x_min: f32,
    /// Minimum y of the clip box.
    pub y_min: f32,
    /// Maximum x of the clip box.
    pub x_max: f32,
    /// Maximum y of the clip box.
    pub y_max: f32,
    /// The format-2 `varIndexBase` (sequence: xMin, yMin, xMax, yMax).
    pub var_index_base: Option<u32>,
}

/// A `Clip` record: a base-glyph-ID range and its clip-box offset.
#[derive(Debug, Clone, Copy)]
struct ClipRecord {
    start_glyph_id: u16,
    end_glyph_id: u16,
    /// Absolute offset of the ClipBox within the COLR data.
    clip_box_offset: u32,
}

/// A parsed `COLR` table.
#[derive(Debug)]
pub struct ColrTable<'a> {
    data: &'a [u8],
    version: u16,
    /// Version-0 BaseGlyph records, sorted by glyph ID per spec.
    base_glyph_records: Vec<BaseGlyphRecord>,
    /// Version-0 Layer records.
    layer_records: Vec<LayerRecord>,
    /// Version-1 `(glyphID, absolute paint offset)` pairs, sorted by
    /// glyph ID per spec.
    base_glyph_paints: Vec<(u16, u32)>,
    /// Version-1 `LayerList` paint offsets (absolute), bottom-up
    /// z-order.
    layer_list: Vec<u32>,
    /// Version-1 `ClipList` records, sorted by `startGlyphID`.
    clip_records: Vec<ClipRecord>,
    /// The `varIndexMap` `DeltaSetIndexMap`, when present.
    var_index_map: Option<DeltaSetIndexMap>,
    /// The COLR-embedded `ItemVariationStore`, when present.
    ivs: Option<ItemVariationStore>,
}

impl<'a> ColrTable<'a> {
    /// Parse a `COLR` table. Versions above 1 parse with version-1
    /// structure (the version-1 header is a forward-compatible prefix
    /// of any future minor version).
    pub fn parse(data: &'a [u8]) -> Result<Self, Error> {
        let version = read_u16(data, 0)?;
        let num_base_glyph_records = read_u16(data, 2)? as usize;
        let base_glyph_records_offset = read_u32(data, 4)? as usize;
        let layer_records_offset = read_u32(data, 8)? as usize;
        let num_layer_records = read_u16(data, 12)? as usize;

        let mut base_glyph_records = Vec::new();
        if base_glyph_records_offset != 0 && num_base_glyph_records != 0 {
            base_glyph_records.reserve(num_base_glyph_records.min(data.len() / 6));
            for i in 0..num_base_glyph_records {
                let off = base_glyph_records_offset + i * 6;
                base_glyph_records.push(BaseGlyphRecord {
                    glyph_id: read_u16(data, off)?,
                    first_layer_index: read_u16(data, off + 2)?,
                    num_layers: read_u16(data, off + 4)?,
                });
            }
        }
        let mut layer_records = Vec::new();
        if layer_records_offset != 0 && num_layer_records != 0 {
            layer_records.reserve(num_layer_records.min(data.len() / 4));
            for i in 0..num_layer_records {
                let off = layer_records_offset + i * 4;
                layer_records.push(LayerRecord {
                    glyph_id: read_u16(data, off)?,
                    palette_index: read_u16(data, off + 2)?,
                });
            }
        }
        // Version-0 layer runs must stay inside the layer-records
        // array.
        for r in &base_glyph_records {
            let end = r.first_layer_index as usize + r.num_layers as usize;
            if end > layer_records.len() {
                return Err(Error::BadStructure(
                    "COLR: BaseGlyph layer run exceeds layerRecords",
                ));
            }
        }

        let mut table = Self {
            data,
            version,
            base_glyph_records,
            layer_records,
            base_glyph_paints: Vec::new(),
            layer_list: Vec::new(),
            clip_records: Vec::new(),
            var_index_map: None,
            ivs: None,
        };
        if version >= 1 {
            table.parse_v1_header()?;
        }
        Ok(table)
    }

    fn parse_v1_header(&mut self) -> Result<(), Error> {
        let data = self.data;
        let base_glyph_list_offset = read_u32(data, 14)? as usize;
        let layer_list_offset = read_u32(data, 18)? as usize;
        let clip_list_offset = read_u32(data, 22)? as usize;
        let var_index_map_offset = read_u32(data, 26)? as usize;
        let ivs_offset = read_u32(data, 30)? as usize;

        if base_glyph_list_offset != 0 {
            let n = read_u32(data, base_glyph_list_offset)? as usize;
            // Each record is 6 bytes; cap the reserve by what can
            // physically fit.
            self.base_glyph_paints.reserve(n.min(data.len() / 6));
            for i in 0..n {
                let off = base_glyph_list_offset + 4 + i * 6;
                let glyph_id = read_u16(data, off)?;
                let paint_offset = read_u32(data, off + 2)?;
                if paint_offset == 0 {
                    return Err(Error::BadStructure("COLR: NULL BaseGlyphPaint offset"));
                }
                let abs = (base_glyph_list_offset as u32)
                    .checked_add(paint_offset)
                    .ok_or(Error::BadOffset)?;
                self.base_glyph_paints.push((glyph_id, abs));
            }
        }
        if layer_list_offset != 0 {
            let n = read_u32(data, layer_list_offset)? as usize;
            self.layer_list.reserve(n.min(data.len() / 4));
            for i in 0..n {
                let paint_offset = read_u32(data, layer_list_offset + 4 + i * 4)?;
                let abs = (layer_list_offset as u32)
                    .checked_add(paint_offset)
                    .ok_or(Error::BadOffset)?;
                self.layer_list.push(abs);
            }
        }
        if clip_list_offset != 0 {
            let format = read_u8(data, clip_list_offset)?;
            if format != 1 {
                return Err(Error::BadStructure("COLR: ClipList format must be 1"));
            }
            let n = read_u32(data, clip_list_offset + 1)? as usize;
            self.clip_records.reserve(n.min(data.len() / 7));
            for i in 0..n {
                let off = clip_list_offset + 5 + i * 7;
                let start_glyph_id = read_u16(data, off)?;
                let end_glyph_id = read_u16(data, off + 2)?;
                let clip_box_offset = read_u24(data, off + 4)?;
                let abs = (clip_list_offset as u32)
                    .checked_add(clip_box_offset)
                    .ok_or(Error::BadOffset)?;
                self.clip_records.push(ClipRecord {
                    start_glyph_id,
                    end_glyph_id,
                    clip_box_offset: abs,
                });
            }
        }
        if var_index_map_offset != 0 {
            self.var_index_map = Some(DeltaSetIndexMap::parse_at(data, var_index_map_offset)?);
        }
        if ivs_offset != 0 {
            self.ivs = Some(ItemVariationStore::parse_at(data, ivs_offset)?);
        }
        Ok(())
    }

    /// The table version (0 or 1).
    pub fn version(&self) -> u16 {
        self.version
    }

    // ---- version 0 ---------------------------------------------------------

    /// The version-0 BaseGlyph records (may be non-empty in a
    /// version-1 table too, as a fallback for old applications).
    pub fn base_glyph_records(&self) -> &[BaseGlyphRecord] {
        &self.base_glyph_records
    }

    /// All version-0 Layer records.
    pub fn layer_records(&self) -> &[LayerRecord] {
        &self.layer_records
    }

    /// The version-0 layer run for a base glyph — bottom-up z-order —
    /// or `None` if the glyph has no version-0 color glyph. Records
    /// are binary-searched (the spec requires them sorted by glyph
    /// ID).
    pub fn v0_layers(&self, glyph_id: u16) -> Option<&[LayerRecord]> {
        let i = self
            .base_glyph_records
            .binary_search_by_key(&glyph_id, |r| r.glyph_id)
            .ok()?;
        let r = &self.base_glyph_records[i];
        let start = r.first_layer_index as usize;
        Some(&self.layer_records[start..start + r.num_layers as usize])
    }

    // ---- version 1 ---------------------------------------------------------

    /// The version-1 `(glyphID, paint root)` pairs, in glyph-ID order.
    pub fn base_glyph_paints(&self) -> impl Iterator<Item = (u16, PaintRef)> + '_ {
        self.base_glyph_paints
            .iter()
            .map(|&(gid, off)| (gid, PaintRef(off)))
    }

    /// The root Paint of a base glyph's version-1 color-glyph graph,
    /// or `None` if the glyph has no `BaseGlyphPaintRecord`. Per spec,
    /// a version-1 color glyph takes precedence over a version-0 one
    /// for the same glyph ID.
    pub fn base_glyph_paint(&self, glyph_id: u16) -> Option<PaintRef> {
        let i = self
            .base_glyph_paints
            .binary_search_by_key(&glyph_id, |&(g, _)| g)
            .ok()?;
        Some(PaintRef(self.base_glyph_paints[i].1))
    }

    /// Number of `LayerList` entries.
    pub fn layer_list_len(&self) -> usize {
        self.layer_list.len()
    }

    /// Resolve a [`Paint::ColrLayers`] slice into the `LayerList`:
    /// `num_layers` paint refs starting at `first_layer_index`, in
    /// bottom-up z-order.
    pub fn layers(&self, first_layer_index: u32, num_layers: u8) -> Result<Vec<PaintRef>, Error> {
        let start = first_layer_index as usize;
        let end = start + num_layers as usize;
        if end > self.layer_list.len() {
            return Err(Error::BadStructure(
                "COLR: PaintColrLayers slice exceeds LayerList",
            ));
        }
        Ok(self.layer_list[start..end]
            .iter()
            .map(|&o| PaintRef(o))
            .collect())
    }

    /// The `varIndexMap`, when present.
    pub fn var_index_map(&self) -> Option<&DeltaSetIndexMap> {
        self.var_index_map.as_ref()
    }

    /// The COLR-embedded `ItemVariationStore`, when present.
    pub fn item_variation_store(&self) -> Option<&ItemVariationStore> {
        self.ivs.as_ref()
    }

    // ---- variation resolution ----------------------------------------------

    /// Interpolated delta (in the stored integer scale) for variable
    /// field number `seq` of a table whose base is `var_index_base`,
    /// under the §7 base/sequence rules: the 0xFFFFFFFF base sentinel,
    /// the map's clamp-to-last-entry rule, the 0xFFFF/0xFFFF "no data"
    /// mapping, and the implicit identity map (high 16 bits = outer,
    /// low 16 = inner) when no `varIndexMap` is present.
    fn var_delta(&self, var_index_base: Option<u32>, seq: u32, coords: Option<&[f32]>) -> f32 {
        let (Some(base), Some(coords)) = (var_index_base, coords) else {
            return 0.0;
        };
        if base == NO_VARIATION_INDEX {
            return 0.0;
        }
        let Some(ivs) = self.ivs.as_ref() else {
            // No ItemVariationStore: varIndexBase is ignored.
            return 0.0;
        };
        // The index sequence must not wrap past 0xFFFFFFFF.
        let Some(idx) = base.checked_add(seq) else {
            return 0.0;
        };
        let (outer, inner) = match self.var_index_map.as_ref() {
            Some(map) => map.index_u32(idx),
            None => ((idx >> 16) as u16, (idx & 0xFFFF) as u16),
        };
        if outer == 0xFFFF && inner == 0xFFFF {
            return 0.0;
        }
        ivs.delta(outer, inner, coords)
    }

    /// Delta-adjusted `FWORD`/`UFWORD` value (deltas are in design
    /// units).
    fn var_fword(&self, raw: f32, vib: Option<u32>, seq: u32, coords: Option<&[f32]>) -> f32 {
        raw + self.var_delta(vib, seq, coords)
    }

    /// Delta-adjusted `F2DOT14` value. Per the variations
    /// common-formats chapter (DeltaSet record note), "the F2DOT14
    /// value is treated like a 16-bit integer" — delta and value are
    /// integers in 1/16384 units — and where a context constrains the
    /// value's range (e.g. alpha in `[0, 1]`), the post-delta value is
    /// clamped to that range (clamping is applied at each use site).
    fn var_f2dot14(&self, raw: f32, vib: Option<u32>, seq: u32, coords: Option<&[f32]>) -> f32 {
        raw + self.var_delta(vib, seq, coords) / 16384.0
    }

    /// Delta-adjusted `Fixed` value. Per the variations common-formats
    /// chapter (DeltaSet record note), "the Fixed value is treated
    /// like a 32-bit integer" — delta and value are integers in
    /// 1/65536 units; 32-bit deltas come from the `LONG_WORDS` form of
    /// `ItemVariationData`, which [`crate::tables::ivs`] decodes.
    fn var_fixed(&self, raw: f32, vib: Option<u32>, seq: u32, coords: Option<&[f32]>) -> f32 {
        raw + self.var_delta(vib, seq, coords) / 65536.0
    }

    // ---- paint decoding ----------------------------------------------------

    /// Decode the Paint table at `paint_ref`.
    ///
    /// With `coords = Some(normalized)` — a normalized instance
    /// coordinate tuple (`Font::normalize_coords`) — every `PaintVar*`
    /// field is returned instance-resolved through the COLR
    /// `ItemVariationStore`. With `coords = None` the stored (default
    /// instance) values are returned. Child paints are returned as
    /// [`PaintRef`] handles; decode them with further `paint` calls
    /// (see [`ColrTable::validate_color_glyph`] for cycle-safe
    /// whole-graph traversal).
    pub fn paint(&self, paint_ref: PaintRef, coords: Option<&[f32]>) -> Result<Paint, Error> {
        let data = self.data;
        let at = paint_ref.0 as usize;
        let format = read_u8(data, at)?;

        // Absolute child-paint ref from an Offset24 field of this
        // table.
        let child = |field_off: usize| -> Result<PaintRef, Error> {
            let off = read_u24(data, at + field_off)?;
            if off == 0 {
                return Err(Error::BadStructure("COLR: NULL child paint offset"));
            }
            Ok(PaintRef(
                (paint_ref.0).checked_add(off).ok_or(Error::BadOffset)?,
            ))
        };

        Ok(match format {
            0 => return Err(Error::BadStructure("COLR: paint format 0 is invalid")),
            1 => Paint::ColrLayers {
                num_layers: read_u8(data, at + 1)?,
                first_layer_index: read_u32(data, at + 2)?,
            },
            2 | 3 => {
                let palette_index = read_u16(data, at + 1)?;
                let alpha = read_f2dot14(data, at + 3)?;
                let vib = if format == 3 {
                    Some(read_u32(data, at + 5)?)
                } else {
                    None
                };
                Paint::Solid {
                    palette_index,
                    alpha: self.var_f2dot14(alpha, vib, 0, coords).clamp(0.0, 1.0),
                    var_index_base: vib,
                }
            }
            4 | 5 => {
                let variable = format == 5;
                let vib = if variable {
                    Some(read_u32(data, at + 16)?)
                } else {
                    None
                };
                let mut pt = [0f32; 6];
                for (i, p) in pt.iter_mut().enumerate() {
                    let raw = read_i16(data, at + 4 + i * 2)? as f32;
                    *p = self.var_fword(raw, vib, i as u32, coords);
                }
                Paint::LinearGradient {
                    color_line: self.color_line(paint_ref, 1, variable, coords)?,
                    x0: pt[0],
                    y0: pt[1],
                    x1: pt[2],
                    y1: pt[3],
                    x2: pt[4],
                    y2: pt[5],
                    var_index_base: vib,
                }
            }
            6 | 7 => {
                let variable = format == 7;
                let vib = if variable {
                    Some(read_u32(data, at + 16)?)
                } else {
                    None
                };
                let x0 = read_i16(data, at + 4)? as f32;
                let y0 = read_i16(data, at + 6)? as f32;
                let radius0 = read_u16(data, at + 8)? as f32;
                let x1 = read_i16(data, at + 10)? as f32;
                let y1 = read_i16(data, at + 12)? as f32;
                let radius1 = read_u16(data, at + 14)? as f32;
                Paint::RadialGradient {
                    color_line: self.color_line(paint_ref, 1, variable, coords)?,
                    x0: self.var_fword(x0, vib, 0, coords),
                    y0: self.var_fword(y0, vib, 1, coords),
                    // Deltas may legitimately drive a radius negative;
                    // the rendering algorithm handles it, so no clamp.
                    radius0: self.var_fword(radius0, vib, 2, coords),
                    x1: self.var_fword(x1, vib, 3, coords),
                    y1: self.var_fword(y1, vib, 4, coords),
                    radius1: self.var_fword(radius1, vib, 5, coords),
                    var_index_base: vib,
                }
            }
            8 | 9 => {
                let variable = format == 9;
                let vib = if variable {
                    Some(read_u32(data, at + 12)?)
                } else {
                    None
                };
                let center_x = read_i16(data, at + 4)? as f32;
                let center_y = read_i16(data, at + 6)? as f32;
                let start_angle = read_f2dot14(data, at + 8)?;
                let end_angle = read_f2dot14(data, at + 10)?;
                Paint::SweepGradient {
                    color_line: self.color_line(paint_ref, 1, variable, coords)?,
                    center_x: self.var_fword(center_x, vib, 0, coords),
                    center_y: self.var_fword(center_y, vib, 1, coords),
                    // Sweep angles carry a +1.0 bias before the ×180°
                    // conversion so ±360° is representable.
                    start_angle: (self.var_f2dot14(start_angle, vib, 2, coords) + 1.0) * 180.0,
                    end_angle: (self.var_f2dot14(end_angle, vib, 3, coords) + 1.0) * 180.0,
                    var_index_base: vib,
                }
            }
            10 => Paint::Glyph {
                paint: child(1)?,
                glyph_id: read_u16(data, at + 4)?,
            },
            11 => Paint::ColrGlyph {
                glyph_id: read_u16(data, at + 1)?,
            },
            12 | 13 => {
                let paint = child(1)?;
                let transform_off = read_u24(data, at + 4)?;
                if transform_off == 0 {
                    return Err(Error::BadStructure("COLR: NULL Affine2x3 offset"));
                }
                let t = (paint_ref.0)
                    .checked_add(transform_off)
                    .ok_or(Error::BadOffset)? as usize;
                let vib = if format == 13 {
                    Some(read_u32(data, t + 24)?)
                } else {
                    None
                };
                let mut m = [0f32; 6];
                for (i, c) in m.iter_mut().enumerate() {
                    let raw = read_fixed(data, t + i * 4)?;
                    *c = self.var_fixed(raw, vib, i as u32, coords);
                }
                Paint::Transform {
                    paint,
                    transform: Affine2x3 {
                        xx: m[0],
                        yx: m[1],
                        xy: m[2],
                        yy: m[3],
                        dx: m[4],
                        dy: m[5],
                    },
                    var_index_base: vib,
                }
            }
            14 | 15 => {
                let paint = child(1)?;
                let vib = if format == 15 {
                    Some(read_u32(data, at + 8)?)
                } else {
                    None
                };
                let dx = read_i16(data, at + 4)? as f32;
                let dy = read_i16(data, at + 6)? as f32;
                Paint::Translate {
                    paint,
                    dx: self.var_fword(dx, vib, 0, coords),
                    dy: self.var_fword(dy, vib, 1, coords),
                    var_index_base: vib,
                }
            }
            16 | 17 => {
                let paint = child(1)?;
                let vib = if format == 17 {
                    Some(read_u32(data, at + 8)?)
                } else {
                    None
                };
                let sx = read_f2dot14(data, at + 4)?;
                let sy = read_f2dot14(data, at + 6)?;
                Paint::Scale {
                    paint,
                    scale_x: self.var_f2dot14(sx, vib, 0, coords),
                    scale_y: self.var_f2dot14(sy, vib, 1, coords),
                    var_index_base: vib,
                }
            }
            18 | 19 => {
                let paint = child(1)?;
                let vib = if format == 19 {
                    Some(read_u32(data, at + 12)?)
                } else {
                    None
                };
                let sx = read_f2dot14(data, at + 4)?;
                let sy = read_f2dot14(data, at + 6)?;
                let cx = read_i16(data, at + 8)? as f32;
                let cy = read_i16(data, at + 10)? as f32;
                Paint::ScaleAroundCenter {
                    paint,
                    scale_x: self.var_f2dot14(sx, vib, 0, coords),
                    scale_y: self.var_f2dot14(sy, vib, 1, coords),
                    center_x: self.var_fword(cx, vib, 2, coords),
                    center_y: self.var_fword(cy, vib, 3, coords),
                    var_index_base: vib,
                }
            }
            20 | 21 => {
                let paint = child(1)?;
                let vib = if format == 21 {
                    Some(read_u32(data, at + 6)?)
                } else {
                    None
                };
                let s = read_f2dot14(data, at + 4)?;
                Paint::ScaleUniform {
                    paint,
                    scale: self.var_f2dot14(s, vib, 0, coords),
                    var_index_base: vib,
                }
            }
            22 | 23 => {
                let paint = child(1)?;
                let vib = if format == 23 {
                    Some(read_u32(data, at + 10)?)
                } else {
                    None
                };
                let s = read_f2dot14(data, at + 4)?;
                let cx = read_i16(data, at + 6)? as f32;
                let cy = read_i16(data, at + 8)? as f32;
                Paint::ScaleUniformAroundCenter {
                    paint,
                    scale: self.var_f2dot14(s, vib, 0, coords),
                    center_x: self.var_fword(cx, vib, 1, coords),
                    center_y: self.var_fword(cy, vib, 2, coords),
                    var_index_base: vib,
                }
            }
            24 | 25 => {
                let paint = child(1)?;
                let vib = if format == 25 {
                    Some(read_u32(data, at + 6)?)
                } else {
                    None
                };
                let a = read_f2dot14(data, at + 4)?;
                Paint::Rotate {
                    paint,
                    // 1.0 = 180° counter-clockwise, no bias.
                    angle: self.var_f2dot14(a, vib, 0, coords) * 180.0,
                    var_index_base: vib,
                }
            }
            26 | 27 => {
                let paint = child(1)?;
                let vib = if format == 27 {
                    Some(read_u32(data, at + 10)?)
                } else {
                    None
                };
                let a = read_f2dot14(data, at + 4)?;
                let cx = read_i16(data, at + 6)? as f32;
                let cy = read_i16(data, at + 8)? as f32;
                Paint::RotateAroundCenter {
                    paint,
                    angle: self.var_f2dot14(a, vib, 0, coords) * 180.0,
                    center_x: self.var_fword(cx, vib, 1, coords),
                    center_y: self.var_fword(cy, vib, 2, coords),
                    var_index_base: vib,
                }
            }
            28 | 29 => {
                let paint = child(1)?;
                let vib = if format == 29 {
                    Some(read_u32(data, at + 8)?)
                } else {
                    None
                };
                let xa = read_f2dot14(data, at + 4)?;
                let ya = read_f2dot14(data, at + 6)?;
                Paint::Skew {
                    paint,
                    x_skew_angle: self.var_f2dot14(xa, vib, 0, coords) * 180.0,
                    y_skew_angle: self.var_f2dot14(ya, vib, 1, coords) * 180.0,
                    var_index_base: vib,
                }
            }
            30 | 31 => {
                let paint = child(1)?;
                let vib = if format == 31 {
                    Some(read_u32(data, at + 12)?)
                } else {
                    None
                };
                let xa = read_f2dot14(data, at + 4)?;
                let ya = read_f2dot14(data, at + 6)?;
                let cx = read_i16(data, at + 8)? as f32;
                let cy = read_i16(data, at + 10)? as f32;
                Paint::SkewAroundCenter {
                    paint,
                    x_skew_angle: self.var_f2dot14(xa, vib, 0, coords) * 180.0,
                    y_skew_angle: self.var_f2dot14(ya, vib, 1, coords) * 180.0,
                    center_x: self.var_fword(cx, vib, 2, coords),
                    center_y: self.var_fword(cy, vib, 3, coords),
                    var_index_base: vib,
                }
            }
            32 => Paint::Composite {
                source: child(1)?,
                mode: CompositeMode::from_u8(read_u8(data, at + 4)?),
                backdrop: child(5)?,
            },
            // Formats above 32 are reserved for future minor versions;
            // the spec directs implementations to ignore them.
            f => Paint::Unknown { format: f },
        })
    }

    /// Decode a (Var)ColorLine reached via the Offset24 at `field_off`
    /// within the paint table at `paint_ref`.
    fn color_line(
        &self,
        paint_ref: PaintRef,
        field_off: usize,
        variable: bool,
        coords: Option<&[f32]>,
    ) -> Result<ColorLine, Error> {
        let data = self.data;
        let rel = read_u24(data, paint_ref.0 as usize + field_off)?;
        if rel == 0 {
            return Err(Error::BadStructure("COLR: NULL ColorLine offset"));
        }
        let at = (paint_ref.0).checked_add(rel).ok_or(Error::BadOffset)? as usize;
        let extend = Extend::from_u8(read_u8(data, at)?);
        let num_stops = read_u16(data, at + 1)? as usize;
        let stop_size = if variable { 10 } else { 6 };
        let mut stops = Vec::with_capacity(num_stops.min(data.len() / stop_size));
        for i in 0..num_stops {
            let off = at + 3 + i * stop_size;
            let stop_offset = read_f2dot14(data, off)?;
            let palette_index = read_u16(data, off + 2)?;
            let alpha = read_f2dot14(data, off + 4)?;
            let vib = if variable {
                Some(read_u32(data, off + 6)?)
            } else {
                None
            };
            stops.push(ColorStop {
                stop_offset: self.var_f2dot14(stop_offset, vib, 0, coords),
                palette_index,
                alpha: self.var_f2dot14(alpha, vib, 1, coords).clamp(0.0, 1.0),
                var_index_base: vib,
            });
        }
        // The spec applies stops in increasing stopOffset order, with
        // the order established after instance values are derived.
        // Stable sort keeps file order for equal offsets.
        stops.sort_by(|a, b| a.stop_offset.total_cmp(&b.stop_offset));
        Ok(ColorLine { extend, stops })
    }

    // ---- clip list ---------------------------------------------------------

    /// The clip box for a base glyph, or `Ok(None)` if the `ClipList`
    /// has no record covering it. With `coords`, a format-2 box is
    /// instance-resolved with the spec's outward rounding (mins toward
    /// −∞, maxes toward +∞) so the box only expands.
    pub fn clip_box(
        &self,
        glyph_id: u16,
        coords: Option<&[f32]>,
    ) -> Result<Option<ClipBox>, Error> {
        // Ranges are sorted by startGlyphID and must not overlap:
        // find the last record starting at or before glyph_id.
        let i = match self
            .clip_records
            .binary_search_by_key(&glyph_id, |r| r.start_glyph_id)
        {
            Ok(i) => i,
            Err(0) => return Ok(None),
            Err(i) => i - 1,
        };
        let rec = &self.clip_records[i];
        if glyph_id < rec.start_glyph_id || glyph_id > rec.end_glyph_id {
            return Ok(None);
        }
        let at = rec.clip_box_offset as usize;
        let data = self.data;
        let format = read_u8(data, at)?;
        let x_min = read_i16(data, at + 1)? as f32;
        let y_min = read_i16(data, at + 3)? as f32;
        let x_max = read_i16(data, at + 5)? as f32;
        let y_max = read_i16(data, at + 7)? as f32;
        match format {
            1 => Ok(Some(ClipBox {
                format,
                x_min,
                y_min,
                x_max,
                y_max,
                var_index_base: None,
            })),
            2 => {
                let vib = Some(read_u32(data, at + 9)?);
                Ok(Some(ClipBox {
                    format,
                    // Round so the box expands.
                    x_min: self.var_fword(x_min, vib, 0, coords).floor(),
                    y_min: self.var_fword(y_min, vib, 1, coords).floor(),
                    x_max: self.var_fword(x_max, vib, 2, coords).ceil(),
                    y_max: self.var_fword(y_max, vib, 3, coords).ceil(),
                    var_index_base: vib,
                }))
            }
            _ => Err(Error::BadStructure("COLR: unknown ClipBox format")),
        }
    }

    // ---- graph traversal ---------------------------------------------------

    /// The child paint refs of a decoded paint, in traversal order.
    /// `PaintColrGlyph` resolves to the referenced base glyph's root
    /// (an error if that base glyph has no `BaseGlyphPaintRecord` —
    /// such a glyph is not well formed per spec).
    fn children(&self, paint: &Paint) -> Result<Vec<PaintRef>, Error> {
        Ok(match paint {
            Paint::ColrLayers {
                num_layers,
                first_layer_index,
            } => self.layers(*first_layer_index, *num_layers)?,
            Paint::Glyph { paint, .. }
            | Paint::Transform { paint, .. }
            | Paint::Translate { paint, .. }
            | Paint::Scale { paint, .. }
            | Paint::ScaleAroundCenter { paint, .. }
            | Paint::ScaleUniform { paint, .. }
            | Paint::ScaleUniformAroundCenter { paint, .. }
            | Paint::Rotate { paint, .. }
            | Paint::RotateAroundCenter { paint, .. }
            | Paint::Skew { paint, .. }
            | Paint::SkewAroundCenter { paint, .. } => vec![*paint],
            Paint::ColrGlyph { glyph_id } => {
                vec![self.base_glyph_paint(*glyph_id).ok_or(Error::BadStructure(
                    "COLR: PaintColrGlyph references a glyph with no BaseGlyphPaintRecord",
                ))?]
            }
            Paint::Composite {
                source, backdrop, ..
            } => vec![*source, *backdrop],
            Paint::Solid { .. }
            | Paint::LinearGradient { .. }
            | Paint::RadialGradient { .. }
            | Paint::SweepGradient { .. }
            | Paint::Unknown { .. } => Vec::new(),
        })
    }

    /// Depth-first traversal from `root` with cycle detection.
    /// `on_exit(offset, paint, child_values)` combines each node's
    /// children values (post-order) into the node's value; results are
    /// memoized per offset so shared sub-graphs are visited once.
    fn traverse<T: Copy>(
        &self,
        root: PaintRef,
        on_exit: &mut dyn FnMut(&Paint, &[T]) -> T,
    ) -> Result<T, Error> {
        enum Frame {
            Enter(u32),
            Exit(u32),
        }
        let mut memo: HashMap<u32, T> = HashMap::new();
        let mut decoded: HashMap<u32, Paint> = HashMap::new();
        let mut on_path: HashSet<u32> = HashSet::new();
        let mut stack = vec![Frame::Enter(root.0)];
        while let Some(frame) = stack.pop() {
            match frame {
                Frame::Enter(off) => {
                    if memo.contains_key(&off) {
                        continue;
                    }
                    if !on_path.insert(off) {
                        return Err(Error::BadStructure("COLR: paint graph contains a cycle"));
                    }
                    let paint = self.paint(PaintRef(off), None)?;
                    let children = self.children(&paint)?;
                    decoded.insert(off, paint);
                    stack.push(Frame::Exit(off));
                    for c in children {
                        // A child already on the current path is a
                        // cycle even if it will be dequeued later.
                        if on_path.contains(&c.0) {
                            return Err(Error::BadStructure("COLR: paint graph contains a cycle"));
                        }
                        stack.push(Frame::Enter(c.0));
                    }
                }
                Frame::Exit(off) => {
                    on_path.remove(&off);
                    let paint = decoded
                        .get(&off)
                        .ok_or(Error::BadStructure("COLR: traversal state"))?;
                    let child_vals: Vec<T> = self
                        .children(paint)?
                        .iter()
                        .map(|c| {
                            memo.get(&c.0)
                                .copied()
                                .ok_or(Error::BadStructure("COLR: traversal state"))
                        })
                        .collect::<Result<_, _>>()?;
                    let v = on_exit(paint, &child_vals);
                    memo.insert(off, v);
                }
            }
        }
        memo.get(&root.0)
            .copied()
            .ok_or(Error::BadStructure("COLR: traversal state"))
    }

    /// Validate the whole paint graph of a base glyph: every reachable
    /// Paint table decodes, every `PaintColrLayers` slice is within the
    /// `LayerList`, every `PaintColrGlyph` target exists, and the graph
    /// is acyclic (including cross-glyph edges).
    pub fn validate_color_glyph(&self, glyph_id: u16) -> Result<(), Error> {
        let root = self
            .base_glyph_paint(glyph_id)
            .ok_or(Error::GlyphOutOfRange(glyph_id))?;
        self.traverse(root, &mut |_, _| ())
    }

    /// Whether the paint sub-graph rooted at `paint_ref` is *bounded*
    /// (renders inside a finite region) — a spec requirement for
    /// version-1 color glyph roots.
    ///
    /// Rules: `PaintGlyph` is inherently bounded (the outline clips
    /// its child); solid fills and gradients are unbounded on their
    /// own; layers are bounded iff every layer is; transforms inherit
    /// their child; `PaintColrGlyph` inherits the referenced graph;
    /// `PaintComposite` follows the per-mode table in the spec;
    /// unrecognized formats are ignored (render nothing → bounded).
    pub fn is_bounded(&self, paint_ref: PaintRef) -> Result<bool, Error> {
        self.traverse(paint_ref, &mut |paint, children| match paint {
            Paint::ColrLayers { .. } => children.iter().all(|&b| b),
            Paint::Solid { .. }
            | Paint::LinearGradient { .. }
            | Paint::RadialGradient { .. }
            | Paint::SweepGradient { .. } => false,
            Paint::Glyph { .. } => true,
            Paint::ColrGlyph { .. }
            | Paint::Transform { .. }
            | Paint::Translate { .. }
            | Paint::Scale { .. }
            | Paint::ScaleAroundCenter { .. }
            | Paint::ScaleUniform { .. }
            | Paint::ScaleUniformAroundCenter { .. }
            | Paint::Rotate { .. }
            | Paint::RotateAroundCenter { .. }
            | Paint::Skew { .. }
            | Paint::SkewAroundCenter { .. } => children[0],
            Paint::Composite { mode, .. } => {
                let (s, b) = (children[0], children[1]);
                match mode {
                    CompositeMode::Clear => true,
                    CompositeMode::Src | CompositeMode::SrcOut => s,
                    CompositeMode::Dest | CompositeMode::DestOut => b,
                    CompositeMode::SrcIn | CompositeMode::DestIn => s || b,
                    _ => s && b,
                }
            }
            Paint::Unknown { .. } => true,
        })
    }
}
