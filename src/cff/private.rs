//! CFF Private DICT (Adobe TN5176 §15).
//!
//! The Private DICT carries hinting parameters (BlueValues, StdHW,
//! StdVW etc.) and — critically — the offset of the Local Subrs
//! INDEX and the `defaultWidthX` / `nominalWidthX` values used by
//! Type 2 charstrings to encode glyph advance width compactly.
//!
//! Round-1 scope: parse `defaultWidthX`, `nominalWidthX`, and the
//! local Subrs offset. Hint values are accepted-and-ignored.

use crate::Error;
use crate::cff::dict::{Dict, Operator};
use crate::cff::index::Index;

/// Parsed Private DICT, with its (optional) Local Subrs INDEX
/// resolved.
#[derive(Debug, Clone)]
pub(crate) struct PrivateDict<'a> {
    pub(crate) default_width_x: f32,
    pub(crate) nominal_width_x: f32,
    pub(crate) local_subrs: Option<Index<'a>>,
}

impl<'a> PrivateDict<'a> {
    /// Parse a Private DICT located at `[private_off, private_off +
    /// size)` within the CFF table bytes.
    ///
    /// The Local Subrs offset is *relative to the Private DICT
    /// start*, per TN5176 §15 (operator 19): "The local subrs offset
    /// is from the start of the Private DICT data".
    pub(crate) fn parse(
        bytes: &'a [u8],
        private_off: usize,
        size: usize,
    ) -> Result<Self, Error> {
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
}
