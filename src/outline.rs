//! Cubic-Bezier outline data structures emitted by the Type 2 charstring
//! interpreter.
//!
//! Type 2 charstrings (Adobe Technical Note #5177) describe glyph
//! outlines as a sequence of cubic Bezier segments and straight lines,
//! organised into closed contours. Unlike TrueType (which uses
//! quadratic Beziers and a "phantom on-curve" rule), Type 2 produces
//! explicit cubic curves with two control points per segment plus a
//! ClosePath terminator at each `endchar` / subpath boundary.
//!
//! Coordinates are in font units, exactly as emitted by the
//! charstring. Y-up. The downstream rasterizer is expected to do its
//! own Y-flip / scaling — keeping the parser oblivious to render
//! conventions.

/// A 2D point in font-unit coordinates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

impl Point {
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
}

/// One element of a cubic-Bezier contour.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CubicSegment {
    /// Move the pen without drawing — opens a new subpath.
    MoveTo(Point),
    /// Straight-line segment from the current point to `0`.
    LineTo(Point),
    /// Cubic Bezier from the current point through control points
    /// `c1` and `c2` to `end`.
    CurveTo { c1: Point, c2: Point, end: Point },
    /// Close the subpath back to the most recent `MoveTo`. Type 2
    /// charstrings emit one of these at every subpath boundary
    /// (implicit before `rmoveto` and at `endchar`).
    ClosePath,
}

/// A single closed contour. There is no explicit "is closed" flag
/// because the contained `ClosePath` segment terminates the loop.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CubicContour {
    pub segments: Vec<CubicSegment>,
}

/// Glyph bounding box in font units (Y-up). Defaults to all-zero.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct BBox {
    pub x_min: f32,
    pub y_min: f32,
    pub x_max: f32,
    pub y_max: f32,
}

impl BBox {
    pub fn width(&self) -> f32 {
        (self.x_max - self.x_min).max(0.0)
    }
    pub fn height(&self) -> f32 {
        (self.y_max - self.y_min).max(0.0)
    }
}

/// A decoded CFF glyph outline.
///
/// Contours and bounds use font-unit coordinates. `bounds` is derived
/// from the actual emitted points (CFF FontBBox in the Top DICT is a
/// font-wide bound, not per-glyph; per-glyph bbox is only available by
/// running the charstring).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CubicOutline {
    pub contours: Vec<CubicContour>,
    pub bounds: BBox,
}

impl CubicOutline {
    pub fn is_empty(&self) -> bool {
        self.contours.is_empty()
    }

    /// Re-derive `bounds` by walking every emitted point.
    pub(crate) fn recompute_bounds(&mut self) {
        let mut x_min = f32::INFINITY;
        let mut y_min = f32::INFINITY;
        let mut x_max = f32::NEG_INFINITY;
        let mut y_max = f32::NEG_INFINITY;
        let mut any = false;
        for c in &self.contours {
            for s in &c.segments {
                match s {
                    CubicSegment::MoveTo(p) | CubicSegment::LineTo(p) => {
                        any = true;
                        x_min = x_min.min(p.x);
                        x_max = x_max.max(p.x);
                        y_min = y_min.min(p.y);
                        y_max = y_max.max(p.y);
                    }
                    CubicSegment::CurveTo { c1, c2, end } => {
                        any = true;
                        for p in [c1, c2, end] {
                            x_min = x_min.min(p.x);
                            x_max = x_max.max(p.x);
                            y_min = y_min.min(p.y);
                            y_max = y_max.max(p.y);
                        }
                    }
                    CubicSegment::ClosePath => {}
                }
            }
        }
        self.bounds = if any {
            BBox {
                x_min,
                y_min,
                x_max,
                y_max,
            }
        } else {
            BBox::default()
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_outline() {
        let o = CubicOutline::default();
        assert!(o.is_empty());
        assert_eq!(o.bounds, BBox::default());
    }

    #[test]
    fn bounds_track_curve_control_points() {
        // A curve whose control points stick out beyond its endpoints
        // should report a bbox that includes them — Type 2 fonts
        // routinely use this for thin caps that would otherwise look
        // pinched if we only sampled segment endpoints.
        let mut o = CubicOutline::default();
        o.contours.push(CubicContour {
            segments: vec![
                CubicSegment::MoveTo(Point::new(0.0, 0.0)),
                CubicSegment::CurveTo {
                    c1: Point::new(50.0, 100.0),
                    c2: Point::new(50.0, -100.0),
                    end: Point::new(100.0, 0.0),
                },
                CubicSegment::ClosePath,
            ],
        });
        o.recompute_bounds();
        assert_eq!(o.bounds.x_min, 0.0);
        assert_eq!(o.bounds.x_max, 100.0);
        assert_eq!(o.bounds.y_min, -100.0);
        assert_eq!(o.bounds.y_max, 100.0);
    }
}
