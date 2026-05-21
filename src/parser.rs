//! sfnt header + table directory parser.
//!
//! An OpenType/CFF font (`OTTO` magic) starts with the same 12-byte
//! sfnt header as TrueType — version + 16-bit table count + 6 bytes of
//! binary-search hints — followed by `numTables * 16` bytes of
//! `(tag[4], checksum, offset, length)` records. The CFF outline data
//! lives in a single `CFF ` table (or `CFF2` for the 1.8+ variation
//! variant); the rest of the directory is the usual `head`, `hhea`,
//! `maxp`, `cmap`, `hmtx`, `name`, `OS/2`, `post` family.
//!
//! This crate's job is to find the CFF table and route everything else
//! either to a tiny inline parser (for the metadata we actually need)
//! or to no-op silence (we don't ship our own glyph rasterizer here —
//! that's `oxideav-scribe`'s problem).

use crate::Error;

/// Maximum table count we will accept in the sfnt header. 1024 is
/// well above any real-world font; the cap exists purely to bound how
/// much we read on malformed input.
const MAX_TABLES: u16 = 1024;

/// Parsed sfnt table directory.
#[derive(Debug, Clone)]
pub(crate) struct TableDirectory {
    entries: Vec<TableRecord>,
    /// Which CFF table flavour this font carries: `b"CFF "` for the
    /// classic Adobe TN5176 variant, `b"CFF2"` for the OpenType 1.8+
    /// variation-aware variant. We store the tag rather than a
    /// boolean so callers can disambiguate without re-walking.
    pub(crate) cff_tag: Option<[u8; 4]>,
}

#[derive(Debug, Clone, Copy)]
struct TableRecord {
    tag: [u8; 4],
    offset: u32,
    length: u32,
}

impl TableDirectory {
    pub(crate) fn parse(bytes: &[u8]) -> Result<Self, Error> {
        if bytes.len() < 12 {
            return Err(Error::UnexpectedEof);
        }
        let version = read_u32(bytes, 0)?;
        // 'OTTO' (CFF flavour) is the canonical OpenType-with-CFF magic.
        // We also accept TrueType-flavoured sfnt magics (`0x00010000`,
        // 'true') because some "OpenType" fonts (e.g. Apple TrueType
        // Collections that re-wrap a CFF subfont) ship CFF data
        // alongside a TT-flavoured directory; the only thing that
        // matters is whether a `CFF ` / `CFF2` table is present.
        match version {
            0x4F54544F /* OTTO */ | 0x00010000 | 0x74727565 /* true */ => {}
            _ => return Err(Error::BadMagic),
        }
        let num_tables = read_u16(bytes, 4)?;
        if num_tables == 0 || num_tables > MAX_TABLES {
            return Err(Error::BadHeader);
        }
        // searchRange / entrySelector / rangeShift skipped.

        let dir_end = 12usize
            .checked_add(num_tables as usize * 16)
            .ok_or(Error::BadHeader)?;
        if bytes.len() < dir_end {
            return Err(Error::UnexpectedEof);
        }

        let mut entries = Vec::with_capacity(num_tables as usize);
        let mut cff_tag = None;
        for i in 0..num_tables as usize {
            let off = 12 + i * 16;
            let tag = [bytes[off], bytes[off + 1], bytes[off + 2], bytes[off + 3]];
            // checksum at off+4, skipped.
            let offset = read_u32(bytes, off + 8)?;
            let length = read_u32(bytes, off + 12)?;
            // Validate offset + length lie inside the file.
            let end = (offset as u64)
                .checked_add(length as u64)
                .ok_or(Error::BadOffset)?;
            if end > bytes.len() as u64 {
                return Err(Error::BadOffset);
            }
            // CFF or CFF2 — first hit wins (a font may not legally
            // have both, per OT spec table 6).
            if cff_tag.is_none() && (tag == *b"CFF " || tag == *b"CFF2") {
                cff_tag = Some(tag);
            }
            entries.push(TableRecord {
                tag,
                offset,
                length,
            });
        }
        Ok(Self { entries, cff_tag })
    }

    /// Return the slice for `tag`, or `None` if the table is absent.
    pub(crate) fn find<'a>(&self, tag: &[u8; 4], bytes: &'a [u8]) -> Option<&'a [u8]> {
        for rec in &self.entries {
            if rec.tag == *tag {
                let start = rec.offset as usize;
                let end = start + rec.length as usize;
                return Some(&bytes[start..end]);
            }
        }
        None
    }

    /// Like `find` but errors with `Error::MissingTable` when absent.
    pub(crate) fn required<'a>(
        &self,
        tag: &'static [u8; 4],
        bytes: &'a [u8],
    ) -> Result<&'a [u8], Error> {
        self.find(tag, bytes).ok_or_else(|| {
            // SAFETY: tag is ASCII per the OpenType spec.
            Error::MissingTable(std::str::from_utf8(tag).unwrap_or("???"))
        })
    }

    /// All `(tag, length)` pairs from the table directory in directory
    /// order. The directory is required by the sfnt spec to be sorted
    /// ascending by tag; we don't re-sort, so iteration follows the
    /// on-disk order.
    pub(crate) fn tag_list(&self) -> impl Iterator<Item = ([u8; 4], u32)> + '_ {
        self.entries.iter().map(|r| (r.tag, r.length))
    }
}

// --- big-endian primitive readers ------------------------------------------

#[inline]
pub(crate) fn read_u8(bytes: &[u8], off: usize) -> Result<u8, Error> {
    bytes.get(off).copied().ok_or(Error::UnexpectedEof)
}

#[inline]
pub(crate) fn read_u16(bytes: &[u8], off: usize) -> Result<u16, Error> {
    let s = bytes.get(off..off + 2).ok_or(Error::UnexpectedEof)?;
    Ok(u16::from_be_bytes([s[0], s[1]]))
}

#[inline]
pub(crate) fn read_i16(bytes: &[u8], off: usize) -> Result<i16, Error> {
    Ok(read_u16(bytes, off)? as i16)
}

#[inline]
pub(crate) fn read_u32(bytes: &[u8], off: usize) -> Result<u32, Error> {
    let s = bytes.get(off..off + 4).ok_or(Error::UnexpectedEof)?;
    Ok(u32::from_be_bytes([s[0], s[1], s[2], s[3]]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_short_input() {
        assert!(matches!(
            TableDirectory::parse(&[0u8; 4]),
            Err(Error::UnexpectedEof)
        ));
    }

    #[test]
    fn rejects_bad_magic() {
        let mut bytes = [0u8; 12];
        bytes[0..4].copy_from_slice(&0xDEADBEEFu32.to_be_bytes());
        assert!(matches!(
            TableDirectory::parse(&bytes),
            Err(Error::BadMagic)
        ));
    }

    #[test]
    fn detects_cff_flavour() {
        // sfnt OTTO header with 1 table record for 'CFF '.
        let mut bytes = vec![0u8; 32];
        bytes[0..4].copy_from_slice(&0x4F54544Fu32.to_be_bytes()); // OTTO
        bytes[4..6].copy_from_slice(&1u16.to_be_bytes());
        bytes[12..16].copy_from_slice(b"CFF ");
        bytes[20..24].copy_from_slice(&28u32.to_be_bytes()); // offset
        bytes[24..28].copy_from_slice(&4u32.to_be_bytes()); // length

        let dir = TableDirectory::parse(&bytes).expect("parse");
        assert_eq!(dir.cff_tag, Some(*b"CFF "));
        assert!(dir.find(b"CFF ", &bytes).is_some());
    }

    #[test]
    fn detects_cff2_flavour() {
        let mut bytes = vec![0u8; 32];
        bytes[0..4].copy_from_slice(&0x4F54544Fu32.to_be_bytes()); // OTTO
        bytes[4..6].copy_from_slice(&1u16.to_be_bytes());
        bytes[12..16].copy_from_slice(b"CFF2");
        bytes[20..24].copy_from_slice(&28u32.to_be_bytes());
        bytes[24..28].copy_from_slice(&4u32.to_be_bytes());

        let dir = TableDirectory::parse(&bytes).expect("parse");
        assert_eq!(dir.cff_tag, Some(*b"CFF2"));
    }
}
