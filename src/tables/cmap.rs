//! `cmap` — character → glyph map.
//!
//! We pick a single subtable at parse time (preferred order: 32-bit
//! formats first, BMP formats second, legacy single-byte last) and
//! run all `lookup` calls through it. Round-1 supports formats
//! 0, 4, 6, 12.

use crate::Error;
use crate::parser::{read_u16, read_u32};

#[derive(Debug, Clone)]
pub struct CmapTable<'a> {
    subtable: Subtable<'a>,
}

#[derive(Debug, Clone)]
enum Subtable<'a> {
    Format0(&'a [u8]),
    Format4(&'a [u8]),
    Format6(&'a [u8]),
    Format12(&'a [u8]),
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

            let candidate = match format {
                0 => Some(Subtable::Format0(sub)),
                4 => Some(Subtable::Format4(sub)),
                6 => Some(Subtable::Format6(sub)),
                12 => Some(Subtable::Format12(sub)),
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
        })
    }

    /// Map a Unicode codepoint to a glyph id, or `None` if absent.
    pub fn lookup(&self, codepoint: u32) -> Option<u16> {
        match &self.subtable {
            Subtable::Format0(b) => lookup_format0(b, codepoint),
            Subtable::Format4(b) => lookup_format4(b, codepoint),
            Subtable::Format6(b) => lookup_format6(b, codepoint),
            Subtable::Format12(b) => lookup_format12(b, codepoint),
        }
    }
}

fn subtable_length(bytes: &[u8], off: usize, format: u16) -> Result<usize, Error> {
    Ok(match format {
        0 | 2 | 4 | 6 => read_u16(bytes, off + 2)? as usize,
        8 | 10 | 12 | 13 => read_u32(bytes, off + 4)? as usize,
        _ => return Err(Error::UnsupportedCmapFormat(format)),
    })
}

fn subtable_rank(format: u16, platform: u16, encoding: u16) -> i32 {
    let format_score = match format {
        12 => 400,
        4 => 300,
        6 => 200,
        0 => 100,
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
}
