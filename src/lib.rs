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
//!   `hmtx`, `cmap` formats 0/4/6/12, `name`, `post`, `OS/2`).
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

pub use cff::{PrivateHints, RegistryOrdering, TopMetadata};
pub use outline::{BBox, CubicContour, CubicOutline, CubicSegment, Point};

use crate::cff::Cff;
use crate::parser::TableDirectory;
use crate::tables::{
    cmap::CmapTable, head::HeadTable, hhea::HheaTable, hmtx::HmtxTable, maxp::MaxpTable,
    name::NameTable, os2::Os2Table, post::PostTable,
};

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

    /// 2x3 affine glyph → PostScript-user-space matrix from CFF Top
    /// DICT `FontMatrix` (TN5176 §9 op 12 07), returned in spec order
    /// `[a, b, c, d, tx, ty]`. Apply as
    /// `x_user = a*x + c*y + tx`, `y_user = b*x + d*y + ty`.
    /// CFF's default is `[0.001, 0, 0, 0.001, 0, 0]` — the
    /// 1000-unit-em convention.
    pub fn font_matrix(&self) -> [f64; 6] {
        self.cff.top_metadata().font_matrix
    }

    /// Paint type from CFF Top DICT `PaintType` (TN5176 §9 op 12 05).
    /// `0` = filled outline (the OpenType-CFF normal case), `2` =
    /// stroked outline whose pen width is [`Font::stroke_width`].
    /// Default: 0.
    pub fn paint_type(&self) -> i32 {
        self.cff.top_metadata().paint_type
    }

    /// Charstring format from CFF Top DICT `CharstringType` (TN5176
    /// §9 op 12 06). `2` is the only value embedded in an OpenType
    /// CFF table; other values correspond to legacy PostScript
    /// packaging. Default: 2.
    pub fn charstring_type(&self) -> i32 {
        self.cff.top_metadata().charstring_type
    }

    /// Stroke width applied when [`Font::paint_type`] is `2`, in font
    /// units (CFF Top DICT `StrokeWidth`, TN5176 §9 op 12 08).
    /// Ignored for filled outlines (`paint_type == 0`). Default: 0.
    pub fn stroke_width(&self) -> f64 {
        self.cff.top_metadata().stroke_width
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

    /// Embedded PostScript language code from CFF Top DICT
    /// `PostScript` (TN5176 §9 op 12 21). Almost always `None` on
    /// shipping OpenType-CFF fonts; non-`None` means the font contains
    /// an arbitrary block of PostScript that the spec says is "added to
    /// the font dictionary." Resolved through the CFF Strings table.
    pub fn postscript(&self) -> Option<&str> {
        self.cff
            .top_metadata()
            .postscript_sid
            .and_then(|sid| self.cff.resolve_sid(sid))
    }

    /// `BaseFontName` from CFF Top DICT (TN5176 §9 op 12 22). For
    /// synthetic fonts derived from a multiple-master master, this is
    /// the FontName of the underlying master font. Resolved through the
    /// CFF Strings table.
    pub fn base_font_name(&self) -> Option<&str> {
        self.cff
            .top_metadata()
            .base_font_name_sid
            .and_then(|sid| self.cff.resolve_sid(sid))
    }

    /// Legacy PostScript `UniqueID` (CFF Top DICT op 13, TN5176 §9
    /// Table 9). Adobe-assigned 32-bit identifier; modern fonts prefer
    /// [`Font::xuid`]. `None` if the operator is absent from the font.
    pub fn unique_id(&self) -> Option<i32> {
        self.cff.top_metadata().unique_id
    }

    /// Extended unique identifier from CFF Top DICT `XUID` (op 14,
    /// TN5176 §9 Table 9). Array of 32-bit numbers (the spec leaves
    /// the length unconstrained beyond "array"). Deprecated in
    /// OpenType-CFF per TN5176 4 Dec 03 Appendix H but still emitted by
    /// older Type 1 / OpenType-CFF tooling. Empty slice if the operator
    /// is absent.
    pub fn xuid(&self) -> &[i32] {
        &self.cff.top_metadata().xuid
    }

    /// Synthetic-font base index from CFF Top DICT `SyntheticBase`
    /// (TN5176 §9 op 12 20). When present, the value is the index into
    /// the Name INDEX of the base font that this synthetic font derives
    /// its glyph shapes from. `None` for non-synthetic fonts (the
    /// overwhelming common case).
    pub fn synthetic_base(&self) -> Option<i32> {
        self.cff.top_metadata().synthetic_base
    }

    /// Multiple-master `BaseFontBlend` user-design vector from CFF Top
    /// DICT (TN5176 §9 op 12 23). The values are undeltified to
    /// absolute floats per TN5176 §4 Table 4 "delta" semantics —
    /// successive entries are running sums of the raw operands.
    /// Empty slice if the operator is absent.
    pub fn base_font_blend(&self) -> &[f64] {
        &self.cff.top_metadata().base_font_blend
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
    pub fn private_hints(&self) -> &PrivateHints {
        self.cff.private_hints()
    }

    /// The CFF Private DICT hint zones that apply to `glyph_id`. For
    /// non-CID fonts this returns the same value as
    /// [`Font::private_hints`]; for CID-keyed fonts (TN5176 §18) the
    /// glyph is routed through `FDSelect` to one of the FDArray Font
    /// DICTs, and the hint zones returned are that FD's. Returns `None`
    /// when `glyph_id` is past `glyph_count()` (since FDSelect has no
    /// entry for it).
    pub fn glyph_private_hints(&self, glyph_id: u16) -> Option<&PrivateHints> {
        if glyph_id >= self.maxp.num_glyphs {
            return None;
        }
        self.cff.private_hints_for_glyph(glyph_id)
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
}
