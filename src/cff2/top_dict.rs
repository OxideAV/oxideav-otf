//! CFF2 Top DICT (OpenType 1.9.1 `CFF2` table, §7 "TopDICT").
//!
//! The CFF2 Top DICT is a `DICT` (same operand encoding as CFF1
//! TN5176 §4 Table 3) carrying exactly five operators:
//!
//! | Hex     | Dec   | Name                  | Required               | Default                |
//! | ------- | ----- | --------------------- | ---------------------- | ---------------------- |
//! | 0x11    | 17    | CharStringINDEXOffset | yes                    | —                      |
//! | 0x18    | 24    | VariationStoreOffset  | only with variations   | —                      |
//! | 0x0c24  | 12,36 | FontDICTINDEXOffset   | yes                    | —                      |
//! | 0x0c25  | 12,37 | FontDICTSelectOffset  | no                     | —                      |
//! | 0x0c07  | 12,7  | FontMatrix            | no (unitsPerEm != 1000)| 0.001 0 0 0.001 0 0    |
//!
//! Offsets are all relative to the start of the CFF2 table. The
//! `FontMatrix` operator's domain is restricted in CFF2: only matrices
//! of the form `[s 0 0 s 0 0]` are allowed (i.e. uniform scaling with
//! no translation), and `s` must equal `1 / unitsPerEm` from the
//! `head` table (spec §7 "FontMatrix" + §14 "Adjustments to existing
//! tables — head").
//!
//! Operand encoding is reused verbatim from the CFF1 DICT parser
//! (`crate::cff::dict`).

use crate::cff::dict::{Dict, Operand};
use crate::Error;

/// CFF2 Top DICT operator codes. Stored as their on-disk encoding:
/// single-byte operators in the low byte, two-byte (escape `12 nn`)
/// operators as `0x0C00 | nn`. The numeric values match the same
/// convention as the CFF1 `Operator` enum (so the shared `Dict`
/// parser's operator slot can be matched against either set).
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u16)]
pub enum Cff2Op {
    /// `CharStringINDEXOffset` — required. Offset to the CharString
    /// INDEX (the per-glyph charstrings) from the CFF2 table start.
    CharStringIndexOffset = 17,
    /// `VariationStoreOffset` — required only for variable fonts.
    /// Offset to the OpenType ItemVariationStore from the CFF2 table
    /// start.
    VariationStoreOffset = 24,
    /// `FontDICTINDEXOffset` — required. Offset to the Font DICT INDEX
    /// (each entry is a Font DICT containing a Private DICT pointer).
    FontDictIndexOffset = 0x0C24,
    /// `FontDICTSelectOffset` — optional. Present only when the font
    /// has more than one Font DICT (analogue of CFF1 FDSelect for
    /// glyph → FD routing).
    FontDictSelectOffset = 0x0C25,
    /// `FontMatrix` — optional. `[scale 0 0 scale 0 0]` where `scale`
    /// must equal `1 / unitsPerEm`. Default `0.001 0 0 0.001 0 0`
    /// (i.e. the unitsPerEm = 1000 case).
    FontMatrix = 0x0C07,
}

impl Cff2Op {
    fn code(self) -> u16 {
        self as u16
    }
}

/// Parsed CFF2 Top DICT. Holds the five spec-permitted operators (with
/// defaults applied where the spec defines one).
#[derive(Debug, Clone)]
pub struct Cff2TopDict {
    /// `CharStringINDEXOffset` — offset to the CharStringINDEX from
    /// the start of the CFF2 table. Required (no default).
    pub charstring_index_offset: u32,
    /// `FontDICTINDEXOffset` — offset to the FontDICTINDEX from the
    /// start of the CFF2 table. Required (no default).
    pub font_dict_index_offset: u32,
    /// `FontDICTSelectOffset` — present only when the font has more
    /// than one Font DICT.
    pub font_dict_select_offset: Option<u32>,
    /// `VariationStoreOffset` — present iff the font has variations
    /// (per spec §7 "VariationStoreOffset" Occurrence note: required
    /// in variable fonts, forbidden otherwise).
    pub variation_store_offset: Option<u32>,
    /// `FontMatrix`, `[a b c d tx ty]`. CFF2 restricts this to
    /// uniform scaling with no translation, so `a == d` and the other
    /// four are `0`. The spec default `0.001 0 0 0.001 0 0` is applied
    /// when the operator is absent (i.e. when `unitsPerEm == 1000`).
    pub font_matrix: [f64; 6],
    /// True if the on-disk Top DICT carried a `FontMatrix` operator;
    /// false if the default was substituted. Useful when round-tripping
    /// the table (writers should omit the operator when the font's
    /// `unitsPerEm` is 1000 per the spec recommendation).
    pub has_font_matrix: bool,
}

impl Default for Cff2TopDict {
    fn default() -> Self {
        Self {
            charstring_index_offset: 0,
            font_dict_index_offset: 0,
            font_dict_select_offset: None,
            variation_store_offset: None,
            font_matrix: DEFAULT_FONT_MATRIX,
            has_font_matrix: false,
        }
    }
}

/// CFF2 spec default `FontMatrix` (§7) — `[0.001 0 0 0.001 0 0]`.
pub const DEFAULT_FONT_MATRIX: [f64; 6] = [0.001, 0.0, 0.0, 0.001, 0.0, 0.0];

impl Cff2TopDict {
    /// Parse a CFF2 Top DICT from raw bytes. The DICT operand stream
    /// is the same as CFF1 (TN5176 §4) so we reuse the existing
    /// parser; only the operator set (and rejection of any
    /// CFF1-specific operator) is CFF2-specific.
    pub(crate) fn parse(bytes: &[u8]) -> Result<Self, Error> {
        let dict = Dict::parse(bytes)?;
        let mut out = Self::default();

        let mut have_charstring = false;
        let mut have_font_dict_index = false;

        for (op, operands) in dict.iter() {
            match *op {
                code if code == Cff2Op::CharStringIndexOffset.code() => {
                    out.charstring_index_offset = take_offset(operands)?;
                    have_charstring = true;
                }
                code if code == Cff2Op::FontDictIndexOffset.code() => {
                    out.font_dict_index_offset = take_offset(operands)?;
                    have_font_dict_index = true;
                }
                code if code == Cff2Op::FontDictSelectOffset.code() => {
                    out.font_dict_select_offset = Some(take_offset(operands)?);
                }
                code if code == Cff2Op::VariationStoreOffset.code() => {
                    out.variation_store_offset = Some(take_offset(operands)?);
                }
                code if code == Cff2Op::FontMatrix.code() => {
                    out.font_matrix = take_font_matrix(operands)?;
                    out.has_font_matrix = true;
                }
                // Per spec §7, no other operators are allowed in a CFF2
                // Top DICT. We tolerate (skip) any unknown one rather
                // than rejecting the font outright, mirroring the
                // CFF1 parser's policy. Strict callers can rely on the
                // recognised-only fields.
                _ => {}
            }
        }

        if !have_charstring {
            return Err(Error::Cff(
                "CFF2 Top DICT missing CharStringINDEXOffset (op 17)",
            ));
        }
        if !have_font_dict_index {
            return Err(Error::Cff(
                "CFF2 Top DICT missing FontDICTINDEXOffset (op 12 36)",
            ));
        }

        Ok(out)
    }

    /// Convenience: `true` when the Top DICT references an OpenType
    /// ItemVariationStore (i.e. the font is a variable font).
    pub fn is_variable(&self) -> bool {
        self.variation_store_offset.is_some()
    }
}

/// Pull a single non-negative offset operand from a DICT entry.
fn take_offset(operands: &[Operand]) -> Result<u32, Error> {
    let last = operands
        .last()
        .ok_or(Error::Cff("CFF2 Top DICT: offset operator with no operand"))?;
    let v = last
        .as_int()
        .ok_or(Error::Cff("CFF2 Top DICT: non-integer offset operand"))?;
    if v < 0 {
        return Err(Error::Cff("CFF2 Top DICT: negative offset"));
    }
    Ok(v as u32)
}

/// Pull a 6-operand FontMatrix and check the CFF2 §7 shape constraint:
/// `[a 0 0 d 0 0]` with `a == d`.
fn take_font_matrix(operands: &[Operand]) -> Result<[f64; 6], Error> {
    if operands.len() < 6 {
        return Err(Error::Cff("CFF2 Top DICT FontMatrix < 6 operands"));
    }
    let m = [
        operands[0].as_f64(),
        operands[1].as_f64(),
        operands[2].as_f64(),
        operands[3].as_f64(),
        operands[4].as_f64(),
        operands[5].as_f64(),
    ];
    // CFF2 §7 "FontMatrix": only matrices with uniform horizontal and
    // vertical scaling without translation are permitted.
    let off_diagonal_or_translation_zero = m[1] == 0.0 && m[2] == 0.0 && m[4] == 0.0 && m[5] == 0.0;
    if !off_diagonal_or_translation_zero {
        return Err(Error::Cff(
            "CFF2 Top DICT FontMatrix: off-diagonal/translation non-zero",
        ));
    }
    if (m[0] - m[3]).abs() > f64::EPSILON {
        return Err(Error::Cff("CFF2 Top DICT FontMatrix: scale[0] != scale[3]"));
    }
    Ok(m)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Encode an unsigned 16-bit value via CFF DICT operand byte 28.
    /// Use this in tests to embed any 0..=32767 offset compactly.
    fn op_i16(v: i16) -> Vec<u8> {
        let b = v.to_be_bytes();
        vec![28, b[0], b[1]]
    }

    /// Encode an unsigned 32-bit value via CFF DICT operand byte 29.
    fn op_i32(v: i32) -> Vec<u8> {
        let b = v.to_be_bytes();
        vec![29, b[0], b[1], b[2], b[3]]
    }

    /// Encode a CFF DICT BCD real for 0.0005 (`= 1 / 2000`). Nibbles:
    /// 0, ., 0, 0, 0, 5, end → 0a 00 05 f0 (with trailing 0xf).
    fn op_bcd_5em4() -> Vec<u8> {
        // "0.0005" → nibbles 0 a 0 0 0 5 f → bytes 0x0a, 0x00, 0x05, 0xff
        // (the trailing 0xff is "5 / f" packed; the spec wants f as end
        // sentinel and any half-byte after f is filled with f, so 0xff
        // works).
        vec![30, 0x0a, 0x00, 0x05, 0xff]
    }

    /// Encode the integer 0 via CFF DICT operand (32..246 → b0-139, so
    /// `0` → byte 139).
    fn op_zero() -> u8 {
        139
    }

    #[test]
    fn parses_minimal_top_dict() {
        // CharStringINDEXOffset = 200 (i16 form), FontDICTINDEXOffset = 100.
        let mut bytes = Vec::new();
        bytes.extend(op_i16(200));
        bytes.push(17); // CharStringINDEXOffset
        bytes.extend(op_i16(100));
        bytes.extend_from_slice(&[12, 36]); // FontDICTINDEXOffset
        let top = Cff2TopDict::parse(&bytes).expect("parse");
        assert_eq!(top.charstring_index_offset, 200);
        assert_eq!(top.font_dict_index_offset, 100);
        assert!(!top.has_font_matrix);
        assert!(top.font_dict_select_offset.is_none());
        assert!(top.variation_store_offset.is_none());
        assert!(!top.is_variable());
        // Spec default font matrix applied.
        assert_eq!(top.font_matrix, DEFAULT_FONT_MATRIX);
    }

    #[test]
    fn parses_variable_font_top_dict() {
        // CharString=200, FontDICT=100, VariationStore=50.
        let mut bytes = Vec::new();
        bytes.extend(op_i16(200));
        bytes.push(17);
        bytes.extend(op_i16(100));
        bytes.extend_from_slice(&[12, 36]);
        bytes.extend(op_i16(50));
        bytes.push(24); // VariationStoreOffset
        let top = Cff2TopDict::parse(&bytes).expect("parse");
        assert_eq!(top.variation_store_offset, Some(50));
        assert!(top.is_variable());
    }

    #[test]
    fn parses_multi_fd_top_dict() {
        let mut bytes = Vec::new();
        bytes.extend(op_i16(200));
        bytes.push(17);
        bytes.extend(op_i16(100));
        bytes.extend_from_slice(&[12, 36]);
        bytes.extend(op_i16(150));
        bytes.extend_from_slice(&[12, 37]); // FontDICTSelectOffset
        let top = Cff2TopDict::parse(&bytes).expect("parse");
        assert_eq!(top.font_dict_select_offset, Some(150));
    }

    #[test]
    fn parses_font_matrix() {
        // FontMatrix = [0.0005, 0, 0, 0.0005, 0, 0] for upem = 2000.
        let mut bytes = Vec::new();
        bytes.extend(op_bcd_5em4()); // a
        bytes.push(op_zero()); // b
        bytes.push(op_zero()); // c
        bytes.extend(op_bcd_5em4()); // d
        bytes.push(op_zero()); // tx
        bytes.push(op_zero()); // ty
        bytes.extend_from_slice(&[12, 7]); // FontMatrix
        bytes.extend(op_i16(200));
        bytes.push(17);
        bytes.extend(op_i16(100));
        bytes.extend_from_slice(&[12, 36]);
        let top = Cff2TopDict::parse(&bytes).expect("parse");
        assert!(top.has_font_matrix);
        assert!((top.font_matrix[0] - 0.0005).abs() < 1e-12);
        assert!((top.font_matrix[3] - 0.0005).abs() < 1e-12);
        assert_eq!(top.font_matrix[1], 0.0);
        assert_eq!(top.font_matrix[5], 0.0);
    }

    #[test]
    fn rejects_missing_charstring_offset() {
        // Only FontDICTINDEXOffset is present.
        let mut bytes = Vec::new();
        bytes.extend(op_i16(100));
        bytes.extend_from_slice(&[12, 36]);
        let err = Cff2TopDict::parse(&bytes).unwrap_err();
        match err {
            Error::Cff(s) => assert!(s.contains("CharStringINDEXOffset")),
            _ => panic!("unexpected: {err:?}"),
        }
    }

    #[test]
    fn rejects_missing_font_dict_offset() {
        let mut bytes = Vec::new();
        bytes.extend(op_i16(200));
        bytes.push(17);
        let err = Cff2TopDict::parse(&bytes).unwrap_err();
        match err {
            Error::Cff(s) => assert!(s.contains("FontDICTINDEXOffset")),
            _ => panic!("unexpected: {err:?}"),
        }
    }

    #[test]
    fn rejects_non_uniform_font_matrix() {
        // FontMatrix = [0.0005, 0, 0, 0.001, 0, 0] — a != d.
        let mut bytes = Vec::new();
        bytes.extend(op_bcd_5em4());
        bytes.push(op_zero());
        bytes.push(op_zero());
        // 0.001 as BCD: 0 . 0 0 1 end → 0a 00 1f
        bytes.extend_from_slice(&[30, 0x0a, 0x00, 0x1f]);
        bytes.push(op_zero());
        bytes.push(op_zero());
        bytes.extend_from_slice(&[12, 7]);
        bytes.extend(op_i16(200));
        bytes.push(17);
        bytes.extend(op_i16(100));
        bytes.extend_from_slice(&[12, 36]);
        let err = Cff2TopDict::parse(&bytes).unwrap_err();
        match err {
            Error::Cff(s) => assert!(s.contains("scale[0] != scale[3]")),
            _ => panic!("unexpected: {err:?}"),
        }
    }

    #[test]
    fn rejects_font_matrix_with_translation() {
        // FontMatrix = [0.0005, 0, 0, 0.0005, 100, 0] — tx != 0.
        let mut bytes = Vec::new();
        bytes.extend(op_bcd_5em4());
        bytes.push(op_zero());
        bytes.push(op_zero());
        bytes.extend(op_bcd_5em4());
        // 100 as DICT int: 100 + 139 = 239.
        bytes.push(239);
        bytes.push(op_zero());
        bytes.extend_from_slice(&[12, 7]);
        bytes.extend(op_i16(200));
        bytes.push(17);
        bytes.extend(op_i16(100));
        bytes.extend_from_slice(&[12, 36]);
        let err = Cff2TopDict::parse(&bytes).unwrap_err();
        match err {
            Error::Cff(s) => assert!(s.contains("off-diagonal/translation non-zero")),
            _ => panic!("unexpected: {err:?}"),
        }
    }

    #[test]
    fn rejects_negative_offset() {
        // 5-byte i32 with a negative value as the CharStrings offset.
        let mut bytes = Vec::new();
        bytes.extend(op_i32(-1));
        bytes.push(17);
        bytes.extend(op_i16(100));
        bytes.extend_from_slice(&[12, 36]);
        let err = Cff2TopDict::parse(&bytes).unwrap_err();
        match err {
            Error::Cff(s) => assert!(s.contains("negative offset")),
            _ => panic!("unexpected: {err:?}"),
        }
    }

    #[test]
    fn skips_unknown_operators() {
        // Add a stray CFF1 operator (e.g. op 5 FontBBox) — spec says it
        // shouldn't appear in CFF2 but we tolerate it.
        let mut bytes = Vec::new();
        bytes.extend(op_i16(200));
        bytes.push(17);
        bytes.extend(op_i16(100));
        bytes.extend_from_slice(&[12, 36]);
        // FontBBox-like garbage:
        bytes.extend(op_i16(0));
        bytes.extend(op_i16(0));
        bytes.extend(op_i16(0));
        bytes.extend(op_i16(0));
        bytes.push(5);
        let top = Cff2TopDict::parse(&bytes).expect("parse");
        assert_eq!(top.charstring_index_offset, 200);
        assert_eq!(top.font_dict_index_offset, 100);
    }
}
