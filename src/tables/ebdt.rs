//! `EBDT` / `CBDT` — Embedded / Color bitmap **data** tables
//! (ISO/IEC 14496-22:2019 §5.6.2 and §5.6.5).
//!
//! The data table is a version header followed by raw glyph bitmap
//! blobs; which bytes belong to which glyph — and in which of the
//! image formats — is answered by the companion `EBLC` / `CBLC`
//! location table (`tables::eblc`), whose [`BitmapLocation`] this
//! module consumes.
//!
//! Image formats (per §5.6.2.2 and §5.6.5.2):
//!
//! - **1 / 2** — small metrics + byte- / bit-aligned image data;
//! - **3** — obsolete, "not supported in OFF" (an error here);
//! - **4** — compressed data, platform-specific; OFF defines no
//!   structure for it (an error here);
//! - **5** — bit-aligned image data only (metrics live in the
//!   location table's constant-metrics index formats 2 / 5);
//! - **6 / 7** — big metrics + byte- / bit-aligned image data;
//! - **8 / 9** — composite glyphs: small / big metrics + a list of
//!   [`EbdtComponent`] records positioning other glyphs' bitmaps;
//! - **17 / 18 / 19** (`CBDT`) — small / big / location-table metrics
//!   + a `dataLen`-prefixed raw PNG payload.
//!
//! Packed image data expands to per-pixel values with
//! [`unpack_pixels`] (bit depths 1 / 2 / 4 / 8, MSB-first, with the
//! byte-aligned formats' per-row padding) or [`unpack_bgra32`] for
//! the `CBLC` `bitDepth` 32 pre-multiplied sRGB BGRA layout.

use crate::parser::{read_u16, read_u32, read_u8};
use crate::tables::eblc::{BigGlyphMetrics, BitmapLocation, SmallGlyphMetrics};
use crate::Error;

/// An `EbdtComponent` record (image formats 8 / 9): one constituent
/// glyph of a composite bitmap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EbdtComponent {
    /// Glyph ID of the component (locate its bitmap through the
    /// location table).
    pub glyph_id: u16,
    /// Position of the component's left edge.
    pub x_offset: i8,
    /// Position of the component's top edge.
    pub y_offset: i8,
}

/// The metrics stored inline with a glyph's image data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GlyphMetrics {
    /// `BigGlyphMetrics` — both layout directions.
    Big(BigGlyphMetrics),
    /// `SmallGlyphMetrics` — one layout direction (which one is the
    /// strike's `flags` choice).
    Small(SmallGlyphMetrics),
}

impl GlyphMetrics {
    /// Bitmap width in pixels.
    pub fn width(&self) -> u8 {
        match self {
            Self::Big(m) => m.width,
            Self::Small(m) => m.width,
        }
    }

    /// Bitmap height in pixels.
    pub fn height(&self) -> u8 {
        match self {
            Self::Big(m) => m.height,
            Self::Small(m) => m.height,
        }
    }
}

/// The image payload of one glyph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BitmapContent<'a> {
    /// Byte-aligned packed rows (formats 1 / 6): each row is padded
    /// to a byte boundary.
    ByteAligned(&'a [u8]),
    /// Bit-aligned packed rows (formats 2 / 5 / 7): each row starts
    /// at the bit after the previous row's last bit.
    BitAligned(&'a [u8]),
    /// A composite (formats 8 / 9): position each component's bitmap
    /// at its offsets.
    Components(Vec<EbdtComponent>),
    /// A raw PNG payload (`CBDT` formats 17 / 18 / 19).
    Png(&'a [u8]),
}

/// One glyph's decoded bitmap-data entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GlyphBitmapData<'a> {
    /// The metrics stored inline with the image data; `None` for
    /// image formats 5 and 19, whose metrics live in the location
    /// table ([`BitmapLocation::metrics`]).
    pub metrics: Option<GlyphMetrics>,
    /// The image payload.
    pub content: BitmapContent<'a>,
}

/// A parsed `EBDT` or `CBDT` table.
#[derive(Debug)]
pub struct BitmapDataTable<'a> {
    data: &'a [u8],
    major_version: u16,
    minor_version: u16,
}

impl<'a> BitmapDataTable<'a> {
    /// Parse an `EBDT` (major version 2) or `CBDT` (major version 3)
    /// table header; the body is decoded per-glyph via
    /// [`BitmapDataTable::glyph_data`].
    pub fn parse(data: &'a [u8]) -> Result<Self, Error> {
        let major_version = read_u16(data, 0)?;
        let minor_version = read_u16(data, 2)?;
        if major_version != 2 && major_version != 3 {
            return Err(Error::BadStructure(
                "EBDT/CBDT: major version must be 2 or 3",
            ));
        }
        Ok(Self {
            data,
            major_version,
            minor_version,
        })
    }

    /// Major version: 2 for `EBDT`, 3 for `CBDT`.
    pub fn major_version(&self) -> u16 {
        self.major_version
    }

    /// Minor version (0).
    pub fn minor_version(&self) -> u16 {
        self.minor_version
    }

    /// Decode the glyph data a location-table lookup points at.
    pub fn glyph_data(&self, loc: &BitmapLocation) -> Result<GlyphBitmapData<'a>, Error> {
        let start = loc.offset as usize;
        let end = start
            .checked_add(loc.length as usize)
            .ok_or(Error::BadOffset)?;
        if end > self.data.len() {
            return Err(Error::BadOffset);
        }
        let d = &self.data[start..end];

        let small = |b: &[u8]| SmallGlyphMetrics::parse(b, 0);
        let big = |b: &[u8]| BigGlyphMetrics::parse(b, 0);

        match loc.image_format {
            // Small metrics + byte-aligned data.
            1 => Ok(GlyphBitmapData {
                metrics: Some(GlyphMetrics::Small(small(d)?)),
                content: BitmapContent::ByteAligned(rest(d, SmallGlyphMetrics::LEN)?),
            }),
            // Small metrics + bit-aligned data.
            2 => Ok(GlyphBitmapData {
                metrics: Some(GlyphMetrics::Small(small(d)?)),
                content: BitmapContent::BitAligned(rest(d, SmallGlyphMetrics::LEN)?),
            }),
            3 => Err(Error::BadStructure(
                "EBDT: image format 3 is obsolete and not supported in OFF",
            )),
            4 => Err(Error::BadStructure(
                "EBDT: image format 4 (compressed) has no OFF-defined structure",
            )),
            // Bit-aligned data only; metrics in the location table.
            5 => Ok(GlyphBitmapData {
                metrics: None,
                content: BitmapContent::BitAligned(d),
            }),
            // Big metrics + byte-aligned data.
            6 => Ok(GlyphBitmapData {
                metrics: Some(GlyphMetrics::Big(big(d)?)),
                content: BitmapContent::ByteAligned(rest(d, BigGlyphMetrics::LEN)?),
            }),
            // Big metrics + bit-aligned data.
            7 => Ok(GlyphBitmapData {
                metrics: Some(GlyphMetrics::Big(big(d)?)),
                content: BitmapContent::BitAligned(rest(d, BigGlyphMetrics::LEN)?),
            }),
            // Small metrics + pad byte + components.
            8 => {
                let n = read_u16(d, SmallGlyphMetrics::LEN + 1)? as usize;
                Ok(GlyphBitmapData {
                    metrics: Some(GlyphMetrics::Small(small(d)?)),
                    content: BitmapContent::Components(components(
                        d,
                        SmallGlyphMetrics::LEN + 3,
                        n,
                    )?),
                })
            }
            // Big metrics + components.
            9 => {
                let n = read_u16(d, BigGlyphMetrics::LEN)? as usize;
                Ok(GlyphBitmapData {
                    metrics: Some(GlyphMetrics::Big(big(d)?)),
                    content: BitmapContent::Components(components(d, BigGlyphMetrics::LEN + 2, n)?),
                })
            }
            // CBDT: small metrics + dataLen + PNG.
            17 => Ok(GlyphBitmapData {
                metrics: Some(GlyphMetrics::Small(small(d)?)),
                content: BitmapContent::Png(png(d, SmallGlyphMetrics::LEN)?),
            }),
            // CBDT: big metrics + dataLen + PNG.
            18 => Ok(GlyphBitmapData {
                metrics: Some(GlyphMetrics::Big(big(d)?)),
                content: BitmapContent::Png(png(d, BigGlyphMetrics::LEN)?),
            }),
            // CBDT: dataLen + PNG; metrics in the location table.
            19 => Ok(GlyphBitmapData {
                metrics: None,
                content: BitmapContent::Png(png(d, 0)?),
            }),
            _ => Err(Error::BadStructure("EBDT/CBDT: unknown image format")),
        }
    }
}

/// The bytes after a fixed-size prefix, bounds-checked.
fn rest(d: &[u8], prefix: usize) -> Result<&[u8], Error> {
    d.get(prefix..).ok_or(Error::UnexpectedEof)
}

/// `count` EbdtComponent records at `at`.
fn components(d: &[u8], at: usize, count: usize) -> Result<Vec<EbdtComponent>, Error> {
    let mut out = Vec::with_capacity(count.min(d.len() / 4));
    for i in 0..count {
        let off = at + i * 4;
        out.push(EbdtComponent {
            glyph_id: read_u16(d, off)?,
            x_offset: read_u8(d, off + 2)? as i8,
            y_offset: read_u8(d, off + 3)? as i8,
        });
    }
    Ok(out)
}

/// A `dataLen`-prefixed PNG payload at `at`.
fn png(d: &[u8], at: usize) -> Result<&[u8], Error> {
    let len = read_u32(d, at)? as usize;
    d.get(at + 4..at + 4 + len)
        .ok_or(Error::BadStructure("EBDT/CBDT: PNG dataLen exceeds entry"))
}

/// Expand packed 1 / 2 / 4 / 8-bit image data into one value per
/// pixel, row-major top-to-bottom, left-to-right.
///
/// Per §5.6.2.2 the data begins with the most significant bit of the
/// first byte at the top-left pixel; a pixel's bits are consecutive.
/// With `byte_aligned = true` (formats 1 / 6) every row is padded to
/// a byte boundary; with `false` (formats 2 / 5 / 7) each row starts
/// on the bit after the previous row ends. 1-bits are black, 0-bits
/// white; multi-bit depths are gray levels.
pub fn unpack_pixels(
    image: &[u8],
    width: usize,
    height: usize,
    bit_depth: u8,
    byte_aligned: bool,
) -> Result<Vec<u8>, Error> {
    if !matches!(bit_depth, 1 | 2 | 4 | 8) {
        return Err(Error::BadStructure(
            "EBDT/CBDT: bitDepth must be 1, 2, 4, or 8",
        ));
    }
    let bd = bit_depth as usize;
    // Total bits needed.
    let row_bits = width * bd;
    let stride_bits = if byte_aligned {
        row_bits.div_ceil(8) * 8
    } else {
        row_bits
    };
    let total_bits = stride_bits
        .checked_mul(height)
        .ok_or(Error::BadStructure("EBDT/CBDT: bitmap dimensions overflow"))?;
    if total_bits.div_ceil(8) > image.len() {
        return Err(Error::UnexpectedEof);
    }

    let bit_at = |i: usize| (image[i / 8] >> (7 - (i % 8))) & 1;
    let mut out = Vec::with_capacity(width * height);
    for row in 0..height {
        let row_start = row * stride_bits;
        for col in 0..width {
            let mut v = 0u8;
            for b in 0..bd {
                v = (v << 1) | bit_at(row_start + col * bd + b);
            }
            out.push(v);
        }
    }
    Ok(out)
}

/// Expand `CBLC` `bitDepth` 32 uncompressed color data: 4 bytes per
/// pixel, **BGRA** channel order, sRGB, pre-multiplied alpha (per
/// §5.6.5.1 — e.g. full-green at half translucency is stored
/// `00 80 00 80`). Returned as `[b, g, r, a]` quadruples, row-major.
pub fn unpack_bgra32(image: &[u8], width: usize, height: usize) -> Result<Vec<[u8; 4]>, Error> {
    let count = width
        .checked_mul(height)
        .ok_or(Error::BadStructure("EBDT/CBDT: bitmap dimensions overflow"))?;
    if count * 4 > image.len() {
        return Err(Error::UnexpectedEof);
    }
    Ok(image[..count * 4]
        .chunks_exact(4)
        .map(|c| [c[0], c[1], c[2], c[3]])
        .collect())
}
