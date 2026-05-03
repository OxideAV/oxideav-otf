//! Type 2 charstring interpreter (Adobe TN5177).
//!
//! A Type 2 charstring is a stream of bytes representing operands
//! (signed integers + 16-bit fixed reals) and operators (path
//! construction, hint declaration, subroutine calls). Operands push
//! onto a stack; operators pop their inputs from it. The interpreter
//! maintains a "current point" (CP) that path operators advance.
//!
//! Operand encoding (TN5177 §3.2 / Tables 1, 2):
//! - byte b0 = 32..246          → integer  b0 - 139           (1 byte)
//! - byte b0 = 247..250         → integer  ((b0-247)*256 + b1 + 108)
//! - byte b0 = 251..254         → integer  -((b0-251)*256) - b1 - 108
//! - byte b0 = 28               → integer  i16 from b1..b2
//! - byte b0 = 255              → fixed real i16.16 from b1..b4
//! - byte b0 = 12               → 2-byte operator (escape)
//!
//! Operators implemented in round 1 (every common one a real font
//! uses):
//! - Path: rmoveto, hmoveto, vmoveto, rlineto, hlineto, vlineto,
//!   rrcurveto, hhcurveto, hvcurveto, vvcurveto, vhcurveto,
//!   rcurveline, rlinecurve, callsubr, callgsubr, return, endchar
//! - Hint: hstem, vstem, hstemhm, vstemhm, hintmask, cntrmask
//!   (recorded but not enforced — we'll antialias at >= 16 px)
//! - Flex: flex, hflex, hflex1, flex1
//! - Arithmetic (rare but spec-listed): not implemented for round 1;
//!   the rare fonts that need them will surface as
//!   `Error::CharstringUnsupportedOp` and we can fold them in
//!   round 2 if any production fixture hits the path.
//!
//! Width handling (TN5177 §4.7): the FIRST operand on the stack at
//! the time of the FIRST hstem / hstemhm / vstem / vstemhm /
//! cntrmask / hintmask / hmoveto / vmoveto / rmoveto / endchar is
//! treated as the glyph's advance-width *delta* from `nominalWidthX`
//! IF the stack count is one more than the operator's normal arity.
//! Otherwise the glyph uses `defaultWidthX`. We record the resolved
//! width even though it isn't currently surfaced — `Font::glyph_advance`
//! routes through `hmtx`, which is the spec-preferred path.

use crate::cff::index::Index;
use crate::cff::subrs::bias_for;
use crate::outline::{CubicContour, CubicOutline, CubicSegment, Point};
use crate::Error;

/// Maximum subroutine recursion depth (TN5177 §4.5 says "no more
/// than 10 deep" for type 2; we use 16 to leave headroom for fonts
/// that push the limit).
const MAX_CALL_DEPTH: u8 = 16;

/// Maximum bytes a single charstring may consume before we abort.
/// Real-world charstrings are well under 2 KB; the cap exists
/// purely to bound adversarial / malformed input.
const MAX_BYTES_PROCESSED: usize = 1 << 20;

/// Type 2 operand stack — TN5177 §3 says it holds at most 48
/// numbers, but the OpenType profile bumps that to 96 (and we add
/// slack to keep panics out of unusual but valid fonts).
const STACK_CAP: usize = 192;

/// A small helper for the interpreter's state.
pub(crate) struct Interpreter<'a> {
    /// Operand stack (cleared at every operator that consumes its
    /// operands).
    stack: Vec<f32>,
    /// Output outline being built.
    out: CubicOutline,
    /// Current subpath, accumulated until the next move terminator.
    current_contour: CubicContour,
    /// Current pen position (font-unit coordinates, Y-up).
    pen: Point,
    /// Most-recent move target — needed for the implicit ClosePath
    /// emitted at every subpath boundary.
    subpath_start: Point,
    /// Whether `current_contour` currently contains any segments
    /// (drives ClosePath emission and avoids producing empty
    /// contours for blank glyphs).
    contour_has_data: bool,

    /// Hints declared so far (not enforced). Counted because
    /// `hintmask` / `cntrmask` operators read a bitmask whose width
    /// depends on the hint count.
    hint_count: u32,

    /// Whether we've processed the first width-relevant operator yet
    /// — used to decode the charstring-encoded glyph advance width.
    seen_width_op: bool,
    /// Resolved glyph width in font units. `defaultWidthX` if no
    /// width was prepended to the first operator; otherwise
    /// `nominalWidthX + the stack's bottom value`.
    #[allow(dead_code)]
    pub(crate) glyph_width: f32,

    /// Local subroutines INDEX (may be `None` if the font has none).
    local_subrs: Option<&'a Index<'a>>,
    /// Global subroutines INDEX.
    global_subrs: &'a Index<'a>,
    /// Subroutine call depth.
    depth: u8,
    /// Bytes processed across all called subroutines (DoS cap).
    bytes_processed: usize,

    /// Width parameters from the Private DICT, used to resolve the
    /// first operator's optional leading width operand.
    nominal_width_x: f32,
    default_width_x: f32,
}

impl<'a> Interpreter<'a> {
    pub(crate) fn new(
        global_subrs: &'a Index<'a>,
        local_subrs: Option<&'a Index<'a>>,
        nominal_width_x: f32,
        default_width_x: f32,
    ) -> Self {
        Self {
            stack: Vec::with_capacity(STACK_CAP),
            out: CubicOutline::default(),
            current_contour: CubicContour::default(),
            pen: Point::new(0.0, 0.0),
            subpath_start: Point::new(0.0, 0.0),
            contour_has_data: false,
            hint_count: 0,
            seen_width_op: false,
            glyph_width: default_width_x,
            local_subrs,
            global_subrs,
            depth: 0,
            bytes_processed: 0,
            nominal_width_x,
            default_width_x,
        }
    }

    /// Run the top-level charstring `bytes` to completion (`endchar`).
    pub(crate) fn run(&mut self, bytes: &[u8]) -> Result<(), Error> {
        let result = self.execute(bytes);
        // .notdef and a small number of malformed glyphs may finish
        // without endchar — for round 1 we surface that as a
        // best-effort outline rather than a hard error, because real
        // fonts often have one or two glyphs that decode silently to
        // empty (the .notdef itself is usually a hand-drawn box that
        // does run to endchar, but some open-source toolchains emit
        // truly empty .notdef charstrings).
        match result {
            Ok(_) | Err(Error::CharstringEnd) => Ok(()),
            Err(e) => Err(e),
        }
    }

    /// Recursively interpret `bytes`. Returns Ok(()) at `return`
    /// (end-of-subr) and `Err(Error::CharstringEnd)` at `endchar`.
    fn execute(&mut self, bytes: &[u8]) -> Result<(), Error> {
        let mut i = 0usize;
        while i < bytes.len() {
            self.bytes_processed += 1;
            if self.bytes_processed > MAX_BYTES_PROCESSED {
                return Err(Error::CharstringTooLong);
            }
            let b0 = bytes[i];
            // Operand encodings — must check 255 (fixed real) BEFORE the
            // generic `>= 32` integer dispatch since the integer ranges
            // top out at 254.
            if b0 == 255 {
                // 16.16 fixed real.
                if i + 4 >= bytes.len() {
                    return Err(Error::UnexpectedEof);
                }
                let raw =
                    i32::from_be_bytes([bytes[i + 1], bytes[i + 2], bytes[i + 3], bytes[i + 4]]);
                let v = raw as f32 / 65536.0;
                self.push(v)?;
                i += 5;
                continue;
            }
            if b0 >= 32 {
                let (val, n) = parse_int_operand(bytes, i, b0)?;
                self.push(val as f32)?;
                i += n;
                continue;
            }
            if b0 == 28 {
                if i + 2 >= bytes.len() {
                    return Err(Error::UnexpectedEof);
                }
                let v = i16::from_be_bytes([bytes[i + 1], bytes[i + 2]]) as i32;
                self.push(v as f32)?;
                i += 3;
                continue;
            }

            // Operator (1-byte or escape).
            let op: u16 = if b0 == 12 {
                if i + 1 >= bytes.len() {
                    return Err(Error::UnexpectedEof);
                }
                let sub = bytes[i + 1];
                i += 2;
                0x0C00u16 | sub as u16
            } else {
                i += 1;
                b0 as u16
            };

            match op {
                // --- Path construction -------------------------------
                21 /* rmoveto */ => {
                    self.handle_initial_width(2);
                    let n = self.stack.len();
                    if n < 2 {
                        return Err(Error::CharstringStackUnderflow);
                    }
                    let dx = self.stack[n - 2];
                    let dy = self.stack[n - 1];
                    self.move_to(dx, dy);
                    self.stack.clear();
                }
                22 /* hmoveto */ => {
                    self.handle_initial_width(1);
                    let n = self.stack.len();
                    if n < 1 {
                        return Err(Error::CharstringStackUnderflow);
                    }
                    let dx = self.stack[n - 1];
                    self.move_to(dx, 0.0);
                    self.stack.clear();
                }
                4 /* vmoveto */ => {
                    self.handle_initial_width(1);
                    let n = self.stack.len();
                    if n < 1 {
                        return Err(Error::CharstringStackUnderflow);
                    }
                    let dy = self.stack[n - 1];
                    self.move_to(0.0, dy);
                    self.stack.clear();
                }
                5 /* rlineto */ => {
                    // Pairs of (dx, dy) until stack is empty.
                    let pairs = self.stack.len() / 2;
                    for k in 0..pairs {
                        let dx = self.stack[k * 2];
                        let dy = self.stack[k * 2 + 1];
                        self.line_to(dx, dy);
                    }
                    self.stack.clear();
                }
                6 /* hlineto */ => {
                    // Alternating: starts horizontal, then vertical, …
                    for (k, v) in self.stack.clone().into_iter().enumerate() {
                        if k % 2 == 0 {
                            self.line_to(v, 0.0);
                        } else {
                            self.line_to(0.0, v);
                        }
                    }
                    self.stack.clear();
                }
                7 /* vlineto */ => {
                    for (k, v) in self.stack.clone().into_iter().enumerate() {
                        if k % 2 == 0 {
                            self.line_to(0.0, v);
                        } else {
                            self.line_to(v, 0.0);
                        }
                    }
                    self.stack.clear();
                }
                8 /* rrcurveto */ => {
                    // Sextets of (dxa, dya, dxb, dyb, dxc, dyc).
                    let groups = self.stack.len() / 6;
                    for k in 0..groups {
                        let s = &self.stack[k * 6..k * 6 + 6];
                        self.r_curve_to(s[0], s[1], s[2], s[3], s[4], s[5]);
                    }
                    self.stack.clear();
                }
                27 /* hhcurveto */ => {
                    self.op_hhcurveto()?;
                }
                31 /* hvcurveto */ => {
                    self.op_hvcurveto()?;
                }
                26 /* vvcurveto */ => {
                    self.op_vvcurveto()?;
                }
                30 /* vhcurveto */ => {
                    self.op_vhcurveto()?;
                }
                24 /* rcurveline */ => {
                    // {dxa dya dxb dyb dxc dyc}+ dxd dyd
                    let n = self.stack.len();
                    if n < 8 || (n - 2) % 6 != 0 {
                        return Err(Error::CharstringStackUnderflow);
                    }
                    let groups = (n - 2) / 6;
                    for k in 0..groups {
                        let s = &self.stack[k * 6..k * 6 + 6];
                        self.r_curve_to(s[0], s[1], s[2], s[3], s[4], s[5]);
                    }
                    let dx = self.stack[n - 2];
                    let dy = self.stack[n - 1];
                    self.line_to(dx, dy);
                    self.stack.clear();
                }
                25 /* rlinecurve */ => {
                    // {dxa dya}+ dxb dyb dxc dyc dxd dyd
                    let n = self.stack.len();
                    if n < 8 || (n - 6) % 2 != 0 {
                        return Err(Error::CharstringStackUnderflow);
                    }
                    let lines = (n - 6) / 2;
                    for k in 0..lines {
                        let dx = self.stack[k * 2];
                        let dy = self.stack[k * 2 + 1];
                        self.line_to(dx, dy);
                    }
                    let s = &self.stack[lines * 2..lines * 2 + 6];
                    self.r_curve_to(s[0], s[1], s[2], s[3], s[4], s[5]);
                    self.stack.clear();
                }

                // --- Subroutines ---------------------------------------
                10 /* callsubr */ => {
                    let n = self.stack.len();
                    if n == 0 {
                        return Err(Error::CharstringStackUnderflow);
                    }
                    let biased = self.stack[n - 1] as i32;
                    self.stack.pop();
                    let subrs = self.local_subrs.ok_or(Error::CharstringNoLocalSubrs)?;
                    let idx = biased + bias_for(subrs.count);
                    self.call_subr(subrs, idx)?;
                }
                29 /* callgsubr */ => {
                    let n = self.stack.len();
                    if n == 0 {
                        return Err(Error::CharstringStackUnderflow);
                    }
                    let biased = self.stack[n - 1] as i32;
                    self.stack.pop();
                    let subrs = self.global_subrs;
                    let idx = biased + bias_for(subrs.count);
                    self.call_subr(subrs, idx)?;
                }
                11 /* return */ => {
                    return Ok(());
                }
                14 /* endchar */ => {
                    self.handle_initial_width(0);
                    self.close_subpath_if_open();
                    return Err(Error::CharstringEnd);
                }

                // --- Hints (recorded but not enforced) -----------------
                1 /* hstem */ | 3 /* vstem */ | 18 /* hstemhm */ | 23 /* vstemhm */ => {
                    self.handle_initial_width_for_stem();
                    // Each hint is a (y, dy) pair. We record the count
                    // because hintmask / cntrmask later consume one
                    // bit per hint.
                    self.hint_count += (self.stack.len() / 2) as u32;
                    self.stack.clear();
                }
                19 /* hintmask */ | 20 /* cntrmask */ => {
                    self.handle_initial_width_for_stem();
                    // Implicit vstem: any operands on the stack are
                    // vertical hints.
                    self.hint_count += (self.stack.len() / 2) as u32;
                    self.stack.clear();
                    // Skip the bitmask: ceil(hint_count / 8) bytes.
                    let mask_bytes = (self.hint_count as usize).div_ceil(8);
                    if i + mask_bytes > bytes.len() {
                        return Err(Error::UnexpectedEof);
                    }
                    i += mask_bytes;
                }

                // --- Flex (two-byte) -----------------------------------
                0x0C22 /* flex */ => self.op_flex()?,
                0x0C23 /* flex1 */ => self.op_flex1()?,
                0x0C24 /* hflex */ => self.op_hflex()?,
                0x0C25 /* hflex1 */ => self.op_hflex1()?,

                _ => {
                    return Err(Error::CharstringUnsupportedOp(op));
                }
            }
        }
        // Spec: charstring should end with an explicit endchar; we
        // tolerate a missing terminator here and let the caller
        // accept the partial outline.
        Ok(())
    }

    fn push(&mut self, v: f32) -> Result<(), Error> {
        if self.stack.len() >= STACK_CAP {
            return Err(Error::CharstringStackOverflow);
        }
        self.stack.push(v);
        Ok(())
    }

    fn move_to(&mut self, dx: f32, dy: f32) {
        // Implicit ClosePath if a subpath was already open.
        self.close_subpath_if_open();
        self.pen.x += dx;
        self.pen.y += dy;
        self.subpath_start = self.pen;
        self.current_contour
            .segments
            .push(CubicSegment::MoveTo(self.pen));
        self.contour_has_data = true;
    }

    fn line_to(&mut self, dx: f32, dy: f32) {
        self.pen.x += dx;
        self.pen.y += dy;
        self.current_contour
            .segments
            .push(CubicSegment::LineTo(self.pen));
        self.contour_has_data = true;
    }

    fn r_curve_to(&mut self, dxa: f32, dya: f32, dxb: f32, dyb: f32, dxc: f32, dyc: f32) {
        let c1 = Point::new(self.pen.x + dxa, self.pen.y + dya);
        let c2 = Point::new(c1.x + dxb, c1.y + dyb);
        let end = Point::new(c2.x + dxc, c2.y + dyc);
        self.pen = end;
        self.current_contour
            .segments
            .push(CubicSegment::CurveTo { c1, c2, end });
        self.contour_has_data = true;
    }

    fn close_subpath_if_open(&mut self) {
        if self.contour_has_data {
            self.current_contour.segments.push(CubicSegment::ClosePath);
            // Move the finished contour into the outline.
            let finished = std::mem::take(&mut self.current_contour);
            self.out.contours.push(finished);
            self.contour_has_data = false;
        }
    }

    fn handle_initial_width(&mut self, normal_arity: usize) {
        if self.seen_width_op {
            return;
        }
        self.seen_width_op = true;
        if self.stack.len() == normal_arity + 1 {
            // Bottom-of-stack value is `width - nominalWidthX`.
            let delta = self.stack.remove(0);
            self.glyph_width = self.nominal_width_x + delta;
        } else {
            self.glyph_width = self.default_width_x;
        }
    }

    /// hstem / vstem-family special width handling: the stem operators
    /// consume an EVEN number of operands (pairs); if the stack count
    /// is odd, the first value is the optional width.
    fn handle_initial_width_for_stem(&mut self) {
        if self.seen_width_op {
            return;
        }
        self.seen_width_op = true;
        if self.stack.len() % 2 == 1 {
            let delta = self.stack.remove(0);
            self.glyph_width = self.nominal_width_x + delta;
        } else {
            self.glyph_width = self.default_width_x;
        }
    }

    fn call_subr(&mut self, subrs: &'a Index<'a>, idx: i32) -> Result<(), Error> {
        if idx < 0 || (idx as u32) >= subrs.count {
            return Err(Error::CharstringBadSubrIndex(idx));
        }
        if self.depth >= MAX_CALL_DEPTH {
            return Err(Error::CharstringTooDeep);
        }
        let body = subrs.entry(idx as u32)?;
        self.depth += 1;
        let r = self.execute(body);
        self.depth -= 1;
        r
    }

    pub(crate) fn into_outline(mut self) -> CubicOutline {
        // Any glyph that ends without endchar still gets its trailing
        // contour closed.
        self.close_subpath_if_open();
        self.out
    }

    // ----------------------------------------------------------------
    //   Curveto family (TN5177 §4.6) — these are encoded compactly
    //   to take advantage of horizontal / vertical-only entry & exit
    //   tangents that real fonts use overwhelmingly.
    // ----------------------------------------------------------------

    fn op_hhcurveto(&mut self) -> Result<(), Error> {
        // {dy1?} {dxa dxb dyb dxc}+
        // 4n or 4n+1 operands. dy1 is the leading vertical step on
        // the first segment; subsequent segments are pure-horizontal
        // tangent-in / tangent-out.
        let mut s = self.stack.clone();
        self.stack.clear();
        let mut dy1 = 0.0f32;
        if s.len() % 4 == 1 {
            dy1 = s.remove(0);
        }
        if s.len() % 4 != 0 {
            return Err(Error::CharstringStackUnderflow);
        }
        let mut first = true;
        for chunk in s.chunks_exact(4) {
            let dxa = chunk[0];
            let (dxb, dyb, dxc) = (chunk[1], chunk[2], chunk[3]);
            let dya = if first { dy1 } else { 0.0 };
            first = false;
            self.r_curve_to(dxa, dya, dxb, dyb, dxc, 0.0);
        }
        Ok(())
    }

    fn op_vvcurveto(&mut self) -> Result<(), Error> {
        // {dx1?} {dya dxb dyb dyc}+
        let mut s = self.stack.clone();
        self.stack.clear();
        let mut dx1 = 0.0f32;
        if s.len() % 4 == 1 {
            dx1 = s.remove(0);
        }
        if s.len() % 4 != 0 {
            return Err(Error::CharstringStackUnderflow);
        }
        let mut first = true;
        for chunk in s.chunks_exact(4) {
            let dya = chunk[0];
            let (dxb, dyb, dyc) = (chunk[1], chunk[2], chunk[3]);
            let dxa = if first { dx1 } else { 0.0 };
            first = false;
            self.r_curve_to(dxa, dya, dxb, dyb, 0.0, dyc);
        }
        Ok(())
    }

    fn op_hvcurveto(&mut self) -> Result<(), Error> {
        // Two-form operator: alternating starts-horizontal and
        // starts-vertical sextet groups, optionally with a final
        // df parameter on the closing curve to pick the off-axis end
        // delta. See TN5177 §4.6 form (a) / (b).
        let s = self.stack.clone();
        self.stack.clear();
        self.alt_curveto(s, /* h_first */ true)
    }

    fn op_vhcurveto(&mut self) -> Result<(), Error> {
        let s = self.stack.clone();
        self.stack.clear();
        self.alt_curveto(s, /* h_first */ false)
    }

    fn alt_curveto(&mut self, mut s: Vec<f32>, h_first: bool) -> Result<(), Error> {
        // The curveto shorthand pairs an ON-AXIS entry (horizontal or
        // vertical depending on `h_first`) with an off-axis exit.
        // After two segments the parity flips. Operands are consumed
        // in 4-byte sextets; if the total count is 4n+1 the trailing
        // value is the final "df" delta on the closing tangent.
        let trailing = if s.len() % 8 == 5 || s.len() % 8 == 1 {
            Some(s.pop().unwrap())
        } else {
            None
        };
        if s.len() % 4 != 0 {
            return Err(Error::CharstringStackUnderflow);
        }
        let mut horiz = h_first;
        let mut chunks: Vec<&[f32]> = s.chunks_exact(4).collect();
        let last_idx = chunks.len().saturating_sub(1);
        for (i, chunk) in chunks.iter_mut().enumerate() {
            let (dxa, dya, dxb, dyb, dxc, dyc);
            if horiz {
                // Entry horizontal, exit vertical (typically).
                dxa = chunk[0];
                dya = 0.0;
                dxb = chunk[1];
                dyb = chunk[2];
                dxc = if i == last_idx {
                    trailing.unwrap_or(0.0)
                } else {
                    0.0
                };
                dyc = chunk[3];
            } else {
                dxa = 0.0;
                dya = chunk[0];
                dxb = chunk[1];
                dyb = chunk[2];
                dxc = chunk[3];
                dyc = if i == last_idx {
                    trailing.unwrap_or(0.0)
                } else {
                    0.0
                };
            }
            self.r_curve_to(dxa, dya, dxb, dyb, dxc, dyc);
            horiz = !horiz;
        }
        Ok(())
    }

    // ----------------------------------------------------------------
    //   Flex (TN5177 §4.6, two-byte ops)
    //
    //   Flex represents a six-curve sequence (two cubic Beziers)
    //   that approximates a near-flat hump. Most callers (us
    //   included) treat each flex operator as two consecutive
    //   curveto's — ignoring the flex-depth operand because we don't
    //   need scan-conversion specialization at the AA threshold.
    // ----------------------------------------------------------------

    fn op_flex(&mut self) -> Result<(), Error> {
        // Operands: dx1 dy1 dx2 dy2 dx3 dy3 dx4 dy4 dx5 dy5 dx6 dy6 fd
        // We accept either 12 (no flex depth) or 13 operands.
        let n = self.stack.len();
        if n != 12 && n != 13 {
            return Err(Error::CharstringStackUnderflow);
        }
        let s = self.stack.clone();
        self.stack.clear();
        self.r_curve_to(s[0], s[1], s[2], s[3], s[4], s[5]);
        self.r_curve_to(s[6], s[7], s[8], s[9], s[10], s[11]);
        Ok(())
    }

    fn op_hflex(&mut self) -> Result<(), Error> {
        // Operands: dx1 dx2 dy2 dx3 dx4 dx5 dx6  (7 operands)
        // Implicit y components for the start / end on the baseline.
        if self.stack.len() != 7 {
            return Err(Error::CharstringStackUnderflow);
        }
        let s = self.stack.clone();
        self.stack.clear();
        self.r_curve_to(s[0], 0.0, s[1], s[2], s[3], 0.0);
        self.r_curve_to(s[4], 0.0, s[5], -s[2], s[6], 0.0);
        Ok(())
    }

    fn op_hflex1(&mut self) -> Result<(), Error> {
        // dx1 dy1 dx2 dy2 dx3 dx4 dx5 dy5 dx6  (9 operands)
        if self.stack.len() != 9 {
            return Err(Error::CharstringStackUnderflow);
        }
        let s = self.stack.clone();
        self.stack.clear();
        self.r_curve_to(s[0], s[1], s[2], s[3], s[4], 0.0);
        // The last segment must land on the original y (start of flex)
        // — TN5177 §4.6: "flex1 ends with the y-coordinate at which
        // it started"; the missing dy6 is computed to bring the pen
        // back to the start y of the flex's first curve.
        let dy_total = s[1] + s[3] + 0.0 + s[7];
        let dy6 = -dy_total;
        self.r_curve_to(s[5], 0.0, s[6], -s[3] - 0.0, s[8], dy6);
        Ok(())
    }

    fn op_flex1(&mut self) -> Result<(), Error> {
        // dx1 dy1 dx2 dy2 dx3 dy3 dx4 dy4 dx5 dy5 d6  (11 operands)
        // The dominant axis of the cumulative motion of the first 5
        // off-curve deltas determines whether d6 is dx6 (motion is
        // mostly vertical) or dy6 (mostly horizontal); the OTHER
        // axis closes back to the start coordinate of the first
        // curve.
        if self.stack.len() != 11 {
            return Err(Error::CharstringStackUnderflow);
        }
        let s = self.stack.clone();
        self.stack.clear();
        let sum_dx = s[0] + s[2] + s[4] + s[6] + s[8];
        let sum_dy = s[1] + s[3] + s[5] + s[7] + s[9];
        let (dx6, dy6) = if sum_dx.abs() > sum_dy.abs() {
            (s[10], -sum_dy)
        } else {
            (-sum_dx, s[10])
        };
        self.r_curve_to(s[0], s[1], s[2], s[3], s[4], s[5]);
        self.r_curve_to(s[6], s[7], s[8], s[9], dx6, dy6);
        Ok(())
    }
}

/// Decode a single integer operand from `bytes` starting at `i`.
/// Returns `(value, bytes_consumed)`.
fn parse_int_operand(bytes: &[u8], i: usize, b0: u8) -> Result<(i32, usize), Error> {
    match b0 {
        32..=246 => Ok((b0 as i32 - 139, 1)),
        247..=250 => {
            if i + 1 >= bytes.len() {
                return Err(Error::UnexpectedEof);
            }
            let v = (b0 as i32 - 247) * 256 + bytes[i + 1] as i32 + 108;
            Ok((v, 2))
        }
        251..=254 => {
            if i + 1 >= bytes.len() {
                return Err(Error::UnexpectedEof);
            }
            let v = -((b0 as i32 - 251) * 256) - bytes[i + 1] as i32 - 108;
            Ok((v, 2))
        }
        _ => Err(Error::Cff("invalid charstring integer operand byte")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_index() -> Vec<u8> {
        vec![0u8, 0]
    }

    fn run(cs: &[u8]) -> CubicOutline {
        let g = empty_index();
        let global = Index::parse(&g, 0).unwrap();
        let mut interp = Interpreter::new(&global, None, 0.0, 500.0);
        interp.run(cs).unwrap();
        let mut o = interp.into_outline();
        o.recompute_bounds();
        o
    }

    #[test]
    fn endchar_only_yields_empty_outline() {
        // Just an endchar.
        let o = run(&[14]);
        assert!(o.is_empty());
    }

    #[test]
    fn single_rmoveto_lineto_box() {
        // width? (none) → uses defaultWidthX = 500
        // rmoveto 100 100 → MoveTo(100, 100)
        // hlineto 50 → LineTo(150, 100)
        // vlineto 50 → LineTo(150, 150)
        // hlineto -50 → LineTo(100, 150)
        // endchar
        // Encode: 100 = 139 + 100 = 239, -50 = 139 - 50 = 89.
        // rmoveto = 21, hlineto = 6, vlineto = 7.
        let cs = [
            239, 239, 21, // rmoveto 100 100
            189, 6, // hlineto 50  (50 = 139+50 = 189)
            189, 7, // vlineto 50
            89, 6,  // hlineto -50
            14, // endchar
        ];
        let o = run(&cs);
        assert_eq!(o.contours.len(), 1);
        let segs = &o.contours[0].segments;
        // MoveTo + 3 LineTo + ClosePath = 5 segments.
        assert_eq!(segs.len(), 5);
        assert!(matches!(segs[0], CubicSegment::MoveTo(_)));
        assert!(matches!(segs[4], CubicSegment::ClosePath));
        // Bounds reflect the box.
        assert_eq!(o.bounds.x_min, 100.0);
        assert_eq!(o.bounds.x_max, 150.0);
        assert_eq!(o.bounds.y_min, 100.0);
        assert_eq!(o.bounds.y_max, 150.0);
    }

    #[test]
    fn rrcurveto_emits_cubic() {
        let cs = [
            239, 239, 21, // rmoveto 100 100
            // rrcurveto 50 0 50 50 0 50 → curve from pen through
            // (150, 100), (200, 150) to (200, 200).
            189, 139, 189, 189, 139, 189, 8, 14,
        ];
        let o = run(&cs);
        assert_eq!(o.contours.len(), 1);
        let segs = &o.contours[0].segments;
        assert_eq!(segs.len(), 3); // MoveTo, CurveTo, ClosePath
        if let CubicSegment::CurveTo { c1, c2, end } = segs[1] {
            assert_eq!(c1, Point::new(150.0, 100.0));
            assert_eq!(c2, Point::new(200.0, 150.0));
            assert_eq!(end, Point::new(200.0, 200.0));
        } else {
            panic!("expected CurveTo, got {:?}", segs[1]);
        }
    }

    #[test]
    fn callgsubr_jumps_and_returns() {
        // Build a global subrs INDEX with one subroutine that emits
        // a single hlineto then `return`. Then call it from the
        // top-level charstring.
        //
        // Subr body: 189, 6, 11   (hlineto 50, return)
        // INDEX header: count=1, offSize=1, offsets=[1, 4], data=189 6 11
        let subr_body = vec![189, 6, 11];
        let mut subr_index_bytes = vec![
            0x00, 0x01, // count
            0x01, // offSize
            0x01, 0x04, // offsets (1, 4)
        ];
        subr_index_bytes.extend_from_slice(&subr_body);

        let subrs = Index::parse(&subr_index_bytes, 0).unwrap();
        assert_eq!(subrs.count, 1);

        // Top-level charstring: rmoveto 100 100, callgsubr -107
        // (which after bias 107 → subroutine 0), endchar.
        // -107 encoded: 139 - 107 = 32. So byte 32.
        let cs = [
            239, 239, 21, // rmoveto 100 100
            32, 29, // callgsubr (subr 0)
            14, // endchar
        ];
        let g = empty_index(); // we don't use locals here; globals provided below
        let global = subrs;
        let local_index_bytes = empty_index();
        let _local = Index::parse(&local_index_bytes, 0).unwrap();
        let mut interp = Interpreter::new(&global, None, 0.0, 500.0);
        interp.run(&cs).unwrap();
        let mut o = interp.into_outline();
        o.recompute_bounds();
        // We expect MoveTo(100,100), LineTo(150,100), ClosePath.
        let segs = &o.contours[0].segments;
        assert_eq!(segs.len(), 3);
        if let CubicSegment::LineTo(p) = segs[1] {
            assert_eq!(p, Point::new(150.0, 100.0));
        } else {
            panic!("expected LineTo");
        }
        let _ = g; // silence "unused" if the empty placeholder is dropped above.
    }

    #[test]
    fn hintmask_skips_correct_bitmask_size() {
        // hstem 0 50 (1 hint), vstem 0 30 (1 hint) → hint_count = 2.
        // hintmask then must skip ceil(2/8) = 1 byte.
        // After the mask: rmoveto 100 100, endchar.
        let cs = [
            139, 189, 1, // hstem 0 50
            139, 167, 3, // vstem 0 (28-139=-111? — let's just use 28: 139+28=167)
            // hintmask + 1 mask byte
            19, 0xFF, // rmoveto 100 100
            239, 239, 21, 14,
        ];
        let o = run(&cs);
        // We don't assert on the geometry — just that parsing succeeded
        // (no UnexpectedEof from over-skipping the mask).
        assert_eq!(o.contours.len(), 1);
    }

    #[test]
    fn width_decoded_from_first_op_extra_operand() {
        // Stack at first hmoveto has 2 operands: width-delta + dx.
        // nominalWidthX = 100, default = 500. width-delta = 50 → resolved
        // width = 150.
        // 50 = 189, 100 = 239
        let cs = [189, 239, 22, 14]; // hmoveto 100, endchar
        let g = empty_index();
        let global = Index::parse(&g, 0).unwrap();
        let mut interp = Interpreter::new(&global, None, 100.0, 500.0);
        interp.run(&cs).unwrap();
        assert_eq!(interp.glyph_width, 150.0);
    }

    #[test]
    fn width_default_when_no_extra_operand() {
        // hmoveto 100 alone → exactly 1 operand → no width prefix →
        // glyph uses defaultWidthX = 500.
        let cs = [239, 22, 14];
        let g = empty_index();
        let global = Index::parse(&g, 0).unwrap();
        let mut interp = Interpreter::new(&global, None, 100.0, 500.0);
        interp.run(&cs).unwrap();
        assert_eq!(interp.glyph_width, 500.0);
    }
}
