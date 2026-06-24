//! CFF2 Private DICT (OpenType 1.9.1 `CFF2` table, PrivateDICT
//! operator summary).
//!
//! Each CFF2 FontDICT names a PrivateDICT through its `Private`
//! operator (`0x12`, `[size offset]`, offset from the start of the
//! CFF2 table). The PrivateDICT carries hinting parameters plus the
//! two pieces the variation-aware charstring interpreter needs:
//!
//! - `LocalSubrINDEXOffset` (`0x13`, dec 19) — offset, **from the
//!   start of the PrivateDICT**, to a per-PrivateDICT LocalSubrINDEX.
//!   Absent when the PrivateDICT has no local subroutines.
//! - `vsindex` (`0x16`, dec 22) — the default ItemVariationData index
//!   (i.e. the default active region list, `k`) for every CharString
//!   associated with this PrivateDICT. Spec default `0`. A CharString
//!   may override it with the CharString-encoded `vsindex` (`0x0f`).
//!
//! The remaining PrivateDICT operators (`BlueValues`, `StdHW`, …) are
//! hinting metadata that the >= 16 px anti-aliased outline path does
//! not enforce; CFF2 also permits a `blend` (`0x17`) operator to make
//! them variable. This module records the two interpreter-relevant
//! operators and the LocalSubrINDEX they reference, deliberately
//! skipping the hint vocabulary (which `cff::PrivateHints` already
//! models for CFF1 and which the outline decoder ignores).
//!
//! Spec: `docs/text/opentype/otspec-cff2.html` (PrivateDICT operator
//! summary + "PrivateDICT subroutine operator" +
//! "PrivateDICT variation operators").

use crate::cff::dict::{Dict, Operand};
use crate::cff2::index::Cff2Index;
use crate::Error;

/// `LocalSubrINDEXOffset` PrivateDICT operator (dec 19 / `0x13`).
const OP_LOCAL_SUBR_INDEX_OFFSET: u16 = 19;
/// `vsindex` PrivateDICT operator (dec 22 / `0x16`). Note this is a
/// *different* encoding from the CharString `vsindex` (`0x0f`).
const OP_VSINDEX: u16 = 22;

/// A parsed CFF2 PrivateDICT — only the fields the charstring
/// interpreter consults. `local_subr_index_offset` is relative to the
/// start of the PrivateDICT (spec "LocalSubrINDEXOffset"); the parsed
/// LocalSubrINDEX itself is carried on [`Cff2Private`].
#[derive(Debug, Clone)]
pub struct Cff2Private<'a> {
    /// Default `vsindex` for CharStrings routed to this PrivateDICT.
    /// Spec default `0` when the operator is absent.
    pub vsindex: u16,
    /// LocalSubrINDEX for this PrivateDICT, or `None` when the
    /// PrivateDICT carries no `LocalSubrINDEXOffset` operator (i.e. no
    /// local subroutines).
    pub local_subrs: Option<Cff2Index<'a>>,
}

impl<'a> Cff2Private<'a> {
    /// Parse the PrivateDICT at `[offset .. offset + size)` within the
    /// CFF2 table `bytes`. `offset` and `size` are the two operands of
    /// the FontDICT `Private` operator (offset from the start of the
    /// CFF2 table). An empty PrivateDICT (`size == 0`) is valid (spec
    /// "When to use multiple PrivateDICTs": "even if the PrivateDICT is
    /// empty") and yields `vsindex = 0`, `local_subrs = None`.
    pub fn parse(bytes: &'a [u8], offset: usize, size: usize) -> Result<Self, Error> {
        let end = offset
            .checked_add(size)
            .ok_or(Error::Cff("CFF2 PrivateDICT extent overflow"))?;
        if end > bytes.len() {
            return Err(Error::UnexpectedEof);
        }
        let dict = Dict::parse(&bytes[offset..end])?;

        let mut vsindex: u16 = 0;
        let mut local_subr_rel: Option<usize> = None;

        for (op, operands) in dict.iter() {
            match *op {
                OP_VSINDEX => {
                    vsindex = take_u16(operands)?;
                }
                OP_LOCAL_SUBR_INDEX_OFFSET => {
                    local_subr_rel = Some(take_usize(operands)?);
                }
                // Hinting operators (BlueValues, StdHW, blend, …) are
                // metadata the anti-aliased outline path does not
                // enforce. Skip them.
                _ => {}
            }
        }

        // The LocalSubrINDEXOffset is relative to the start of the
        // PrivateDICT, not the CFF2 table (spec "LocalSubrINDEXOffset"
        // Description).
        let local_subrs = match local_subr_rel {
            Some(rel) => {
                let abs = offset
                    .checked_add(rel)
                    .ok_or(Error::Cff("CFF2 LocalSubrINDEXOffset overflow"))?;
                Some(Cff2Index::parse(bytes, abs)?)
            }
            None => None,
        };

        Ok(Self {
            vsindex,
            local_subrs,
        })
    }
}

/// Pull the last operand as a non-negative `u16` (for `vsindex`).
fn take_u16(operands: &[Operand]) -> Result<u16, Error> {
    let v = take_i32(operands)?;
    if !(0..=u16::MAX as i32).contains(&v) {
        return Err(Error::Cff("CFF2 PrivateDICT vsindex out of range"));
    }
    Ok(v as u16)
}

/// Pull the last operand as a non-negative `usize` (for an offset).
fn take_usize(operands: &[Operand]) -> Result<usize, Error> {
    let v = take_i32(operands)?;
    if v < 0 {
        return Err(Error::Cff("CFF2 PrivateDICT negative offset"));
    }
    Ok(v as usize)
}

/// Pull the last operand of a PrivateDICT entry as an `i32`.
fn take_i32(operands: &[Operand]) -> Result<i32, Error> {
    operands
        .last()
        .ok_or(Error::Cff("CFF2 PrivateDICT operator with no operand"))?
        .as_int()
        .ok_or(Error::Cff("CFF2 PrivateDICT non-integer operand"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Encode an unsigned value 0..=32767 as a CFF DICT `28` operand.
    fn op_i16(v: i16) -> Vec<u8> {
        let b = v.to_be_bytes();
        vec![28, b[0], b[1]]
    }

    /// Encode a small non-negative integer as a single DICT byte
    /// (32..246 → b0 - 139).
    fn op_small(v: u8) -> u8 {
        v + 139
    }

    #[test]
    fn empty_private_dict_defaults() {
        // size = 0 → no operators; vsindex defaults to 0, no subrs.
        let bytes = vec![0u8; 4];
        let p = Cff2Private::parse(&bytes, 0, 0).expect("parse");
        assert_eq!(p.vsindex, 0);
        assert!(p.local_subrs.is_none());
    }

    #[test]
    fn parses_vsindex() {
        // PrivateDICT bytes: [operand 3] [op 22].
        let dict = vec![op_small(3), 22]; // vsindex
        let p = Cff2Private::parse(&dict, 0, dict.len()).expect("parse");
        assert_eq!(p.vsindex, 3);
        assert!(p.local_subrs.is_none());
    }

    #[test]
    fn parses_local_subr_index() {
        // Build: PrivateDICT then a LocalSubrINDEX directly after it.
        // PrivateDICT carries LocalSubrINDEXOffset = (its own size).
        // DICT: [operand N] [op 19].
        let mut dict = Vec::new();
        dict.extend(op_i16(0)); // placeholder, patched below
        dict.push(19); // LocalSubrINDEXOffset
        let priv_size = dict.len();
        // Patch the operand to equal priv_size (offset from PrivateDICT
        // start to the LocalSubrINDEX that follows immediately).
        let off = priv_size as i16;
        let b = off.to_be_bytes();
        dict[1] = b[0];
        dict[2] = b[1];

        // LocalSubrINDEX: a single 1-byte subroutine "S".
        // CFF2 INDEX: count=1 (u32), offSize=1, offsets=[1,2], data="S".
        let mut local = Vec::new();
        local.extend_from_slice(&[0, 0, 0, 1]); // count
        local.push(1); // offSize
        local.extend_from_slice(&[1, 2]); // offsets
        local.push(b'S');

        let mut whole = dict.clone();
        whole.extend_from_slice(&local);

        let p = Cff2Private::parse(&whole, 0, priv_size).expect("parse");
        assert_eq!(p.vsindex, 0);
        let subrs = p.local_subrs.expect("local subrs");
        assert_eq!(subrs.count, 1);
        assert_eq!(subrs.entry(0).unwrap(), b"S");
    }

    #[test]
    fn rejects_extent_past_eof() {
        let bytes = vec![0u8; 4];
        let err = Cff2Private::parse(&bytes, 2, 10).unwrap_err();
        assert!(matches!(err, Error::UnexpectedEof));
    }

    #[test]
    fn rejects_negative_local_subr_offset() {
        // operand -1 (5-byte 29 form) then op 19.
        let mut dict = vec![29];
        dict.extend_from_slice(&(-1i32).to_be_bytes());
        dict.push(19);
        let n = dict.len();
        let err = Cff2Private::parse(&dict, 0, n).unwrap_err();
        match err {
            Error::Cff(s) => assert!(s.contains("negative offset")),
            _ => panic!("unexpected: {err:?}"),
        }
    }
}
