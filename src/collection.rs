//! Font collection (`ttcf`) header.
//!
//! A collection file packs several font resources behind one header
//! (OpenType "Font Collections" / "TTC Header"):
//!
//! ```text
//! Tag      ttcTag                          'ttcf'
//! uint16   majorVersion                    1 or 2
//! uint16   minorVersion                    0
//! uint32   numFonts
//! Offset32 tableDirectoryOffsets[numFonts] from the beginning of the file
//! -- version 2.0 only --
//! uint32   dsigTag, dsigLength, dsigOffset
//! ```
//!
//! The same tag is used whether the members carry TrueType, CFF or
//! CFF2 outlines, and one file may mix them. Each member has a complete
//! table directory of its own; the table offsets in every directory are
//! measured from the beginning of the collection file, which is what
//! lets members share tables. A collection is therefore always opened
//! with the whole file in hand: see
//! [`Font::from_collection_bytes`](crate::Font::from_collection_bytes).
//!
//! The version 2.0 signature fields are not interpreted.

use crate::Error;
use crate::parser::{read_u16, read_u32};

/// `'ttcf'` as a big-endian `u32`.
const COLLECTION_TAG: u32 = u32::from_be_bytes(*b"ttcf");

/// Size of the fixed part of the header (tag, version, `numFonts`).
const FIXED_HEADER_LEN: usize = 12;

/// Size of the sfnt header every member's table directory starts with.
const SFNT_HEADER_LEN: usize = 12;

/// Upper bound on `numFonts`. Far above any real collection; it only
/// bounds the work done on a damaged header.
const MAX_MEMBERS: u32 = 4096;

/// `true` when `bytes` starts with the collection tag `'ttcf'`.
///
/// This is a pure probe: it says nothing about whether the rest of the
/// header is well formed. Use [`font_count`] or
/// [`CollectionHeader::parse`] for that.
pub fn is_collection(bytes: &[u8]) -> bool {
    bytes.get(..4) == Some(b"ttcf".as_slice())
}

/// Number of members in the collection `bytes`.
///
/// Fails with [`Error::BadMagic`] when `bytes` is not a collection and
/// with the errors of [`CollectionHeader::parse`] for a damaged header.
pub fn font_count(bytes: &[u8]) -> Result<u32, Error> {
    CollectionHeader::parse(bytes).map(|h| h.num_fonts())
}

/// A parsed collection header: the version and one table-directory
/// offset per member.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct CollectionHeader {
    major_version: u16,
    minor_version: u16,
    directory_offsets: Vec<u32>,
}

impl CollectionHeader {
    /// Parse the header at the start of `bytes`.
    ///
    /// * [`Error::UnexpectedEof`] — shorter than the fixed header or
    ///   than the offset array `numFonts` announces.
    /// * [`Error::BadMagic`] — the tag is not `'ttcf'`.
    /// * [`Error::BadHeader`] — major version other than 1 or 2, or a
    ///   `numFonts` of zero / implausibly large.
    /// * [`Error::BadOffset`] — a member's table directory would start
    ///   (or its 12-byte sfnt header would end) outside `bytes`.
    pub fn parse(bytes: &[u8]) -> Result<Self, Error> {
        if bytes.len() < 4 {
            return Err(Error::UnexpectedEof);
        }
        if read_u32(bytes, 0)? != COLLECTION_TAG {
            return Err(Error::BadMagic);
        }
        if bytes.len() < FIXED_HEADER_LEN {
            return Err(Error::UnexpectedEof);
        }
        let major_version = read_u16(bytes, 4)?;
        let minor_version = read_u16(bytes, 6)?;
        if !(1..=2).contains(&major_version) {
            return Err(Error::BadHeader);
        }
        let num_fonts = read_u32(bytes, 8)?;
        if num_fonts == 0 || num_fonts > MAX_MEMBERS {
            return Err(Error::BadHeader);
        }
        let array = bytes
            .get(FIXED_HEADER_LEN..FIXED_HEADER_LEN + num_fonts as usize * 4)
            .ok_or(Error::UnexpectedEof)?;
        let mut directory_offsets = Vec::with_capacity(num_fonts as usize);
        for entry in array.as_chunks::<4>().0 {
            let offset = u32::from_be_bytes(*entry);
            let header_end = (offset as u64) + SFNT_HEADER_LEN as u64;
            if header_end > bytes.len() as u64 {
                return Err(Error::BadOffset);
            }
            directory_offsets.push(offset);
        }
        Ok(Self {
            major_version,
            minor_version,
            directory_offsets,
        })
    }

    /// `(majorVersion, minorVersion)` of the header.
    pub fn version(&self) -> (u16, u16) {
        (self.major_version, self.minor_version)
    }

    /// Number of members (`numFonts`).
    pub fn num_fonts(&self) -> u32 {
        self.directory_offsets.len() as u32
    }

    /// File-relative offset of member `index`'s table directory, or
    /// `None` when `index` is out of range.
    pub fn font_offset(&self, index: u32) -> Option<u32> {
        self.directory_offsets.get(index as usize).copied()
    }

    /// All table-directory offsets, in member order.
    pub fn font_offsets(&self) -> &[u32] {
        &self.directory_offsets
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(major: u16, offsets: &[u32], total_len: usize) -> Vec<u8> {
        let mut b = Vec::new();
        b.extend_from_slice(b"ttcf");
        b.extend_from_slice(&major.to_be_bytes());
        b.extend_from_slice(&0u16.to_be_bytes());
        b.extend_from_slice(&(offsets.len() as u32).to_be_bytes());
        for o in offsets {
            b.extend_from_slice(&o.to_be_bytes());
        }
        b.resize(total_len.max(b.len()), 0);
        b
    }

    #[test]
    fn parses_both_versions() {
        for major in [1u16, 2] {
            let b = header(major, &[40, 100], 200);
            let h = CollectionHeader::parse(&b).unwrap();
            assert_eq!(h.version(), (major, 0));
            assert_eq!(h.num_fonts(), 2);
            assert_eq!(h.font_offsets(), &[40, 100]);
            assert_eq!(h.font_offset(1), Some(100));
            assert_eq!(h.font_offset(2), None);
            assert_eq!(font_count(&b), Ok(2));
            assert!(is_collection(&b));
        }
    }

    #[test]
    fn typed_errors() {
        assert!(!is_collection(b"OTTO\0\0\0\0\0\0\0\0"));
        assert!(!is_collection(b"ttc"));
        assert_eq!(
            CollectionHeader::parse(b"ttc").unwrap_err(),
            Error::UnexpectedEof
        );
        assert_eq!(
            CollectionHeader::parse(b"OTTO\0\0\0\0\0\0\0\0").unwrap_err(),
            Error::BadMagic
        );
        assert_eq!(
            CollectionHeader::parse(b"ttcf\0\x01\0\0").unwrap_err(),
            Error::UnexpectedEof
        );
        // Version 0 and 3 are not defined.
        for major in [0u16, 3] {
            let b = header(major, &[40], 100);
            assert_eq!(CollectionHeader::parse(&b).unwrap_err(), Error::BadHeader);
        }
        // numFonts == 0.
        let b = header(1, &[], 100);
        assert_eq!(CollectionHeader::parse(&b).unwrap_err(), Error::BadHeader);
        // numFonts far past the data: bounded, no allocation by count.
        let mut b = header(1, &[40], 100);
        b[8..12].copy_from_slice(&u32::MAX.to_be_bytes());
        assert_eq!(CollectionHeader::parse(&b).unwrap_err(), Error::BadHeader);
        b[8..12].copy_from_slice(&1000u32.to_be_bytes());
        assert_eq!(
            CollectionHeader::parse(&b).unwrap_err(),
            Error::UnexpectedEof
        );
        // Offset array cut short.
        let b = header(1, &[40, 60], 100);
        assert_eq!(
            CollectionHeader::parse(&b[..18]).unwrap_err(),
            Error::UnexpectedEof
        );
        // A directory whose sfnt header would cross the end of the file.
        for bad in [89u32, 100, u32::MAX] {
            let b = header(1, &[40, bad], 100);
            assert_eq!(CollectionHeader::parse(&b).unwrap_err(), Error::BadOffset);
        }
        // ... while one ending exactly at the end is accepted here (the
        // directory parse then rejects it on its own terms).
        let b = header(1, &[88], 100);
        assert!(CollectionHeader::parse(&b).is_ok());
    }
}
