//! `cmap` — character → glyph map.
//!
//! We pick a single subtable at parse time (preferred order: 32-bit
//! formats first, BMP formats second, legacy single-byte last) and
//! run all `lookup` calls through it. Supported formats: 0, 2 (legacy
//! high-byte CJK mapping), 4, 6, 12, and 13 (the "last resort"
//! many-to-one constant-glyph ranges; ranked below every real-coverage
//! format so it only wins when nothing better is present). The
//! format-14 Unicode Variation Sequences subtable is retained
//! alongside the chosen base subtable (see [`crate::tables::cmap_uvs`]).

use crate::parser::{read_u16, read_u32};
use crate::tables::cmap_uvs::CmapUvs;
use crate::Error;

#[derive(Debug, Clone)]
pub struct CmapTable<'a> {
    subtable: Subtable<'a>,
    /// Format-14 (Unicode Variation Sequences) subtable bytes, if the
    /// font carries one (platform 0 / encoding 5). Retained separately
    /// from the chosen base `subtable` because format 14 supplements —
    /// rather than replaces — the base cmap.
    uvs: Option<&'a [u8]>,
}

#[derive(Debug, Clone)]
enum Subtable<'a> {
    Format0(&'a [u8]),
    Format2(&'a [u8]),
    Format4(&'a [u8]),
    Format6(&'a [u8]),
    Format12(&'a [u8]),
    Format13(&'a [u8]),
}

impl<'a> CmapTable<'a> {
    pub fn parse(bytes: &'a [u8]) -> Result<Self, Error> {
        if bytes.len() < 4 {
            return Err(Error::UnexpectedEof);
        }
        let _version = read_u16(bytes, 0)?;
        let num_tables = read_u16(bytes, 2)?;
        let header_end = 4 + (num_tables as usize) * 8;
        if bytes.len() < header_end {
            return Err(Error::UnexpectedEof);
        }

        let mut best: Option<Subtable<'_>> = None;
        let mut best_rank = i32::MIN;
        let mut uvs: Option<&[u8]> = None;

        for i in 0..num_tables as usize {
            let off = 4 + i * 8;
            let platform_id = read_u16(bytes, off)?;
            let encoding_id = read_u16(bytes, off + 2)?;
            let sub_off = read_u32(bytes, off + 4)? as usize;
            if sub_off + 2 > bytes.len() {
                return Err(Error::BadOffset);
            }
            let format = read_u16(bytes, sub_off)?;
            let length = subtable_length(bytes, sub_off, format)?;
            let sub = bytes
                .get(sub_off..sub_off + length)
                .ok_or(Error::BadOffset)?;

            // Format 14 (Unicode Variation Sequences) supplements — it
            // does not replace — the base subtable, so it is retained
            // separately and never participates in the base-subtable
            // ranking.
            if format == 14 {
                uvs = Some(sub);
                continue;
            }

            let candidate = match format {
                0 => Some(Subtable::Format0(sub)),
                2 => Some(Subtable::Format2(sub)),
                4 => Some(Subtable::Format4(sub)),
                6 => Some(Subtable::Format6(sub)),
                12 => Some(Subtable::Format12(sub)),
                13 => Some(Subtable::Format13(sub)),
                _ => None,
            };
            if let Some(c) = candidate {
                let rank = subtable_rank(format, platform_id, encoding_id);
                if rank > best_rank {
                    best_rank = rank;
                    best = Some(c);
                }
            }
        }

        Ok(Self {
            subtable: best.ok_or(Error::UnsupportedCmapFormat(0xFFFF))?,
            uvs,
        })
    }

    /// Map a Unicode codepoint to a glyph id, or `None` if absent.
    pub fn lookup(&self, codepoint: u32) -> Option<u16> {
        match &self.subtable {
            Subtable::Format0(b) => lookup_format0(b, codepoint),
            Subtable::Format2(b) => lookup_format2(b, codepoint),
            Subtable::Format4(b) => lookup_format4(b, codepoint),
            Subtable::Format6(b) => lookup_format6(b, codepoint),
            Subtable::Format12(b) => lookup_format12(b, codepoint),
            Subtable::Format13(b) => lookup_format13(b, codepoint),
        }
    }

    /// The font's Unicode Variation Sequences (format-14) subtable, if
    /// present. Parsed lazily so the common no-UVS case costs nothing.
    /// Use [`CmapUvs::lookup`] with a base character + variation
    /// selector; combine its result with [`CmapTable::lookup`] for the
    /// default-UVS case.
    pub fn uvs(&self) -> Option<Result<CmapUvs<'a>, Error>> {
        self.uvs.map(CmapUvs::parse)
    }

    /// Convenience: resolve a variation sequence `(base,
    /// variation_selector)` to a glyph. A non-default UVS yields its
    /// explicit glyph; a default UVS resolves `base` through the base
    /// cmap [`CmapTable::lookup`]; an unsupported sequence yields
    /// `None`. Returns `None` when the font has no format-14 subtable.
    pub fn lookup_variation(&self, base: u32, variation_selector: u32) -> Option<u16> {
        let uvs = self.uvs()?.ok()?;
        match uvs.lookup(base, variation_selector) {
            crate::tables::cmap_uvs::UvsMapping::Glyph(g) => Some(g),
            crate::tables::cmap_uvs::UvsMapping::UseDefault => self.lookup(base),
            crate::tables::cmap_uvs::UvsMapping::NotFound => None,
        }
    }
}

fn subtable_length(bytes: &[u8], off: usize, format: u16) -> Result<usize, Error> {
    Ok(match format {
        0 | 2 | 4 | 6 => read_u16(bytes, off + 2)? as usize,
        8 | 10 | 12 | 13 => read_u32(bytes, off + 4)? as usize,
        // Format 14's length is a uint32 immediately after the uint16
        // format field (no reserved/language words in between).
        14 => read_u32(bytes, off + 2)? as usize,
        _ => return Err(Error::UnsupportedCmapFormat(format)),
    })
}

fn subtable_rank(format: u16, platform: u16, encoding: u16) -> i32 {
    let format_score = match format {
        12 => 400,
        4 => 300,
        6 => 200,
        0 => 100,
        // Format 2 (legacy high-byte CJK mapping) ranks just above the
        // single-byte format 0 but below the Unicode formats.
        2 => 150,
        // Format 13 is a "last resort" / fallback subtable (it maps wide
        // ranges to a single glyph), so it is ranked below every
        // real-coverage format — it only wins when nothing better
        // exists.
        13 => 50,
        _ => 0,
    };
    let platform_score = match (platform, encoding) {
        (0, _) => 30,
        (3, 10) => 25,
        (3, 1) => 20,
        _ => 5,
    };
    format_score + platform_score
}

fn lookup_format0(bytes: &[u8], codepoint: u32) -> Option<u16> {
    if codepoint > 0xFF {
        return None;
    }
    let glyph_array_off = 6;
    if bytes.len() < glyph_array_off + 256 {
        return None;
    }
    let g = bytes[glyph_array_off + codepoint as usize];
    if g == 0 {
        None
    } else {
        Some(g as u16)
    }
}

/// cmap subtable format 2 (high-byte mapping through table) — the
/// legacy mixed 8-/16-bit encoding for CJK code pages. Layout: a
/// `subHeaderKeys[256]` array (each value = `subHeader index * 8`) at
/// offset 6, then variable-length `SubHeader[]` and `glyphIdArray[]`.
///
/// A code point is split into a high byte and a low byte. `subHeader
/// 0` is special: it maps single-byte (high byte 0) characters. For a
/// 2-byte character, `subHeaderKeys[hi]` selects the SubHeader; the low
/// byte indexes into that SubHeader's `glyphIdArray` sub-array via
/// `firstCode` / `entryCount` / `idRangeOffset`, then `idDelta` is
/// applied (mod 65536) to a non-zero result.
fn lookup_format2(bytes: &[u8], codepoint: u32) -> Option<u16> {
    if codepoint > 0xFFFF {
        return None;
    }
    let hi = (codepoint >> 8) as usize & 0xFF;
    let lo = (codepoint & 0xFF) as u16;

    // subHeaderKeys[256] at offset 6 (each a uint16 = subHeader idx * 8).
    let sub_header_keys_off = 6usize;
    let key = read_u16(bytes, sub_header_keys_off + hi * 2).ok()?;
    // SubHeaders array begins right after the 512-byte keys array.
    let sub_headers_off = sub_header_keys_off + 256 * 2;
    let sub_header_off = sub_headers_off + key as usize;

    // For a single-byte char with high byte 0, the spec routes the
    // single byte through subHeader 0 as the "low byte". For a 2-byte
    // char the second (low) byte is mapped. We always treat `lo` as the
    // byte mapped through the selected SubHeader. When `key == 0`
    // (subHeader 0) and the char is single-byte, `hi == 0` already and
    // `lo` is the single byte — consistent with the spec.
    let first_code = read_u16(bytes, sub_header_off).ok()?;
    let entry_count = read_u16(bytes, sub_header_off + 2).ok()?;
    let id_delta = read_u16(bytes, sub_header_off + 4).ok()? as i16;
    let id_range_offset = read_u16(bytes, sub_header_off + 6).ok()?;

    // The mapped byte must fall in [firstCode, firstCode + entryCount).
    if lo < first_code || lo >= first_code.wrapping_add(entry_count) {
        return None;
    }
    let index_in_sub = (lo - first_code) as usize;

    // idRangeOffset is the byte distance from the idRangeOffset word
    // itself to the glyphIdArray element for firstCode.
    let id_range_offset_word = sub_header_off + 6;
    let glyph_off = id_range_offset_word
        .checked_add(id_range_offset as usize)?
        .checked_add(index_in_sub * 2)?;
    let raw = read_u16(bytes, glyph_off).ok()?;
    if raw == 0 {
        return None;
    }
    // idDelta is applied modulo 65536 to a non-zero glyph value.
    let g = (raw as i32 + id_delta as i32) & 0xFFFF;
    Some(g as u16)
}

fn lookup_format4(bytes: &[u8], codepoint: u32) -> Option<u16> {
    if codepoint > 0xFFFF {
        return None;
    }
    let cp = codepoint as u16;
    let seg_count_x2 = read_u16(bytes, 6).ok()? as usize;
    let seg_count = seg_count_x2 / 2;
    if seg_count == 0 {
        return None;
    }
    let end_code_off = 14usize;
    let reserved_pad = end_code_off + seg_count_x2;
    let start_code_off = reserved_pad + 2;
    let id_delta_off = start_code_off + seg_count_x2;
    let id_range_offset_off = id_delta_off + seg_count_x2;
    let glyph_id_array_off = id_range_offset_off + seg_count_x2;
    if bytes.len() < glyph_id_array_off {
        return None;
    }
    let mut seg = None;
    for i in 0..seg_count {
        let end = read_u16(bytes, end_code_off + i * 2).ok()?;
        if end >= cp {
            seg = Some(i);
            break;
        }
    }
    let seg = seg?;
    let start = read_u16(bytes, start_code_off + seg * 2).ok()?;
    if start > cp {
        return None;
    }
    let id_delta = read_u16(bytes, id_delta_off + seg * 2).ok()? as i32 as i16;
    let id_range_offset = read_u16(bytes, id_range_offset_off + seg * 2).ok()?;
    if id_range_offset == 0 {
        let g = (cp as i32 + id_delta as i32) & 0xFFFF;
        if g == 0 {
            return None;
        }
        return Some(g as u16);
    }
    let target = id_range_offset_off
        + seg * 2
        + id_range_offset as usize
        + 2 * (cp as usize - start as usize);
    let raw = read_u16(bytes, target).ok()?;
    if raw == 0 {
        return None;
    }
    let g = (raw as i32 + id_delta as i32) & 0xFFFF;
    Some(g as u16)
}

fn lookup_format6(bytes: &[u8], codepoint: u32) -> Option<u16> {
    if codepoint > 0xFFFF {
        return None;
    }
    let cp = codepoint as u16;
    let first_code = read_u16(bytes, 6).ok()?;
    let entry_count = read_u16(bytes, 8).ok()?;
    if cp < first_code {
        return None;
    }
    let idx = cp - first_code;
    if idx >= entry_count {
        return None;
    }
    let g = read_u16(bytes, 10 + idx as usize * 2).ok()?;
    if g == 0 {
        None
    } else {
        Some(g)
    }
}

fn lookup_format12(bytes: &[u8], codepoint: u32) -> Option<u16> {
    let num_groups = read_u32(bytes, 12).ok()? as usize;
    if 16 + num_groups * 12 > bytes.len() {
        return None;
    }
    let mut lo = 0usize;
    let mut hi = num_groups;
    while lo < hi {
        let mid = (lo + hi) / 2;
        let off = 16 + mid * 12;
        let start = read_u32(bytes, off).ok()?;
        let end = read_u32(bytes, off + 4).ok()?;
        if codepoint < start {
            hi = mid;
        } else if codepoint > end {
            lo = mid + 1;
        } else {
            let start_glyph = read_u32(bytes, off + 8).ok()?;
            let g = start_glyph.checked_add(codepoint - start)?;
            if g > u16::MAX as u32 {
                return None;
            }
            return Some(g as u16);
        }
    }
    None
}

/// cmap subtable format 13 (many-to-one range mappings). Identical
/// on-disk layout to format 12 except every codepoint in a
/// `ConstantMapGroup` `[startCharCode, endCharCode]` maps to the SAME
/// `glyphID` (not a sequential run). Groups are sorted by start code,
/// so we binary-search them.
fn lookup_format13(bytes: &[u8], codepoint: u32) -> Option<u16> {
    let num_groups = read_u32(bytes, 12).ok()? as usize;
    if 16 + num_groups * 12 > bytes.len() {
        return None;
    }
    let mut lo = 0usize;
    let mut hi = num_groups;
    while lo < hi {
        let mid = (lo + hi) / 2;
        let off = 16 + mid * 12;
        let start = read_u32(bytes, off).ok()?;
        let end = read_u32(bytes, off + 4).ok()?;
        if codepoint < start {
            hi = mid;
        } else if codepoint > end {
            lo = mid + 1;
        } else {
            let glyph = read_u32(bytes, off + 8).ok()?;
            if glyph == 0 || glyph > u16::MAX as u32 {
                return None;
            }
            return Some(glyph as u16);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build_cmap_with_subtable(format: u16, sub: &[u8]) -> Vec<u8> {
        let mut out = vec![0u8; 4 + 8];
        out[0..2].copy_from_slice(&0u16.to_be_bytes());
        out[2..4].copy_from_slice(&1u16.to_be_bytes());
        out[4..6].copy_from_slice(&3u16.to_be_bytes());
        let enc: u16 = if format == 12 { 10 } else { 1 };
        out[6..8].copy_from_slice(&enc.to_be_bytes());
        out[8..12].copy_from_slice(&12u32.to_be_bytes());
        out.extend_from_slice(sub);
        let _ = format;
        out
    }

    #[test]
    fn format4_round_trip() {
        let seg_count: u16 = 2;
        let seg_count_x2: u16 = seg_count * 2;
        let header = 14;
        let arrays_len = seg_count_x2 as usize * 4 + 2;
        let length = header + arrays_len;
        let mut sub = vec![0u8; length];
        sub[0..2].copy_from_slice(&4u16.to_be_bytes());
        sub[2..4].copy_from_slice(&(length as u16).to_be_bytes());
        sub[6..8].copy_from_slice(&seg_count_x2.to_be_bytes());
        sub[14..16].copy_from_slice(&67u16.to_be_bytes());
        sub[16..18].copy_from_slice(&0xFFFFu16.to_be_bytes());
        sub[18..20].copy_from_slice(&0u16.to_be_bytes());
        sub[20..22].copy_from_slice(&65u16.to_be_bytes());
        sub[22..24].copy_from_slice(&0xFFFFu16.to_be_bytes());
        sub[24..26].copy_from_slice(&35u16.to_be_bytes());
        sub[26..28].copy_from_slice(&1u16.to_be_bytes());

        let cmap_bytes = build_cmap_with_subtable(4, &sub);
        let cmap = CmapTable::parse(&cmap_bytes).unwrap();
        assert_eq!(cmap.lookup('A' as u32), Some(100));
        assert_eq!(cmap.lookup('B' as u32), Some(101));
        assert_eq!(cmap.lookup('C' as u32), Some(102));
        assert_eq!(cmap.lookup('D' as u32), None);
    }

    /// Build a cmap whose single subtable lives at offset 12, using the
    /// (platform 3, encoding 10) UCS-4 encoding so a format-12/13
    /// subtable ranks highest.
    fn build_cmap_ucs4(sub: &[u8]) -> Vec<u8> {
        let mut out = vec![0u8; 12];
        out[0..2].copy_from_slice(&0u16.to_be_bytes()); // version
        out[2..4].copy_from_slice(&1u16.to_be_bytes()); // numTables
        out[4..6].copy_from_slice(&3u16.to_be_bytes()); // platformID
        out[6..8].copy_from_slice(&10u16.to_be_bytes()); // encodingID (UCS-4)
        out[8..12].copy_from_slice(&12u32.to_be_bytes()); // subtableOffset
        out.extend_from_slice(sub);
        out
    }

    #[test]
    fn format13_constant_glyph_ranges() {
        // Two ConstantMapGroups: [0x4E00..=0x4FFF] → glyph 1,
        // [0x20000..=0x2A6DF] → glyph 2. Format 13 maps EVERY codepoint
        // in a group to the same glyph.
        let groups: [(u32, u32, u32); 2] = [(0x4E00, 0x4FFF, 1), (0x20000, 0x2A6DF, 2)];
        let length = 16 + groups.len() * 12;
        let mut sub = vec![0u8; length];
        sub[0..2].copy_from_slice(&13u16.to_be_bytes()); // format
        sub[4..8].copy_from_slice(&(length as u32).to_be_bytes()); // length
        sub[12..16].copy_from_slice(&(groups.len() as u32).to_be_bytes()); // numGroups
        for (i, (s, e, g)) in groups.iter().enumerate() {
            let off = 16 + i * 12;
            sub[off..off + 4].copy_from_slice(&s.to_be_bytes());
            sub[off + 4..off + 8].copy_from_slice(&e.to_be_bytes());
            sub[off + 8..off + 12].copy_from_slice(&g.to_be_bytes());
        }

        let cmap_bytes = build_cmap_ucs4(&sub);
        let cmap = CmapTable::parse(&cmap_bytes).unwrap();
        // Every codepoint in the first range → glyph 1 (constant).
        assert_eq!(cmap.lookup(0x4E00), Some(1));
        assert_eq!(cmap.lookup(0x4F00), Some(1));
        assert_eq!(cmap.lookup(0x4FFF), Some(1));
        // Second range → glyph 2.
        assert_eq!(cmap.lookup(0x20000), Some(2));
        assert_eq!(cmap.lookup(0x2A6DF), Some(2));
        // Gap between ranges and out-of-range → None.
        assert_eq!(cmap.lookup(0x5000), None);
        assert_eq!(cmap.lookup(0x10000), None);
    }

    #[test]
    fn format2_high_byte_mapping() {
        // Build a format-2 subtable with:
        //   - subHeader 0 (single-byte): firstCode 0x20, entryCount 2,
        //     idDelta 0, glyphIdArray {0x20→100, 0x21→101}.
        //   - high byte 0x81 → subHeader 1 (key = 8): firstCode 0x40,
        //     entryCount 2, idDelta 0, glyphIdArray {0x40→500, 0x41→501}.
        //
        // Layout:
        //   0/2   format = 2
        //   2/2   length
        //   4/2   language = 0
        //   6/512 subHeaderKeys[256] (all 0 except keys[0x81] = 8)
        //   518   SubHeader 0 (8 bytes)
        //   526   SubHeader 1 (8 bytes)
        //   534   glyphIdArray (single-byte sub-array, 2 u16)
        //   538   glyphIdArray (2-byte sub-array, 2 u16)
        let sub_headers_off = 6 + 512;
        let sh0_off = sub_headers_off; // 518
        let sh1_off = sh0_off + 8; // 526
        let gid_single_off = sh1_off + 8; // 534
        let gid_double_off = gid_single_off + 4; // 538
        let length = gid_double_off + 4;
        let mut sub = vec![0u8; length];
        sub[0..2].copy_from_slice(&2u16.to_be_bytes()); // format
        sub[2..4].copy_from_slice(&(length as u16).to_be_bytes()); // length
                                                                   // subHeaderKeys[0x81] = 8 (→ SubHeader 1).
        let key_off = 6 + 0x81 * 2;
        sub[key_off..key_off + 2].copy_from_slice(&8u16.to_be_bytes());

        // SubHeader 0: firstCode 0x20, entryCount 2, idDelta 0,
        // idRangeOffset points from the idRangeOffset word (sh0_off+6) to
        // gid_single_off.
        sub[sh0_off..sh0_off + 2].copy_from_slice(&0x20u16.to_be_bytes());
        sub[sh0_off + 2..sh0_off + 4].copy_from_slice(&2u16.to_be_bytes());
        sub[sh0_off + 4..sh0_off + 6].copy_from_slice(&0u16.to_be_bytes());
        let ro0 = (gid_single_off - (sh0_off + 6)) as u16;
        sub[sh0_off + 6..sh0_off + 8].copy_from_slice(&ro0.to_be_bytes());

        // SubHeader 1: firstCode 0x40, entryCount 2, idDelta 0.
        sub[sh1_off..sh1_off + 2].copy_from_slice(&0x40u16.to_be_bytes());
        sub[sh1_off + 2..sh1_off + 4].copy_from_slice(&2u16.to_be_bytes());
        sub[sh1_off + 4..sh1_off + 6].copy_from_slice(&0u16.to_be_bytes());
        let ro1 = (gid_double_off - (sh1_off + 6)) as u16;
        sub[sh1_off + 6..sh1_off + 8].copy_from_slice(&ro1.to_be_bytes());

        // glyphIdArrays.
        sub[gid_single_off..gid_single_off + 2].copy_from_slice(&100u16.to_be_bytes());
        sub[gid_single_off + 2..gid_single_off + 4].copy_from_slice(&101u16.to_be_bytes());
        sub[gid_double_off..gid_double_off + 2].copy_from_slice(&500u16.to_be_bytes());
        sub[gid_double_off + 2..gid_double_off + 4].copy_from_slice(&501u16.to_be_bytes());

        // Single-byte chars route through subHeader 0.
        assert_eq!(lookup_format2(&sub, 0x20), Some(100));
        assert_eq!(lookup_format2(&sub, 0x21), Some(101));
        // Outside the single-byte subrange → None.
        assert_eq!(lookup_format2(&sub, 0x22), None);
        // 2-byte chars 0x8140 / 0x8141 route through subHeader 1.
        assert_eq!(lookup_format2(&sub, 0x8140), Some(500));
        assert_eq!(lookup_format2(&sub, 0x8141), Some(501));
        // Outside the 2-byte subrange → None.
        assert_eq!(lookup_format2(&sub, 0x8142), None);
        // A high byte with no subHeader key (0x99 → key 0 = subHeader 0,
        // low byte 0x00 outside [0x20, 0x22)) → None.
        assert_eq!(lookup_format2(&sub, 0x9900), None);
    }
}
