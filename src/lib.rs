//! Pure-Rust OpenType / CFF font parser.
//!
//! Round-1 scope:
//! - sfnt header + table directory walker (`parser`).
//! - CFF (Adobe TN5176) Top DICT / Name / String INDEX / Charset /
//!   Encoding / Private DICT / Local + Global Subrs, plus CID-keyed
//!   fonts (ROS + FDArray Font DICTs + FDSelect GID→FD routing,
//!   TN5176 §§18, 19).
//! - Type 2 charstring interpreter (Adobe TN5177): every common path
//!   construction operator, the four flex variants, the deprecated
//!   four-operand `seac` `endchar`, hint-recording stubs (no
//!   enforcement; we anti-alias at >= 16 px), and subroutine
//!   resolution with the well-known 107 / 1131 / 32768 bias formula.
//! - Selected sfnt tables for metadata (`head`, `hhea`, `maxp`,
//!   `hmtx`, `cmap` formats 0/4/6/12, `name`).
//!
//! The crate is read-only (parsing-only) and dependency-light: only
//! `oxideav-core` for shared types. CFF2 (variable-aware), per-glyph
//! hinting interpretation, advanced GSUB/GPOS, and Bidi are deferred.
//!
//! See `README.md` for a tour of the public API.

#![deny(missing_debug_implementations)]
#![warn(rust_2018_idioms)]

pub mod cff;
pub mod outline;
pub mod parser;
pub mod tables;

pub use cff::{RegistryOrdering, TopMetadata};
pub use outline::{BBox, CubicContour, CubicOutline, CubicSegment, Point};

use crate::cff::Cff;
use crate::parser::TableDirectory;
use crate::tables::{
    cmap::CmapTable, head::HeadTable, hhea::HheaTable, hmtx::HmtxTable, maxp::MaxpTable,
    name::NameTable,
};

/// Errors emitted during font parsing or glyph lookup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// The input slice was too short for the requested header / structure.
    UnexpectedEof,
    /// The sfnt magic version did not match `OTTO`, `0x00010000`, or `true`.
    BadMagic,
    /// The table count in the sfnt header is implausibly large.
    BadHeader,
    /// An offset / length field pointed outside the file.
    BadOffset,
    /// A required table was missing from the table directory.
    MissingTable(&'static str),
    /// The font has no `CFF ` or `CFF2` table.
    MissingCff,
    /// The font carries CFF2; round-1 only handles CFF (TN5176 v1).
    Cff2NotImplemented,
    /// A glyph index was out of range vs. `maxp.numGlyphs` /
    /// `CharStrings INDEX count`.
    GlyphOutOfRange(u16),
    /// A cmap subtable used a format we do not implement in round 1.
    UnsupportedCmapFormat(u16),
    /// CFF-specific failure with a brief reason.
    Cff(&'static str),
    /// A varying-length structure was malformed in a non-CFF table
    /// (head, hhea, maxp, hmtx, name, cmap).
    BadStructure(&'static str),

    // --- Charstring interpreter errors ----------------------------------
    /// Operand stack overflowed (>= 192 entries).
    CharstringStackOverflow,
    /// Operator consumed more operands than the stack held.
    CharstringStackUnderflow,
    /// Operator referenced a subroutine number outside the INDEX range.
    CharstringBadSubrIndex(i32),
    /// `callsubr` was used in a font that has no Local Subrs INDEX.
    CharstringNoLocalSubrs,
    /// Subroutine recursion exceeded the spec cap (TN5177 §4.5: 10).
    CharstringTooDeep,
    /// Charstring processed too many bytes (DoS bound).
    CharstringTooLong,
    /// Charstring used an operator we don't yet implement.
    CharstringUnsupportedOp(u16),
    /// Internal sentinel used by the interpreter to signal `endchar`;
    /// never escapes the public API.
    #[doc(hidden)]
    CharstringEnd,
    /// `endchar` was used in its deprecated four-operand `seac` form
    /// (TN5177 Appendix C / Type 1 `seac`) but a referenced
    /// component glyph could not be resolved through the Standard
    /// Encoding table + the font's charset. The contained byte is
    /// the unresolved Standard-Encoding code (bchar or achar).
    CharstringSeacBadComponent(u8),
    /// Nested `seac` was attempted. The spec forbids it (TN5177
    /// Appendix C: "This construct may not be nested.").
    CharstringSeacNested,
}

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::UnexpectedEof => f.write_str("unexpected end of font data"),
            Self::BadMagic => f.write_str("not a TrueType / OpenType font (bad magic)"),
            Self::BadHeader => f.write_str("malformed sfnt header"),
            Self::BadOffset => f.write_str("table offset out of range"),
            Self::MissingTable(t) => write!(f, "required table missing: {t}"),
            Self::MissingCff => f.write_str("font has no CFF/CFF2 table"),
            Self::Cff2NotImplemented => f.write_str("CFF2 (variable) not implemented in round 1"),
            Self::GlyphOutOfRange(g) => write!(f, "glyph index {g} out of range"),
            Self::UnsupportedCmapFormat(fmt) => {
                write!(f, "cmap format {fmt} not implemented in round 1")
            }
            Self::Cff(s) => write!(f, "CFF: {s}"),
            Self::BadStructure(s) => write!(f, "malformed structure: {s}"),
            Self::CharstringStackOverflow => {
                f.write_str("Type 2 charstring: operand stack overflow")
            }
            Self::CharstringStackUnderflow => {
                f.write_str("Type 2 charstring: operand stack underflow")
            }
            Self::CharstringBadSubrIndex(i) => {
                write!(f, "Type 2 charstring: subr index {i} out of range")
            }
            Self::CharstringNoLocalSubrs => {
                f.write_str("Type 2 charstring: callsubr but no local subrs INDEX")
            }
            Self::CharstringTooDeep => {
                f.write_str("Type 2 charstring: subroutine recursion too deep")
            }
            Self::CharstringTooLong => f.write_str("Type 2 charstring: too many bytes processed"),
            Self::CharstringUnsupportedOp(op) => {
                write!(f, "Type 2 charstring: unsupported operator {op:#06x}")
            }
            Self::CharstringEnd => f.write_str("Type 2 charstring: end (internal)"),
            Self::CharstringSeacBadComponent(code) => write!(
                f,
                "Type 2 charstring: seac component (Standard Encoding code {code}) \
                 has no matching glyph in this font's charset"
            ),
            Self::CharstringSeacNested => {
                f.write_str("Type 2 charstring: nested seac is forbidden (TN5177 Appendix C)")
            }
        }
    }
}

impl std::error::Error for Error {}

/// A parsed OpenType / CFF font, lifetime-bound to the input bytes.
///
/// `Font::from_bytes` walks the sfnt header + table directory plus the
/// CFF top-level structures once; per-glyph charstrings are decoded on
/// demand by [`Font::glyph_outline`]. Lookup methods are O(log n) /
/// O(n) over the raw table bytes — no glyphs are pre-decoded.
#[derive(Debug)]
pub struct Font<'a> {
    bytes: &'a [u8],
    dir: TableDirectory,
    head: HeadTable,
    hhea: HheaTable,
    maxp: MaxpTable,
    cmap: CmapTable<'a>,
    name: NameTable<'a>,
    hmtx: HmtxTable<'a>,
    cff: Cff<'a>,
}

impl<'a> Font<'a> {
    /// Parse a font from a borrowed byte slice.
    pub fn from_bytes(bytes: &'a [u8]) -> Result<Self, Error> {
        let dir = TableDirectory::parse(bytes)?;
        let cff_tag = dir.cff_tag.ok_or(Error::MissingCff)?;
        if cff_tag == *b"CFF2" {
            return Err(Error::Cff2NotImplemented);
        }

        let head = HeadTable::parse(dir.required(b"head", bytes)?)?;
        let hhea = HheaTable::parse(dir.required(b"hhea", bytes)?)?;
        let maxp = MaxpTable::parse(dir.required(b"maxp", bytes)?)?;
        let cmap = CmapTable::parse(dir.required(b"cmap", bytes)?)?;
        let name = NameTable::parse(dir.required(b"name", bytes)?)?;
        let hmtx = HmtxTable::parse(
            dir.required(b"hmtx", bytes)?,
            hhea.num_long_hor_metrics,
            maxp.num_glyphs,
        )?;

        let cff_bytes = dir.required(b"CFF ", bytes)?;
        let cff = Cff::parse(cff_bytes)?;

        Ok(Self {
            bytes,
            dir,
            head,
            hhea,
            maxp,
            cmap,
            name,
            hmtx,
            cff,
        })
    }

    /// Raw bytes used to build this `Font`. Mostly useful for debugging.
    pub fn bytes(&self) -> &'a [u8] {
        self.bytes
    }

    // ---- metadata ----------------------------------------------------------

    /// Family name from the `name` table.
    pub fn family_name(&self) -> Option<&str> {
        self.name.find(1)
    }

    /// Full name (typically family + style) from the `name` table.
    pub fn full_name(&self) -> Option<&str> {
        self.name.find(4)
    }

    /// `head.unitsPerEm`. Almost always 1000 (CFF default) or 2048;
    /// never zero in valid fonts.
    pub fn units_per_em(&self) -> u16 {
        self.head.units_per_em
    }

    /// Number of glyphs (`maxp.numGlyphs`).
    pub fn glyph_count(&self) -> u16 {
        self.maxp.num_glyphs
    }

    /// Typographic ascent from `hhea`.
    pub fn ascent(&self) -> i16 {
        self.hhea.ascent
    }

    /// Typographic descent from `hhea` (typically negative).
    pub fn descent(&self) -> i16 {
        self.hhea.descent
    }

    /// Suggested gap between lines from `hhea`.
    pub fn line_gap(&self) -> i16 {
        self.hhea.line_gap
    }

    /// PostScript font name from the CFF Name INDEX.
    pub fn ps_name(&self) -> Option<&str> {
        std::str::from_utf8(self.cff.ps_name()).ok()
    }

    // ---- glyph lookup ------------------------------------------------------

    /// Map a Unicode codepoint to its glyph id.
    pub fn glyph_index(&self, codepoint: char) -> Option<u16> {
        self.cmap.lookup(codepoint as u32)
    }

    /// Decode the cubic-Bezier outline for `glyph_id`.
    pub fn glyph_outline(&self, glyph_id: u16) -> Result<CubicOutline, Error> {
        if glyph_id >= self.maxp.num_glyphs {
            return Err(Error::GlyphOutOfRange(glyph_id));
        }
        self.cff.glyph_outline(glyph_id)
    }

    /// Per-glyph advance width in font units.
    pub fn glyph_advance(&self, glyph_id: u16) -> i16 {
        self.hmtx.advance(glyph_id) as i16
    }

    /// Per-glyph left-side bearing in font units.
    pub fn glyph_lsb(&self, glyph_id: u16) -> i16 {
        self.hmtx.lsb(glyph_id)
    }

    /// Glyph name (from CFF charset / strings) — useful for diagnostics
    /// and for round-2 PostScript-style lookups. Returns `None` if the
    /// charset doesn't have a SID for this gid.
    pub fn glyph_name(&self, glyph_id: u16) -> Option<&str> {
        let sid = self.cff.charset().sid_of(glyph_id)?;
        self.cff.strings().get(sid)
    }

    /// Borrow the CFF table view (mostly for tests / advanced callers).
    pub fn cff(&self) -> &Cff<'a> {
        &self.cff
    }

    // ---- CID-keyed font metadata ------------------------------------------

    /// `true` if the embedded CFF is a CID-keyed font (carries the
    /// `ROS` operator + an FDArray / FDSelect, Adobe TN5176 §18).
    /// CID-keyed fonts route each glyph to one of several Font DICTs;
    /// the public glyph-outline / metrics API is identical either way.
    pub fn is_cid(&self) -> bool {
        self.cff.is_cid()
    }

    /// Registry string of a CID-keyed font's `ROS` operator (e.g.
    /// `"Adobe"`), resolved through the CFF Strings table. `None` for
    /// non-CID fonts.
    pub fn cid_registry(&self) -> Option<&str> {
        let ros = self.cff.registry_ordering()?;
        self.cff.resolve_sid(ros.registry_sid)
    }

    /// Ordering string of a CID-keyed font's `ROS` operator (e.g.
    /// `"Japan1"`, `"GB1"`, `"Identity"`). `None` for non-CID fonts.
    pub fn cid_ordering(&self) -> Option<&str> {
        let ros = self.cff.registry_ordering()?;
        self.cff.resolve_sid(ros.ordering_sid)
    }

    /// Supplement number of a CID-keyed font's `ROS` operator (the
    /// character-collection revision). `None` for non-CID fonts.
    pub fn cid_supplement(&self) -> Option<i32> {
        Some(self.cff.registry_ordering()?.supplement)
    }

    /// Number of Font DICTs in a CID-keyed font's FDArray (TN5176
    /// §18). `0` for non-CID fonts.
    pub fn cff_fd_count(&self) -> usize {
        self.cff.fd_count()
    }

    // ---- CFF Top DICT metadata --------------------------------------------

    /// Font-wide bounding box from CFF Top DICT `FontBBox` (TN5176
    /// §9 op 5), in font-unit coordinates `[xMin, yMin, xMax, yMax]`.
    /// CFF's default is `[0, 0, 0, 0]` (a sentinel telling the
    /// consumer to compute the bbox per-glyph by walking the
    /// charstrings — use [`Font::glyph_bbox`] for the per-glyph
    /// alternative).
    pub fn font_bbox(&self) -> [f32; 4] {
        self.cff.top_metadata().font_bbox
    }

    /// Italic angle in degrees, counterclockwise from vertical
    /// (CFF Top DICT `ItalicAngle`, TN5176 §9 op 12 02). `0.0` for
    /// upright fonts.
    pub fn italic_angle(&self) -> f64 {
        self.cff.top_metadata().italic_angle
    }

    /// Underline position in font units (CFF Top DICT
    /// `UnderlinePosition`, TN5176 §9 op 12 03). Negative values
    /// (the typographic convention) place the underline below the
    /// baseline. Default per spec: -100.
    pub fn underline_position(&self) -> f64 {
        self.cff.top_metadata().underline_position
    }

    /// Underline stroke thickness in font units (CFF Top DICT
    /// `UnderlineThickness`, TN5176 §9 op 12 04). Default: 50.
    pub fn underline_thickness(&self) -> f64 {
        self.cff.top_metadata().underline_thickness
    }

    /// Whether the font is monospaced (CFF Top DICT `isFixedPitch`,
    /// TN5176 §9 op 12 01). Default: false.
    pub fn is_fixed_pitch(&self) -> bool {
        self.cff.top_metadata().is_fixed_pitch
    }

    /// Weight name from CFF Top DICT (op 4), e.g. `"Regular"`,
    /// `"Bold"`, `"Light"`. SID-resolved through the CFF Strings
    /// table; for SIDs in the standard-strings range these are
    /// PostScript-style ASCII names from TN5176 Appendix A.
    pub fn weight_name(&self) -> Option<&str> {
        self.cff
            .top_metadata()
            .weight_sid
            .and_then(|sid| self.cff.resolve_sid(sid))
    }

    /// Copyright / trademark notice from CFF Top DICT (op 1).
    pub fn notice(&self) -> Option<&str> {
        self.cff
            .top_metadata()
            .notice_sid
            .and_then(|sid| self.cff.resolve_sid(sid))
    }

    /// Extended copyright field from CFF Top DICT (op 12 00).
    pub fn copyright(&self) -> Option<&str> {
        self.cff
            .top_metadata()
            .copyright_sid
            .and_then(|sid| self.cff.resolve_sid(sid))
    }

    /// Version string from CFF Top DICT (op 0), typically dotted-decimal.
    pub fn version_string(&self) -> Option<&str> {
        self.cff
            .top_metadata()
            .version_sid
            .and_then(|sid| self.cff.resolve_sid(sid))
    }

    // ---- per-glyph derived metrics ---------------------------------------

    /// Per-glyph bounding box in font units, derived by decoding the
    /// glyph's charstring and walking every emitted point + control
    /// point. Returns `None` if the glyph has no outline (e.g.
    /// `.notdef` in some fonts, or any glyph whose `endchar` is
    /// reached without emitting a path).
    ///
    /// This is a convenience over [`Font::glyph_outline`] for callers
    /// that only want the metrics — but note it still does the full
    /// charstring decode, so callers that need both should prefer
    /// `glyph_outline().bounds` directly to avoid duplicating work.
    pub fn glyph_bbox(&self, glyph_id: u16) -> Result<Option<BBox>, Error> {
        let outline = self.glyph_outline(glyph_id)?;
        if outline.is_empty() {
            Ok(None)
        } else {
            Ok(Some(outline.bounds))
        }
    }

    // ---- table-directory enumeration -------------------------------------

    /// Iterate all `(tag, length)` pairs present in the sfnt table
    /// directory, in on-disk order (which the spec requires to be
    /// ascending by tag). Useful for diagnostics, dumping a font's
    /// table inventory, or deciding whether to fall back to an
    /// alternative table.
    pub fn table_tags(&self) -> impl Iterator<Item = ([u8; 4], u32)> + '_ {
        self.dir.tag_list()
    }

    /// Raw byte slice for the sfnt table with `tag`, or `None` if the
    /// table is absent. The slice is borrowed from the original font
    /// bytes; the layout is exactly what the OpenType spec specifies
    /// for that table.
    pub fn table_data(&self, tag: &[u8; 4]) -> Option<&'a [u8]> {
        self.dir.find(tag, self.bytes)
    }

    /// `true` if the font carries a table with `tag`.
    pub fn has_table(&self, tag: &[u8; 4]) -> bool {
        self.dir.find(tag, self.bytes).is_some()
    }
}
