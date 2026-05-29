//! CFF Private DICT (Adobe TN5176 §15).
//!
//! The Private DICT carries hinting parameters (BlueValues, StdHW,
//! StdVW etc.) and — critically — the offset of the Local Subrs
//! INDEX and the `defaultWidthX` / `nominalWidthX` values used by
//! Type 2 charstrings to encode glyph advance width compactly.
//!
//! Round-1 scope parsed `defaultWidthX`, `nominalWidthX`, and the
//! local Subrs offset, leaving hint zones accepted-and-ignored.
//! Round-183 surfaces the full hint-zone operator set on a public
//! [`PrivateHints`] struct so renderers and font-info inspectors can
//! consult them without re-parsing the DICT. Hinting is still not
//! *enforced* by the round-1 outline decoder.

use crate::cff::dict::{Dict, Operand, Operator};
use crate::cff::index::Index;
use crate::Error;

/// PostScript-style hinting parameters recovered from a CFF Private
/// DICT, per Adobe TN5176 §15 Table 23. Every field is surfaced even
/// though the round-1 outline pipeline does not enforce hints; callers
/// rendering at small sizes (or interrogating font metadata) can use
/// the values directly.
///
/// **Delta-encoded fields**. `blue_values`, `other_blues`,
/// `family_blues`, `family_other_blues`, `stem_snap_h`, `stem_snap_v`
/// are declared as the "delta" operand type in TN5176 §4 Table 4: the
/// first operand is absolute and each subsequent operand is the
/// **difference from the running total**. This struct stores the
/// **undeltified** (running-sum) absolute values, matching the
/// convention already in use for `TopMetadata::base_font_blend`. So a
/// raw operand stream of `[-14, 14, 662, 14, -226, 10, 223, 0]` for
/// `BlueValues` (one of TN5176's worked Appendix-D examples) decodes
/// to absolute values `[-14, 0, 662, 676, 450, 460, 683, 683]`.
///
/// **Scalar fields**. `blue_scale`, `blue_shift`, `blue_fuzz`,
/// `std_hw`, `std_vw`, `language_group`, `expansion_factor`,
/// `initial_random_seed` are plain "number" operands; the value is
/// kept as `f64` to preserve BCD-real precision. `force_bold` is the
/// only "boolean" operand and is decoded as `false` for `0` and
/// `true` for any non-zero integer (the spec only assigns 0 / 1).
///
/// Defaults match TN5176 §15 Table 23 exactly: zeros / empty vectors
/// for the unspecified fields, the spec-listed defaults for the
/// scalar fields with documented defaults (`blue_scale = 0.039625`,
/// `blue_shift = 7`, `blue_fuzz = 1`, `language_group = 0`,
/// `expansion_factor = 0.06`, `initial_random_seed = 0`,
/// `force_bold = false`). The two "delta -" defaults (BlueValues,
/// OtherBlues, FamilyBlues, FamilyOtherBlues, StemSnapH, StemSnapV)
/// surface as empty vectors. The "number -" defaults (StdHW, StdVW)
/// surface as `None` so callers can distinguish "absent" from "zero."
#[derive(Debug, Clone, PartialEq)]
pub struct PrivateHints {
    /// `BlueValues` (op 6) — pairs of `(bottom, top)` y-coordinates
    /// for the *primary* (lowercase + uppercase x-height / cap-height)
    /// horizontal alignment zones. Undeltified into absolute values.
    pub blue_values: Vec<f64>,
    /// `OtherBlues` (op 7) — secondary alignment zones below the
    /// baseline (descenders). Same `(bottom, top)` pair convention as
    /// `blue_values`. Undeltified.
    pub other_blues: Vec<f64>,
    /// `FamilyBlues` (op 8) — `BlueValues` of the family's *base*
    /// design when this font is a derived weight; lets the rasterizer
    /// keep heavier weights aligned with the base. Undeltified.
    pub family_blues: Vec<f64>,
    /// `FamilyOtherBlues` (op 9) — `OtherBlues` of the family's base
    /// design. Undeltified.
    pub family_other_blues: Vec<f64>,
    /// `BlueScale` (op 12 9) — point size below which overshoot
    /// suppression is disabled. Spec default: `0.039625`.
    pub blue_scale: f64,
    /// `BlueShift` (op 12 10) — minimum stem width at which overshoot
    /// is suppressed (in character-space units). Spec default: `7`.
    pub blue_shift: f64,
    /// `BlueFuzz` (op 12 11) — number of character-space units to
    /// expand each alignment zone outward when matching. Spec default:
    /// `1`.
    pub blue_fuzz: f64,
    /// `StdHW` (op 10) — dominant horizontal stem width. `None` if the
    /// operator is absent (spec lists no default).
    pub std_hw: Option<f64>,
    /// `StdVW` (op 11) — dominant vertical stem width. `None` if the
    /// operator is absent (spec lists no default).
    pub std_vw: Option<f64>,
    /// `StemSnapH` (op 12 12) — supplementary horizontal stem widths;
    /// rasterizer "snaps" stems within tolerance to one of these.
    /// Undeltified into absolute widths.
    pub stem_snap_h: Vec<f64>,
    /// `StemSnapV` (op 12 13) — supplementary vertical stem widths.
    /// Undeltified.
    pub stem_snap_v: Vec<f64>,
    /// `ForceBold` (op 12 14) — synthetic-bold flag for Multiple Master
    /// instances. Boolean (0 / 1). Spec default: `false`.
    pub force_bold: bool,
    /// `LanguageGroup` (op 12 17) — `0` for Latin / Cyrillic / etc.,
    /// `1` for Chinese / Japanese / Korean (CJK). Spec default: `0`.
    pub language_group: i32,
    /// `ExpansionFactor` (op 12 18) — limit on the per-counter expansion
    /// allowed when forcing bold. Spec default: `0.06`.
    pub expansion_factor: f64,
    /// `initialRandomSeed` (op 12 19) — seed for the Type 2 `random`
    /// operator. Spec default: `0`.
    pub initial_random_seed: i32,
}

impl Default for PrivateHints {
    fn default() -> Self {
        // Spec defaults straight from TN5176 §15 Table 23.
        Self {
            blue_values: Vec::new(),
            other_blues: Vec::new(),
            family_blues: Vec::new(),
            family_other_blues: Vec::new(),
            blue_scale: 0.039625,
            blue_shift: 7.0,
            blue_fuzz: 1.0,
            std_hw: None,
            std_vw: None,
            stem_snap_h: Vec::new(),
            stem_snap_v: Vec::new(),
            force_bold: false,
            language_group: 0,
            expansion_factor: 0.06,
            initial_random_seed: 0,
        }
    }
}

/// Undeltify a "delta" operand array (TN5176 §4 Table 4): the first
/// entry is absolute, each subsequent entry is the difference from the
/// running total. Returns the running sums as `f64`. The transient
/// `BaseFontBlend` extractor in `cff::mod` already uses the same shape;
/// keeping a copy here avoids a cross-module dep cycle on the metadata
/// struct.
fn undeltify(operands: &[Operand]) -> Vec<f64> {
    let mut acc = 0.0_f64;
    let mut out = Vec::with_capacity(operands.len());
    for op in operands {
        acc += op.as_f64();
        out.push(acc);
    }
    out
}

/// Pull a scalar `f64` value out of a single-operand DICT entry,
/// falling back to `default` if the operator is absent. The CFF spec
/// allows both `Int` and `Real` encodings for "number" operands; both
/// are accepted.
fn number_or(dict: &Dict, op: Operator, default: f64) -> f64 {
    dict.get_array(op)
        .and_then(|v| v.last().copied())
        .map(|o| o.as_f64())
        .unwrap_or(default)
}

/// Same as [`number_or`] but returns `None` when the operator is
/// absent. Used for the two `StdHW` / `StdVW` operators that have no
/// spec-defined default value.
fn number_opt(dict: &Dict, op: Operator) -> Option<f64> {
    dict.get_array(op)
        .and_then(|v| v.last().copied())
        .map(|o| o.as_f64())
}

/// Pull an integer with a fallback default.
fn int_or(dict: &Dict, op: Operator, default: i32) -> i32 {
    dict.get_array(op)
        .and_then(|v| v.last().copied())
        .map(|o| match o {
            Operand::Int(n) => n,
            Operand::Real(r) => r as i32,
        })
        .unwrap_or(default)
}

/// Parsed Private DICT, with its (optional) Local Subrs INDEX
/// resolved and the full hint-zone operator set surfaced on
/// [`PrivateHints`].
#[derive(Debug, Clone)]
pub(crate) struct PrivateDict<'a> {
    pub(crate) default_width_x: f32,
    pub(crate) nominal_width_x: f32,
    pub(crate) local_subrs: Option<Index<'a>>,
    pub(crate) hints: PrivateHints,
}

impl<'a> PrivateDict<'a> {
    /// Parse a Private DICT located at `[private_off, private_off +
    /// size)` within the CFF table bytes.
    ///
    /// The Local Subrs offset is *relative to the Private DICT
    /// start*, per TN5176 §15 (operator 19): "The local subrs offset
    /// is from the start of the Private DICT data".
    pub(crate) fn parse(bytes: &'a [u8], private_off: usize, size: usize) -> Result<Self, Error> {
        let end = private_off
            .checked_add(size)
            .ok_or(Error::Cff("Private DICT size overflow"))?;
        if end > bytes.len() {
            return Err(Error::UnexpectedEof);
        }
        let dict = Dict::parse(&bytes[private_off..end])?;

        // defaultWidthX is opcode 20; nominalWidthX is opcode 21.
        // Both default to 0 if absent.
        let default_width_x = dict
            .get_array(Operator::DefaultWidthX)
            .and_then(|v| v.last().copied())
            .map(|o| o.as_f64() as f32)
            .unwrap_or(0.0);
        let nominal_width_x = dict
            .get_array(Operator::NominalWidthX)
            .and_then(|v| v.last().copied())
            .map(|o| o.as_f64() as f32)
            .unwrap_or(0.0);

        // Hint zones (TN5176 §15 Table 23) — see PrivateHints docs.
        let hints = PrivateHints {
            blue_values: dict
                .get_array(Operator::BlueValues)
                .map(undeltify)
                .unwrap_or_default(),
            other_blues: dict
                .get_array(Operator::OtherBlues)
                .map(undeltify)
                .unwrap_or_default(),
            family_blues: dict
                .get_array(Operator::FamilyBlues)
                .map(undeltify)
                .unwrap_or_default(),
            family_other_blues: dict
                .get_array(Operator::FamilyOtherBlues)
                .map(undeltify)
                .unwrap_or_default(),
            blue_scale: number_or(&dict, Operator::BlueScale, 0.039625),
            blue_shift: number_or(&dict, Operator::BlueShift, 7.0),
            blue_fuzz: number_or(&dict, Operator::BlueFuzz, 1.0),
            std_hw: number_opt(&dict, Operator::StdHW),
            std_vw: number_opt(&dict, Operator::StdVW),
            stem_snap_h: dict
                .get_array(Operator::StemSnapH)
                .map(undeltify)
                .unwrap_or_default(),
            stem_snap_v: dict
                .get_array(Operator::StemSnapV)
                .map(undeltify)
                .unwrap_or_default(),
            // ForceBold is the only "boolean" operand in Table 23.
            // TN5176 §4 Table 4 defines the boolean type as an integer
            // operand whose value is 0 (false) or 1 (true).
            force_bold: dict
                .get_array(Operator::ForceBold)
                .and_then(|v| v.last().copied())
                .and_then(|o| o.as_int())
                .map(|n| n != 0)
                .unwrap_or(false),
            language_group: int_or(&dict, Operator::LanguageGroup, 0),
            expansion_factor: number_or(&dict, Operator::ExpansionFactor, 0.06),
            initial_random_seed: int_or(&dict, Operator::InitialRandomSeed, 0),
        };

        let local_subrs = if let Some(off) = dict.get_int(Operator::Subrs) {
            if off < 0 {
                return Err(Error::Cff("negative local subrs offset"));
            }
            // Offset is relative to the start of the Private DICT —
            // but we hand the absolute index location to Index::parse.
            let abs = private_off
                .checked_add(off as usize)
                .ok_or(Error::Cff("local subrs overflow"))?;
            Some(Index::parse(bytes, abs)?)
        } else {
            None
        };

        Ok(Self {
            default_width_x,
            nominal_width_x,
            local_subrs,
            hints,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_when_absent() {
        // Empty Private DICT.
        let bytes = vec![];
        let p = PrivateDict::parse(&bytes, 0, 0).unwrap();
        assert_eq!(p.default_width_x, 0.0);
        assert_eq!(p.nominal_width_x, 0.0);
        assert!(p.local_subrs.is_none());
        // PrivateHints spec defaults straight from Table 23.
        assert_eq!(p.hints, PrivateHints::default());
        assert_eq!(p.hints.blue_scale, 0.039625);
        assert_eq!(p.hints.blue_shift, 7.0);
        assert_eq!(p.hints.blue_fuzz, 1.0);
        assert!(p.hints.std_hw.is_none());
        assert!(p.hints.std_vw.is_none());
        assert!(!p.hints.force_bold);
        assert_eq!(p.hints.language_group, 0);
        assert_eq!(p.hints.expansion_factor, 0.06);
        assert_eq!(p.hints.initial_random_seed, 0);
        assert!(p.hints.blue_values.is_empty());
    }

    #[test]
    fn picks_up_widths() {
        // defaultWidthX = 500 (opcode 20), nominalWidthX = 400 (opcode 21).
        // Encode 500: needs the 247..250 form. 500 - 108 = 392; 392 / 256
        // = 1, remainder 136 → b0 = 247 + 1 = 248, b1 = 136.
        // Encode 400: 400 - 108 = 292; 292 / 256 = 1, remainder 36 →
        // b0 = 248, b1 = 36.
        let dict_bytes = vec![248, 136, 20, 248, 36, 21];
        // Wrap into a CFF byte buffer: Private DICT lives at offset 4.
        let mut whole = vec![0u8, 0, 0, 0];
        whole.extend_from_slice(&dict_bytes);
        let p = PrivateDict::parse(&whole, 4, dict_bytes.len()).unwrap();
        assert_eq!(p.default_width_x, 500.0);
        assert_eq!(p.nominal_width_x, 400.0);
    }

    /// Encode a positive small integer per TN5176 §4 Table 3.
    /// Range -107..=107 fits in one byte (b0 = n + 139).
    fn enc_int_1(n: i32) -> Vec<u8> {
        assert!((-107..=107).contains(&n));
        vec![(n + 139) as u8]
    }

    /// Encode an integer using the 2-byte short-int form (28 b1 b2,
    /// big-endian i16). Covers -32768..=32767.
    fn enc_int_short(n: i32) -> Vec<u8> {
        let n = n as i16;
        let b = n.to_be_bytes();
        vec![28, b[0], b[1]]
    }

    #[test]
    fn blue_values_undeltified() {
        // TN5176 §4 Table 4 worked example for "delta": the raw
        // operand stream encodes deltas-from-running-total; the
        // accessor returns the running totals.
        //
        // BlueValues = [-14, 14, 662, 14, -226, 10, 223, 0]
        // Running sum: [-14, 0, 662, 676, 450, 460, 683, 683]
        let mut dict_bytes = Vec::new();
        for n in [-14, 14, 662, 14, -226, 10, 223, 0] {
            if (-107..=107).contains(&n) {
                dict_bytes.extend(enc_int_1(n));
            } else {
                dict_bytes.extend(enc_int_short(n));
            }
        }
        dict_bytes.push(6); // BlueValues operator
        let p = PrivateDict::parse(&dict_bytes, 0, dict_bytes.len()).unwrap();
        assert_eq!(
            p.hints.blue_values,
            vec![-14.0, 0.0, 662.0, 676.0, 450.0, 460.0, 683.0, 683.0]
        );
    }

    #[test]
    fn other_and_family_blues() {
        // OtherBlues (op 7) = [-250, 8]  → [-250, -242]
        // FamilyBlues (op 8) = [0, 10]   → [0, 10]
        let mut dict_bytes = Vec::new();
        dict_bytes.extend(enc_int_short(-250));
        dict_bytes.extend(enc_int_1(8));
        dict_bytes.push(7);
        dict_bytes.extend(enc_int_1(0));
        dict_bytes.extend(enc_int_1(10));
        dict_bytes.push(8);
        let p = PrivateDict::parse(&dict_bytes, 0, dict_bytes.len()).unwrap();
        assert_eq!(p.hints.other_blues, vec![-250.0, -242.0]);
        assert_eq!(p.hints.family_blues, vec![0.0, 10.0]);
        // FamilyOtherBlues not emitted → default empty.
        assert!(p.hints.family_other_blues.is_empty());
    }

    #[test]
    fn std_widths_and_stem_snaps() {
        // StdHW (op 10) = 28
        // StdVW (op 11) = 84
        // StemSnapH (op 12 12) = [28, 8]   → [28, 36]
        // StemSnapV (op 12 13) = [84, -10] → [84, 74]
        let mut dict_bytes = Vec::new();
        dict_bytes.extend(enc_int_1(28));
        dict_bytes.push(10);
        dict_bytes.extend(enc_int_1(84));
        dict_bytes.push(11);
        dict_bytes.extend(enc_int_1(28));
        dict_bytes.extend(enc_int_1(8));
        dict_bytes.extend_from_slice(&[12, 12]);
        dict_bytes.extend(enc_int_1(84));
        dict_bytes.extend(enc_int_1(-10));
        dict_bytes.extend_from_slice(&[12, 13]);
        let p = PrivateDict::parse(&dict_bytes, 0, dict_bytes.len()).unwrap();
        assert_eq!(p.hints.std_hw, Some(28.0));
        assert_eq!(p.hints.std_vw, Some(84.0));
        assert_eq!(p.hints.stem_snap_h, vec![28.0, 36.0]);
        assert_eq!(p.hints.stem_snap_v, vec![84.0, 74.0]);
    }

    #[test]
    fn force_bold_and_language_group() {
        // ForceBold (op 12 14) = true (1)
        // LanguageGroup (op 12 17) = 1  (CJK)
        let mut dict_bytes = Vec::new();
        dict_bytes.extend(enc_int_1(1));
        dict_bytes.extend_from_slice(&[12, 14]);
        dict_bytes.extend(enc_int_1(1));
        dict_bytes.extend_from_slice(&[12, 17]);
        let p = PrivateDict::parse(&dict_bytes, 0, dict_bytes.len()).unwrap();
        assert!(p.hints.force_bold);
        assert_eq!(p.hints.language_group, 1);
        // Non-default value sticks; other defaults still hold.
        assert_eq!(p.hints.blue_scale, 0.039625);
    }

    #[test]
    fn blue_scale_shift_fuzz_overrides() {
        // BlueScale (op 12 9) overridden to non-default value.
        // We can't reach 0.039625 with int operands; use a small int
        // override (BlueScale = 50) to prove the parse path picks up
        // an override at all.
        let mut dict_bytes = Vec::new();
        dict_bytes.extend(enc_int_1(50));
        dict_bytes.extend_from_slice(&[12, 9]);
        dict_bytes.extend(enc_int_1(5));
        dict_bytes.extend_from_slice(&[12, 10]);
        dict_bytes.extend(enc_int_1(2));
        dict_bytes.extend_from_slice(&[12, 11]);
        let p = PrivateDict::parse(&dict_bytes, 0, dict_bytes.len()).unwrap();
        assert_eq!(p.hints.blue_scale, 50.0);
        assert_eq!(p.hints.blue_shift, 5.0);
        assert_eq!(p.hints.blue_fuzz, 2.0);
    }

    #[test]
    fn expansion_factor_and_random_seed() {
        // ExpansionFactor (op 12 18) = 0 (override default 0.06)
        // initialRandomSeed (op 12 19) = 42
        let mut dict_bytes = Vec::new();
        dict_bytes.extend(enc_int_1(0));
        dict_bytes.extend_from_slice(&[12, 18]);
        dict_bytes.extend(enc_int_1(42));
        dict_bytes.extend_from_slice(&[12, 19]);
        let p = PrivateDict::parse(&dict_bytes, 0, dict_bytes.len()).unwrap();
        assert_eq!(p.hints.expansion_factor, 0.0);
        assert_eq!(p.hints.initial_random_seed, 42);
    }

    /// TN5176 §A worked Appendix-D example: the spec's full Private
    /// DICT lays out
    ///   -14 14 662 14 -226 10 223 0  BlueValues
    ///   262 8 -488 1                 OtherBlues
    ///   -14 14 450 10 202 14         FamilyBlues
    ///   -218 1 479 8 124 0           FamilyOtherBlues
    ///   28 StdHW
    ///   84 StdVW
    ///   250 defaultWidthX
    /// Verify every undeltified vector + StdHW + StdVW + defaultWidthX
    /// against TN5176 §A's listing line "Private DICT (00000066)".
    #[test]
    fn tn5176_appendix_d_private_dict_example() {
        let mut d = Vec::new();
        // BlueValues
        for n in [-14, 14, 662, 14, -226, 10, 223, 0] {
            if (-107..=107).contains(&n) {
                d.extend(enc_int_1(n));
            } else {
                d.extend(enc_int_short(n));
            }
        }
        d.push(6);
        // OtherBlues
        for n in [262, 8, -488, 1] {
            if (-107..=107).contains(&n) {
                d.extend(enc_int_1(n));
            } else {
                d.extend(enc_int_short(n));
            }
        }
        d.push(7);
        // FamilyBlues
        for n in [-14, 14, 450, 10, 202, 14] {
            if (-107..=107).contains(&n) {
                d.extend(enc_int_1(n));
            } else {
                d.extend(enc_int_short(n));
            }
        }
        d.push(8);
        // FamilyOtherBlues
        for n in [-218, 1, 479, 8, 124, 0] {
            if (-107..=107).contains(&n) {
                d.extend(enc_int_1(n));
            } else {
                d.extend(enc_int_short(n));
            }
        }
        d.push(9);
        // StdHW
        d.extend(enc_int_1(28));
        d.push(10);
        // StdVW
        d.extend(enc_int_1(84));
        d.push(11);
        // defaultWidthX
        d.extend(enc_int_short(250));
        d.push(20);
        let p = PrivateDict::parse(&d, 0, d.len()).unwrap();
        assert_eq!(
            p.hints.blue_values,
            vec![-14.0, 0.0, 662.0, 676.0, 450.0, 460.0, 683.0, 683.0]
        );
        assert_eq!(p.hints.other_blues, vec![262.0, 270.0, -218.0, -217.0]);
        assert_eq!(
            p.hints.family_blues,
            vec![-14.0, 0.0, 450.0, 460.0, 662.0, 676.0]
        );
        assert_eq!(
            p.hints.family_other_blues,
            vec![-218.0, -217.0, 262.0, 270.0, 394.0, 394.0]
        );
        assert_eq!(p.hints.std_hw, Some(28.0));
        assert_eq!(p.hints.std_vw, Some(84.0));
        assert_eq!(p.default_width_x, 250.0);
    }
}
