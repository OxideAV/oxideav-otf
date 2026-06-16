//! `post` — PostScript Table.
//!
//! Spec: Microsoft / ISO/IEC 14496-22 OpenType `post` table
//! (`docs/text/opentype/otspec-post.html`). Provides additional
//! information needed to use OpenType fonts on PostScript printers,
//! including italic angle / underline metrics / fixed-pitch flag /
//! virtual-memory hints, and (for versions 1.0 / 2.0 / 2.5) the
//! PostScript names of every glyph.
//!
//! Version landscape (per the spec's "Versions" preamble):
//!
//! * **0x00010000 (1.0)** — 32-byte header only. Glyph names come from
//!   the standard Macintosh 258-glyph set; the font must contain
//!   exactly those 258 glyphs in standard order.
//! * **0x00020000 (2.0)** — 32-byte header + `numGlyphs` u16 +
//!   `glyphNameIndex[numGlyphs]` (u16) + Pascal-string `stringData`
//!   tail. Names below 258 reference the Macintosh standard set;
//!   names 258..65535 are `index − 258` into the Pascal-string array.
//! * **0x00025000 (2.5)** — deprecated. Header + `numGlyphs` u16 +
//!   `offset[numGlyphs]` (i8) into a reordering of the Mac standard
//!   set. Pure subset / reorder of the 258 glyphs.
//! * **0x00030000 (3.0)** — 32-byte header only, no glyph-name
//!   information is provided. OpenType-CFF1 fonts must use this
//!   version per the spec's "Versions" preamble.
//!
//! Header layout is identical across every version:
//!
//! ```text
//!   0  / 4 / version (Version16Dot16: 0x000m0n00)
//!   4  / 4 / italicAngle (Fixed: 16.16 signed)
//!   8  / 2 / underlinePosition (FWORD = i16)
//!  10  / 2 / underlineThickness (FWORD = i16)
//!  12  / 4 / isFixedPitch (uint32; non-zero = monospaced)
//!  16  / 4 / minMemType42 (uint32)
//!  20  / 4 / maxMemType42 (uint32)
//!  24  / 4 / minMemType1 (uint32)
//!  28  / 4 / maxMemType1 (uint32)
//! ```
//!
//! This implementation:
//!
//! * Decodes the 32-byte header for every version.
//! * For version 2.0, records the raw `numGlyphs` + the
//!   `glyphNameIndex` slice + the raw `stringData` Pascal-string tail
//!   so callers wanting **non-standard** glyph names (the `index ≥ 258`
//!   path) can iterate them without re-parsing.
//! * For version 2.5, records the raw `numGlyphs` + the offset slice.
//!
//! Resolution of `index < 258` glyph names (and the version-2.5
//! reordering) uses the 258-entry standard-Macintosh glyph-name table
//! ([`STANDARD_MAC_GLYPH_NAMES`]), transcribed from Apple's *TrueType
//! Reference Manual*, `post` table chapter ("`'post'` Format 1"), per
//! the clean-room note
//! `docs/text/opentype/post-standard-mac-glyph-names.md` (snapshot
//! `apple-chap6post.html`). The OpenType spec (`otspec-post.html`)
//! defers to Apple for this list. [`PostTable::glyph_name`] applies it
//! across formats 1.0, 2.0 (`glyphNameIndex < 258`), and 2.5
//! (`glyph_id + offset`). The non-standard Pascal-string names
//! (`glyphNameIndex >= 258`) are resolved from the string-data tail
//! exposed by [`PostTable::name_string`].

use crate::parser::{read_i16, read_u16, read_u32};
use crate::Error;

/// The 258 standard Macintosh glyph names, in their canonical ordering.
///
/// Index 0 = `.notdef`, 1 = `.null`, 2 = `nonmarkingreturn`, …,
/// 257 = `dcroat`. This ordering is referenced by `post` table formats
/// 1.0, 2.0 (`glyphNameIndex < 258`), and 2.5. Transcribed verbatim
/// from the clean-room note
/// `docs/text/opentype/post-standard-mac-glyph-names.md` (Apple
/// *TrueType Reference Manual* `post` chapter; the OpenType spec defers
/// to Apple for the list).
pub static STANDARD_MAC_GLYPH_NAMES: [&str; 258] = [
    ".notdef",
    ".null",
    "nonmarkingreturn",
    "space",
    "exclam",
    "quotedbl",
    "numbersign",
    "dollar",
    "percent",
    "ampersand",
    "quotesingle",
    "parenleft",
    "parenright",
    "asterisk",
    "plus",
    "comma",
    "hyphen",
    "period",
    "slash",
    "zero",
    "one",
    "two",
    "three",
    "four",
    "five",
    "six",
    "seven",
    "eight",
    "nine",
    "colon",
    "semicolon",
    "less",
    "equal",
    "greater",
    "question",
    "at",
    "A",
    "B",
    "C",
    "D",
    "E",
    "F",
    "G",
    "H",
    "I",
    "J",
    "K",
    "L",
    "M",
    "N",
    "O",
    "P",
    "Q",
    "R",
    "S",
    "T",
    "U",
    "V",
    "W",
    "X",
    "Y",
    "Z",
    "bracketleft",
    "backslash",
    "bracketright",
    "asciicircum",
    "underscore",
    "grave",
    "a",
    "b",
    "c",
    "d",
    "e",
    "f",
    "g",
    "h",
    "i",
    "j",
    "k",
    "l",
    "m",
    "n",
    "o",
    "p",
    "q",
    "r",
    "s",
    "t",
    "u",
    "v",
    "w",
    "x",
    "y",
    "z",
    "braceleft",
    "bar",
    "braceright",
    "asciitilde",
    "Adieresis",
    "Aring",
    "Ccedilla",
    "Eacute",
    "Ntilde",
    "Odieresis",
    "Udieresis",
    "aacute",
    "agrave",
    "acircumflex",
    "adieresis",
    "atilde",
    "aring",
    "ccedilla",
    "eacute",
    "egrave",
    "ecircumflex",
    "edieresis",
    "iacute",
    "igrave",
    "icircumflex",
    "idieresis",
    "ntilde",
    "oacute",
    "ograve",
    "ocircumflex",
    "odieresis",
    "otilde",
    "uacute",
    "ugrave",
    "ucircumflex",
    "udieresis",
    "dagger",
    "degree",
    "cent",
    "sterling",
    "section",
    "bullet",
    "paragraph",
    "germandbls",
    "registered",
    "copyright",
    "trademark",
    "acute",
    "dieresis",
    "notequal",
    "AE",
    "Oslash",
    "infinity",
    "plusminus",
    "lessequal",
    "greaterequal",
    "yen",
    "mu",
    "partialdiff",
    "summation",
    "product",
    "pi",
    "integral",
    "ordfeminine",
    "ordmasculine",
    "Omega",
    "ae",
    "oslash",
    "questiondown",
    "exclamdown",
    "logicalnot",
    "radical",
    "florin",
    "approxequal",
    "Delta",
    "guillemotleft",
    "guillemotright",
    "ellipsis",
    "nonbreakingspace",
    "Agrave",
    "Atilde",
    "Otilde",
    "OE",
    "oe",
    "endash",
    "emdash",
    "quotedblleft",
    "quotedblright",
    "quoteleft",
    "quoteright",
    "divide",
    "lozenge",
    "ydieresis",
    "Ydieresis",
    "fraction",
    "currency",
    "guilsinglleft",
    "guilsinglright",
    "fi",
    "fl",
    "daggerdbl",
    "periodcentered",
    "quotesinglbase",
    "quotedblbase",
    "perthousand",
    "Acircumflex",
    "Ecircumflex",
    "Aacute",
    "Edieresis",
    "Egrave",
    "Iacute",
    "Icircumflex",
    "Idieresis",
    "Igrave",
    "Oacute",
    "Ocircumflex",
    "apple",
    "Ograve",
    "Uacute",
    "Ucircumflex",
    "Ugrave",
    "dotlessi",
    "circumflex",
    "tilde",
    "macron",
    "breve",
    "dotaccent",
    "ring",
    "cedilla",
    "hungarumlaut",
    "ogonek",
    "caron",
    "Lslash",
    "lslash",
    "Scaron",
    "scaron",
    "Zcaron",
    "zcaron",
    "brokenbar",
    "Eth",
    "eth",
    "Yacute",
    "yacute",
    "Thorn",
    "thorn",
    "minus",
    "multiply",
    "onesuperior",
    "twosuperior",
    "threesuperior",
    "onehalf",
    "onequarter",
    "threequarters",
    "franc",
    "Gbreve",
    "gbreve",
    "Idotaccent",
    "Scedilla",
    "scedilla",
    "Cacute",
    "cacute",
    "Ccaron",
    "ccaron",
    "dcroat",
];

/// Look up the standard Macintosh glyph name at `index` (0..=257).
///
/// Returns `None` for `index >= 258`. This is the table referenced by
/// `post` formats 1.0, 2.0 (`glyphNameIndex < 258`), and 2.5.
pub fn standard_mac_glyph_name(index: u16) -> Option<&'static str> {
    STANDARD_MAC_GLYPH_NAMES.get(index as usize).copied()
}

/// Parsed `post` table.
///
/// Header fields (`italic_angle` … `max_mem_type1`) are populated for
/// every version. For versions 2.0 / 2.5 the additional tail data is
/// stored as borrowed slices into the table bytes; for versions 1.0 /
/// 3.0 the tail-related getters are empty.
#[derive(Debug, Clone, Copy)]
pub struct PostTable<'a> {
    raw_version: u32,
    italic_angle_fixed: i32,
    underline_position: i16,
    underline_thickness: i16,
    is_fixed_pitch_raw: u32,
    min_mem_type42: u32,
    max_mem_type42: u32,
    min_mem_type1: u32,
    max_mem_type1: u32,
    /// For format 2.0 / 2.5: number of glyphs declared in this table.
    /// Zero for format 1.0 / 3.0 (no glyph array).
    num_glyphs: u16,
    /// Format 2.0 only: `glyphNameIndex[numGlyphs]` raw big-endian u16
    /// pairs (length = `2 * num_glyphs`). Empty otherwise.
    name_index_bytes: &'a [u8],
    /// Format 2.0 only: Pascal-string string data tail. Empty
    /// otherwise.
    string_data: &'a [u8],
    /// Format 2.5 only: `offset[numGlyphs]` signed-byte array. Empty
    /// otherwise.
    offset_bytes: &'a [u8],
}

/// `post` table format discriminant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PostFormat {
    /// `0x00010000` — names come from the 258-glyph standard
    /// Macintosh set in standard order; no per-glyph data in the
    /// table.
    V1_0,
    /// `0x00020000` — full glyph-name table (Mac standard + Pascal
    /// strings).
    V2_0,
    /// `0x00025000` — deprecated. Reordering of the Mac standard set
    /// via signed-byte offsets.
    V2_5,
    /// `0x00030000` — no glyph-name information provided. Required
    /// for OpenType-CFF1 fonts.
    V3_0,
    /// Some other Version16Dot16 value (e.g. Apple's 4.0 extension,
    /// which the OpenType spec explicitly notes is "not supported in
    /// OpenType"). The header still decoded; the format-specific
    /// tail is treated as absent.
    Other(u32),
}

impl<'a> PostTable<'a> {
    /// Parse the table from a raw `post` byte slice.
    pub fn parse(bytes: &'a [u8]) -> Result<Self, Error> {
        if bytes.len() < 32 {
            return Err(Error::UnexpectedEof);
        }
        let raw_version = read_u32(bytes, 0)?;
        // `Fixed` is a 32-bit signed fixed-point with the integer
        // part in the upper 16 bits and a 16-bit fractional part.
        let italic_angle_fixed = read_u32(bytes, 4)? as i32;
        let underline_position = read_i16(bytes, 8)?;
        let underline_thickness = read_i16(bytes, 10)?;
        let is_fixed_pitch_raw = read_u32(bytes, 12)?;
        let min_mem_type42 = read_u32(bytes, 16)?;
        let max_mem_type42 = read_u32(bytes, 20)?;
        let min_mem_type1 = read_u32(bytes, 24)?;
        let max_mem_type1 = read_u32(bytes, 28)?;

        let (num_glyphs, name_index_bytes, string_data, offset_bytes) = match raw_version {
            0x0002_0000 => {
                // Header + numGlyphs (u16) + glyphNameIndex[numGlyphs]
                // (u16 each) + Pascal-string tail.
                if bytes.len() < 34 {
                    return Err(Error::BadStructure("post 2.0 truncated before numGlyphs"));
                }
                let n = read_u16(bytes, 32)?;
                let idx_start = 34usize;
                let idx_end = idx_start
                    .checked_add((n as usize).checked_mul(2).ok_or(Error::BadStructure(
                        "post 2.0 glyphNameIndex length overflow",
                    ))?)
                    .ok_or(Error::BadStructure(
                        "post 2.0 glyphNameIndex length overflow",
                    ))?;
                if bytes.len() < idx_end {
                    return Err(Error::BadStructure(
                        "post 2.0 truncated inside glyphNameIndex",
                    ));
                }
                let name_index = &bytes[idx_start..idx_end];
                let strings = &bytes[idx_end..];
                (n, name_index, strings, &[][..])
            }
            0x0002_5000 => {
                // Header + numGlyphs (u16) + offset[numGlyphs] (i8).
                if bytes.len() < 34 {
                    return Err(Error::BadStructure("post 2.5 truncated before numGlyphs"));
                }
                let n = read_u16(bytes, 32)?;
                let off_start = 34usize;
                let off_end = off_start
                    .checked_add(n as usize)
                    .ok_or(Error::BadStructure("post 2.5 offset length overflow"))?;
                if bytes.len() < off_end {
                    return Err(Error::BadStructure("post 2.5 truncated inside offsets"));
                }
                let offsets = &bytes[off_start..off_end];
                (n, &[][..], &[][..], offsets)
            }
            _ => (0u16, &[][..], &[][..], &[][..]),
        };

        Ok(Self {
            raw_version,
            italic_angle_fixed,
            underline_position,
            underline_thickness,
            is_fixed_pitch_raw,
            min_mem_type42,
            max_mem_type42,
            min_mem_type1,
            max_mem_type1,
            num_glyphs,
            name_index_bytes,
            string_data,
            offset_bytes,
        })
    }

    /// The on-disk `Version16Dot16` value as raw `u32`.
    pub fn raw_version(&self) -> u32 {
        self.raw_version
    }

    /// Decoded format.
    pub fn format(&self) -> PostFormat {
        match self.raw_version {
            0x0001_0000 => PostFormat::V1_0,
            0x0002_0000 => PostFormat::V2_0,
            0x0002_5000 => PostFormat::V2_5,
            0x0003_0000 => PostFormat::V3_0,
            other => PostFormat::Other(other),
        }
    }

    /// Italic angle in counter-clockwise degrees from the vertical
    /// (spec: "Zero for upright text, negative for text that leans to
    /// the right (forward)"). Decoded from the on-disk `Fixed`
    /// 16.16 signed fixed-point value.
    pub fn italic_angle(&self) -> f64 {
        self.italic_angle_fixed as f64 / 65536.0
    }

    /// Raw 16.16 `Fixed` italic-angle value, for callers that want to
    /// inspect the exact on-disk bits.
    pub fn italic_angle_fixed(&self) -> i32 {
        self.italic_angle_fixed
    }

    /// "Suggested y-coordinate of the top of the underline" (spec).
    /// Note the spec adds: the PostScript-language `UnderlinePosition`
    /// FontInfo key specifies the *center* of the underline; the
    /// `post` table's field does **not**.
    pub fn underline_position(&self) -> i16 {
        self.underline_position
    }

    /// Underline thickness in font units. The spec recommends matching
    /// the U+005F LOW LINE thickness + the strikeout thickness from
    /// `OS/2`.
    pub fn underline_thickness(&self) -> i16 {
        self.underline_thickness
    }

    /// `true` when the font is monospaced. The on-disk field is a
    /// `uint32` and the spec says "Set to 0 if the font is
    /// proportionally spaced, non-zero if the font is not
    /// proportionally spaced (i.e. monospaced)" — so any non-zero
    /// value rounds up to `true`.
    pub fn is_fixed_pitch(&self) -> bool {
        self.is_fixed_pitch_raw != 0
    }

    /// Raw `isFixedPitch` u32, for callers that want the on-disk
    /// integer (some legacy tooling encodes additional meaning in the
    /// high bits).
    pub fn is_fixed_pitch_raw(&self) -> u32 {
        self.is_fixed_pitch_raw
    }

    /// `minMemType42`: minimum memory usage when downloaded as a
    /// Type 42 font. Per spec: zero when not known.
    pub fn min_mem_type42(&self) -> u32 {
        self.min_mem_type42
    }

    /// `maxMemType42`: maximum memory usage when downloaded as a
    /// Type 42 font. Zero when not known.
    pub fn max_mem_type42(&self) -> u32 {
        self.max_mem_type42
    }

    /// `minMemType1`: minimum memory usage when downloaded as a
    /// Type 1 font. Zero when not known.
    pub fn min_mem_type1(&self) -> u32 {
        self.min_mem_type1
    }

    /// `maxMemType1`: maximum memory usage when downloaded as a
    /// Type 1 font. Zero when not known.
    pub fn max_mem_type1(&self) -> u32 {
        self.max_mem_type1
    }

    /// Number of glyphs declared by formats 2.0 / 2.5. Zero for
    /// formats 1.0 / 3.0 (and the `Other` fallback).
    pub fn num_glyphs(&self) -> u16 {
        self.num_glyphs
    }

    /// Format 2.0 only: raw `glyphNameIndex` value for `glyph_id`.
    ///
    /// * `Some(i)` where `i < 258` — name is the `i`-th entry in the
    ///   standard Macintosh 258-glyph set (the table itself is not
    ///   shipped in this crate; see module docs).
    /// * `Some(i)` where `i >= 258` — name is the `i - 258`-th Pascal
    ///   string in the table's string-data tail; reachable through
    ///   [`PostTable::name_string`].
    /// * `None` — format is not 2.0, or `glyph_id` is past the
    ///   table's `num_glyphs`.
    pub fn name_index(&self, glyph_id: u16) -> Option<u16> {
        if self.raw_version != 0x0002_0000 || glyph_id >= self.num_glyphs {
            return None;
        }
        let off = (glyph_id as usize) * 2;
        let s = self.name_index_bytes.get(off..off + 2)?;
        Some(u16::from_be_bytes([s[0], s[1]]))
    }

    /// Format 2.0 only: the `pascal_index`-th Pascal-style name from
    /// the string-data tail, decoded as UTF-8 bytes (PostScript names
    /// are ASCII by convention but the spec doesn't *forbid* high
    /// bytes, so we return the raw bytes and let the caller decide).
    ///
    /// `pascal_index` is `0`-based: `pascal_index = 0` is the *first*
    /// Pascal string after the `glyphNameIndex` array, i.e. the name
    /// for the `glyph_name_index == 258` glyph.
    ///
    /// `None` if format is not 2.0, the index walks past the end of
    /// the string-data tail, or a Pascal length byte points past the
    /// end of the table.
    pub fn name_string(&self, pascal_index: u16) -> Option<&'a [u8]> {
        if self.raw_version != 0x0002_0000 {
            return None;
        }
        let mut cursor = 0usize;
        for current in 0..=pascal_index as usize {
            let len_byte = *self.string_data.get(cursor)?;
            let start = cursor + 1;
            let end = start.checked_add(len_byte as usize)?;
            if end > self.string_data.len() {
                return None;
            }
            if current == pascal_index as usize {
                return Some(&self.string_data[start..end]);
            }
            cursor = end;
        }
        None
    }

    /// Format 2.5 only: the signed-byte standard-Mac-glyph offset for
    /// `glyph_id`. The byte is added to `glyph_id` to obtain an index
    /// into the standard Mac glyph set (see module docs). Returns
    /// `None` for formats other than 2.5 or for out-of-range
    /// `glyph_id`.
    pub fn standard_offset(&self, glyph_id: u16) -> Option<i8> {
        if self.raw_version != 0x0002_5000 || glyph_id >= self.num_glyphs {
            return None;
        }
        self.offset_bytes
            .get(glyph_id as usize)
            .copied()
            .map(|b| b as i8)
    }

    /// Resolve the PostScript glyph name for `glyph_id`, applying the
    /// standard Macintosh glyph-name table where the format calls for
    /// it. This unifies the three name-bearing formats:
    ///
    /// * **1.0** — every glyph's name is its glyph ID's entry in the
    ///   258-name standard set; `Some(Standard)` for `glyph_id < 258`,
    ///   `None` beyond (a conforming 1.0 font has exactly 258 glyphs).
    /// * **2.0** — looks up `glyphNameIndex[glyph_id]`. `< 258` →
    ///   `Some(Standard)`; `>= 258` → `Some(Custom)` (the
    ///   `(idx - 258)`-th Pascal string from the string-data tail).
    /// * **2.5** — resolves `standard_index = glyph_id + offset[glyph_id]`
    ///   and names it from the standard set; `Some(Standard)` when the
    ///   resolved index lands in `0..258`, else `None`.
    ///
    /// `None` for formats 3.0 / `Other`, for `glyph_id` past
    /// `num_glyphs`, when a 2.0 Pascal string is malformed, or when a
    /// 2.5 resolved index is out of range.
    pub fn glyph_name(&self, glyph_id: u16) -> Option<PostGlyphName<'a>> {
        match self.raw_version {
            0x0001_0000 => standard_mac_glyph_name(glyph_id).map(PostGlyphName::Standard),
            0x0002_0000 => {
                let idx = self.name_index(glyph_id)?;
                if idx < 258 {
                    standard_mac_glyph_name(idx).map(PostGlyphName::Standard)
                } else {
                    self.name_string(idx - 258).map(PostGlyphName::Custom)
                }
            }
            0x0002_5000 => {
                let off = self.standard_offset(glyph_id)?;
                // standard_index = glyph_id + offset (signed). Resolve
                // through i32 to avoid wraparound on extreme deltas.
                let std_index = (glyph_id as i32) + (off as i32);
                if !(0..258).contains(&std_index) {
                    return None;
                }
                standard_mac_glyph_name(std_index as u16).map(PostGlyphName::Standard)
            }
            _ => None,
        }
    }
}

/// A glyph name resolved from a `post` table.
///
/// Distinguishes a name drawn from the static 258-entry standard
/// Macintosh set (`'static`, always valid UTF-8) from a font-supplied
/// custom Pascal-string name (raw bytes borrowed from the table; ASCII
/// by PostScript convention but not guaranteed UTF-8 by the spec).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PostGlyphName<'a> {
    /// A name from [`STANDARD_MAC_GLYPH_NAMES`].
    Standard(&'static str),
    /// A font-supplied custom name (raw Pascal-string bytes).
    Custom(&'a [u8]),
}

impl<'a> PostGlyphName<'a> {
    /// The name as raw bytes (standard names are returned as their
    /// UTF-8 byte view).
    pub fn as_bytes(&self) -> &'a [u8] {
        match self {
            PostGlyphName::Standard(s) => s.as_bytes(),
            PostGlyphName::Custom(b) => b,
        }
    }

    /// The name as `&str` when valid UTF-8. Standard names are always
    /// `Some`; custom names are `Some` only when their on-disk bytes
    /// are UTF-8-clean.
    pub fn as_str(&self) -> Option<&'a str> {
        match self {
            PostGlyphName::Standard(s) => Some(s),
            PostGlyphName::Custom(b) => core::str::from_utf8(b).ok(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a 32-byte `post` header with the given version.
    fn build_header(version: u32) -> Vec<u8> {
        let mut b = vec![0u8; 32];
        b[0..4].copy_from_slice(&version.to_be_bytes());
        // italicAngle = -12.0 in 16.16 fixed = -12 * 65536 = -786432.
        b[4..8].copy_from_slice(&(-786_432i32 as u32).to_be_bytes());
        // underlinePosition = -75 (typographic)
        b[8..10].copy_from_slice(&(-75i16).to_be_bytes());
        // underlineThickness = 50
        b[10..12].copy_from_slice(&(50i16).to_be_bytes());
        // isFixedPitch = 0 (proportional)
        b[12..16].copy_from_slice(&0u32.to_be_bytes());
        // minMemType42 / maxMemType42 / minMemType1 / maxMemType1 (4 zeros)
        // already zero from vec! init.
        b
    }

    #[test]
    fn parses_v3_header() {
        let bytes = build_header(0x0003_0000);
        let p = PostTable::parse(&bytes).unwrap();
        assert_eq!(p.format(), PostFormat::V3_0);
        assert_eq!(p.raw_version(), 0x0003_0000);
        // -786432 / 65536 = -12.0
        assert!((p.italic_angle() - -12.0).abs() < 1e-12);
        assert_eq!(p.italic_angle_fixed(), -786_432);
        assert_eq!(p.underline_position(), -75);
        assert_eq!(p.underline_thickness(), 50);
        assert!(!p.is_fixed_pitch());
        assert_eq!(p.num_glyphs(), 0);
    }

    #[test]
    fn parses_v1_header() {
        let bytes = build_header(0x0001_0000);
        let p = PostTable::parse(&bytes).unwrap();
        assert_eq!(p.format(), PostFormat::V1_0);
        assert_eq!(p.num_glyphs(), 0);
        // No format-specific tail.
        assert_eq!(p.name_index(0), None);
        assert_eq!(p.standard_offset(0), None);
        assert_eq!(p.name_string(0), None);
    }

    #[test]
    fn fixed_pitch_decodes_nonzero() {
        let mut bytes = build_header(0x0003_0000);
        bytes[12..16].copy_from_slice(&1u32.to_be_bytes());
        let p = PostTable::parse(&bytes).unwrap();
        assert!(p.is_fixed_pitch());
        assert_eq!(p.is_fixed_pitch_raw(), 1);
    }

    #[test]
    fn fixed_pitch_decodes_high_bit() {
        // Some legacy tooling sets the high bit; spec says
        // "non-zero = monospaced" so any non-zero rounds to true.
        let mut bytes = build_header(0x0003_0000);
        bytes[12..16].copy_from_slice(&0x8000_0000u32.to_be_bytes());
        let p = PostTable::parse(&bytes).unwrap();
        assert!(p.is_fixed_pitch());
        assert_eq!(p.is_fixed_pitch_raw(), 0x8000_0000);
    }

    #[test]
    fn italic_angle_decodes_fractional() {
        let mut bytes = build_header(0x0003_0000);
        // -9.5 in 16.16 fixed = -9.5 * 65536 = -622592.
        bytes[4..8].copy_from_slice(&(-622_592i32 as u32).to_be_bytes());
        let p = PostTable::parse(&bytes).unwrap();
        assert!((p.italic_angle() - -9.5).abs() < 1e-12);
    }

    #[test]
    fn mem_fields_decode() {
        let mut bytes = build_header(0x0003_0000);
        bytes[16..20].copy_from_slice(&100u32.to_be_bytes());
        bytes[20..24].copy_from_slice(&200u32.to_be_bytes());
        bytes[24..28].copy_from_slice(&300u32.to_be_bytes());
        bytes[28..32].copy_from_slice(&400u32.to_be_bytes());
        let p = PostTable::parse(&bytes).unwrap();
        assert_eq!(p.min_mem_type42(), 100);
        assert_eq!(p.max_mem_type42(), 200);
        assert_eq!(p.min_mem_type1(), 300);
        assert_eq!(p.max_mem_type1(), 400);
    }

    #[test]
    fn rejects_short_buffer() {
        let bytes = vec![0u8; 16];
        assert!(matches!(
            PostTable::parse(&bytes),
            Err(Error::UnexpectedEof)
        ));
    }

    #[test]
    fn other_version_decodes_header_only() {
        // 4.0 (Apple extension; not supported in OpenType per spec)
        let bytes = build_header(0x0004_0000);
        let p = PostTable::parse(&bytes).unwrap();
        assert_eq!(p.format(), PostFormat::Other(0x0004_0000));
        assert_eq!(p.num_glyphs(), 0);
        assert_eq!(p.name_index(0), None);
    }

    /// Build a complete format-2.0 `post` table with two glyphs:
    ///
    /// * glyph 0 → glyphNameIndex = 0 (standard Mac glyph index 0,
    ///   which the spec says is `.notdef`)
    /// * glyph 1 → glyphNameIndex = 258 (first Pascal string)
    ///
    /// Pascal-string tail: `["MyGlyph"]`.
    fn build_v2(g0_idx: u16, g1_idx: u16) -> Vec<u8> {
        let mut b = build_header(0x0002_0000);
        // numGlyphs = 2
        b.extend_from_slice(&2u16.to_be_bytes());
        // glyphNameIndex[0..2]
        b.extend_from_slice(&g0_idx.to_be_bytes());
        b.extend_from_slice(&g1_idx.to_be_bytes());
        // Pascal-string tail: one string "MyGlyph" (length 7).
        b.push(7);
        b.extend_from_slice(b"MyGlyph");
        b
    }

    #[test]
    fn v2_parses_header_and_indices() {
        let bytes = build_v2(0, 258);
        let p = PostTable::parse(&bytes).unwrap();
        assert_eq!(p.format(), PostFormat::V2_0);
        assert_eq!(p.num_glyphs(), 2);
        assert_eq!(p.name_index(0), Some(0));
        assert_eq!(p.name_index(1), Some(258));
        // Past num_glyphs → None.
        assert_eq!(p.name_index(2), None);
    }

    #[test]
    fn v2_resolves_pascal_string() {
        let bytes = build_v2(0, 258);
        let p = PostTable::parse(&bytes).unwrap();
        // glyph 1's glyphNameIndex is 258; pascal index = 258 - 258 = 0.
        let name = p.name_string(0).unwrap();
        assert_eq!(name, b"MyGlyph");
        // Past end of Pascal-string tail → None.
        assert!(p.name_string(1).is_none());
    }

    #[test]
    fn v2_resolves_multiple_pascal_strings() {
        // Three Pascal strings: "A", "BB", "CCC".
        let mut b = build_header(0x0002_0000);
        b.extend_from_slice(&3u16.to_be_bytes());
        // glyphNameIndex = [258, 259, 260] (all custom names).
        b.extend_from_slice(&258u16.to_be_bytes());
        b.extend_from_slice(&259u16.to_be_bytes());
        b.extend_from_slice(&260u16.to_be_bytes());
        b.push(1);
        b.extend_from_slice(b"A");
        b.push(2);
        b.extend_from_slice(b"BB");
        b.push(3);
        b.extend_from_slice(b"CCC");
        let p = PostTable::parse(&b).unwrap();
        assert_eq!(p.name_string(0).unwrap(), b"A");
        assert_eq!(p.name_string(1).unwrap(), b"BB");
        assert_eq!(p.name_string(2).unwrap(), b"CCC");
        assert!(p.name_string(3).is_none());
    }

    #[test]
    fn v2_rejects_truncated_name_index() {
        // numGlyphs claims 4, but only 2 u16 follow → BadStructure.
        let mut b = build_header(0x0002_0000);
        b.extend_from_slice(&4u16.to_be_bytes());
        b.extend_from_slice(&0u16.to_be_bytes());
        b.extend_from_slice(&0u16.to_be_bytes());
        // Missing 2 more u16 entries.
        assert!(matches!(PostTable::parse(&b), Err(Error::BadStructure(_))));
    }

    #[test]
    fn v2_rejects_pascal_length_past_end() {
        // numGlyphs = 1, glyphNameIndex = [258], then a length byte
        // claiming 50 chars but only 1 byte follows.
        let mut b = build_header(0x0002_0000);
        b.extend_from_slice(&1u16.to_be_bytes());
        b.extend_from_slice(&258u16.to_be_bytes());
        b.push(50);
        b.push(b'x');
        let p = PostTable::parse(&b).unwrap();
        // Parsing succeeds (the tail isn't sanity-walked at parse
        // time), but resolution returns None per spec-defensive
        // behaviour.
        assert!(p.name_string(0).is_none());
    }

    #[test]
    fn v25_decodes_offset_array() {
        // numGlyphs = 3, offset = [+36, +36, +36] per the spec's
        // worked example (A, B, C at standard positions 37, 38, 39).
        let mut b = build_header(0x0002_5000);
        b.extend_from_slice(&3u16.to_be_bytes());
        b.push(36);
        b.push(36);
        b.push(36);
        let p = PostTable::parse(&b).unwrap();
        assert_eq!(p.format(), PostFormat::V2_5);
        assert_eq!(p.num_glyphs(), 3);
        assert_eq!(p.standard_offset(0), Some(36));
        assert_eq!(p.standard_offset(1), Some(36));
        assert_eq!(p.standard_offset(2), Some(36));
        assert_eq!(p.standard_offset(3), None);
    }

    #[test]
    fn v25_decodes_negative_offset() {
        let mut b = build_header(0x0002_5000);
        b.extend_from_slice(&1u16.to_be_bytes());
        b.push(0xFEu8); // -2 as i8
        let p = PostTable::parse(&b).unwrap();
        assert_eq!(p.standard_offset(0), Some(-2));
    }

    #[test]
    fn v25_rejects_truncated_offset_array() {
        let mut b = build_header(0x0002_5000);
        b.extend_from_slice(&5u16.to_be_bytes());
        // Only 2 offset bytes follow instead of 5.
        b.push(0);
        b.push(0);
        assert!(matches!(PostTable::parse(&b), Err(Error::BadStructure(_))));
    }

    #[test]
    fn standard_table_is_well_formed() {
        // 258 entries; key landmarks from the staged note.
        assert_eq!(STANDARD_MAC_GLYPH_NAMES.len(), 258);
        assert_eq!(STANDARD_MAC_GLYPH_NAMES[0], ".notdef");
        assert_eq!(STANDARD_MAC_GLYPH_NAMES[1], ".null");
        assert_eq!(STANDARD_MAC_GLYPH_NAMES[2], "nonmarkingreturn");
        assert_eq!(STANDARD_MAC_GLYPH_NAMES[3], "space");
        assert_eq!(STANDARD_MAC_GLYPH_NAMES[36], "A");
        assert_eq!(STANDARD_MAC_GLYPH_NAMES[68], "a");
        assert_eq!(STANDARD_MAC_GLYPH_NAMES[192], "fi");
        assert_eq!(STANDARD_MAC_GLYPH_NAMES[193], "fl");
        assert_eq!(STANDARD_MAC_GLYPH_NAMES[210], "apple");
        assert_eq!(STANDARD_MAC_GLYPH_NAMES[257], "dcroat");
        // All distinct.
        let mut sorted: Vec<&str> = STANDARD_MAC_GLYPH_NAMES.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), 258, "all 258 standard names must be distinct");
    }

    #[test]
    fn standard_mac_glyph_name_bounds() {
        assert_eq!(standard_mac_glyph_name(0), Some(".notdef"));
        assert_eq!(standard_mac_glyph_name(257), Some("dcroat"));
        assert_eq!(standard_mac_glyph_name(258), None);
        assert_eq!(standard_mac_glyph_name(u16::MAX), None);
    }

    #[test]
    fn v1_resolves_standard_names_by_gid() {
        let bytes = build_header(0x0001_0000);
        let p = PostTable::parse(&bytes).unwrap();
        // Glyph ID is its own standard index in format 1.0.
        assert_eq!(p.glyph_name(0), Some(PostGlyphName::Standard(".notdef")));
        assert_eq!(p.glyph_name(36), Some(PostGlyphName::Standard("A")));
        assert_eq!(p.glyph_name(257), Some(PostGlyphName::Standard("dcroat")));
        // Beyond the standard set → None.
        assert_eq!(p.glyph_name(258), None);
    }

    #[test]
    fn v2_glyph_name_resolves_standard_and_custom() {
        // glyph 0 → standard index 36 ("A"); glyph 1 → custom "MyGlyph".
        let bytes = build_v2(36, 258);
        let p = PostTable::parse(&bytes).unwrap();
        assert_eq!(p.glyph_name(0), Some(PostGlyphName::Standard("A")));
        assert_eq!(p.glyph_name(1), Some(PostGlyphName::Custom(b"MyGlyph")));
        // Custom name as_str round-trips.
        assert_eq!(p.glyph_name(1).unwrap().as_str(), Some("MyGlyph"));
        // Past num_glyphs → None.
        assert_eq!(p.glyph_name(2), None);
    }

    #[test]
    fn v25_glyph_name_applies_offset() {
        // numGlyphs = 3, offset = [+36, +36, +36] → standard indices
        // 36, 37, 38 = "A", "B", "C" (the spec's worked example shape).
        let mut b = build_header(0x0002_5000);
        b.extend_from_slice(&3u16.to_be_bytes());
        b.push(36);
        b.push(36);
        b.push(36);
        let p = PostTable::parse(&b).unwrap();
        assert_eq!(p.glyph_name(0), Some(PostGlyphName::Standard("A")));
        assert_eq!(p.glyph_name(1), Some(PostGlyphName::Standard("B")));
        assert_eq!(p.glyph_name(2), Some(PostGlyphName::Standard("C")));
        assert_eq!(p.glyph_name(3), None);
    }

    #[test]
    fn v25_glyph_name_rejects_out_of_range_resolved_index() {
        // glyph 0 offset 0 → index 0 (".notdef"); glyph 1 offset -1 → 0.
        let mut b = build_header(0x0002_5000);
        b.extend_from_slice(&2u16.to_be_bytes());
        b.push(0u8);
        b.push(0xFFu8); // -1 → 1 + (-1) = 0
        let p = PostTable::parse(&b).unwrap();
        assert_eq!(p.glyph_name(0), Some(PostGlyphName::Standard(".notdef")));
        assert_eq!(p.glyph_name(1), Some(PostGlyphName::Standard(".notdef")));
        // A negative resolved index → None.
        let mut b3 = build_header(0x0002_5000);
        b3.extend_from_slice(&1u16.to_be_bytes());
        b3.push(0xFFu8); // glyph 0 + (-1) = -1 → out of range
        let p3 = PostTable::parse(&b3).unwrap();
        assert_eq!(p3.glyph_name(0), None);
    }

    #[test]
    fn v3_glyph_name_is_none() {
        let bytes = build_header(0x0003_0000);
        let p = PostTable::parse(&bytes).unwrap();
        assert_eq!(p.glyph_name(0), None);
        assert_eq!(p.glyph_name(36), None);
    }

    #[test]
    fn post_glyph_name_kind_accessors() {
        let std = PostGlyphName::Standard("A");
        assert_eq!(std.as_bytes(), b"A");
        assert_eq!(std.as_str(), Some("A"));
        let custom = PostGlyphName::Custom(b"foo");
        assert_eq!(custom.as_bytes(), b"foo");
        assert_eq!(custom.as_str(), Some("foo"));
        // Non-UTF-8 custom name: as_bytes works, as_str is None.
        let bad = PostGlyphName::Custom(&[0xFF, 0xFE]);
        assert_eq!(bad.as_bytes(), &[0xFF, 0xFE]);
        assert_eq!(bad.as_str(), None);
    }

    #[test]
    fn v1_v3_have_no_format_specific_data() {
        for v in [0x0001_0000u32, 0x0003_0000u32] {
            let bytes = build_header(v);
            let p = PostTable::parse(&bytes).unwrap();
            assert_eq!(p.num_glyphs(), 0);
            assert!(p.name_index(0).is_none());
            assert!(p.name_string(0).is_none());
            assert!(p.standard_offset(0).is_none());
        }
    }
}
