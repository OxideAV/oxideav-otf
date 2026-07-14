//! `EBSC` — Embedded bitmap scaling table
//! (ISO/IEC 14496-22:2019 §5.6.4).
//!
//! Defines bitmap strikes that are produced by **scaling** another
//! strike that exists as real sbit data in `EBLC` / `EBDT`, for the
//! cases (small CJK sizes, typically) where a scaled bitmap is more
//! legible than a scan-converted outline. Each `BitmapScale` record
//! names the target size (`ppem_x` / `ppem_y`), the substitute strike
//! to scale (`substitute_ppem_x` / `substitute_ppem_y`), and the
//! font-wide line metrics **after** scaling. x and y scaling are
//! independent; glyph metrics scale by the same per-axis ppem factor,
//! rounded to the nearest integer pixel.

use crate::parser::{read_u16, read_u32};
use crate::tables::eblc::SbitLineMetrics;
use crate::Error;

/// One `BitmapScale` record (28 bytes on disk).
#[derive(Debug, Clone, Copy)]
pub struct BitmapScale {
    /// Line metrics for horizontal text, after scaling.
    pub hori: SbitLineMetrics,
    /// Line metrics for vertical text, after scaling.
    pub vert: SbitLineMetrics,
    /// Target horizontal pixels per em.
    pub ppem_x: u8,
    /// Target vertical pixels per em.
    pub ppem_y: u8,
    /// Horizontal PPEM of the real strike to scale.
    pub substitute_ppem_x: u8,
    /// Vertical PPEM of the real strike to scale.
    pub substitute_ppem_y: u8,
}

impl BitmapScale {
    const LEN: usize = 28;

    fn parse(data: &[u8], at: usize) -> Result<Self, Error> {
        let hori = SbitLineMetrics::parse(data, at)?;
        let vert = SbitLineMetrics::parse(data, at + SbitLineMetrics::LEN)?;
        let tail = at + 2 * SbitLineMetrics::LEN;
        let b = data.get(tail..tail + 4).ok_or(Error::UnexpectedEof)?;
        Ok(Self {
            hori,
            vert,
            ppem_x: b[0],
            ppem_y: b[1],
            substitute_ppem_x: b[2],
            substitute_ppem_y: b[3],
        })
    }
}

/// A parsed `EBSC` table.
#[derive(Debug)]
pub struct EbscTable {
    major_version: u16,
    minor_version: u16,
    scales: Vec<BitmapScale>,
}

impl EbscTable {
    /// Parse an `EBSC` table (major version 2).
    pub fn parse(data: &[u8]) -> Result<Self, Error> {
        let major_version = read_u16(data, 0)?;
        let minor_version = read_u16(data, 2)?;
        if major_version != 2 {
            return Err(Error::BadStructure("EBSC: major version must be 2"));
        }
        let num_sizes = read_u32(data, 4)? as usize;
        if num_sizes > data.len() / BitmapScale::LEN {
            return Err(Error::BadStructure("EBSC: numSizes exceeds table size"));
        }
        let mut scales = Vec::with_capacity(num_sizes);
        for i in 0..num_sizes {
            scales.push(BitmapScale::parse(data, 8 + i * BitmapScale::LEN)?);
        }
        Ok(Self {
            major_version,
            minor_version,
            scales,
        })
    }

    /// Major version (2).
    pub fn major_version(&self) -> u16 {
        self.major_version
    }

    /// Minor version (0).
    pub fn minor_version(&self) -> u16 {
        self.minor_version
    }

    /// The `BitmapScale` records, in table order.
    pub fn scales(&self) -> &[BitmapScale] {
        &self.scales
    }

    /// The scaled-strike definition targeting exactly
    /// `(ppem_x, ppem_y)`, if any.
    pub fn scale_for(&self, ppem_x: u8, ppem_y: u8) -> Option<&BitmapScale> {
        self.scales
            .iter()
            .find(|s| s.ppem_x == ppem_x && s.ppem_y == ppem_y)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build(scales: &[(u8, u8, u8, u8)]) -> Vec<u8> {
        let mut b = Vec::new();
        b.extend_from_slice(&2u16.to_be_bytes());
        b.extend_from_slice(&0u16.to_be_bytes());
        b.extend_from_slice(&(scales.len() as u32).to_be_bytes());
        for &(px, py, sx, sy) in scales {
            let mut hori = [0u8; 12];
            hori[0] = px.wrapping_add(2); // ascender, arbitrary but checked
            hori[1] = 0xFC; // descender -4
            hori[2] = px; // widthMax
            b.extend_from_slice(&hori);
            b.extend_from_slice(&[0u8; 12]); // vert
            b.extend_from_slice(&[px, py, sx, sy]);
        }
        b
    }

    #[test]
    fn scales_decode_and_lookup() {
        let bytes = build(&[(11, 11, 12, 12), (13, 14, 16, 16)]);
        let t = EbscTable::parse(&bytes).expect("parse");
        assert_eq!(t.major_version(), 2);
        assert_eq!(t.minor_version(), 0);
        assert_eq!(t.scales().len(), 2);

        let s = t.scale_for(11, 11).expect("11x11 scale");
        assert_eq!((s.substitute_ppem_x, s.substitute_ppem_y), (12, 12));
        assert_eq!(s.hori.ascender, 13);
        assert_eq!(s.hori.descender, -4);
        assert_eq!(s.hori.width_max, 11);
        assert_eq!(s.vert.ascender, 0);

        // Non-square target with square substitute.
        let s = t.scale_for(13, 14).expect("13x14 scale");
        assert_eq!((s.substitute_ppem_x, s.substitute_ppem_y), (16, 16));

        assert!(t.scale_for(9, 9).is_none());
    }

    #[test]
    fn rejects_bad_version_and_truncation() {
        let mut bytes = build(&[(11, 11, 12, 12)]);
        assert!(EbscTable::parse(&bytes[..bytes.len() - 1]).is_err());
        bytes[0..2].copy_from_slice(&1u16.to_be_bytes());
        assert!(EbscTable::parse(&bytes).is_err());
        // numSizes larger than the data can hold.
        let mut bytes = build(&[(11, 11, 12, 12)]);
        bytes[4..8].copy_from_slice(&9999u32.to_be_bytes());
        assert!(EbscTable::parse(&bytes).is_err());
    }
}
