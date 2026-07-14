//! `EBLC` / `CBLC` — Embedded / Color bitmap **location** tables
//! (ISO/IEC 14496-22:2019 §5.6.3 and §5.6.6).
//!
//! Both tables share one structure — the `CBLC` (major version 3) is
//! backward compatible with `EBLC` (major version 2), differing only
//! in the additional `bitDepth` value 32 (per-pixel 8-bit BGRA) and
//! the three PNG image formats its companion `CBDT` adds. This module
//! decodes either flavour.
//!
//! Layout: a header (version + `numSizes`) followed by one
//! `BitmapSize` record per strike. Each strike points at an
//! `IndexSubTableArray` — `(firstGlyphIndex, lastGlyphIndex,
//! additionalOffsetToIndexSubtable)` ranges — whose IndexSubTables
//! map glyph IDs to image data in the companion `EBDT` / `CBDT`
//! table. All five index formats are decoded:
//!
//! 1. variable metrics, 4-byte offsets (per-glyph `Offset32` + one
//!    extra so the last length is computable);
//! 2. constant metrics + constant image size (a single
//!    `BigGlyphMetrics` copy in the index table);
//! 3. variable metrics, 2-byte offsets;
//! 4. variable metrics, sparse glyph IDs (`(glyphID, offset)` pairs,
//!    `numGlyphs + 1` entries);
//! 5. constant metrics, sparse glyph IDs (sorted `glyphIDArray`).
//!
//! A successful lookup yields a [`BitmapLocation`]: the `EBDT`/`CBDT`
//! image format, the absolute byte range within that table, and — for
//! the constant-metrics formats — the shared [`BigGlyphMetrics`].

use crate::parser::{read_u16, read_u32, read_u8};
use crate::Error;

/// `BitmapSize.flags` bit 0: small glyph metrics are for horizontal
/// layout.
pub const BITMAP_FLAG_HORIZONTAL_METRICS: u8 = 0x01;
/// `BitmapSize.flags` bit 1: small glyph metrics are for vertical
/// layout.
pub const BITMAP_FLAG_VERTICAL_METRICS: u8 = 0x02;

/// `SbitLineMetrics` — per-strike line metrics (one horizontal, one
/// vertical copy per `BitmapSize`). All fields are single bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SbitLineMetrics {
    /// Ascender, pixels.
    pub ascender: i8,
    /// Descender, pixels.
    pub descender: i8,
    /// Maximum glyph width, pixels.
    pub width_max: u8,
    /// Caret slope numerator (rise).
    pub caret_slope_numerator: i8,
    /// Caret slope denominator (run).
    pub caret_slope_denominator: i8,
    /// Pixels to shift the caret (+ or -).
    pub caret_offset: i8,
    /// Minimum origin-side side bearing.
    pub min_origin_sb: i8,
    /// Minimum advance-side side bearing.
    pub min_advance_sb: i8,
    /// Maximum extent above the baseline.
    pub max_before_bl: i8,
    /// Minimum extent below the baseline.
    pub min_after_bl: i8,
}

impl SbitLineMetrics {
    /// 10 metric bytes + 2 pad bytes.
    pub const LEN: usize = 12;

    /// Read an `SbitLineMetrics` record at byte offset `at` of `data`
    /// (the two trailing pad bytes are skipped, not validated).
    pub fn parse(data: &[u8], at: usize) -> Result<Self, Error> {
        Ok(Self {
            ascender: read_u8(data, at)? as i8,
            descender: read_u8(data, at + 1)? as i8,
            width_max: read_u8(data, at + 2)?,
            caret_slope_numerator: read_u8(data, at + 3)? as i8,
            caret_slope_denominator: read_u8(data, at + 4)? as i8,
            caret_offset: read_u8(data, at + 5)? as i8,
            min_origin_sb: read_u8(data, at + 6)? as i8,
            min_advance_sb: read_u8(data, at + 7)? as i8,
            max_before_bl: read_u8(data, at + 8)? as i8,
            min_after_bl: read_u8(data, at + 9)? as i8,
            // pad1 / pad2 skipped.
        })
    }
}

/// `BigGlyphMetrics` — metrics for both layout directions (8 bytes).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BigGlyphMetrics {
    /// Bitmap height in pixels.
    pub height: u8,
    /// Bitmap width in pixels.
    pub width: u8,
    /// Horizontal-layout x bearing.
    pub hori_bearing_x: i8,
    /// Horizontal-layout y bearing.
    pub hori_bearing_y: i8,
    /// Horizontal advance, pixels.
    pub hori_advance: u8,
    /// Vertical-layout x bearing.
    pub vert_bearing_x: i8,
    /// Vertical-layout y bearing.
    pub vert_bearing_y: i8,
    /// Vertical advance, pixels.
    pub vert_advance: u8,
}

impl BigGlyphMetrics {
    /// On-disk length.
    pub const LEN: usize = 8;

    /// Read a `BigGlyphMetrics` record at byte offset `at` of `data`.
    pub fn parse(data: &[u8], at: usize) -> Result<Self, Error> {
        Ok(Self {
            height: read_u8(data, at)?,
            width: read_u8(data, at + 1)?,
            hori_bearing_x: read_u8(data, at + 2)? as i8,
            hori_bearing_y: read_u8(data, at + 3)? as i8,
            hori_advance: read_u8(data, at + 4)?,
            vert_bearing_x: read_u8(data, at + 5)? as i8,
            vert_bearing_y: read_u8(data, at + 6)? as i8,
            vert_advance: read_u8(data, at + 7)?,
        })
    }
}

/// `SmallGlyphMetrics` — metrics for one layout direction (5 bytes);
/// which direction is given by the strike's `flags`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SmallGlyphMetrics {
    /// Bitmap height in pixels.
    pub height: u8,
    /// Bitmap width in pixels.
    pub width: u8,
    /// Bearing in the layout direction's cross axis.
    pub bearing_x: i8,
    /// Bearing in the layout direction's main axis.
    pub bearing_y: i8,
    /// Advance, pixels.
    pub advance: u8,
}

impl SmallGlyphMetrics {
    /// On-disk length.
    pub const LEN: usize = 5;

    /// Read a `SmallGlyphMetrics` record at byte offset `at` of
    /// `data`.
    pub fn parse(data: &[u8], at: usize) -> Result<Self, Error> {
        Ok(Self {
            height: read_u8(data, at)?,
            width: read_u8(data, at + 1)?,
            bearing_x: read_u8(data, at + 2)? as i8,
            bearing_y: read_u8(data, at + 3)? as i8,
            advance: read_u8(data, at + 4)?,
        })
    }
}

/// One `BitmapSize` record — a strike (48 bytes on disk).
#[derive(Debug, Clone, Copy)]
pub struct BitmapSize {
    /// Offset to this strike's `IndexSubTableArray`, from the
    /// beginning of the location table.
    pub index_subtable_array_offset: u32,
    /// Total bytes in the array + its IndexSubTables.
    pub index_tables_size: u32,
    /// Number of `IndexSubTableArray` elements for this strike.
    pub number_of_index_subtables: u32,
    /// Line metrics for horizontal text.
    pub hori: SbitLineMetrics,
    /// Line metrics for vertical text.
    pub vert: SbitLineMetrics,
    /// Lowest glyph ID in this strike (advisory; the IndexSubTables
    /// decide actual coverage).
    pub start_glyph_index: u16,
    /// Highest glyph ID in this strike (advisory).
    pub end_glyph_index: u16,
    /// Horizontal pixels per em.
    pub ppem_x: u8,
    /// Vertical pixels per em.
    pub ppem_y: u8,
    /// Bits per pixel: 1 / 2 / 4 / 8 grayscale levels, or 32 for
    /// `CBLC` per-pixel BGRA color.
    pub bit_depth: u8,
    /// Direction of small glyph metrics — see
    /// [`BITMAP_FLAG_HORIZONTAL_METRICS`] /
    /// [`BITMAP_FLAG_VERTICAL_METRICS`].
    pub flags: u8,
}

impl BitmapSize {
    const LEN: usize = 48;

    fn parse(data: &[u8], at: usize) -> Result<Self, Error> {
        Ok(Self {
            index_subtable_array_offset: read_u32(data, at)?,
            index_tables_size: read_u32(data, at + 4)?,
            number_of_index_subtables: read_u32(data, at + 8)?,
            // colorRef at +12: "Not used; set to 0" — skipped.
            hori: SbitLineMetrics::parse(data, at + 16)?,
            vert: SbitLineMetrics::parse(data, at + 16 + SbitLineMetrics::LEN)?,
            start_glyph_index: read_u16(data, at + 40)?,
            end_glyph_index: read_u16(data, at + 42)?,
            ppem_x: read_u8(data, at + 44)?,
            ppem_y: read_u8(data, at + 45)?,
            bit_depth: read_u8(data, at + 46)?,
            flags: read_u8(data, at + 47)?,
        })
    }

    /// Whether small glyph metrics in this strike are horizontal.
    pub fn horizontal_metrics(&self) -> bool {
        self.flags & BITMAP_FLAG_HORIZONTAL_METRICS != 0
    }

    /// Whether small glyph metrics in this strike are vertical.
    pub fn vertical_metrics(&self) -> bool {
        self.flags & BITMAP_FLAG_VERTICAL_METRICS != 0
    }
}

/// A located glyph bitmap: where its image data lives in the
/// companion `EBDT` / `CBDT` table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BitmapLocation {
    /// The `EBDT`/`CBDT` glyph bitmap data format (1–9, 17–19).
    pub image_format: u16,
    /// Absolute byte offset of the glyph's image data within the
    /// `EBDT` / `CBDT` table.
    pub offset: u32,
    /// Byte length of the glyph's image data (for the
    /// constant-metrics index formats 2 / 5 this is `imageSize`).
    pub length: u32,
    /// The shared metrics stored in the index table itself (index
    /// formats 2 and 5 only) — image formats 5 carry no metrics of
    /// their own and rely on these.
    pub metrics: Option<BigGlyphMetrics>,
}

/// A parsed `EBLC` or `CBLC` table.
#[derive(Debug)]
pub struct BitmapLocationTable<'a> {
    data: &'a [u8],
    major_version: u16,
    minor_version: u16,
    sizes: Vec<BitmapSize>,
}

impl<'a> BitmapLocationTable<'a> {
    /// Parse an `EBLC` (major version 2) or `CBLC` (major version 3)
    /// table; the two structures are identical.
    pub fn parse(data: &'a [u8]) -> Result<Self, Error> {
        let major_version = read_u16(data, 0)?;
        let minor_version = read_u16(data, 2)?;
        if major_version != 2 && major_version != 3 {
            return Err(Error::BadStructure(
                "EBLC/CBLC: major version must be 2 or 3",
            ));
        }
        let num_sizes = read_u32(data, 4)? as usize;
        if num_sizes > data.len() / BitmapSize::LEN {
            return Err(Error::BadStructure(
                "EBLC/CBLC: numSizes exceeds table size",
            ));
        }
        let mut sizes = Vec::with_capacity(num_sizes);
        for i in 0..num_sizes {
            sizes.push(BitmapSize::parse(data, 8 + i * BitmapSize::LEN)?);
        }
        Ok(Self {
            data,
            major_version,
            minor_version,
            sizes,
        })
    }

    /// Major version: 2 for `EBLC`, 3 for `CBLC`.
    pub fn major_version(&self) -> u16 {
        self.major_version
    }

    /// Minor version (0).
    pub fn minor_version(&self) -> u16 {
        self.minor_version
    }

    /// The strikes (`BitmapSize` records), in table order.
    pub fn sizes(&self) -> &[BitmapSize] {
        &self.sizes
    }

    /// The strike best matching a requested `ppem` (compared against
    /// `ppem_y`): exact, else smallest larger, else largest; ties
    /// resolve to the higher bit depth. Returns an index into
    /// [`BitmapLocationTable::sizes`].
    pub fn best_size(&self, ppem: u8) -> Option<usize> {
        let key = |s: &BitmapSize| {
            if s.ppem_y >= ppem {
                (0u8, (s.ppem_y - ppem) as u16)
            } else {
                (1u8, (ppem - s.ppem_y) as u16)
            }
        };
        let mut best: Option<usize> = None;
        for (i, s) in self.sizes.iter().enumerate() {
            let better = match best {
                None => true,
                Some(b) => {
                    let (ks, kb) = (key(s), key(&self.sizes[b]));
                    ks < kb || (ks == kb && s.bit_depth > self.sizes[b].bit_depth)
                }
            };
            if better {
                best = Some(i);
            }
        }
        best
    }

    /// Locate `glyph_id`'s image data within strike number
    /// `size_index`. `Ok(None)` when no IndexSubTable range covers the
    /// glyph or its data length is zero (glyph absent from the
    /// strike).
    pub fn locate(
        &self,
        size_index: usize,
        glyph_id: u16,
    ) -> Result<Option<BitmapLocation>, Error> {
        let size = self
            .sizes
            .get(size_index)
            .ok_or(Error::BadStructure("EBLC/CBLC: size index out of range"))?;
        let array_at = size.index_subtable_array_offset as usize;
        // Scan the IndexSubTableArray for the range holding glyph_id.
        for i in 0..size.number_of_index_subtables as usize {
            let rec = array_at + i * 8;
            let first = read_u16(self.data, rec)?;
            let last = read_u16(self.data, rec + 2)?;
            if glyph_id < first || glyph_id > last {
                continue;
            }
            let additional = read_u32(self.data, rec + 4)?;
            let sub_at = array_at
                .checked_add(additional as usize)
                .ok_or(Error::BadOffset)?;
            return self.locate_in_subtable(sub_at, first, last, glyph_id);
        }
        Ok(None)
    }

    /// Decode one IndexSubTable (any of the five formats) for one
    /// glyph in its `[first, last]` range.
    fn locate_in_subtable(
        &self,
        at: usize,
        first: u16,
        last: u16,
        glyph_id: u16,
    ) -> Result<Option<BitmapLocation>, Error> {
        debug_assert!(glyph_id >= first && glyph_id <= last);
        let data = self.data;
        let index_format = read_u16(data, at)?;
        let image_format = read_u16(data, at + 2)?;
        let image_data_offset = read_u32(data, at + 4)?;
        let rel = (glyph_id - first) as usize;

        let variable = |cur: u32, next: u32| -> Result<Option<BitmapLocation>, Error> {
            if next < cur {
                return Err(Error::BadStructure("EBLC/CBLC: offsetArray not ascending"));
            }
            if next == cur {
                // Zero data size: glyph absent from the range.
                return Ok(None);
            }
            Ok(Some(BitmapLocation {
                image_format,
                offset: image_data_offset.checked_add(cur).ok_or(Error::BadOffset)?,
                length: next - cur,
                metrics: None,
            }))
        };

        match index_format {
            // Format 1: Offset32 per glyph (+1 extra).
            1 => {
                let arr = at + 8;
                let cur = read_u32(data, arr + rel * 4)?;
                let next = read_u32(data, arr + rel * 4 + 4)?;
                variable(cur, next)
            }
            // Format 2: constant imageSize + shared BigGlyphMetrics.
            2 => {
                let image_size = read_u32(data, at + 8)?;
                let metrics = BigGlyphMetrics::parse(data, at + 12)?;
                let offset = image_data_offset
                    .checked_add(image_size.checked_mul(rel as u32).ok_or(Error::BadOffset)?)
                    .ok_or(Error::BadOffset)?;
                Ok(Some(BitmapLocation {
                    image_format,
                    offset,
                    length: image_size,
                    metrics: Some(metrics),
                }))
            }
            // Format 3: Offset16 per glyph (+1 extra).
            3 => {
                let arr = at + 8;
                let cur = read_u16(data, arr + rel * 2)? as u32;
                let next = read_u16(data, arr + rel * 2 + 2)? as u32;
                variable(cur, next)
            }
            // Format 4: sparse (glyphID, offset16) pairs, numGlyphs+1
            // entries (the extra one closes the last length).
            4 => {
                let num_glyphs = read_u32(data, at + 8)? as usize;
                let arr = at + 12;
                for j in 0..num_glyphs {
                    let gid = read_u16(data, arr + j * 4)?;
                    if gid != glyph_id {
                        continue;
                    }
                    let cur = read_u16(data, arr + j * 4 + 2)? as u32;
                    let next = read_u16(data, arr + (j + 1) * 4 + 2)? as u32;
                    return variable(cur, next);
                }
                Ok(None)
            }
            // Format 5: constant imageSize + shared metrics + sorted
            // sparse glyphIDArray.
            5 => {
                let image_size = read_u32(data, at + 8)?;
                let metrics = BigGlyphMetrics::parse(data, at + 12)?;
                let num_glyphs = read_u32(data, at + 12 + BigGlyphMetrics::LEN)? as usize;
                let arr = at + 16 + BigGlyphMetrics::LEN;
                for j in 0..num_glyphs {
                    let gid = read_u16(data, arr + j * 2)?;
                    if gid != glyph_id {
                        continue;
                    }
                    let offset = image_data_offset
                        .checked_add(image_size.checked_mul(j as u32).ok_or(Error::BadOffset)?)
                        .ok_or(Error::BadOffset)?;
                    return Ok(Some(BitmapLocation {
                        image_format,
                        offset,
                        length: image_size,
                        metrics: Some(metrics),
                    }));
                }
                Ok(None)
            }
            _ => Err(Error::BadStructure(
                "EBLC/CBLC: unknown IndexSubTable format",
            )),
        }
    }
}
