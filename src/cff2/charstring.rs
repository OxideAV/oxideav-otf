//! CFF2 Type 2 charstring interpreter (OpenType 1.9.1 `CFF2` table,
//! CharString operators + "OpenType Font Variations in CFF2").
//!
//! The CFF2 CharString uses the same operand encoding and the same
//! path/hint/subroutine operators as the CFF1 Type 2 charstring
//! (Adobe TN5177), with four spec-visible differences relevant to a
//! decoder:
//!
//! 1. **No `endchar`** — a CFF2 CharString has no terminating operator;
//!    it ends when its byte stream is exhausted (and any open contour
//!    is implicitly closed). The deprecated `seac` (four-operand
//!    `endchar`) form does not exist in CFF2.
//! 2. **No glyph-width prefix** — CFF2 advance widths come from the
//!    sfnt `hmtx`/`HVAR` tables, so there is no optional leading
//!    width operand on the first stem/move/hint operator.
//! 3. **No arithmetic / storage / conditional operators** — the CFF2
//!    CharString operator set is restricted to path construction,
//!    hinting, subroutines, and the two variation operators.
//! 4. **Two variation operators** are added:
//!    - `vsindex` (`0x0f`, dec 15) — selects the active
//!      ItemVariationData (and thus the active region list, hence `k`)
//!      from the font's ItemVariationStore. May be used only once and
//!      must precede the first `blend`.
//!    - `blend` (`0x10`, dec 16) — pops `n + n*k + 1` operands
//!      (`n` default values, then `n*k` deltas in `n` groups of `k`,
//!      then `n`), interpolates each default with its `k` deltas
//!      scaled by the `k` region scalars, and pushes the `n` blended
//!      results back onto the stack.
//!
//! The region scalars themselves are supplied by the caller (the
//! `region_scalars` argument): the algorithm that derives them from
//! the font's normalized axis settings is specified in the OpenType
//! *Font Variations Common Table Formats* chapter and is the shaping
//! client's responsibility. An empty scalar slice selects the default
//! instance (every scalar `0`), where `blend` deltas contribute
//! nothing and the outline is the default design.
//!
//! Spec: `docs/text/opentype/otspec-cff2.html`.

use crate::cff::subrs::bias_for;
use crate::cff2::index::Cff2Index;
use crate::cff2::varstore::ItemVariationStore;
use crate::outline::{CubicContour, CubicOutline, CubicSegment, Point};
use crate::Error;

/// Maximum subroutine recursion depth. CFF2 keeps the Type 2 cap of
/// 10; we allow 16 for headroom.
const MAX_CALL_DEPTH: u8 = 16;

/// Maximum bytes processed across all subroutines (DoS bound).
const MAX_BYTES_PROCESSED: usize = 1 << 20;

/// Operand stack cap (OpenType bumps Type 2's 48 to 96; we add slack).
const STACK_CAP: usize = 513;

/// A CFF2 charstring interpreter. Holds the global/local subroutine
/// INDEXes, the active ItemVariationStore + region scalars (for
/// `blend`/`vsindex`), and the outline being built. Glyph-width
/// handling, `seac`, `endchar`, and the arithmetic/storage operator
/// families are deliberately absent (they are not part of CFF2).
#[derive(Debug)]
pub struct Cff2Interpreter<'a> {
    stack: Vec<f32>,
    out: CubicOutline,
    current_contour: CubicContour,
    pen: Point,
    subpath_start: Point,
    contour_has_data: bool,
    hint_count: u32,

    local_subrs: Option<&'a Cff2Index<'a>>,
    global_subrs: &'a Cff2Index<'a>,
    depth: u8,
    bytes_processed: usize,

    /// The font's ItemVariationStore, or `None` for a non-variable
    /// CFF2 font (where `blend`/`vsindex` must not appear).
    variation_store: Option<&'a ItemVariationStore>,
    /// Active ItemVariationData index (the default `vsindex` from the
    /// PrivateDICT, possibly overridden once by a CharString
    /// `vsindex`).
    vsindex: u16,
    /// Whether a `blend` has been seen yet (used to enforce the
    /// "`vsindex` must precede the first `blend`" rule).
    seen_blend: bool,
    /// Per-region interpolation scalars supplied by the caller. Index
    /// `j` applies to the `j`-th active region. A shorter-than-`k`
    /// slice treats missing entries as `0`; an empty slice is the
    /// default instance.
    region_scalars: &'a [f32],
}

impl<'a> Cff2Interpreter<'a> {
    /// Build an interpreter for one glyph.
    pub fn new(
        global_subrs: &'a Cff2Index<'a>,
        local_subrs: Option<&'a Cff2Index<'a>>,
        variation_store: Option<&'a ItemVariationStore>,
        default_vsindex: u16,
        region_scalars: &'a [f32],
    ) -> Self {
        Self {
            stack: Vec::with_capacity(64),
            out: CubicOutline::default(),
            current_contour: CubicContour::default(),
            pen: Point::new(0.0, 0.0),
            subpath_start: Point::new(0.0, 0.0),
            contour_has_data: false,
            hint_count: 0,
            local_subrs,
            global_subrs,
            depth: 0,
            bytes_processed: 0,
            variation_store,
            vsindex: default_vsindex,
            seen_blend: false,
            region_scalars,
        }
    }

    /// Run the top-level charstring to completion (its byte stream is
    /// exhausted, since CFF2 has no `endchar`).
    pub fn run(&mut self, bytes: &[u8]) -> Result<(), Error> {
        self.execute(bytes)
    }

    /// Finish the glyph and yield its outline, closing any open
    /// trailing contour.
    pub fn into_outline(mut self) -> CubicOutline {
        self.close_subpath_if_open();
        self.out.recompute_bounds();
        self.out
    }

    /// The number of active variation regions `k` for the current
    /// `vsindex` — the length of the active ItemVariationData's
    /// `regionIndexes` array.
    fn active_region_count(&self) -> Result<usize, Error> {
        let ivs = self
            .variation_store
            .ok_or(Error::Cff("CFF2 blend/vsindex without VariationStore"))?;
        let ivd = ivs
            .item_variation_data_at(self.vsindex as usize)
            .ok_or(Error::Cff("CFF2 vsindex out of range of VariationStore"))?;
        Ok(ivd.region_count())
    }

    fn execute(&mut self, bytes: &[u8]) -> Result<(), Error> {
        let mut i = 0usize;
        while i < bytes.len() {
            self.bytes_processed += 1;
            if self.bytes_processed > MAX_BYTES_PROCESSED {
                return Err(Error::CharstringTooLong);
            }
            let b0 = bytes[i];

            // Operand encodings (TN5177 §3.2, identical in CFF2).
            if b0 == 255 {
                if i + 4 >= bytes.len() {
                    return Err(Error::UnexpectedEof);
                }
                let raw =
                    i32::from_be_bytes([bytes[i + 1], bytes[i + 2], bytes[i + 3], bytes[i + 4]]);
                self.push(raw as f32 / 65536.0)?;
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
                    let n = self.stack.len();
                    if n < 1 {
                        return Err(Error::CharstringStackUnderflow);
                    }
                    self.move_to(self.stack[n - 1], 0.0);
                    self.stack.clear();
                }
                4 /* vmoveto */ => {
                    let n = self.stack.len();
                    if n < 1 {
                        return Err(Error::CharstringStackUnderflow);
                    }
                    self.move_to(0.0, self.stack[n - 1]);
                    self.stack.clear();
                }
                5 /* rlineto */ => {
                    let pairs = self.stack.len() / 2;
                    for k in 0..pairs {
                        let dx = self.stack[k * 2];
                        let dy = self.stack[k * 2 + 1];
                        self.line_to(dx, dy);
                    }
                    self.stack.clear();
                }
                6 /* hlineto */ => {
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
                    let groups = self.stack.len() / 6;
                    for k in 0..groups {
                        let s = &self.stack[k * 6..k * 6 + 6];
                        self.r_curve_to(s[0], s[1], s[2], s[3], s[4], s[5]);
                    }
                    self.stack.clear();
                }
                27 /* hhcurveto */ => self.op_hhcurveto()?,
                31 /* hvcurveto */ => self.op_hvcurveto()?,
                26 /* vvcurveto */ => self.op_vvcurveto()?,
                30 /* vhcurveto */ => self.op_vhcurveto()?,
                24 /* rcurveline */ => {
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

                // --- Subroutines -------------------------------------
                10 /* callsubr */ => {
                    let n = self.stack.len();
                    if n == 0 {
                        return Err(Error::CharstringStackUnderflow);
                    }
                    let biased = self.stack[n - 1] as i32;
                    self.stack.pop();
                    let subrs = self.local_subrs.ok_or(Error::CharstringNoLocalSubrs)?;
                    let idx = biased + bias_for(subrs.count);
                    self.call_local(idx)?;
                }
                29 /* callgsubr */ => {
                    let n = self.stack.len();
                    if n == 0 {
                        return Err(Error::CharstringStackUnderflow);
                    }
                    let biased = self.stack[n - 1] as i32;
                    self.stack.pop();
                    let idx = biased + bias_for(self.global_subrs.count);
                    self.call_global(idx)?;
                }
                11 /* return */ => return Ok(()),

                // --- Hints (recorded but not enforced) ---------------
                1 /* hstem */ | 3 /* vstem */ | 18 /* hstemhm */ | 23 /* vstemhm */ => {
                    self.hint_count += (self.stack.len() / 2) as u32;
                    self.stack.clear();
                }
                19 /* hintmask */ | 20 /* cntrmask */ => {
                    // Implicit vstem: any operands on the stack are
                    // vertical hints.
                    self.hint_count += (self.stack.len() / 2) as u32;
                    self.stack.clear();
                    let mask_bytes = (self.hint_count as usize).div_ceil(8);
                    if i + mask_bytes > bytes.len() {
                        return Err(Error::UnexpectedEof);
                    }
                    i += mask_bytes;
                }

                // --- Variation operators (CFF2) ----------------------
                15 /* vsindex */ => self.op_vsindex()?,
                16 /* blend */ => self.op_blend()?,

                // --- Flex (two-byte) ---------------------------------
                0x0C22 /* hflex */ => self.op_hflex()?,
                0x0C23 /* flex */ => self.op_flex()?,
                0x0C24 /* hflex1 */ => self.op_hflex1()?,
                0x0C25 /* flex1 */ => self.op_flex1()?,

                other => return Err(Error::CharstringUnsupportedOp(other)),
            }
        }
        Ok(())
    }

    // --- variation operators --------------------------------------

    fn op_vsindex(&mut self) -> Result<(), Error> {
        // `vsindex` may be used only once and must precede the first
        // `blend` (spec "CharString variation operators").
        if self.seen_blend {
            return Err(Error::Cff("CFF2 vsindex used after blend"));
        }
        let v = self
            .stack
            .last()
            .copied()
            .ok_or(Error::CharstringStackUnderflow)?;
        if v < 0.0 || v > u16::MAX as f32 {
            return Err(Error::Cff("CFF2 vsindex operand out of range"));
        }
        let ivd = v as u16;
        // Validate against the VariationStore.
        let ivs = self
            .variation_store
            .ok_or(Error::Cff("CFF2 vsindex without VariationStore"))?;
        if ivs.item_variation_data_at(ivd as usize).is_none() {
            return Err(Error::Cff("CFF2 vsindex out of range of VariationStore"));
        }
        self.vsindex = ivd;
        self.stack.clear();
        Ok(())
    }

    /// `blend`: stack is `… <n defaults> <n*k deltas> n`. Replace the
    /// `n` defaults + `n*k` deltas with `n` blended values
    /// (`default[i] + Σ_j scalar[j] * delta[i*k + j]`), leaving the
    /// `…` lower portion of the stack intact and not clearing it
    /// (`blend` is one of the three non-stack-clearing operators).
    fn op_blend(&mut self) -> Result<(), Error> {
        self.seen_blend = true;
        let k = self.active_region_count()?;

        // Pop n (the count of values to be blended).
        let n_f = self.stack.pop().ok_or(Error::CharstringStackUnderflow)?;
        if n_f < 0.0 {
            return Err(Error::Cff("CFF2 blend: negative operand count"));
        }
        let n = n_f as usize;

        // Operands consumed below n: n defaults + n*k deltas.
        let consumed = n
            .checked_mul(k + 1)
            .ok_or(Error::Cff("CFF2 blend: operand count overflow"))?;
        if self.stack.len() < consumed {
            return Err(Error::CharstringStackUnderflow);
        }
        let base = self.stack.len() - consumed;

        // The `consumed` operands are laid out as:
        //   [base .. base+n)              → n default values
        //   [base+n .. base+n+n*k)        → n groups of k deltas,
        //                                    group i = deltas for default i.
        let mut blended = Vec::with_capacity(n);
        for i in 0..n {
            let mut value = self.stack[base + i];
            let group = base + n + i * k;
            for (j, &scalar) in self.region_scalars.iter().take(k).enumerate() {
                value += scalar * self.stack[group + j];
            }
            blended.push(value);
        }

        // Replace the consumed region with the n blended values.
        self.stack.truncate(base);
        self.stack.extend_from_slice(&blended);
        if self.stack.len() > STACK_CAP {
            return Err(Error::CharstringStackOverflow);
        }
        Ok(())
    }

    // --- subroutine dispatch --------------------------------------

    fn call_local(&mut self, idx: i32) -> Result<(), Error> {
        let subrs = self.local_subrs.ok_or(Error::CharstringNoLocalSubrs)?;
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

    fn call_global(&mut self, idx: i32) -> Result<(), Error> {
        let subrs = self.global_subrs;
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

    // --- geometry -------------------------------------------------

    fn push(&mut self, v: f32) -> Result<(), Error> {
        if self.stack.len() >= STACK_CAP {
            return Err(Error::CharstringStackOverflow);
        }
        self.stack.push(v);
        Ok(())
    }

    fn move_to(&mut self, dx: f32, dy: f32) {
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
            let finished = std::mem::take(&mut self.current_contour);
            self.out.contours.push(finished);
            self.contour_has_data = false;
        }
    }

    // --- curve shorthands (identical geometry to CFF1) ------------

    fn op_hhcurveto(&mut self) -> Result<(), Error> {
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
        let s = self.stack.clone();
        self.stack.clear();
        self.alt_curveto(s, true)
    }

    fn op_vhcurveto(&mut self) -> Result<(), Error> {
        let s = self.stack.clone();
        self.stack.clear();
        self.alt_curveto(s, false)
    }

    fn alt_curveto(&mut self, mut s: Vec<f32>, h_first: bool) -> Result<(), Error> {
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
        for (idx, chunk) in chunks.iter_mut().enumerate() {
            let (dxa, dya, dxb, dyb, dxc, dyc);
            if horiz {
                dxa = chunk[0];
                dya = 0.0;
                dxb = chunk[1];
                dyb = chunk[2];
                dxc = if idx == last_idx {
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
                dyc = if idx == last_idx {
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

    // --- flex family (identical geometry to CFF1) -----------------

    fn op_flex(&mut self) -> Result<(), Error> {
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
        if self.stack.len() != 7 {
            return Err(Error::CharstringStackUnderflow);
        }
        let s = self.stack.clone();
        self.stack.clear();
        let (dx1, dx2, dy2, dx3, dx4, dx5, dx6) = (s[0], s[1], s[2], s[3], s[4], s[5], s[6]);
        self.r_curve_to(dx1, 0.0, dx2, dy2, dx3, 0.0);
        self.r_curve_to(dx4, 0.0, dx5, -dy2, dx6, 0.0);
        Ok(())
    }

    fn op_hflex1(&mut self) -> Result<(), Error> {
        if self.stack.len() != 9 {
            return Err(Error::CharstringStackUnderflow);
        }
        let s = self.stack.clone();
        self.stack.clear();
        let (dx1, dy1, dx2, dy2, dx3) = (s[0], s[1], s[2], s[3], s[4]);
        let (dx4, dx5, dy5, dx6) = (s[5], s[6], s[7], s[8]);
        let dy6 = -(dy1 + dy2 + dy5);
        self.r_curve_to(dx1, dy1, dx2, dy2, dx3, 0.0);
        self.r_curve_to(dx4, 0.0, dx5, dy5, dx6, dy6);
        Ok(())
    }

    fn op_flex1(&mut self) -> Result<(), Error> {
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

/// Decode a single integer operand from `bytes` starting at `i`
/// (TN5177 §3.2, identical in CFF2). Returns `(value, bytes_consumed)`.
fn parse_int_operand(bytes: &[u8], i: usize, b0: u8) -> Result<(i32, usize), Error> {
    match b0 {
        32..=246 => Ok((b0 as i32 - 139, 1)),
        247..=250 => {
            let b1 = *bytes.get(i + 1).ok_or(Error::UnexpectedEof)?;
            Ok(((b0 as i32 - 247) * 256 + b1 as i32 + 108, 2))
        }
        251..=254 => {
            let b1 = *bytes.get(i + 1).ok_or(Error::UnexpectedEof)?;
            Ok((-(b0 as i32 - 251) * 256 - b1 as i32 - 108, 2))
        }
        _ => Err(Error::Cff("CFF2 charstring: invalid integer operand byte")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cff2::varstore::{ItemVariationData, RegionAxisCoordinates, VariationRegion};

    /// Build an empty CFF2 INDEX (4 bytes, count 0).
    fn empty_index_bytes() -> Vec<u8> {
        vec![0, 0, 0, 0]
    }

    /// Build a CFF2 INDEX with one single-byte-or-more entry per
    /// `entries` slice. offSize is fixed at 1 (entries must be short).
    fn index_bytes(entries: &[&[u8]]) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(&(entries.len() as u32).to_be_bytes());
        v.push(1); // offSize
        let mut off = 1u8;
        let mut offsets = vec![off];
        for e in entries {
            off += e.len() as u8;
            offsets.push(off);
        }
        v.extend_from_slice(&offsets);
        for e in entries {
            v.extend_from_slice(e);
        }
        v
    }

    /// DICT/charstring single-byte integer operand (value = byte - 139,
    /// valid for −107..=107).
    fn op_int(v: i32) -> u8 {
        assert!((-107..=107).contains(&v), "op_int out of single-byte range");
        (v + 139) as u8
    }

    /// Charstring integer operand as a 3-byte `28 hi lo` (covers the
    /// full i16 range), appended to `out`.
    fn push_i16(v: i16, out: &mut Vec<u8>) {
        out.push(28);
        out.extend_from_slice(&v.to_be_bytes());
    }

    /// Decode `cs` with no subrs and no variation store, returning the
    /// outline.
    fn decode(cs: &[u8]) -> CubicOutline {
        let g = empty_index_bytes();
        let gsubrs = Cff2Index::parse(&g, 0).unwrap();
        let mut interp = Cff2Interpreter::new(&gsubrs, None, None, 0, &[]);
        interp.run(cs).expect("decode");
        interp.into_outline()
    }

    #[test]
    fn rmoveto_then_rlineto_builds_contour() {
        // 100 100 rmoveto  50 0 rlineto  0 50 rlineto
        let cs = [
            op_int(100),
            op_int(100),
            21, // rmoveto
            op_int(50),
            op_int(0),
            5, // rlineto
            op_int(0),
            op_int(50),
            5, // rlineto
        ];
        let o = decode(&cs);
        assert_eq!(o.contours.len(), 1);
        let segs = &o.contours[0].segments;
        // MoveTo(100,100), LineTo(150,100), LineTo(150,150), ClosePath.
        assert_eq!(segs[0], CubicSegment::MoveTo(Point::new(100.0, 100.0)));
        assert_eq!(segs[1], CubicSegment::LineTo(Point::new(150.0, 100.0)));
        assert_eq!(segs[2], CubicSegment::LineTo(Point::new(150.0, 150.0)));
        assert_eq!(segs[3], CubicSegment::ClosePath);
    }

    #[test]
    fn rrcurveto_emits_cubic() {
        // 0 0 rmoveto  10 20 30 40 50 60 rrcurveto
        let cs = [
            op_int(0),
            op_int(0),
            21,
            op_int(10),
            op_int(20),
            op_int(30),
            op_int(40),
            op_int(50),
            op_int(60),
            8, // rrcurveto
        ];
        let o = decode(&cs);
        let segs = &o.contours[0].segments;
        match segs[1] {
            CubicSegment::CurveTo { c1, c2, end } => {
                assert_eq!(c1, Point::new(10.0, 20.0));
                assert_eq!(c2, Point::new(40.0, 60.0));
                assert_eq!(end, Point::new(90.0, 120.0));
            }
            _ => panic!("expected CurveTo, got {:?}", segs[1]),
        }
    }

    #[test]
    fn no_endchar_terminates_at_stream_end() {
        // A charstring with a move + line and no terminator closes its
        // contour at end-of-stream (CFF2 has no endchar).
        let cs = [op_int(5), op_int(5), 21, op_int(10), op_int(0), 5];
        let o = decode(&cs);
        assert_eq!(o.contours.len(), 1);
        assert_eq!(
            *o.contours[0].segments.last().unwrap(),
            CubicSegment::ClosePath
        );
    }

    #[test]
    fn rejects_endchar_operator() {
        // 0x0E (endchar) is not a CFF2 operator.
        let g = empty_index_bytes();
        let gsubrs = Cff2Index::parse(&g, 0).unwrap();
        let mut interp = Cff2Interpreter::new(&gsubrs, None, None, 0, &[]);
        let err = interp.run(&[0x0E]).unwrap_err();
        assert!(matches!(err, Error::CharstringUnsupportedOp(14)));
    }

    #[test]
    fn callgsubr_executes_global_subr() {
        // Global subr 0 (bias for count 1 = 107; so subr# = -107).
        // The subr draws: 0 0 rmoveto 10 10 rlineto, then returns.
        let subr = [
            op_int(0),
            op_int(0),
            21,
            op_int(10),
            op_int(10),
            5,
            11, /* return */
        ];
        let g = index_bytes(&[&subr]);
        let gsubrs = Cff2Index::parse(&g, 0).unwrap();
        // Charstring: call gsubr 0 → biased index -107.
        let cs = [op_int(-107), 29 /* callgsubr */];
        let mut interp = Cff2Interpreter::new(&gsubrs, None, None, 0, &[]);
        interp.run(&cs).expect("decode");
        let o = interp.into_outline();
        assert_eq!(o.contours.len(), 1);
        assert_eq!(
            o.contours[0].segments[0],
            CubicSegment::MoveTo(Point::new(0.0, 0.0))
        );
        assert_eq!(
            o.contours[0].segments[1],
            CubicSegment::LineTo(Point::new(10.0, 10.0))
        );
    }

    /// A VariationStore with one ItemVariationData selecting `k` regions
    /// (each a dummy region — only the count matters for `blend`).
    fn ivs_with_k(k: usize) -> ItemVariationStore {
        let regions = (0..k)
            .map(|_| VariationRegion {
                region_axes: vec![RegionAxisCoordinates {
                    start: 0.0,
                    peak: 1.0,
                    end: 1.0,
                }],
            })
            .collect();
        let region_indexes = (0..k as u16).collect();
        ItemVariationStore {
            axis_count: 1,
            regions,
            item_variation_data: vec![ItemVariationData {
                item_count: 0,
                short_delta_count: 0,
                region_indexes,
            }],
        }
    }

    #[test]
    fn blend_default_instance_drops_deltas() {
        // Spec example: "120 52 1 blend hlineto" with k=1. At the
        // default instance (scalar 0) the blended value is the default
        // (120), so the line goes 120 units horizontally.
        let ivs = ivs_with_k(1);
        let g = empty_index_bytes();
        let gsubrs = Cff2Index::parse(&g, 0).unwrap();
        // 0 0 rmoveto, then 120 52 1 blend hlineto.
        let mut cs = vec![op_int(0), op_int(0), 21 /* rmoveto */];
        push_i16(120, &mut cs);
        push_i16(52, &mut cs);
        cs.push(op_int(1));
        cs.push(16); // blend (n=1, k=1)
        cs.push(6); // hlineto (consumes the 1 blended value 120)
        let mut interp = Cff2Interpreter::new(&gsubrs, None, Some(&ivs), 0, &[]);
        interp.run(&cs).expect("decode");
        let o = interp.into_outline();
        // MoveTo(0,0), LineTo(120,0).
        assert_eq!(
            o.contours[0].segments[1],
            CubicSegment::LineTo(Point::new(120.0, 0.0))
        );
    }

    #[test]
    fn blend_one_region_applies_scalar() {
        // "120 52 1 blend hlineto" with k=1 and scalar 0.75:
        // blended = 120 + 0.75 * 52 = 159.
        let ivs = ivs_with_k(1);
        let g = empty_index_bytes();
        let gsubrs = Cff2Index::parse(&g, 0).unwrap();
        let mut cs = vec![op_int(0), op_int(0), 21];
        push_i16(120, &mut cs);
        push_i16(52, &mut cs);
        cs.push(op_int(1));
        cs.push(16);
        cs.push(6);
        let scalars = [0.75f32];
        let mut interp = Cff2Interpreter::new(&gsubrs, None, Some(&ivs), 0, &scalars);
        interp.run(&cs).expect("decode");
        let o = interp.into_outline();
        match o.contours[0].segments[1] {
            CubicSegment::LineTo(p) => assert!((p.x - 159.0).abs() < 1e-3, "got {}", p.x),
            ref s => panic!("expected LineTo, got {s:?}"),
        }
    }

    #[test]
    fn blend_two_regions_applies_scalars() {
        // "120 52 36 1 blend hlineto" with k=2, scalars 0.75 & 0.50:
        // blended = 120 + 0.75*52 + 0.50*36 = 120 + 39 + 18 = 177.
        let ivs = ivs_with_k(2);
        let g = empty_index_bytes();
        let gsubrs = Cff2Index::parse(&g, 0).unwrap();
        let mut cs = vec![op_int(0), op_int(0), 21];
        push_i16(120, &mut cs); // default
        push_i16(52, &mut cs); // delta region 0
        push_i16(36, &mut cs); // delta region 1
        cs.push(op_int(1)); // n = 1
        cs.push(16); // blend
        cs.push(6); // hlineto
        let scalars = [0.75f32, 0.50];
        let mut interp = Cff2Interpreter::new(&gsubrs, None, Some(&ivs), 0, &scalars);
        interp.run(&cs).expect("decode");
        let o = interp.into_outline();
        match o.contours[0].segments[1] {
            CubicSegment::LineTo(p) => assert!((p.x - 177.0).abs() < 1e-3, "got {}", p.x),
            ref s => panic!("expected LineTo, got {s:?}"),
        }
    }

    #[test]
    fn blend_multiple_values() {
        // Two blended operands (n=2), k=1, then rmoveto consumes both.
        // defaults (300, 400), deltas (10, 20), scalar 0.5 →
        // (305, 410).
        let ivs = ivs_with_k(1);
        let g = empty_index_bytes();
        let gsubrs = Cff2Index::parse(&g, 0).unwrap();
        // Layout: 300 400 10 20 2 blend rmoveto. Build with byte-28
        // operands since the values exceed the single-byte range.
        let mut full = Vec::new();
        let push16 = |v: i16, out: &mut Vec<u8>| {
            out.push(28);
            out.extend_from_slice(&v.to_be_bytes());
        };
        push16(300, &mut full);
        push16(400, &mut full);
        push16(10, &mut full);
        push16(20, &mut full);
        full.push(op_int(2)); // n = 2
        full.push(16); // blend
        full.push(21); // rmoveto (consumes 2 blended → dx, dy)
        let scalars = [0.5f32];
        let mut interp = Cff2Interpreter::new(&gsubrs, None, Some(&ivs), 0, &scalars);
        interp.run(&full).expect("decode");
        let o = interp.into_outline();
        match o.contours[0].segments[0] {
            CubicSegment::MoveTo(p) => {
                assert!((p.x - 305.0).abs() < 1e-3, "x={}", p.x);
                assert!((p.y - 410.0).abs() < 1e-3, "y={}", p.y);
            }
            ref s => panic!("expected MoveTo, got {s:?}"),
        }
    }

    #[test]
    fn vsindex_selects_region_list() {
        // Two ItemVariationData: ivd0 → k=1, ivd1 → k=2. Build the IVS
        // by hand. vsindex 1 selects ivd1 (k=2), so a following blend
        // needs 2 deltas.
        let ivs = ItemVariationStore {
            axis_count: 1,
            regions: vec![
                VariationRegion {
                    region_axes: vec![RegionAxisCoordinates {
                        start: 0.0,
                        peak: 1.0,
                        end: 1.0,
                    }],
                },
                VariationRegion {
                    region_axes: vec![RegionAxisCoordinates {
                        start: 0.0,
                        peak: 1.0,
                        end: 1.0,
                    }],
                },
            ],
            item_variation_data: vec![
                ItemVariationData {
                    item_count: 0,
                    short_delta_count: 0,
                    region_indexes: vec![0],
                },
                ItemVariationData {
                    item_count: 0,
                    short_delta_count: 0,
                    region_indexes: vec![0, 1],
                },
            ],
        };
        let g = empty_index_bytes();
        let gsubrs = Cff2Index::parse(&g, 0).unwrap();
        // 1 vsindex   0 0 rmoveto   100 10 20 1 blend hlineto
        // scalars (1.0, 1.0) → 100 + 10 + 20 = 130.
        let mut cs = vec![
            op_int(1),
            15, // vsindex → ivd1 (k=2)
            op_int(0),
            op_int(0),
            21, // rmoveto
        ];
        push_i16(100, &mut cs);
        cs.push(op_int(10));
        cs.push(op_int(20));
        cs.push(op_int(1));
        cs.push(16); // blend (n=1, k=2)
        cs.push(6); // hlineto
        let scalars = [1.0f32, 1.0];
        let mut interp = Cff2Interpreter::new(&gsubrs, None, Some(&ivs), 0, &scalars);
        interp.run(&cs).expect("decode");
        let o = interp.into_outline();
        match o.contours[0].segments[1] {
            CubicSegment::LineTo(p) => assert!((p.x - 130.0).abs() < 1e-3, "x={}", p.x),
            ref s => panic!("expected LineTo, got {s:?}"),
        }
    }

    #[test]
    fn vsindex_after_blend_is_rejected() {
        let ivs = ivs_with_k(1);
        let g = empty_index_bytes();
        let gsubrs = Cff2Index::parse(&g, 0).unwrap();
        // 0 0 rmoveto 100 0 1 blend  1 vsindex (illegal: vsindex after blend)
        let cs = [
            op_int(0),
            op_int(0),
            21,
            op_int(100),
            op_int(0),
            op_int(1),
            16, // blend
            op_int(0),
            15, // vsindex AFTER blend
        ];
        let mut interp = Cff2Interpreter::new(&gsubrs, None, Some(&ivs), 0, &[]);
        let err = interp.run(&cs).unwrap_err();
        assert!(matches!(err, Error::Cff(s) if s.contains("vsindex used after blend")));
    }

    #[test]
    fn blend_without_variation_store_errors() {
        let g = empty_index_bytes();
        let gsubrs = Cff2Index::parse(&g, 0).unwrap();
        let cs = [op_int(100), op_int(0), op_int(1), 16 /* blend */];
        let mut interp = Cff2Interpreter::new(&gsubrs, None, None, 0, &[]);
        let err = interp.run(&cs).unwrap_err();
        assert!(matches!(err, Error::Cff(s) if s.contains("without VariationStore")));
    }
}
