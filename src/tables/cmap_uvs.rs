//! `cmap` subtable format 14 — Unicode Variation Sequences (UVS).
//!
//! A variation sequence is a base character followed by a Unicode
//! variation selector (e.g. `<U+82A6, U+E0101>`). Format 14 partitions
//! the sequences a font supports into:
//!
//! - **default** UVSes — the glyph to use is the one the base cmap
//!   subtable already maps the *base character* to (the selector does
//!   not change the glyph), and
//! - **non-default** UVSes — the format-14 subtable itself names a
//!   specific glyph for the `(base, selector)` pair.
//!
//! On-disk layout (only valid under platform 0 / encoding 5):
//!
//! ```text
//! format 14 subtable
//!   uint16 format = 14
//!   uint32 length
//!   uint32 numVarSelectorRecords
//!   VariationSelector varSelector[numVarSelectorRecords]
//!
//! VariationSelector
//!   uint24  varSelector
//!   Offset32 defaultUVSOffset      // from subtable start; may be 0
//!   Offset32 nonDefaultUVSOffset   // from subtable start; may be 0
//!
//! DefaultUVS table
//!   uint32 numUnicodeValueRanges
//!   UnicodeRange { uint24 startUnicodeValue; uint8 additionalCount }[]
//!
//! NonDefaultUVS table
//!   uint32 numUVSMappings
//!   UVSMapping { uint24 unicodeValue; uint16 glyphID }[]
//! ```
//!
//! `varSelector[]`, the DefaultUVS ranges, and the NonDefaultUVS
//! mappings are each sorted ascending, so all three are binary-searched.
//!
//! Spec: `docs/text/opentype/otspec-cmap.html` (Format 14: Unicode
//! variation sequences).

use crate::parser::{read_u16, read_u32, read_u8};
use crate::Error;

/// The outcome of a UVS lookup for a `(base, variation_selector)` pair.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UvsMapping {
    /// The pair is a **default** UVS: the caller should use whatever
    /// glyph the base cmap subtable maps `base` to (the selector is a
    /// no-op for this base).
    UseDefault,
    /// The pair is a **non-default** UVS mapped to this explicit glyph.
    Glyph(u16),
    /// The font defines no mapping for this `(base, selector)` pair.
    NotFound,
}

/// A parsed `cmap` format-14 (Unicode Variation Sequences) subtable.
#[derive(Debug, Clone)]
pub struct CmapUvs<'a> {
    /// The whole format-14 subtable; every offset is relative to byte 0.
    bytes: &'a [u8],
    /// Number of `VariationSelector` records.
    num_records: u32,
}

impl<'a> CmapUvs<'a> {
    /// Parse a format-14 subtable from a buffer whose first two bytes
    /// are the `format` identifier (which must be `14`).
    pub fn parse(bytes: &'a [u8]) -> Result<Self, Error> {
        let format = read_u16(bytes, 0)?;
        if format != 14 {
            return Err(Error::BadStructure("cmap/UVS: format is not 14"));
        }
        let num_records = read_u32(bytes, 6)?;
        // Each VariationSelector record is 11 bytes (uint24 + 2*uint32),
        // starting at offset 10.
        let need = 10usize
            .checked_add(num_records as usize * 11)
            .ok_or(Error::BadStructure("cmap/UVS: record array overflow"))?;
        if need > bytes.len() {
            return Err(Error::UnexpectedEof);
        }
        Ok(Self { bytes, num_records })
    }

    /// Number of variation-selector records.
    pub fn variation_selector_count(&self) -> u32 {
        self.num_records
    }

    /// The `varSelector` value of record `i` (a Unicode variation
    /// selector code point). `None` if `i` is out of range.
    pub fn variation_selector(&self, i: u32) -> Option<u32> {
        if i >= self.num_records {
            return None;
        }
        let off = 10 + i as usize * 11;
        read_u24(self.bytes, off).ok()
    }

    /// Look up the glyph mapping for the variation sequence
    /// `(base, variation_selector)`.
    ///
    /// Returns:
    /// * [`UvsMapping::Glyph`] — a non-default UVS with an explicit
    ///   glyph (the caller uses this glyph).
    /// * [`UvsMapping::UseDefault`] — a default UVS (the caller uses the
    ///   glyph the base cmap subtable maps `base` to).
    /// * [`UvsMapping::NotFound`] — the font defines no mapping for this
    ///   sequence (the caller falls back to the base glyph, treating the
    ///   selector as unsupported).
    pub fn lookup(&self, base: u32, variation_selector: u32) -> UvsMapping {
        let rec_off = match self.find_selector_record(variation_selector) {
            Some(off) => off,
            None => return UvsMapping::NotFound,
        };
        let default_off = read_u32(self.bytes, rec_off + 3).unwrap_or(0) as usize;
        let non_default_off = read_u32(self.bytes, rec_off + 7).unwrap_or(0) as usize;

        // Non-default UVS table wins (it names an explicit glyph).
        if non_default_off != 0 {
            if let Some(g) = self.lookup_non_default(non_default_off, base) {
                return UvsMapping::Glyph(g);
            }
        }
        if default_off != 0 && self.is_default_uvs(default_off, base) {
            return UvsMapping::UseDefault;
        }
        UvsMapping::NotFound
    }

    /// Byte offset of the `VariationSelector` record whose `varSelector`
    /// equals `selector`, via binary search (records sorted ascending).
    fn find_selector_record(&self, selector: u32) -> Option<usize> {
        let mut lo = 0u32;
        let mut hi = self.num_records;
        while lo < hi {
            let mid = (lo + hi) / 2;
            let off = 10 + mid as usize * 11;
            let v = read_u24(self.bytes, off).ok()?;
            match v.cmp(&selector) {
                core::cmp::Ordering::Less => lo = mid + 1,
                core::cmp::Ordering::Greater => hi = mid,
                core::cmp::Ordering::Equal => return Some(off),
            }
        }
        None
    }

    /// `true` if `base` is listed in the DefaultUVS table at `off`
    /// (range-compressed, sorted) — binary-search the ranges.
    fn is_default_uvs(&self, off: usize, base: u32) -> bool {
        let count = match read_u32(self.bytes, off) {
            Ok(c) => c as usize,
            Err(_) => return false,
        };
        let ranges_at = off + 4;
        // Each UnicodeRange is 4 bytes (uint24 start + uint8 additional).
        let mut lo = 0usize;
        let mut hi = count;
        while lo < hi {
            let mid = (lo + hi) / 2;
            let r = ranges_at + mid * 4;
            let start = match read_u24(self.bytes, r) {
                Ok(s) => s,
                Err(_) => return false,
            };
            let additional = match read_u8(self.bytes, r + 3) {
                Ok(a) => a as u32,
                Err(_) => return false,
            };
            let end = start + additional;
            if base < start {
                hi = mid;
            } else if base > end {
                lo = mid + 1;
            } else {
                return true;
            }
        }
        false
    }

    /// Glyph for `base` in the NonDefaultUVS table at `off` (sorted UVS
    /// mappings) — binary-search the mappings.
    fn lookup_non_default(&self, off: usize, base: u32) -> Option<u16> {
        let count = read_u32(self.bytes, off).ok()? as usize;
        let maps_at = off + 4;
        // Each UVSMapping is 5 bytes (uint24 unicodeValue + uint16 glyph).
        let mut lo = 0usize;
        let mut hi = count;
        while lo < hi {
            let mid = (lo + hi) / 2;
            let m = maps_at + mid * 5;
            let uni = read_u24(self.bytes, m).ok()?;
            match uni.cmp(&base) {
                core::cmp::Ordering::Less => lo = mid + 1,
                core::cmp::Ordering::Greater => hi = mid,
                core::cmp::Ordering::Equal => return read_u16(self.bytes, m + 3).ok(),
            }
        }
        None
    }
}

/// Read a big-endian 24-bit unsigned value from `bytes[off..off+3]`.
fn read_u24(bytes: &[u8], off: usize) -> Result<u32, Error> {
    let s = bytes.get(off..off + 3).ok_or(Error::UnexpectedEof)?;
    Ok(((s[0] as u32) << 16) | ((s[1] as u32) << 8) | s[2] as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Push a big-endian uint24.
    fn push_u24(v: u32, out: &mut Vec<u8>) {
        out.push((v >> 16) as u8);
        out.push((v >> 8) as u8);
        out.push(v as u8);
    }

    /// Build a format-14 subtable with one variation selector
    /// (U+E0100) whose default UVS ranges are {U+4E00..=U+4E02} and
    /// whose non-default mappings are {U+0041 → glyph 5, U+0042 → 9}.
    fn build_uvs() -> Vec<u8> {
        // Layout: header (10) + 1 record (11) = 21 bytes, then the two
        // tables appended; patch the offsets after.
        let mut v = Vec::new();
        v.extend_from_slice(&14u16.to_be_bytes()); // format
        v.extend_from_slice(&0u32.to_be_bytes()); // length (patched)
        v.extend_from_slice(&1u32.to_be_bytes()); // numVarSelectorRecords
                                                  // VariationSelector record:
        push_u24(0xE0100, &mut v); // varSelector
        let def_off_at = v.len();
        v.extend_from_slice(&0u32.to_be_bytes()); // defaultUVSOffset (patched)
        let ndef_off_at = v.len();
        v.extend_from_slice(&0u32.to_be_bytes()); // nonDefaultUVSOffset (patched)

        // DefaultUVS table @ here.
        let def_off = v.len() as u32;
        v.extend_from_slice(&1u32.to_be_bytes()); // numUnicodeValueRanges
        push_u24(0x4E00, &mut v); // startUnicodeValue
        v.push(2); // additionalCount → covers 0x4E00..=0x4E02

        // NonDefaultUVS table @ here.
        let ndef_off = v.len() as u32;
        v.extend_from_slice(&2u32.to_be_bytes()); // numUVSMappings
        push_u24(0x0041, &mut v); // unicodeValue
        v.extend_from_slice(&5u16.to_be_bytes()); // glyphID
        push_u24(0x0042, &mut v);
        v.extend_from_slice(&9u16.to_be_bytes());

        // Patch length + offsets.
        let len = v.len() as u32;
        v[2..6].copy_from_slice(&len.to_be_bytes());
        v[def_off_at..def_off_at + 4].copy_from_slice(&def_off.to_be_bytes());
        v[ndef_off_at..ndef_off_at + 4].copy_from_slice(&ndef_off.to_be_bytes());
        v
    }

    #[test]
    fn parses_and_lists_selector() {
        let raw = build_uvs();
        let uvs = CmapUvs::parse(&raw).unwrap();
        assert_eq!(uvs.variation_selector_count(), 1);
        assert_eq!(uvs.variation_selector(0), Some(0xE0100));
        assert_eq!(uvs.variation_selector(1), None);
    }

    #[test]
    fn non_default_uvs_maps_glyph() {
        let raw = build_uvs();
        let uvs = CmapUvs::parse(&raw).unwrap();
        assert_eq!(uvs.lookup(0x0041, 0xE0100), UvsMapping::Glyph(5));
        assert_eq!(uvs.lookup(0x0042, 0xE0100), UvsMapping::Glyph(9));
    }

    #[test]
    fn default_uvs_uses_base_glyph() {
        let raw = build_uvs();
        let uvs = CmapUvs::parse(&raw).unwrap();
        // Every base in the default range → UseDefault.
        assert_eq!(uvs.lookup(0x4E00, 0xE0100), UvsMapping::UseDefault);
        assert_eq!(uvs.lookup(0x4E01, 0xE0100), UvsMapping::UseDefault);
        assert_eq!(uvs.lookup(0x4E02, 0xE0100), UvsMapping::UseDefault);
    }

    #[test]
    fn unknown_pairs_are_not_found() {
        let raw = build_uvs();
        let uvs = CmapUvs::parse(&raw).unwrap();
        // Base outside both tables.
        assert_eq!(uvs.lookup(0x9999, 0xE0100), UvsMapping::NotFound);
        // Unknown variation selector.
        assert_eq!(uvs.lookup(0x0041, 0xE0101), UvsMapping::NotFound);
        // Just past the default range.
        assert_eq!(uvs.lookup(0x4E03, 0xE0100), UvsMapping::NotFound);
    }

    #[test]
    fn rejects_wrong_format() {
        let mut raw = build_uvs();
        raw[0..2].copy_from_slice(&12u16.to_be_bytes());
        assert!(CmapUvs::parse(&raw).is_err());
    }

    #[test]
    fn rejects_truncated_records() {
        let mut raw = build_uvs();
        // Claim 100 records.
        raw[6..10].copy_from_slice(&100u32.to_be_bytes());
        assert!(CmapUvs::parse(&raw).is_err());
    }
}
