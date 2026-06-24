//! Device and VariationIndex tables (OpenType *Common Table Formats*
//! chapter, "Device and VariationIndex tables").
//!
//! GPOS, GDEF, BASE, and JSTF reference a `Device` table (non-variable
//! fonts) or a `VariationIndex` table (variable fonts) wherever a
//! positioning value may need per-ppem or per-instance adjustment —
//! e.g. each of an `Anchor` format-3 `xDeviceOffset` / `yDeviceOffset`,
//! and each `ValueRecord` `*DeviceOffset`. Both tables share a leading
//! three-`uint16` shape; the trailing `deltaFormat` field disambiguates
//! them:
//!
//! ```text
//! Device table (deltaFormat 0x0001 / 0x0002 / 0x0003)
//!   uint16 startSize       // smallest ppem to correct
//!   uint16 endSize         // largest ppem to correct
//!   uint16 deltaFormat     // 1 = 2-bit, 2 = 4-bit, 3 = 8-bit deltas
//!   uint16 deltaValue[]    // packed signed deltas, MSB-first
//!
//! VariationIndex table (deltaFormat 0x8000)
//!   uint16 deltaSetOuterIndex   // selects an ItemVariationData subtable
//!   uint16 deltaSetInnerIndex   // selects a delta-set row within it
//!   uint16 deltaFormat          // = 0x8000
//! ```
//!
//! For a Device table, `delta(ppem)` returns the signed pixel
//! adjustment for a given ppem size (or `0` outside `[startSize,
//! endSize]`). The `deltaValue[]` array packs one signed value per
//! ppem size in `[startSize, endSize]` at 2 / 4 / 8 bits each,
//! most-significant bits first, into `uint16` words (spec's worked
//! example: 4-bit `{1, 2, 3, -1}` → `0x123F`).
//!
//! For a VariationIndex table, `(outer, inner)` is the delta-set index
//! pair into the GDEF/BASE `ItemVariationStore`; resolving it to a
//! delta requires the store + an instance's region scalars, which is
//! the caller's responsibility (this crate surfaces the index pair).
//!
//! Spec: `docs/text/opentype/otspec-chapter2-common-layout-tables.html`.

use crate::parser::read_u16;
use crate::Error;

/// `deltaFormat` discriminant: 2-bit signed deltas (8 per `uint16`).
const DELTA_FORMAT_LOCAL_2_BIT: u16 = 0x0001;
/// `deltaFormat` discriminant: 4-bit signed deltas (4 per `uint16`).
const DELTA_FORMAT_LOCAL_4_BIT: u16 = 0x0002;
/// `deltaFormat` discriminant: 8-bit signed deltas (2 per `uint16`).
const DELTA_FORMAT_LOCAL_8_BIT: u16 = 0x0003;
/// `deltaFormat` discriminant: VariationIndex table (delta-set index).
const DELTA_FORMAT_VARIATION_INDEX: u16 = 0x8000;

/// A decoded Device or VariationIndex table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeviceOrVariationIndex<'a> {
    /// A non-variable-font `Device` table: per-ppem pixel corrections.
    Device(DeviceTable<'a>),
    /// A variable-font `VariationIndex` table: a delta-set index pair
    /// into the GDEF/BASE `ItemVariationStore`.
    VariationIndex(VariationIndexTable),
}

impl<'a> DeviceOrVariationIndex<'a> {
    /// Parse a Device / VariationIndex table from a buffer whose first
    /// byte is the table's `startSize` (Device) or `deltaSetOuterIndex`
    /// (VariationIndex) field. The `deltaFormat` field (third `uint16`)
    /// selects the interpretation.
    pub fn parse(bytes: &'a [u8]) -> Result<Self, Error> {
        let delta_format = read_u16(bytes, 4)?;
        match delta_format {
            DELTA_FORMAT_VARIATION_INDEX => {
                Ok(Self::VariationIndex(VariationIndexTable::parse(bytes)?))
            }
            DELTA_FORMAT_LOCAL_2_BIT | DELTA_FORMAT_LOCAL_4_BIT | DELTA_FORMAT_LOCAL_8_BIT => {
                Ok(Self::Device(DeviceTable::parse(bytes)?))
            }
            _ => Err(Error::BadStructure("Device: unknown deltaFormat")),
        }
    }

    /// Borrow the inner Device table, or `None` for a VariationIndex.
    pub fn as_device(&self) -> Option<&DeviceTable<'a>> {
        match self {
            Self::Device(d) => Some(d),
            Self::VariationIndex(_) => None,
        }
    }

    /// Borrow the inner VariationIndex table, or `None` for a Device.
    pub fn as_variation_index(&self) -> Option<&VariationIndexTable> {
        match self {
            Self::VariationIndex(v) => Some(v),
            Self::Device(_) => None,
        }
    }
}

/// A decoded `Device` table: signed per-ppem pixel corrections over
/// `[start_size, end_size]`, packed at 2 / 4 / 8 bits per value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceTable<'a> {
    start_size: u16,
    end_size: u16,
    /// Bits per packed delta (2, 4, or 8), derived from `deltaFormat`.
    bits: u8,
    /// `deltaValue[]` payload (the packed `uint16` words).
    delta_values: &'a [u8],
}

impl<'a> DeviceTable<'a> {
    /// Parse a Device table (`deltaFormat` 1 / 2 / 3).
    pub fn parse(bytes: &'a [u8]) -> Result<Self, Error> {
        let start_size = read_u16(bytes, 0)?;
        let end_size = read_u16(bytes, 2)?;
        let delta_format = read_u16(bytes, 4)?;
        if end_size < start_size {
            return Err(Error::BadStructure("Device: endSize < startSize"));
        }
        let bits = match delta_format {
            DELTA_FORMAT_LOCAL_2_BIT => 2u8,
            DELTA_FORMAT_LOCAL_4_BIT => 4,
            DELTA_FORMAT_LOCAL_8_BIT => 8,
            _ => return Err(Error::BadStructure("Device: not a Device deltaFormat")),
        };
        // Number of delta values = one per ppem in [startSize, endSize].
        let count = (end_size - start_size) as usize + 1;
        let total_bits = count * bits as usize;
        let words = total_bits.div_ceil(16);
        let need = words * 2;
        let delta_values = bytes.get(6..6 + need).ok_or(Error::UnexpectedEof)?;
        Ok(Self {
            start_size,
            end_size,
            bits,
            delta_values,
        })
    }

    /// Smallest ppem size that carries a correction.
    pub fn start_size(&self) -> u16 {
        self.start_size
    }

    /// Largest ppem size that carries a correction.
    pub fn end_size(&self) -> u16 {
        self.end_size
    }

    /// Bits per packed delta (2, 4, or 8).
    pub fn delta_bits(&self) -> u8 {
        self.bits
    }

    /// The signed pixel correction for `ppem`. Returns `0` for any
    /// `ppem` outside `[start_size, end_size]` (no correction defined).
    pub fn delta(&self, ppem: u16) -> i32 {
        if ppem < self.start_size || ppem > self.end_size {
            return 0;
        }
        let index = (ppem - self.start_size) as usize;
        self.delta_at(index)
    }

    /// The signed delta at array position `index` (0 = `start_size`).
    /// The 2/4/8-bit values are packed MSB-first into the `uint16`
    /// words: value `index` occupies bit positions
    /// `[index*bits .. index*bits + bits)` counting from the MSB of the
    /// word stream.
    fn delta_at(&self, index: usize) -> i32 {
        let bits = self.bits as usize;
        let bit_pos = index * bits;
        let word_i = bit_pos / 16;
        let word_byte = word_i * 2;
        // Defensive: out-of-range index → 0 (parse already bounds the
        // backing slice, but a caller could pass an arbitrary index).
        if word_byte + 1 >= self.delta_values.len() {
            return 0;
        }
        let word = u16::from_be_bytes([
            self.delta_values[word_byte],
            self.delta_values[word_byte + 1],
        ]);
        // Bit offset of this value's MSB from the top of the word.
        let shift_from_top = bit_pos % 16;
        // Move the value's bits down to the low end of a u16.
        let right_shift = 16 - shift_from_top - bits;
        let raw = (word >> right_shift) & ((1u16 << bits) - 1);
        sign_extend(raw, bits)
    }

    /// Iterate `(ppem, delta)` pairs for every size in
    /// `[start_size, end_size]`.
    pub fn iter(&self) -> impl Iterator<Item = (u16, i32)> + '_ {
        (self.start_size..=self.end_size).map(move |ppem| (ppem, self.delta(ppem)))
    }
}

/// A decoded `VariationIndex` table (`deltaFormat 0x8000`): a delta-set
/// index pair `(outer, inner)` into the GDEF/BASE `ItemVariationStore`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VariationIndexTable {
    /// `deltaSetOuterIndex` — selects an `ItemVariationData` subtable.
    pub outer_index: u16,
    /// `deltaSetInnerIndex` — selects a delta-set row within it.
    pub inner_index: u16,
}

impl VariationIndexTable {
    /// Parse a VariationIndex table (`deltaFormat` must be `0x8000`).
    pub fn parse(bytes: &[u8]) -> Result<Self, Error> {
        let outer_index = read_u16(bytes, 0)?;
        let inner_index = read_u16(bytes, 2)?;
        let delta_format = read_u16(bytes, 4)?;
        if delta_format != DELTA_FORMAT_VARIATION_INDEX {
            return Err(Error::BadStructure(
                "VariationIndex: deltaFormat is not 0x8000",
            ));
        }
        Ok(Self {
            outer_index,
            inner_index,
        })
    }
}

/// Sign-extend the low `bits` bits of `raw` to a signed `i32`.
#[inline]
fn sign_extend(raw: u16, bits: usize) -> i32 {
    let shift = 32 - bits;
    ((raw as i32) << shift) >> shift
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_variation_index() {
        // outer = 3, inner = 7, deltaFormat = 0x8000.
        let bytes = [0x00, 0x03, 0x00, 0x07, 0x80, 0x00];
        let v = DeviceOrVariationIndex::parse(&bytes).unwrap();
        let vi = v.as_variation_index().unwrap();
        assert_eq!(vi.outer_index, 3);
        assert_eq!(vi.inner_index, 7);
        assert!(v.as_device().is_none());
    }

    #[test]
    fn device_4bit_spec_example() {
        // Spec worked example: 4-bit (deltaFormat 2) values
        // {1, 2, 3, -1} pack to deltaValue 0x123F.
        // startSize = 12, endSize = 15 (4 sizes).
        let bytes = [
            0x00, 0x0C, // startSize = 12
            0x00, 0x0F, // endSize = 15
            0x00, 0x02, // deltaFormat = 2 (4-bit)
            0x12, 0x3F, // deltaValue[0] = 0x123F
        ];
        let d = DeviceTable::parse(&bytes).unwrap();
        assert_eq!(d.start_size(), 12);
        assert_eq!(d.end_size(), 15);
        assert_eq!(d.delta_bits(), 4);
        assert_eq!(d.delta(12), 1);
        assert_eq!(d.delta(13), 2);
        assert_eq!(d.delta(14), 3);
        assert_eq!(d.delta(15), -1);
        // Out of range → 0.
        assert_eq!(d.delta(11), 0);
        assert_eq!(d.delta(16), 0);
        let collected: Vec<_> = d.iter().collect();
        assert_eq!(collected, vec![(12, 1), (13, 2), (14, 3), (15, -1)]);
    }

    #[test]
    fn device_2bit_values() {
        // 2-bit (deltaFormat 1): {-2, -1, 0, 1, 1, 0, -1, -2}, 8 values,
        // startSize = 9, endSize = 16. Pack MSB-first:
        //   -2=10, -1=11, 0=00, 1=01, 1=01, 0=00, -1=11, -2=10
        //   bits: 10 11 00 01 01 00 11 10 → 0b1011000101001110 = 0xB14E
        let bytes = [
            0x00, 0x09, // startSize = 9
            0x00, 0x10, // endSize = 16
            0x00, 0x01, // deltaFormat = 1 (2-bit)
            0xB1, 0x4E, // deltaValue[0]
        ];
        let d = DeviceTable::parse(&bytes).unwrap();
        let got: Vec<i32> = (9u16..=16).map(|p| d.delta(p)).collect();
        assert_eq!(got, vec![-2, -1, 0, 1, 1, 0, -1, -2]);
    }

    #[test]
    fn device_8bit_values() {
        // 8-bit (deltaFormat 3): {127, -128}, startSize=20, endSize=21.
        let bytes = [
            0x00, 0x14, // startSize = 20
            0x00, 0x15, // endSize = 21
            0x00, 0x03, // deltaFormat = 3 (8-bit)
            0x7F, 0x80, // 127, -128
        ];
        let d = DeviceTable::parse(&bytes).unwrap();
        assert_eq!(d.delta(20), 127);
        assert_eq!(d.delta(21), -128);
    }

    #[test]
    fn device_multiword_4bit() {
        // 4-bit, 5 values → 20 bits → 2 uint16 words.
        // values {1, 2, 3, 4, 5}: word0 = 0x1234, word1 = 0x5000.
        let bytes = [
            0x00, 0x0A, // startSize = 10
            0x00, 0x0E, // endSize = 14 (5 sizes)
            0x00, 0x02, // 4-bit
            0x12, 0x34, // {1,2,3,4}
            0x50, 0x00, // {5, pad pad pad}
        ];
        let d = DeviceTable::parse(&bytes).unwrap();
        let got: Vec<i32> = (10u16..=14).map(|p| d.delta(p)).collect();
        assert_eq!(got, vec![1, 2, 3, 4, 5]);
    }

    #[test]
    fn dispatch_picks_device_vs_variation() {
        let dev = [0x00, 0x0C, 0x00, 0x0C, 0x00, 0x03, 0x05, 0x00];
        assert!(DeviceOrVariationIndex::parse(&dev)
            .unwrap()
            .as_device()
            .is_some());
        let vi = [0x00, 0x01, 0x00, 0x02, 0x80, 0x00];
        assert!(DeviceOrVariationIndex::parse(&vi)
            .unwrap()
            .as_variation_index()
            .is_some());
    }

    #[test]
    fn rejects_unknown_delta_format() {
        let bytes = [0x00, 0x0C, 0x00, 0x0C, 0x00, 0x05, 0x00, 0x00];
        assert!(DeviceOrVariationIndex::parse(&bytes).is_err());
    }

    #[test]
    fn rejects_end_before_start() {
        let bytes = [0x00, 0x10, 0x00, 0x0C, 0x00, 0x02, 0x00, 0x00];
        assert!(DeviceTable::parse(&bytes).is_err());
    }

    #[test]
    fn rejects_truncated_delta_values() {
        // Claims 4 values at 8 bits (4 bytes) but only 1 word present.
        let bytes = [0x00, 0x0C, 0x00, 0x0F, 0x00, 0x03, 0x01];
        assert!(DeviceTable::parse(&bytes).is_err());
    }
}
