//! `sbix` — Standard bitmap graphics table
//! (ISO/IEC 14496-22:2019 §5.6.7).
//!
//! Provides per-glyph bitmap data in standard graphics formats (PNG /
//! JPEG / TIFF), organized as **strikes**: each strike targets one
//! PPEM size and device pixel density (PPI) and carries a
//! `glyphDataOffsets[numGlyphs + 1]` array (offsets from the start of
//! the strike header), so the data length for glyph N is
//! `offset[N+1] - offset[N]` — zero meaning "no bitmap for this glyph
//! in this strike".
//!
//! Glyph data is an 8-byte header — origin offsets + a `graphicType`
//! tag — followed by the embedded graphic. Three standard formats are
//! defined (`'png '`, `'jpg '`, `'tiff'`) plus the special `'dupe'`
//! type whose payload is a big-endian glyph ID to borrow bitmap data
//! from ([`SbixStrike::glyph_graphic_resolved`] follows those chains
//! with cycle protection). Apple's `'pdf '` / `'mask'` types are not
//! part of the OFF specification; unknown tags are surfaced as-is for
//! the caller to accept or reject.
//!
//! This crate does not decode the embedded PNG/JPEG/TIFF payloads —
//! it surfaces the raw bytes; image decoding belongs to the image
//! codec crates.

use crate::parser::{read_i16, read_tag, read_u16, read_u32};
use crate::Error;

/// `flags` bit 0: historically always set.
pub const SBIX_FLAG_ALWAYS_SET: u16 = 0x0001;
/// `flags` bit 1: draw the outline **and** the bitmap (outline overlaid
/// on top). Clear = draw only the bitmap for glyphs the table covers.
pub const SBIX_FLAG_DRAW_OUTLINES: u16 = 0x0002;

/// The `graphicType` of one glyph's embedded graphic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GraphicType {
    /// `'png '` — PNG data.
    Png,
    /// `'jpg '` — JPEG data.
    Jpg,
    /// `'tiff'` — TIFF data.
    Tiff,
    /// `'dupe'` — the payload is a big-endian glyph ID whose bitmap
    /// data should be used for this glyph.
    Dupe,
    /// Any other tag (e.g. Apple's `'pdf '` / `'mask'`, which the OFF
    /// specification does not support).
    Other([u8; 4]),
}

impl GraphicType {
    fn from_tag(tag: [u8; 4]) -> Self {
        match &tag {
            b"png " => Self::Png,
            b"jpg " => Self::Jpg,
            b"tiff" => Self::Tiff,
            b"dupe" => Self::Dupe,
            _ => Self::Other(tag),
        }
    }
}

/// One glyph's bitmap graphic within a strike.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GlyphGraphic<'a> {
    /// x-offset from the left edge of the graphic to the glyph origin
    /// (the x-coordinate of the baseline point at the glyph's left
    /// edge), in pixels.
    pub origin_offset_x: i16,
    /// y-offset from the **bottom** edge of the graphic to the glyph
    /// origin, in pixels.
    pub origin_offset_y: i16,
    /// Format of `data`.
    pub graphic_type: GraphicType,
    /// The embedded graphic payload (for [`GraphicType::Dupe`], a
    /// 2-byte big-endian glyph ID).
    pub data: &'a [u8],
}

/// One bitmap strike: a PPEM/PPI-targeted set of glyph graphics.
#[derive(Debug, Clone, Copy)]
pub struct SbixStrike<'a> {
    /// The whole `sbix`-relative strike data, starting at the strike
    /// header (glyph data offsets are relative to this).
    data: &'a [u8],
    /// PPEM size this strike was designed for.
    pub ppem: u16,
    /// Device pixel density (PPI) this strike was designed for.
    pub ppi: u16,
    /// Number of glyphs (`maxp.numGlyphs`); the offsets array holds
    /// `num_glyphs + 1` entries.
    num_glyphs: u16,
}

impl<'a> SbixStrike<'a> {
    /// The raw bitmap graphic for `glyph_id`, or `Ok(None)` when the
    /// strike has no data for the glyph (zero-length entry). `'dupe'`
    /// entries are returned as-is — see
    /// [`SbixStrike::glyph_graphic_resolved`].
    pub fn glyph_graphic(&self, glyph_id: u16) -> Result<Option<GlyphGraphic<'a>>, Error> {
        if glyph_id >= self.num_glyphs {
            return Err(Error::GlyphOutOfRange(glyph_id));
        }
        let at = 4 + glyph_id as usize * 4;
        let start = read_u32(self.data, at)? as usize;
        let end = read_u32(self.data, at + 4)? as usize;
        if end == start {
            return Ok(None);
        }
        // The glyph data header is 8 bytes (2 x int16 + Tag); a
        // non-empty entry shorter than that, a backwards range, or a
        // range past the table end is malformed.
        if end < start || end - start < 8 || end > self.data.len() {
            return Err(Error::BadStructure(
                "sbix: malformed glyphDataOffsets range",
            ));
        }
        Ok(Some(GlyphGraphic {
            origin_offset_x: read_i16(self.data, start)?,
            origin_offset_y: read_i16(self.data, start + 2)?,
            graphic_type: GraphicType::from_tag(read_tag(self.data, start + 4)?),
            data: &self.data[start + 8..end],
        }))
    }

    /// Like [`SbixStrike::glyph_graphic`], but `'dupe'` entries are
    /// followed (within this strike) until a concrete graphic or a
    /// missing entry is reached; a cycle or non-terminating chain is
    /// an error. The dupe target's full record is returned — "the
    /// bitmap data for the indicated glyph should be used for the
    /// current glyph" (§5.6.7.3).
    pub fn glyph_graphic_resolved(&self, glyph_id: u16) -> Result<Option<GlyphGraphic<'a>>, Error> {
        let mut gid = glyph_id;
        // A dupe chain longer than the glyph count necessarily loops.
        for _ in 0..=self.num_glyphs {
            match self.glyph_graphic(gid)? {
                Some(g) if g.graphic_type == GraphicType::Dupe => {
                    if g.data.len() < 2 {
                        return Err(Error::BadStructure("sbix: dupe payload shorter than 2"));
                    }
                    let next = u16::from_be_bytes([g.data[0], g.data[1]]);
                    if next == gid {
                        return Err(Error::BadStructure("sbix: dupe cycle"));
                    }
                    gid = next;
                }
                other => return Ok(other),
            }
        }
        Err(Error::BadStructure("sbix: dupe chain does not terminate"))
    }
}

/// A parsed `sbix` table.
#[derive(Debug)]
pub struct SbixTable<'a> {
    data: &'a [u8],
    version: u16,
    flags: u16,
    /// Validated strike offsets (each strike header + offsets array is
    /// in bounds).
    strike_offsets: Vec<u32>,
    num_glyphs: u16,
}

impl<'a> SbixTable<'a> {
    /// Parse an `sbix` table. `num_glyphs` comes from `maxp` (§5.6.7.4
    /// Table dependencies); it fixes the length of every strike's
    /// `glyphDataOffsets` array.
    pub fn parse(data: &'a [u8], num_glyphs: u16) -> Result<Self, Error> {
        let version = read_u16(data, 0)?;
        let flags = read_u16(data, 2)?;
        let num_strikes = read_u32(data, 4)? as usize;
        // Each strike offset is 4 bytes; bound the count by what fits.
        if num_strikes > data.len() / 4 {
            return Err(Error::BadStructure("sbix: numStrikes exceeds table size"));
        }
        let offsets_len = 4 + (num_glyphs as usize + 1) * 4;
        let mut strike_offsets = Vec::with_capacity(num_strikes);
        for i in 0..num_strikes {
            let off = read_u32(data, 8 + i * 4)?;
            let end = (off as usize)
                .checked_add(offsets_len)
                .ok_or(Error::BadOffset)?;
            if off == 0 || end > data.len() {
                return Err(Error::BadOffset);
            }
            strike_offsets.push(off);
        }
        Ok(Self {
            data,
            version,
            flags,
            strike_offsets,
            num_glyphs,
        })
    }

    /// The table version (1).
    pub fn version(&self) -> u16 {
        self.version
    }

    /// The raw `flags` field. See [`SBIX_FLAG_DRAW_OUTLINES`].
    pub fn flags(&self) -> u16 {
        self.flags
    }

    /// Whether bit 1 instructs the application to draw the glyph
    /// outline overlaid on top of the bitmap.
    pub fn draw_outlines(&self) -> bool {
        self.flags & SBIX_FLAG_DRAW_OUTLINES != 0
    }

    /// Number of bitmap strikes.
    pub fn num_strikes(&self) -> usize {
        self.strike_offsets.len()
    }

    /// Strike number `index` (table order), or `None` out of range.
    pub fn strike(&self, index: usize) -> Option<SbixStrike<'a>> {
        let off = *self.strike_offsets.get(index)? as usize;
        // Header in bounds by the parse-time check.
        let data = &self.data[off..];
        Some(SbixStrike {
            data,
            ppem: read_u16(data, 0).ok()?,
            ppi: read_u16(data, 2).ok()?,
            num_glyphs: self.num_glyphs,
        })
    }

    /// All strikes in table order.
    pub fn strikes(&self) -> impl Iterator<Item = SbixStrike<'a>> + '_ {
        (0..self.strike_offsets.len()).filter_map(|i| self.strike(i))
    }

    /// The strike best matching a requested `ppem`: an exact PPEM
    /// match if one exists, else the smallest strike larger than the
    /// request (per §5.6.7.2's closest-available-larger-size
    /// recommendation), else the largest strike. Ties (same PPEM,
    /// several PPIs) resolve to the highest PPI. `None` when the
    /// table has no strikes.
    pub fn best_strike(&self, ppem: u16) -> Option<SbixStrike<'a>> {
        let mut best: Option<SbixStrike<'a>> = None;
        for s in self.strikes() {
            let better = match &best {
                None => true,
                Some(b) => {
                    // Rank: smallest ppem >= request first, then
                    // largest ppem below the request.
                    let key = |x: &SbixStrike<'_>| {
                        if x.ppem >= ppem {
                            (0u8, (x.ppem - ppem) as u32)
                        } else {
                            (1u8, (ppem - x.ppem) as u32)
                        }
                    };
                    let (ks, kb) = (key(&s), key(b));
                    ks < kb || (ks == kb && s.ppi > b.ppi)
                }
            };
            if better {
                best = Some(s);
            }
        }
        best
    }
}
