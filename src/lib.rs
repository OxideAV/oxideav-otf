//! Pure-Rust OpenType / CFF font parser.
//!
//! Scope:
//! - sfnt header + table directory walker (`parser`).
//! - CFF (Adobe TN5176) Top DICT / Name / String INDEX / Charset /
//!   Encoding / Private DICT / Local + Global Subrs, plus CID-keyed
//!   fonts (ROS + FDArray Font DICTs + FDSelect GID→FD routing,
//!   TN5176 §§18, 19).
//! - CFF2 (OpenType 1.9.1 §6–§8): header, Top DICT, GlobalSubrINDEX,
//!   CharStringINDEX, and FontDICTINDEX walks (the `cff2` module
//!   defers variation-aware charstring decoding to a later round).
//! - Type 2 charstring interpreter (Adobe TN5177): every common path
//!   construction operator, the four flex variants, the deprecated
//!   four-operand `seac` `endchar`, hint-recording stubs (no
//!   enforcement; we anti-alias at >= 16 px), and subroutine
//!   resolution with the well-known 107 / 1131 / 32768 bias formula.
//! - Selected sfnt tables for metadata (`head`, `hhea`, `maxp`,
//!   `hmtx`, `cmap` formats 0/4/6/12, `name`, `post`, `OS/2`).
//!
//! The crate is read-only (parsing-only) and dependency-light: only
//! `oxideav-core` for shared types. CFF2 charstring decoding (with
//! blend / vsindex resolution against the VariationStore), per-glyph
//! hinting interpretation, advanced GSUB/GPOS, and Bidi are deferred.
//!
//! See `README.md` for a tour of the public API.

#![deny(missing_debug_implementations)]
#![warn(rust_2018_idioms)]

pub mod cff;
pub mod cff2;
pub mod outline;
pub mod parser;
pub mod tables;

pub use cff::{PrivateHints, RegistryOrdering, TopMetadata};
pub use cff2::{Cff2, Cff2Header, Cff2Op, Cff2TopDict, DEFAULT_FONT_MATRIX};
pub use outline::{BBox, CubicContour, CubicOutline, CubicSegment, Point};

use crate::cff::Cff;
use crate::parser::TableDirectory;
use crate::tables::{
    cmap::CmapTable, head::HeadTable, hhea::HheaTable, hmtx::HmtxTable, maxp::MaxpTable,
    name::NameTable, os2::Os2Table, post::PostTable,
};

pub use crate::tables::name::{NameId, NameRecord};
pub use crate::tables::os2::{
    EmbeddingPermission, FS_SELECTION_BOLD, FS_SELECTION_ITALIC, FS_SELECTION_NEGATIVE,
    FS_SELECTION_OBLIQUE, FS_SELECTION_OUTLINED, FS_SELECTION_REGULAR, FS_SELECTION_STRIKEOUT,
    FS_SELECTION_UNDERSCORE, FS_SELECTION_USE_TYPO_METRICS, FS_SELECTION_WWS,
    FS_TYPE_BITMAP_EMBEDDING_ONLY, FS_TYPE_EDITABLE, FS_TYPE_NO_SUBSETTING,
    FS_TYPE_PREVIEW_AND_PRINT, FS_TYPE_RESTRICTED_LICENSE, FS_TYPE_USAGE_MASK,
};
pub use crate::tables::post::PostFormat;

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
    /// CFF2-flavoured fonts are parsed for metadata (header, Top
    /// DICT, structural INDEXes — see the `cff2` module), but Type 2
    /// charstring decoding for CFF2 (with `blend` + `vsindex`
    /// resolution against the ItemVariationStore) is not yet
    /// implemented; [`Font::glyph_outline`] returns this error on a
    /// CFF2 font.
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
    /// A `put` / `get` storage operator (TN5177 §4.5) referenced a
    /// transient-array index outside `0..32` (Appendix B fixes the
    /// array at 32 elements). The contained value is the offending
    /// index.
    CharstringTransientIndex(i32),
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
            Self::Cff2NotImplemented => {
                f.write_str("CFF2 charstring decode not implemented (metadata is parsed)")
            }
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
            Self::CharstringTransientIndex(i) => write!(
                f,
                "Type 2 charstring: transient-array index {i} out of range (0..32)"
            ),
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
    /// Optional per OpenType spec: every well-formed OpenType font
    /// carries `post`, but some real-world stripped-down fonts omit
    /// it. We tolerate absence rather than reject the whole font.
    post: Option<PostTable<'a>>,
    /// `OS/2 and Windows Metrics`. Required for OpenType but
    /// occasionally missing on stripped-down or legacy TrueType-only
    /// fonts; absence surfaces as `None` rather than rejecting the
    /// whole font.
    os2: Option<Os2Table>,
    /// The font's CFF outline data, either CFF1 (Adobe TN5176) or CFF2
    /// (OpenType 1.9.1). CFF1 carries full charstring decoding +
    /// metadata; CFF2 carries structural metadata (header + Top DICT +
    /// CharStringINDEX count) but defers Type 2 + blend charstring
    /// decoding to a future round.
    cff: CffFlavour<'a>,
}

/// Internal discriminant for the font's CFF table flavour. The two
/// variants are boxed to keep the `Font` struct size and `CffFlavour`
/// discriminant cheap to move; the CFF1 variant in particular carries
/// a TopMetadata struct + 4 INDEX views + a Strings table and is ~500
/// bytes on its own.
#[derive(Debug)]
enum CffFlavour<'a> {
    Cff1(Box<Cff<'a>>),
    Cff2(Box<Cff2<'a>>),
}

/// Process-wide spec-default [`PrivateHints`] (TN5176 §15 defaults).
/// Returned by the `Font::private_hints` family for CFF2 fonts, whose
/// Private DICT decoding is deferred to a future round. Lazily
/// initialised so the cost is paid only when first queried.
fn default_private_hints() -> &'static PrivateHints {
    use std::sync::OnceLock;
    static DEFAULTS: OnceLock<PrivateHints> = OnceLock::new();
    DEFAULTS.get_or_init(PrivateHints::default)
}

impl<'a> Font<'a> {
    /// Parse a font from a borrowed byte slice.
    pub fn from_bytes(bytes: &'a [u8]) -> Result<Self, Error> {
        let dir = TableDirectory::parse(bytes)?;
        let cff_tag = dir.cff_tag.ok_or(Error::MissingCff)?;

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

        // `post` is one of the OpenType-spec required tables (per
        // `otspec-otff.html` "Required Tables"); for OpenType-CFF1 the
        // spec mandates version 3.0. Some real-world stripped-down
        // fonts omit it, so we tolerate absence and surface a `None`.
        let post = match dir.find(b"post", bytes) {
            Some(slice) => Some(PostTable::parse(slice)?),
            None => None,
        };

        // `OS/2` is one of the OpenType-spec required tables (per
        // `otspec-otff.html` "Required Tables") — same tolerance
        // policy as `post`: parse if present, surface `None`
        // otherwise so a stripped-down TrueType-only `.otf` that
        // omitted the table doesn't fail open.
        let os2 = match dir.find(b"OS/2", bytes) {
            Some(slice) => Some(Os2Table::parse(slice)?),
            None => None,
        };

        let cff = if cff_tag == *b"CFF2" {
            let cff2_bytes = dir.required(b"CFF2", bytes)?;
            CffFlavour::Cff2(Box::new(Cff2::parse(cff2_bytes)?))
        } else {
            let cff_bytes = dir.required(b"CFF ", bytes)?;
            CffFlavour::Cff1(Box::new(Cff::parse(cff_bytes)?))
        };

        Ok(Self {
            bytes,
            dir,
            head,
            hhea,
            maxp,
            cmap,
            name,
            hmtx,
            post,
            os2,
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
    ///
    /// CFF2 has no Name INDEX (the PostScript name lives in the sfnt
    /// `name` table at name ID 6 instead); this accessor returns
    /// `None` for CFF2 fonts. Callers wanting a PostScript name that
    /// works for both flavours should use
    /// `Font::name_string(NameId::PostScript)`.
    pub fn ps_name(&self) -> Option<&str> {
        std::str::from_utf8(self.cff1()?.ps_name()).ok()
    }

    /// Borrow the parsed CFF1 view, or `None` if this font uses CFF2
    /// (TN5176 vs. OpenType 1.9.1 CFF2 are different table flavours
    /// and only one is present in a given font).
    fn cff1(&self) -> Option<&Cff<'a>> {
        match &self.cff {
            CffFlavour::Cff1(c) => Some(c),
            CffFlavour::Cff2(_) => None,
        }
    }

    /// Borrow the parsed CFF2 view, or `None` if this font uses CFF1.
    fn cff2_view(&self) -> Option<&Cff2<'a>> {
        match &self.cff {
            CffFlavour::Cff1(_) => None,
            CffFlavour::Cff2(c) => Some(c),
        }
    }

    /// `true` if the font carries a `CFF2` table (OpenType 1.9.1
    /// variation-aware CFF flavour) rather than the original `CFF `
    /// table (Adobe TN5176 CFF version 1).
    pub fn is_cff2(&self) -> bool {
        matches!(self.cff, CffFlavour::Cff2(_))
    }

    /// Borrow the parsed CFF2 table, or `None` for CFF1 fonts. The
    /// returned view exposes the CFF2 header, Top DICT, CharString
    /// count, FontDICT INDEX, and per-glyph CharString bytes;
    /// per-glyph outline decoding (with `blend` and `vsindex`
    /// resolution against the `VariationStore`) is deferred to a
    /// future round and currently surfaces as
    /// [`Error::Cff2NotImplemented`] from [`Font::glyph_outline`].
    pub fn cff2(&self) -> Option<&Cff2<'a>> {
        self.cff2_view()
    }

    /// Borrow the parsed CFF2 header (`major`, `minor`, `headerSize`,
    /// `topDICTSize`), or `None` for CFF1 fonts.
    pub fn cff2_header(&self) -> Option<&Cff2Header> {
        self.cff2_view().map(Cff2::header)
    }

    /// Borrow the parsed CFF2 Top DICT, or `None` for CFF1 fonts. The
    /// returned struct surfaces all five spec-permitted operators
    /// (`CharStringINDEXOffset`, `VariationStoreOffset`,
    /// `FontDICTINDEXOffset`, `FontDICTSelectOffset`, `FontMatrix`).
    pub fn cff2_top_dict(&self) -> Option<&Cff2TopDict> {
        self.cff2_view().map(Cff2::top_dict)
    }

    /// `true` if this is a CFF2 variable font — that is, the Top DICT
    /// carries a `VariationStoreOffset` operator (per spec §7
    /// "VariationStoreOffset" Occurrence: required in fonts with
    /// variations, forbidden otherwise). Always `false` for CFF1
    /// fonts (CFF1 has no variation mechanism).
    pub fn is_variable(&self) -> bool {
        self.cff2_view().is_some_and(Cff2::is_variable)
    }

    // ---- glyph lookup ------------------------------------------------------

    /// Map a Unicode codepoint to its glyph id.
    pub fn glyph_index(&self, codepoint: char) -> Option<u16> {
        self.cmap.lookup(codepoint as u32)
    }

    /// Decode the cubic-Bezier outline for `glyph_id`.
    ///
    /// CFF2 outlines (with `blend` + `vsindex` resolution against the
    /// font's `VariationStore`) are not decoded this round; callers on
    /// a CFF2 font receive [`Error::Cff2NotImplemented`] regardless of
    /// `glyph_id`. The CFF2 charstring bytes are still reachable via
    /// `Font::cff2().unwrap().charstring(gid)` for inspection.
    pub fn glyph_outline(&self, glyph_id: u16) -> Result<CubicOutline, Error> {
        if glyph_id >= self.maxp.num_glyphs {
            return Err(Error::GlyphOutOfRange(glyph_id));
        }
        match &self.cff {
            CffFlavour::Cff1(c) => c.glyph_outline(glyph_id),
            CffFlavour::Cff2(_) => Err(Error::Cff2NotImplemented),
        }
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
    ///
    /// CFF2 fonts have no Charset or String INDEX (the per-glyph name
    /// list lives in the sfnt `post` table or the AGL fallback); this
    /// accessor returns `None` for CFF2 fonts.
    pub fn glyph_name(&self, glyph_id: u16) -> Option<&str> {
        let cff = self.cff1()?;
        let sid = cff.charset().sid_of(glyph_id)?;
        cff.strings().get(sid)
    }

    /// Borrow the CFF1 table view, or `None` for CFF2 fonts. Mostly for
    /// tests and advanced callers; the higher-level accessors on
    /// `Font` route through this internally.
    pub fn cff(&self) -> Option<&Cff<'a>> {
        self.cff1()
    }

    // ---- CID-keyed font metadata ------------------------------------------

    /// `true` if the embedded CFF is a CID-keyed font (carries the
    /// `ROS` operator + an FDArray / FDSelect, Adobe TN5176 §18).
    /// CID-keyed fonts route each glyph to one of several Font DICTs;
    /// the public glyph-outline / metrics API is identical either way.
    /// Always `false` for CFF2 fonts (CFF2 has no `ROS` operator —
    /// every glyph routes through FontDICTSelect to one of the
    /// FontDICTs by spec §7.2 regardless of CID-ness).
    pub fn is_cid(&self) -> bool {
        self.cff1().is_some_and(Cff::is_cid)
    }

    /// Registry string of a CID-keyed font's `ROS` operator (e.g.
    /// `"Adobe"`), resolved through the CFF Strings table. `None` for
    /// non-CID fonts and for CFF2 fonts.
    pub fn cid_registry(&self) -> Option<&str> {
        let cff = self.cff1()?;
        let ros = cff.registry_ordering()?;
        cff.resolve_sid(ros.registry_sid)
    }

    /// Ordering string of a CID-keyed font's `ROS` operator (e.g.
    /// `"Japan1"`, `"GB1"`, `"Identity"`). `None` for non-CID fonts
    /// and for CFF2 fonts.
    pub fn cid_ordering(&self) -> Option<&str> {
        let cff = self.cff1()?;
        let ros = cff.registry_ordering()?;
        cff.resolve_sid(ros.ordering_sid)
    }

    /// Supplement number of a CID-keyed font's `ROS` operator (the
    /// character-collection revision). `None` for non-CID fonts and
    /// for CFF2 fonts.
    pub fn cid_supplement(&self) -> Option<i32> {
        Some(self.cff1()?.registry_ordering()?.supplement)
    }

    /// Number of Font DICTs in a CID-keyed font's FDArray (TN5176
    /// §18) for CFF1, or in a CFF2 font's FontDICTINDEX (spec §7.2)
    /// for CFF2. `0` for non-CID CFF1 fonts.
    pub fn cff_fd_count(&self) -> usize {
        match &self.cff {
            CffFlavour::Cff1(c) => c.fd_count(),
            CffFlavour::Cff2(c) => c.font_dict_count() as usize,
        }
    }

    // ---- CFF Top DICT metadata --------------------------------------------
    //
    // Every accessor in this section returns a CFF1 Top DICT value
    // when the font is CFF1, and a sensible default when the font is
    // CFF2 (CFF2 deliberately omits these operators because the
    // equivalent information lives in sfnt-level tables — see CFF2
    // §1.2 "Comparison of 'glyf', 'CFF ' and CFF2 tables"). The one
    // exception is `font_matrix`, which IS defined in CFF2 §7 with
    // the spec's restricted `[s 0 0 s 0 0]` shape.

    /// CFF1 Top DICT metadata, or `None` for CFF2 fonts. CFF2 callers
    /// should use [`Font::cff2_top_dict`] instead — the two structs
    /// are not interchangeable because CFF2's Top DICT carries only
    /// five operators (per spec §7) and none of them are CFF1's
    /// FontBBox / italic / underline / weight / notice family.
    fn top_metadata_view(&self) -> Option<&TopMetadata> {
        self.cff1().map(Cff::top_metadata)
    }

    /// Font-wide bounding box from CFF Top DICT `FontBBox` (TN5176
    /// §9 op 5), in font-unit coordinates `[xMin, yMin, xMax, yMax]`.
    /// CFF1's default is `[0, 0, 0, 0]` (a sentinel telling the
    /// consumer to compute the bbox per-glyph by walking the
    /// charstrings — use [`Font::glyph_bbox`] for the per-glyph
    /// alternative). CFF2 has no `FontBBox` operator (spec §7) and
    /// this accessor returns `[0, 0, 0, 0]`.
    pub fn font_bbox(&self) -> [f32; 4] {
        self.top_metadata_view()
            .map(|m| m.font_bbox)
            .unwrap_or([0.0; 4])
    }

    /// Italic angle in degrees, counterclockwise from vertical
    /// (CFF Top DICT `ItalicAngle`, TN5176 §9 op 12 02). `0.0` for
    /// upright fonts and for CFF2 fonts (CFF2 has no `ItalicAngle`
    /// operator; the equivalent lives in `post.italicAngle`).
    pub fn italic_angle(&self) -> f64 {
        self.top_metadata_view()
            .map(|m| m.italic_angle)
            .unwrap_or(0.0)
    }

    /// Underline position in font units (CFF Top DICT
    /// `UnderlinePosition`, TN5176 §9 op 12 03). Negative values
    /// (the typographic convention) place the underline below the
    /// baseline. Default per spec: -100. Returns `-100.0` for CFF2
    /// fonts (`post.underlinePosition` is the CFF2-era source).
    pub fn underline_position(&self) -> f64 {
        self.top_metadata_view()
            .map(|m| m.underline_position)
            .unwrap_or(-100.0)
    }

    /// Underline stroke thickness in font units (CFF Top DICT
    /// `UnderlineThickness`, TN5176 §9 op 12 04). Default: 50. Returns
    /// `50.0` for CFF2 fonts.
    pub fn underline_thickness(&self) -> f64 {
        self.top_metadata_view()
            .map(|m| m.underline_thickness)
            .unwrap_or(50.0)
    }

    /// Whether the font is monospaced (CFF Top DICT `isFixedPitch`,
    /// TN5176 §9 op 12 01). Default: false. Returns `false` for CFF2
    /// fonts (`post.isFixedPitch` is the CFF2-era source).
    pub fn is_fixed_pitch(&self) -> bool {
        self.top_metadata_view().is_some_and(|m| m.is_fixed_pitch)
    }

    /// 2x3 affine glyph → PostScript-user-space matrix from the CFF
    /// Top DICT `FontMatrix` operator, returned in spec order
    /// `[a, b, c, d, tx, ty]`. Apply as
    /// `x_user = a*x + c*y + tx`, `y_user = b*x + d*y + ty`.
    ///
    /// - CFF1 (TN5176 §9 op 12 07): unconstrained 2×3 affine; default
    ///   `[0.001, 0, 0, 0.001, 0, 0]` (the 1000-unit-em convention).
    /// - CFF2 (OpenType 1.9.1 §7): restricted to `[s 0 0 s 0 0]` with
    ///   `s == 1 / unitsPerEm`; the operator is typically omitted
    ///   when `unitsPerEm == 1000` and the spec default
    ///   `[0.001, 0, 0, 0.001, 0, 0]` applies. We surface either the
    ///   on-disk matrix or the default per [`DEFAULT_FONT_MATRIX`].
    pub fn font_matrix(&self) -> [f64; 6] {
        match &self.cff {
            CffFlavour::Cff1(c) => c.top_metadata().font_matrix,
            CffFlavour::Cff2(c) => c.top_dict().font_matrix,
        }
    }

    /// Paint type from CFF Top DICT `PaintType` (TN5176 §9 op 12 05).
    /// `0` = filled outline (the OpenType-CFF normal case), `2` =
    /// stroked outline whose pen width is [`Font::stroke_width`].
    /// Default: 0. CFF2 has no `PaintType` operator (every CFF2 glyph
    /// is filled), so this returns `0` for CFF2 fonts.
    pub fn paint_type(&self) -> i32 {
        self.top_metadata_view().map(|m| m.paint_type).unwrap_or(0)
    }

    /// Charstring format from CFF Top DICT `CharstringType` (TN5176
    /// §9 op 12 06). `2` is the only value embedded in an OpenType
    /// CFF table; other values correspond to legacy PostScript
    /// packaging. Default: 2. CFF2 uses a different charstring
    /// dialect (§9 of the CFF2 spec, including `blend` and
    /// `vsindex`); we still report `2` for CFF2 to match the on-disk
    /// "CharString Type 2" lineage.
    pub fn charstring_type(&self) -> i32 {
        self.top_metadata_view()
            .map(|m| m.charstring_type)
            .unwrap_or(2)
    }

    /// Stroke width applied when [`Font::paint_type`] is `2`, in font
    /// units (CFF Top DICT `StrokeWidth`, TN5176 §9 op 12 08).
    /// Ignored for filled outlines (`paint_type == 0`). Default: 0.
    /// Returns `0.0` for CFF2 fonts (no `StrokeWidth` operator).
    pub fn stroke_width(&self) -> f64 {
        self.top_metadata_view()
            .map(|m| m.stroke_width)
            .unwrap_or(0.0)
    }

    /// Weight name from CFF Top DICT (op 4), e.g. `"Regular"`,
    /// `"Bold"`, `"Light"`. SID-resolved through the CFF Strings
    /// table; for SIDs in the standard-strings range these are
    /// PostScript-style ASCII names from TN5176 Appendix A. `None`
    /// for CFF2 fonts (no Strings table; use [`Font::name_string`]
    /// with `NameId::FontSubfamily`).
    pub fn weight_name(&self) -> Option<&str> {
        let cff = self.cff1()?;
        cff.top_metadata()
            .weight_sid
            .and_then(|sid| cff.resolve_sid(sid))
    }

    /// Copyright / trademark notice from CFF Top DICT (op 1). `None`
    /// for CFF2 fonts (use `Font::name_string(NameId::Copyright)`).
    pub fn notice(&self) -> Option<&str> {
        let cff = self.cff1()?;
        cff.top_metadata()
            .notice_sid
            .and_then(|sid| cff.resolve_sid(sid))
    }

    /// Extended copyright field from CFF Top DICT (op 12 00). `None`
    /// for CFF2 fonts.
    pub fn copyright(&self) -> Option<&str> {
        let cff = self.cff1()?;
        cff.top_metadata()
            .copyright_sid
            .and_then(|sid| cff.resolve_sid(sid))
    }

    /// Version string from CFF Top DICT (op 0), typically dotted-decimal.
    /// `None` for CFF2 fonts (use
    /// `Font::name_string(NameId::Version)`).
    pub fn version_string(&self) -> Option<&str> {
        let cff = self.cff1()?;
        cff.top_metadata()
            .version_sid
            .and_then(|sid| cff.resolve_sid(sid))
    }

    /// Embedded PostScript language code from CFF Top DICT
    /// `PostScript` (TN5176 §9 op 12 21). Almost always `None` on
    /// shipping OpenType-CFF fonts; non-`None` means the font contains
    /// an arbitrary block of PostScript that the spec says is "added to
    /// the font dictionary." Resolved through the CFF Strings table.
    /// `None` for CFF2 fonts.
    pub fn postscript(&self) -> Option<&str> {
        let cff = self.cff1()?;
        cff.top_metadata()
            .postscript_sid
            .and_then(|sid| cff.resolve_sid(sid))
    }

    /// `BaseFontName` from CFF Top DICT (TN5176 §9 op 12 22). For
    /// synthetic fonts derived from a multiple-master master, this is
    /// the FontName of the underlying master font. Resolved through
    /// the CFF Strings table. `None` for CFF2 fonts.
    pub fn base_font_name(&self) -> Option<&str> {
        let cff = self.cff1()?;
        cff.top_metadata()
            .base_font_name_sid
            .and_then(|sid| cff.resolve_sid(sid))
    }

    /// Legacy PostScript `UniqueID` (CFF Top DICT op 13, TN5176 §9
    /// Table 9). Adobe-assigned 32-bit identifier; modern fonts prefer
    /// [`Font::xuid`]. `None` if the operator is absent from the font
    /// and `None` for CFF2 fonts.
    pub fn unique_id(&self) -> Option<i32> {
        self.top_metadata_view().and_then(|m| m.unique_id)
    }

    /// Extended unique identifier from CFF Top DICT `XUID` (op 14,
    /// TN5176 §9 Table 9). Array of 32-bit numbers (the spec leaves
    /// the length unconstrained beyond "array"). Deprecated in
    /// OpenType-CFF per TN5176 4 Dec 03 Appendix H but still emitted
    /// by older Type 1 / OpenType-CFF tooling. Empty slice if the
    /// operator is absent or the font is CFF2.
    pub fn xuid(&self) -> &[i32] {
        self.top_metadata_view()
            .map_or(&[][..], |m| m.xuid.as_slice())
    }

    /// Synthetic-font base index from CFF Top DICT `SyntheticBase`
    /// (TN5176 §9 op 12 20). When present, the value is the index
    /// into the Name INDEX of the base font that this synthetic font
    /// derives its glyph shapes from. `None` for non-synthetic fonts
    /// (the overwhelming common case) and for CFF2 fonts.
    pub fn synthetic_base(&self) -> Option<i32> {
        self.top_metadata_view().and_then(|m| m.synthetic_base)
    }

    /// Multiple-master `BaseFontBlend` user-design vector from CFF
    /// Top DICT (TN5176 §9 op 12 23). The values are undeltified to
    /// absolute floats per TN5176 §4 Table 4 "delta" semantics —
    /// successive entries are running sums of the raw operands.
    /// Empty slice if the operator is absent and for CFF2 fonts.
    pub fn base_font_blend(&self) -> &[f64] {
        self.top_metadata_view()
            .map_or(&[][..], |m| m.base_font_blend.as_slice())
    }

    // ---- CFF Private DICT hint zones --------------------------------------

    /// PostScript-style alignment / stem hinting parameters for the
    /// Private DICT this font carries (CFF TN5176 §15 Table 23). For
    /// non-CID fonts this is the single top-level Private DICT (every
    /// glyph shares it); for CID-keyed fonts it is the FDArray entry at
    /// index 0. The returned struct exposes the full TN5176 §15 hint
    /// vocabulary: BlueValues / OtherBlues / FamilyBlues /
    /// FamilyOtherBlues (undeltified into absolute y-coordinate pairs),
    /// StdHW / StdVW (dominant stem widths), StemSnapH / StemSnapV
    /// (supplementary stem widths, undeltified), BlueScale / BlueShift
    /// / BlueFuzz (overshoot suppression tunables), ForceBold,
    /// LanguageGroup, ExpansionFactor, and initialRandomSeed. The
    /// round-1 outline decoder still does not enforce hints (we
    /// anti-alias at >= 16 px); this surface is for callers inspecting
    /// font metadata or implementing their own hinting.
    ///
    /// Callers wanting the per-FD hints of a CID-keyed font should use
    /// [`Font::cff`].`private_hints_fd(fd_index)` directly. The
    /// "hints that apply to a specific glyph" routing is
    /// [`Font::glyph_private_hints`].
    ///
    /// For CFF2 fonts, the Private DICT vocabulary is parsed by the
    /// CFF2 spec §10 with the same operators but is not yet exposed
    /// through this accessor (a future round will lift it onto a
    /// `cff2::PrivateDict` view); for now a spec-default
    /// [`PrivateHints`] is returned.
    pub fn private_hints(&self) -> &PrivateHints {
        match &self.cff {
            CffFlavour::Cff1(c) => c.private_hints(),
            CffFlavour::Cff2(_) => default_private_hints(),
        }
    }

    /// The CFF Private DICT hint zones that apply to `glyph_id`. For
    /// non-CID fonts this returns the same value as
    /// [`Font::private_hints`]; for CID-keyed fonts (TN5176 §18) the
    /// glyph is routed through `FDSelect` to one of the FDArray Font
    /// DICTs, and the hint zones returned are that FD's. Returns
    /// `None` when `glyph_id` is past `glyph_count()` (since FDSelect
    /// has no entry for it). For CFF2 fonts the returned hints are
    /// the spec-default values (see [`Font::private_hints`]).
    pub fn glyph_private_hints(&self, glyph_id: u16) -> Option<&PrivateHints> {
        if glyph_id >= self.maxp.num_glyphs {
            return None;
        }
        match &self.cff {
            CffFlavour::Cff1(c) => c.private_hints_for_glyph(glyph_id),
            CffFlavour::Cff2(_) => Some(default_private_hints()),
        }
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

    // ---- `post` PostScript table ------------------------------------------

    /// Borrow the parsed `post` table, if present. The table is one of
    /// OpenType's nine required tables (per `otff` spec) but some
    /// real-world stripped-down fonts omit it.
    ///
    /// For OpenType-CFF1 (this crate's only supported flavour) the
    /// spec mandates `post` version 3.0; the table still carries the
    /// 32-byte header (italic angle / underline / fixed-pitch / VM
    /// hints) regardless of version, and version 2.0 adds the
    /// PostScript-name array.
    pub fn post(&self) -> Option<&PostTable<'a>> {
        self.post.as_ref()
    }

    /// `post` table format discriminant, if present.
    pub fn post_format(&self) -> Option<PostFormat> {
        self.post.as_ref().map(PostTable::format)
    }

    /// Italic angle in degrees from the `post` table, if present.
    /// Equivalent to [`Font::italic_angle`] (sourced from CFF Top
    /// DICT) when both are populated; the spec recommends they match
    /// but does not require it.
    pub fn post_italic_angle(&self) -> Option<f64> {
        self.post.as_ref().map(PostTable::italic_angle)
    }

    /// Underline position in font units from the `post` table. The
    /// spec defines this as the y-coordinate of the *top* of the
    /// underline (CFF Top DICT's `UnderlinePosition` operates on the
    /// same coordinate definition).
    pub fn post_underline_position(&self) -> Option<i16> {
        self.post.as_ref().map(PostTable::underline_position)
    }

    /// Underline stroke thickness in font units from the `post`
    /// table.
    pub fn post_underline_thickness(&self) -> Option<i16> {
        self.post.as_ref().map(PostTable::underline_thickness)
    }

    /// `post.isFixedPitch` — `true` when the font is monospaced.
    /// `None` if `post` is absent. Note the on-disk field is a
    /// `uint32` and any non-zero value rounds up to `true`.
    pub fn post_is_fixed_pitch(&self) -> Option<bool> {
        self.post.as_ref().map(PostTable::is_fixed_pitch)
    }

    /// Glyph name for `glyph_id` from the `post` table, if the table
    /// is present in format 2.0 *and* the glyph maps to a non-
    /// standard Pascal string. For format-2.0 glyphs that map to the
    /// 258-entry standard Macintosh set (`glyphNameIndex < 258`),
    /// this returns `None` because the standard-Macintosh glyph-name
    /// list is not yet staged in `docs/text/opentype/` — see the
    /// module-level docs in `tables::post` and the round-187 report
    /// for the docs gap. Callers wanting names that work for every
    /// CFF1 glyph should prefer [`Font::glyph_name`] (CFF charset
    /// → strings, which has no docs gap).
    pub fn post_glyph_name(&self, glyph_id: u16) -> Option<&'a [u8]> {
        let post = self.post.as_ref()?;
        let idx = post.name_index(glyph_id)?;
        if idx < 258 {
            // Standard-Mac name — table not staged. See module docs.
            return None;
        }
        post.name_string(idx - 258)
    }

    // ---- `OS/2` table ------------------------------------------------------

    /// Borrow the parsed `OS/2` table, if present. Required by the
    /// OpenType spec but occasionally omitted from stripped-down
    /// fonts; absence surfaces as `None` (and the per-field
    /// convenience getters below return `None` in lock-step).
    pub fn os2(&self) -> Option<&Os2Table> {
        self.os2.as_ref()
    }

    /// `OS/2` table version (0..=5), if the table is present.
    pub fn os2_version(&self) -> Option<u16> {
        self.os2.as_ref().map(Os2Table::version)
    }

    /// `OS/2.usWeightClass` (1..=1000; 400 = Regular, 700 = Bold per
    /// the spec's common values).
    pub fn weight_class(&self) -> Option<u16> {
        self.os2.as_ref().map(Os2Table::weight_class)
    }

    /// `OS/2.usWidthClass` (1..=9; 5 = Medium).
    pub fn width_class(&self) -> Option<u16> {
        self.os2.as_ref().map(Os2Table::width_class)
    }

    /// `usWidthClass` interpreted as the spec's "% of normal" scale
    /// (50, 62.5, …, 200) — convenient for driving the variable-font
    /// `wdth` axis.
    pub fn width_class_percent(&self) -> Option<f32> {
        self.os2.as_ref().map(Os2Table::width_class_percent)
    }

    /// `OS/2.fsType` raw embedding-licensing bitfield.
    pub fn fs_type(&self) -> Option<u16> {
        self.os2.as_ref().map(Os2Table::fs_type)
    }

    /// `OS/2.fsType` bits 0..3 decoded into the named permission.
    pub fn embedding_permission(&self) -> Option<EmbeddingPermission> {
        self.os2.as_ref().map(Os2Table::embedding_permission)
    }

    /// `OS/2.fsSelection.ITALIC` (bit 0). The spec requires this to
    /// agree with `head.macStyle` bit 1.
    pub fn is_italic(&self) -> Option<bool> {
        self.os2.as_ref().map(Os2Table::is_italic)
    }

    /// `OS/2.fsSelection.BOLD` (bit 5). The spec requires this to
    /// agree with `head.macStyle` bit 0.
    pub fn is_bold(&self) -> Option<bool> {
        self.os2.as_ref().map(Os2Table::is_bold)
    }

    /// `OS/2.fsSelection.REGULAR` (bit 6).
    pub fn is_regular(&self) -> Option<bool> {
        self.os2.as_ref().map(Os2Table::is_regular)
    }

    /// `OS/2.fsSelection.USE_TYPO_METRICS` (bit 7, v4+).
    pub fn use_typo_metrics(&self) -> Option<bool> {
        self.os2.as_ref().map(Os2Table::use_typo_metrics)
    }

    /// `OS/2.fsSelection.OBLIQUE` (bit 9, v4+).
    pub fn is_oblique(&self) -> Option<bool> {
        self.os2.as_ref().map(Os2Table::is_oblique)
    }

    /// Four-byte registered vendor tag (`OS/2.achVendID`), interpreted
    /// as ASCII when possible.
    pub fn vendor_id(&self) -> Option<&str> {
        self.os2.as_ref().and_then(Os2Table::ach_vend_id_str)
    }

    /// 10-byte PANOSE classification (`OS/2.panose`).
    pub fn panose(&self) -> Option<&[u8; 10]> {
        self.os2.as_ref().map(Os2Table::panose)
    }

    /// `OS/2.sTypoAscender` — typographic ascender (v0-full or
    /// later). Combine with [`Font::typo_descender`] +
    /// [`Font::typo_line_gap`] for default line spacing when
    /// [`Font::use_typo_metrics`] is set.
    pub fn typo_ascender(&self) -> Option<i16> {
        self.os2.as_ref().and_then(Os2Table::typo_ascender)
    }

    /// `OS/2.sTypoDescender` — typically negative.
    pub fn typo_descender(&self) -> Option<i16> {
        self.os2.as_ref().and_then(Os2Table::typo_descender)
    }

    /// `OS/2.sTypoLineGap`.
    pub fn typo_line_gap(&self) -> Option<i16> {
        self.os2.as_ref().and_then(Os2Table::typo_line_gap)
    }

    /// `OS/2.usWinAscent` — Windows GDI clipping ascender.
    pub fn win_ascent(&self) -> Option<u16> {
        self.os2.as_ref().and_then(Os2Table::win_ascent)
    }

    /// `OS/2.usWinDescent` — Windows GDI clipping descender (positive).
    pub fn win_descent(&self) -> Option<u16> {
        self.os2.as_ref().and_then(Os2Table::win_descent)
    }

    /// `OS/2.sxHeight` (v2+) — height of lowercase `x`.
    pub fn x_height(&self) -> Option<i16> {
        self.os2.as_ref().and_then(Os2Table::x_height)
    }

    /// `OS/2.sCapHeight` (v2+) — height of uppercase letters.
    pub fn cap_height(&self) -> Option<i16> {
        self.os2.as_ref().and_then(Os2Table::cap_height)
    }

    /// `OS/2.usDefaultChar` (v2+).
    pub fn default_char(&self) -> Option<u16> {
        self.os2.as_ref().and_then(Os2Table::default_char)
    }

    /// `OS/2.usBreakChar` (v2+); conventionally `0x0020` (space).
    pub fn break_char(&self) -> Option<u16> {
        self.os2.as_ref().and_then(Os2Table::break_char)
    }

    /// `OS/2.usMaxContext` (v2+) — maximum target-glyph context length
    /// for any GSUB / GPOS lookup. `1` means single-glyph only.
    pub fn max_context(&self) -> Option<u16> {
        self.os2.as_ref().and_then(Os2Table::max_context)
    }

    // ---- `name` table -----------------------------------------------------

    /// Borrow the parsed `name` table view. Use this for callers that
    /// want to iterate every `NameRecord` directly via
    /// `name().records()` or to test for version-1 language-tag
    /// support via `name().version()` / `name().lang_tag(id)`.
    pub fn name(&self) -> &NameTable<'a> {
        &self.name
    }

    /// `name` table version (`0` for platform-specific language IDs
    /// only, `1` when language-tag records are present).
    pub fn name_version(&self) -> u16 {
        self.name.version()
    }

    /// Resolve a name-record `languageID >= 0x8000` to its
    /// version-1 BCP 47 language-tag string (per `otspec-name.html`
    /// "naming table version 1"). Returns `None` on a version-0 table
    /// (which has no language-tag records), for IDs `< 0x8000` (which
    /// are platform-specific numeric IDs, not tags), and for IDs
    /// outside the `[0x8000, 0x8000 + langTagCount)` declared range
    /// (which the spec says "should not be used").
    pub fn name_lang_tag(&self, language_id: u16) -> Option<String> {
        self.name.lang_tag(language_id)
    }

    /// Generic lookup by standard `NameId`, picking the best-ranked
    /// encoding (Windows / Unicode BMP English first). Sibling of
    /// [`Font::family_name`] / [`Font::full_name`] for callers that
    /// want any of the 26 spec-defined name IDs without a separate
    /// helper.
    pub fn name_string(&self, name_id: NameId) -> Option<&str> {
        self.name.get(name_id)
    }

    /// Designer name (name ID 9).
    pub fn designer(&self) -> Option<&str> {
        self.name.get(NameId::Designer)
    }

    /// Manufacturer name (name ID 8).
    pub fn manufacturer(&self) -> Option<&str> {
        self.name.get(NameId::Manufacturer)
    }

    /// Typeface description (name ID 10).
    pub fn description(&self) -> Option<&str> {
        self.name.get(NameId::Description)
    }

    /// Vendor URL (name ID 11).
    pub fn vendor_url(&self) -> Option<&str> {
        self.name.get(NameId::VendorUrl)
    }

    /// Designer URL (name ID 12).
    pub fn designer_url(&self) -> Option<&str> {
        self.name.get(NameId::DesignerUrl)
    }

    /// License description (name ID 13).
    pub fn license(&self) -> Option<&str> {
        self.name.get(NameId::License)
    }

    /// License-info URL (name ID 14).
    pub fn license_url(&self) -> Option<&str> {
        self.name.get(NameId::LicenseUrl)
    }

    /// Trademark string (name ID 7).
    pub fn trademark(&self) -> Option<&str> {
        self.name.get(NameId::Trademark)
    }

    /// Sample text (name ID 19).
    pub fn sample_text(&self) -> Option<&str> {
        self.name.get(NameId::SampleText)
    }

    /// Typographic Family name (name ID 16; "Preferred Family" in
    /// earlier spec text). The unconstrained extended-family grouping
    /// used by applications that look past the 4-style style-linking
    /// `font_family` cap.
    pub fn typographic_family(&self) -> Option<&str> {
        self.name.get(NameId::TypographicFamily)
    }

    /// Typographic Subfamily name (name ID 17; "Preferred Subfamily"
    /// in earlier spec text).
    pub fn typographic_subfamily(&self) -> Option<&str> {
        self.name.get(NameId::TypographicSubfamily)
    }

    /// WWS Family name (name ID 21). Provides a WWS-conformant family
    /// name when name IDs 16 / 17 carry extra non-WWS attributes; see
    /// `OS/2.fsSelection` bit 8.
    pub fn wws_family(&self) -> Option<&str> {
        self.name.get(NameId::WwsFamily)
    }

    /// WWS Subfamily name (name ID 22).
    pub fn wws_subfamily(&self) -> Option<&str> {
        self.name.get(NameId::WwsSubfamily)
    }

    /// Variations PostScript Name Prefix (name ID 25; variable fonts).
    pub fn variations_ps_name_prefix(&self) -> Option<&str> {
        self.name.get(NameId::VariationsPsNamePrefix)
    }

    /// Unique font identifier from the `name` table (name ID 3).
    /// Distinct from [`Font::unique_id`] (which is the CFF Top DICT's
    /// legacy PostScript `UniqueID` integer).
    pub fn unique_font_id(&self) -> Option<&str> {
        self.name.get(NameId::UniqueId)
    }
}
