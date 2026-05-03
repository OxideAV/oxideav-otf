//! CFF DICT — operator/operand stream (Adobe TN5176 §4).
//!
//! A DICT is a stream of (operands*, operator) tuples. Operators
//! are 1- or 2-byte tokens; operands are integers (multiple-byte
//! encodings) or real numbers (a single-prefix BCD encoding).
//!
//! Operand encodings (TN5176 Table 3):
//! - byte b0 = 32..246           → integer  (b0 - 139)            // 1 byte
//! - byte b0 = 247..250          → integer  ((b0-247)*256 + b1 + 108)  // 2 bytes
//! - byte b0 = 251..254          → integer  (-((b0-251)*256) - b1 - 108) // 2 bytes
//! - byte b0 = 28                → integer  i16 from b1..b2       // 3 bytes
//! - byte b0 = 29                → integer  i32 from b1..b4       // 5 bytes
//! - byte b0 = 30                → real  (BCD; nibble stream;
//!                                        terminator 0xf)
//!
//! Operators (TN5176 Table 9 / Table 23 / Table 24):
//! Single-byte 0..21, escape 12 then 0..38 for the two-byte operators.
//!
//! Round-1 scope: every Top DICT operator we actually consult, plus
//! all the Private DICT operators that affect charstring decoding
//! (default/nominalWidth, subroutine offset, defaultWidthX,
//! nominalWidthX). Unknown operators are tolerated — we record the
//! raw operands and move on, so an unrecognised optional operator
//! doesn't crash a valid font.

use crate::Error;

/// A single DICT operand value.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Operand {
    Int(i32),
    /// CFF "real number" — we keep the parsed `f64` plus skip BCD
    /// digit precision worries. Real operands are rare in Top DICT
    /// (FontMatrix, italicAngle, BlueScale) and for round 1 we only
    /// need them as f32-ish.
    Real(f64),
}

impl Operand {
    pub(crate) fn as_int(self) -> Option<i32> {
        match self {
            Self::Int(n) => Some(n),
            Self::Real(_) => None,
        }
    }

    #[allow(dead_code)]
    pub(crate) fn as_f64(self) -> f64 {
        match self {
            Self::Int(n) => n as f64,
            Self::Real(r) => r,
        }
    }
}

/// CFF DICT operator. Numeric values match TN5176; the two-byte
/// (escape) operators are stored as `0x0C00 | sub`.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u16)]
pub(crate) enum Operator {
    // --- Top DICT (TN5176 Table 9) ---
    Version = 0,
    Notice = 1,
    FullName = 2,
    FamilyName = 3,
    Weight = 4,
    FontBBox = 5,
    UniqueID = 13,
    XUID = 14,
    Charset = 15,
    Encoding = 16,
    CharStrings = 17,
    Private = 18,

    // --- Private DICT (TN5176 Table 23) ---
    BlueValues = 6,
    OtherBlues = 7,
    FamilyBlues = 8,
    FamilyOtherBlues = 9,
    StdHW = 10,
    StdVW = 11,

    // --- Two-byte operators (TN5176 Table 10) ---
    Copyright = 0x0C00,
    IsFixedPitch = 0x0C01,
    ItalicAngle = 0x0C02,
    UnderlinePosition = 0x0C03,
    UnderlineThickness = 0x0C04,
    PaintType = 0x0C05,
    CharstringType = 0x0C06,
    FontMatrix = 0x0C07,
    StrokeWidth = 0x0C08,

    // --- Two-byte Private (TN5176 Table 23 cont'd) ---
    BlueScale = 0x0C09,
    BlueShift = 0x0C0A,
    BlueFuzz = 0x0C0B,
    StemSnapH = 0x0C0C,
    StemSnapV = 0x0C0D,
    ForceBold = 0x0C0E,
    LanguageGroup = 0x0C11,
    ExpansionFactor = 0x0C12,
    InitialRandomSeed = 0x0C13,
    DefaultWidthX = 20,
    NominalWidthX = 21,
    Subrs = 19,

    // --- CIDFont (defer to round 2; we accept-and-ignore but list
    //     tags we may encounter) ---
    Ros = 0x0C1E,
    CidFontVersion = 0x0C1F,
    CidFontRevision = 0x0C20,
    CidFontType = 0x0C21,
    CidCount = 0x0C22,
    UidBase = 0x0C23,
    FdArray = 0x0C24,
    FdSelect = 0x0C25,
    FontName = 0x0C26,
}

/// A parsed CFF DICT — a map from operator → its operand list.
///
/// We don't use `HashMap` to keep the crate dependency-light; a
/// linear scan over <40 entries is fine.
#[derive(Debug, Default, Clone)]
pub(crate) struct Dict {
    entries: Vec<(u16, Vec<Operand>)>,
}

impl Dict {
    pub(crate) fn parse(bytes: &[u8]) -> Result<Self, Error> {
        let mut entries: Vec<(u16, Vec<Operand>)> = Vec::new();
        let mut operands: Vec<Operand> = Vec::new();
        let mut i = 0usize;
        while i < bytes.len() {
            let b0 = bytes[i];
            if b0 <= 21 {
                // Operator (1 or 2 bytes).
                let op = if b0 == 12 {
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
                let mut consumed = Vec::new();
                std::mem::swap(&mut consumed, &mut operands);
                entries.push((op, consumed));
            } else {
                // Operand.
                let (operand, consumed) = parse_operand(bytes, i)?;
                operands.push(operand);
                i += consumed;
            }
        }
        // Trailing operands without a closing operator are tolerated
        // silently — TN5176 §4 doesn't strictly forbid them, and we
        // would rather not reject borderline-malformed font data.
        Ok(Self { entries })
    }

    /// Look up the integer operand for a 1-operand operator.
    pub(crate) fn get_int(&self, op: Operator) -> Option<i32> {
        let want = op as u16;
        for (k, v) in &self.entries {
            if *k == want {
                return v.last().and_then(|o| o.as_int());
            }
        }
        None
    }

    /// Look up the full operand array for an operator (used for
    /// `Private` which is `[size, offset]` and `FontMatrix` /
    /// `BlueValues` which are arrays of N).
    pub(crate) fn get_array(&self, op: Operator) -> Option<&[Operand]> {
        let want = op as u16;
        for (k, v) in &self.entries {
            if *k == want {
                return Some(v);
            }
        }
        None
    }

    /// Iterate all parsed `(operator_code, operands)` pairs. Used by
    /// the higher-level `Cff::parse` for a subroutine-offset hand-roll.
    #[allow(dead_code)]
    pub(crate) fn iter(&self) -> impl Iterator<Item = &(u16, Vec<Operand>)> {
        self.entries.iter()
    }
}

/// Parse one operand starting at byte `i`. Returns the operand and
/// the number of bytes it consumed.
fn parse_operand(bytes: &[u8], i: usize) -> Result<(Operand, usize), Error> {
    let b0 = bytes[i];
    match b0 {
        // Integer (32..246) → b0 - 139.
        32..=246 => Ok((Operand::Int(b0 as i32 - 139), 1)),
        // Integer (247..250) → (b0 - 247) * 256 + b1 + 108.
        247..=250 => {
            if i + 1 >= bytes.len() {
                return Err(Error::UnexpectedEof);
            }
            let v = (b0 as i32 - 247) * 256 + bytes[i + 1] as i32 + 108;
            Ok((Operand::Int(v), 2))
        }
        // Integer (251..254) → -((b0 - 251) * 256) - b1 - 108.
        251..=254 => {
            if i + 1 >= bytes.len() {
                return Err(Error::UnexpectedEof);
            }
            let v = -((b0 as i32 - 251) * 256) - bytes[i + 1] as i32 - 108;
            Ok((Operand::Int(v), 2))
        }
        // 3-byte signed integer.
        28 => {
            if i + 2 >= bytes.len() {
                return Err(Error::UnexpectedEof);
            }
            let v = i16::from_be_bytes([bytes[i + 1], bytes[i + 2]]) as i32;
            Ok((Operand::Int(v), 3))
        }
        // 5-byte signed integer.
        29 => {
            if i + 4 >= bytes.len() {
                return Err(Error::UnexpectedEof);
            }
            let v = i32::from_be_bytes([bytes[i + 1], bytes[i + 2], bytes[i + 3], bytes[i + 4]]);
            Ok((Operand::Int(v), 5))
        }
        // BCD real.
        30 => parse_bcd(bytes, i),
        _ => Err(Error::Cff("invalid DICT operand byte")),
    }
}

/// Parse a CFF "real number" (TN5176 Table 5). The encoding is a
/// nibble stream starting at byte `i+1`:
///
/// ```text
/// 0..9     decimal digit
/// a (0xa)  decimal point
/// b        positive exponent E
/// c        negative exponent E-
/// d        reserved
/// e        minus sign
/// f        end of number
/// ```
///
/// Returns the parsed value + the number of bytes consumed (including
/// the leading 0x1e).
fn parse_bcd(bytes: &[u8], i: usize) -> Result<(Operand, usize), Error> {
    let mut s = String::with_capacity(16);
    let mut j = i + 1;
    loop {
        if j >= bytes.len() {
            return Err(Error::UnexpectedEof);
        }
        let b = bytes[j];
        let nibbles = [(b >> 4) & 0xf, b & 0xf];
        let mut done = false;
        for &n in &nibbles {
            match n {
                0..=9 => s.push((b'0' + n) as char),
                0xa => s.push('.'),
                0xb => s.push('E'),
                0xc => s.push_str("E-"),
                0xe => s.push('-'),
                0xf => {
                    done = true;
                    break;
                }
                0xd => return Err(Error::Cff("reserved BCD nibble (d)")),
                _ => unreachable!("masked to 4 bits"),
            }
        }
        j += 1;
        if done {
            break;
        }
        if j - i > 32 {
            // Bound runaway parsing on adversarial input. 32 bytes =
            // 64 nibbles which is way past any sensible real-number
            // representation.
            return Err(Error::Cff("BCD real too long"));
        }
    }
    let v: f64 = s.parse().map_err(|_| Error::Cff("malformed BCD real"))?;
    Ok((Operand::Real(v), j - i))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integer_ranges() {
        // 0 → encoded as 139.
        let d = Dict::parse(&[139, 0]).unwrap();
        assert_eq!(d.get_int(Operator::Version), Some(0));

        // 100 → encoded as 239.
        let d = Dict::parse(&[239, 0]).unwrap();
        assert_eq!(d.get_int(Operator::Version), Some(100));

        // -100 → encoded as 39.
        let d = Dict::parse(&[39, 0]).unwrap();
        assert_eq!(d.get_int(Operator::Version), Some(-100));

        // 1000 → 247, (1000-108)/256=3 → byte = 247+3=250?  Let's just
        // verify the decoder: (b0-247)*256 + b1 + 108. For 1000: pick
        // b0=250 → (250-247)*256 + b1 + 108 = 768 + b1 + 108. So b1 =
        // 1000 - 876 = 124.
        let d = Dict::parse(&[250, 124, 0]).unwrap();
        assert_eq!(d.get_int(Operator::Version), Some(1000));

        // -1000 → -((b0-251)*256) - b1 - 108. b0=254 → -768 - b1 - 108
        // = -876 - b1; for -1000 → b1 = 124.
        let d = Dict::parse(&[254, 124, 0]).unwrap();
        assert_eq!(d.get_int(Operator::Version), Some(-1000));

        // 30000 via opcode 28 (i16).
        let d = Dict::parse(&[28, 0x75, 0x30, 0]).unwrap();
        assert_eq!(d.get_int(Operator::Version), Some(0x7530));

        // 200000 via opcode 29 (i32).
        let d = Dict::parse(&[29, 0, 3, 0x0d, 0x40, 0]).unwrap();
        assert_eq!(d.get_int(Operator::Version), Some(200000));
    }

    #[test]
    fn private_array_pair() {
        // Private = [size, offset] = [42, 100] under operator 18.
        // 42 = 139+42 → byte 181. 100 = 139+100 → byte 239 (oops 239
        // already used; just compute: 100+139=239, 42+139=181).
        let d = Dict::parse(&[181, 239, 18]).unwrap();
        let arr = d.get_array(Operator::Private).expect("Private");
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[0].as_int(), Some(42));
        assert_eq!(arr[1].as_int(), Some(100));
    }

    #[test]
    fn two_byte_operator() {
        // ItalicAngle = -10 (real or int — TN5176 says number; we
        // pick int here). Op = 12 02.
        // -10 → 139 - 10 = 129.
        let d = Dict::parse(&[129, 12, 2]).unwrap();
        assert_eq!(d.get_int(Operator::ItalicAngle), Some(-10));
    }

    #[test]
    fn bcd_real_one_point_five() {
        // 1.5: nibbles 1, a, 5, f → 0x1a, 0x5f.
        let d = Dict::parse(&[30, 0x1a, 0x5f, 12, 2]).unwrap();
        let arr = d.get_array(Operator::ItalicAngle).unwrap();
        assert_eq!(arr.len(), 1);
        assert!(matches!(arr[0], Operand::Real(_)));
        let v = arr[0].as_f64();
        assert!((v - 1.5).abs() < 1e-9);
    }

    #[test]
    fn bcd_real_negative_exponent() {
        // -2.25e-3: -, 2, ., 2, 5, E-, 3, end
        // nibbles: e, 2, a, 2, 5, c, 3, f
        // bytes:   e2,  a2,  5c,  3f
        let d = Dict::parse(&[30, 0xe2, 0xa2, 0x5c, 0x3f, 12, 2]).unwrap();
        let arr = d.get_array(Operator::ItalicAngle).unwrap();
        let v = arr[0].as_f64();
        assert!((v - -0.00225).abs() < 1e-12, "got {v}");
    }
}
