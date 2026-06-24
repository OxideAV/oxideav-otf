//! CFF2 FontDICT (OpenType 1.9.1 `CFF2` table, FontDICTINDEX / FontDICT).
//!
//! Each entry of the FontDICTINDEX is a FontDICT. In CFF2 a FontDICT
//! carries a single meaningful operator:
//!
//! - `Private` (`0x12`, dec 18, `[size offset]`) — the byte size and
//!   the offset (from the start of the CFF2 table) of the FontDICT's
//!   PrivateDICT. Spec "PrivateDICTOffset": *"The two operands are the
//!   size and offset of the corresponding PrivateDICT table. The
//!   offset is from the start of the CFF2 table."*
//!
//! A CFF2 table requires at least one FontDICT / PrivateDICT pair even
//! if the PrivateDICT is empty (`size == 0`). When multiple FontDICTs
//! are present a FontDICTSelect table routes each glyph to its
//! FontDICT (see [`crate::cff2::fdselect`]).
//!
//! Spec: `docs/text/opentype/otspec-cff2.html` (FontDICTINDEX,
//! FontDICTSelect and FontDICT).

use crate::cff::dict::{Dict, Operand};
use crate::Error;

/// FontDICT `Private` operator (dec 18 / `0x12`).
const OP_PRIVATE: u16 = 18;

/// The `(size, offset)` of a CFF2 FontDICT's PrivateDICT, both taken
/// from the FontDICT's `Private` operator. `offset` is from the start
/// of the CFF2 table. A FontDICT with no `Private` operator (or a
/// `size == 0`) denotes an empty PrivateDICT.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cff2FontDict {
    /// PrivateDICT byte size (the first `Private` operand). `0` for an
    /// empty PrivateDICT.
    pub private_size: usize,
    /// PrivateDICT offset from the start of the CFF2 table (the second
    /// `Private` operand).
    pub private_offset: usize,
}

impl Cff2FontDict {
    /// Parse a FontDICT from its raw INDEX entry bytes. A FontDICT that
    /// carries no `Private` operator is treated as referencing an empty
    /// PrivateDICT (`size = 0`, `offset = 0`), which the PrivateDICT
    /// parser accepts.
    pub fn parse(bytes: &[u8]) -> Result<Self, Error> {
        let dict = Dict::parse(bytes)?;
        let mut private_size = 0usize;
        let mut private_offset = 0usize;
        for (op, operands) in dict.iter() {
            if *op == OP_PRIVATE {
                let (size, offset) = take_private(operands)?;
                private_size = size;
                private_offset = offset;
            }
        }
        Ok(Self {
            private_size,
            private_offset,
        })
    }
}

/// Pull the `Private` operator's two operands `(size, offset)`. Both
/// must be non-negative integers.
fn take_private(operands: &[Operand]) -> Result<(usize, usize), Error> {
    if operands.len() < 2 {
        return Err(Error::Cff("CFF2 FontDICT Private needs (size, offset)"));
    }
    let size = operands[operands.len() - 2]
        .as_int()
        .ok_or(Error::Cff("CFF2 FontDICT Private size not an integer"))?;
    let offset = operands[operands.len() - 1]
        .as_int()
        .ok_or(Error::Cff("CFF2 FontDICT Private offset not an integer"))?;
    if size < 0 || offset < 0 {
        return Err(Error::Cff("CFF2 FontDICT Private negative size/offset"));
    }
    Ok((size as usize, offset as usize))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Encode a 0..=32767 value as a CFF DICT `28` operand.
    fn op_i16(v: i16) -> Vec<u8> {
        let b = v.to_be_bytes();
        vec![28, b[0], b[1]]
    }

    #[test]
    fn parses_private_operator() {
        // [size=40] [offset=300] [op 18].
        let mut dict = Vec::new();
        dict.extend(op_i16(40));
        dict.extend(op_i16(300));
        dict.push(18);
        let fd = Cff2FontDict::parse(&dict).expect("parse");
        assert_eq!(fd.private_size, 40);
        assert_eq!(fd.private_offset, 300);
    }

    #[test]
    fn font_dict_without_private_is_empty() {
        // Empty DICT → empty PrivateDICT.
        let fd = Cff2FontDict::parse(&[]).expect("parse");
        assert_eq!(fd.private_size, 0);
        assert_eq!(fd.private_offset, 0);
    }

    #[test]
    fn rejects_single_operand_private() {
        let mut dict = op_i16(40);
        dict.push(18);
        let err = Cff2FontDict::parse(&dict).unwrap_err();
        match err {
            Error::Cff(s) => assert!(s.contains("needs (size, offset)")),
            _ => panic!("unexpected: {err:?}"),
        }
    }

    #[test]
    fn rejects_negative_private() {
        let mut dict = vec![29];
        dict.extend_from_slice(&(-1i32).to_be_bytes()); // size = -1
        dict.extend(op_i16(0));
        dict.push(18);
        let err = Cff2FontDict::parse(&dict).unwrap_err();
        match err {
            Error::Cff(s) => assert!(s.contains("negative size/offset")),
            _ => panic!("unexpected: {err:?}"),
        }
    }
}
