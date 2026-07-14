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
//!   `hmtx`, `cmap` formats 0/4/6/12, `name`, `post`, `OS/2`,
//!   `GDEF`, and the `GSUB` / `GPOS` headers with their
//!   `ScriptList` / `FeatureList` / `LookupList` walks).
//!
//! The crate is read-only (parsing-only) and dependency-light: only
//! `oxideav-core` for shared types. CFF2 charstring decoding (with
//! blend / vsindex resolution against the VariationStore), per-glyph
//! hinting interpretation, advanced GSUB/GPOS, and Bidi are deferred.
//!
//! See `README.md` for a tour of the public API.

#![deny(missing_debug_implementations)]
#![warn(rust_2018_idioms)]

pub mod agl;
pub mod cff;
pub mod cff2;
pub mod feature_tags;
pub mod outline;
pub mod parser;
pub mod shape;
pub mod tables;
pub mod unicode_script;

pub use cff::{PrivateHints, RegistryOrdering, TopMetadata};
pub use cff2::{
    Cff2, Cff2Header, Cff2Op, Cff2TopDict, ItemVariationData, ItemVariationStore,
    RegionAxisCoordinates, VariationRegion, DEFAULT_FONT_MATRIX,
};
pub use feature_tags::{
    feature_tag, is_registered_feature_tag, registered_feature_tags, FeatureTag, FeatureTagRecord,
    FEATURE_TAG_REGISTRY, REGISTERED_FEATURE_TAG_COUNT,
};
pub use outline::{BBox, CubicContour, CubicOutline, CubicSegment, Point};
pub use shape::{FeatureSetting, ShapeOptions, ShapedGlyph};

use crate::cff::Cff;
use crate::parser::TableDirectory;
use crate::tables::{
    avar::AvarTable, base::BaseTable, cmap::CmapTable, colr::ColrTable, cpal::CpalTable,
    ebdt::BitmapDataTable, eblc::BitmapLocationTable, ebsc::EbscTable, fvar::FvarTable,
    gdef::GdefTable, gpos::GposTable, gsub::GsubTable, head::HeadTable, hhea::HheaTable,
    hmtx::HmtxTable, kern::KernTable, maxp::MaxpTable, mvar::MvarTable, name::NameTable,
    os2::Os2Table, sbix::SbixTable, stat::StatTable, svg::SvgTable, vhea::VheaTable,
    vmtx::VmtxTable, vorg::VorgTable, xvar::MetricsVariations,
};

pub use crate::tables::avar::{AvarTable as AvarView, AxisValueMap, SegmentMap};
pub use crate::tables::base::{
    AxisTable as BaseAxisTable, BaseAxis, BaseScript, BaseTable as BaseView,
};
pub use crate::tables::cmap_uvs::{CmapUvs, UvsMapping};
pub use crate::tables::colr::{
    resolve_paint_color, Affine2x3, BaseGlyphRecord, ClipBox, ColorLine, ColorStop,
    ColrTable as ColrView, CompositeMode, Extend, LayerRecord, Paint, PaintRef, ResolvedColor,
    COLR_FOREGROUND_PALETTE_INDEX,
};
pub use crate::tables::context::{
    ChainedSequenceContext, ChainedSequenceRule, SequenceContext, SequenceLookupRecord,
    SequenceRule,
};
pub use crate::tables::cpal::{
    ColorRecord, CpalTable as CpalView, PaletteType, CPAL_USABLE_WITH_DARK_BACKGROUND,
    CPAL_USABLE_WITH_LIGHT_BACKGROUND,
};
pub use crate::tables::device::{DeviceOrVariationIndex, DeviceTable, VariationIndexTable};
pub use crate::tables::ebdt::{
    unpack_bgra32, unpack_pixels, BitmapContent, BitmapDataTable as BitmapDataView, EbdtComponent,
    GlyphBitmapData, GlyphMetrics,
};
pub use crate::tables::eblc::{
    BigGlyphMetrics, BitmapLocation, BitmapLocationTable as BitmapLocationView, BitmapSize,
    SbitLineMetrics, SmallGlyphMetrics, BITMAP_FLAG_HORIZONTAL_METRICS,
    BITMAP_FLAG_VERTICAL_METRICS,
};
pub use crate::tables::ebsc::{BitmapScale, EbscTable as EbscView};
pub use crate::tables::fvar::{
    FvarTable as FvarView, NamedInstance, VariationAxis, FVAR_AXIS_HIDDEN,
};
pub use crate::tables::gdef::{
    AttachList, AttachPoint, CaretValue, ClassDef, Coverage, CoverageIter, GlyphClass,
    LigCaretList, LigGlyph, MarkGlyphSets,
};
pub use crate::tables::gpos::GposTable as GposView;
pub use crate::tables::gpos::{
    Anchor, CursiveAttachment, CursivePos, EntryExit, ExtensionPos, LigatureAttachment,
    MarkAttachment, MarkBasePos, MarkLigPos, MarkMarkAttachment, MarkMarkPos, MarkRecord, PairPos,
    PairPosIter, PairValue, SinglePos, SinglePosIter, ValueFormat, ValueRecord,
    GPOS_LOOKUP_TYPE_CHAINED_CONTEXT, GPOS_LOOKUP_TYPE_CONTEXT, GPOS_LOOKUP_TYPE_CURSIVE,
    GPOS_LOOKUP_TYPE_EXTENSION, GPOS_LOOKUP_TYPE_MARK_TO_BASE, GPOS_LOOKUP_TYPE_MARK_TO_LIGATURE,
    GPOS_LOOKUP_TYPE_MARK_TO_MARK, GPOS_LOOKUP_TYPE_PAIR, GPOS_LOOKUP_TYPE_SINGLE,
};
pub use crate::tables::gsub::GsubTable as GsubView;
pub use crate::tables::gsub::{
    AlternateGlyphIter, AlternateSet, AlternateSubst, AlternateSubstIter, ExtensionSubst, Ligature,
    LigatureComponentIter, LigatureSet, LigatureSubst, LigatureSubstIter, MultipleSubst,
    MultipleSubstIter, ReverseChainSingleSubst, Sequence, SequenceGlyphIter, SingleSubst,
    SingleSubstIter, GSUB_LOOKUP_TYPE_ALTERNATE, GSUB_LOOKUP_TYPE_CHAINED_CONTEXT,
    GSUB_LOOKUP_TYPE_CONTEXT, GSUB_LOOKUP_TYPE_EXTENSION, GSUB_LOOKUP_TYPE_LIGATURE,
    GSUB_LOOKUP_TYPE_MULTIPLE, GSUB_LOOKUP_TYPE_REVERSE_CHAINED_SINGLE, GSUB_LOOKUP_TYPE_SINGLE,
};
pub use crate::tables::ivs::{
    DeltaSetIndexMap, ItemVariationData as DeltaSetItemVariationData,
    ItemVariationStore as DeltaSetItemVariationStore,
};
pub use crate::tables::kern::{
    KernSubtable, KernTable as KernView, KERN_COVERAGE_CROSS_STREAM, KERN_COVERAGE_HORIZONTAL,
    KERN_COVERAGE_MINIMUM, KERN_COVERAGE_OVERRIDE,
};
pub use crate::tables::layout::{
    Feature, FeatureList, FeatureListIter, FeatureTableSubstitution, FeatureVariations, LangSys,
    Lookup, LookupFlag, LookupList, LookupListIter, Script, ScriptList, ScriptListIter,
    NO_REQUIRED_FEATURE,
};
pub use crate::tables::mvar::{MvarTable as MvarView, ValueRecord as MvarValueRecord};
pub use crate::tables::name::{NameId, NameRecord};
pub use crate::tables::os2::{
    EmbeddingPermission, FS_SELECTION_BOLD, FS_SELECTION_ITALIC, FS_SELECTION_NEGATIVE,
    FS_SELECTION_OBLIQUE, FS_SELECTION_OUTLINED, FS_SELECTION_REGULAR, FS_SELECTION_STRIKEOUT,
    FS_SELECTION_UNDERSCORE, FS_SELECTION_USE_TYPO_METRICS, FS_SELECTION_WWS,
    FS_TYPE_BITMAP_EMBEDDING_ONLY, FS_TYPE_EDITABLE, FS_TYPE_NO_SUBSETTING,
    FS_TYPE_PREVIEW_AND_PRINT, FS_TYPE_RESTRICTED_LICENSE, FS_TYPE_USAGE_MASK,
};
pub use crate::tables::post::{
    standard_mac_glyph_name, PostFormat, PostGlyphName, PostTable, STANDARD_MAC_GLYPH_NAMES,
};
pub use crate::tables::sbix::{
    GlyphGraphic, GraphicType, SbixStrike, SbixTable as SbixView, SBIX_FLAG_ALWAYS_SET,
    SBIX_FLAG_DRAW_OUTLINES,
};
pub use crate::tables::stat::{
    AxisValue, StatAxisRecord, StatTable as StatView, STAT_ELIDABLE_AXIS_VALUE_NAME,
    STAT_OLDER_SIBLING_FONT_ATTRIBUTE,
};
pub use crate::tables::svg::{SvgDocument, SvgTable as SvgView};
pub use crate::tables::vhea::VheaTable as VheaView;
pub use crate::tables::vmtx::VmtxTable as VmtxView;
pub use crate::tables::vorg::VorgTable as VorgView;
pub use crate::tables::xvar::MetricsVariations as MetricsVariationsView;

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
    /// **Deprecated / no longer returned.** CFF2 glyph outlines are now
    /// decoded by the variation-aware CFF2 charstring interpreter (see
    /// the `cff2` module); [`Font::glyph_outline`] decodes a CFF2 font
    /// at its default variation instance and never returns this error.
    /// The variant is retained so existing `match` arms keep compiling.
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
    /// `vhea` — vertical header. Optional: only present in fonts that
    /// support vertical writing (typically CJK). Carries
    /// `numOfLongVerMetrics`, consumed by `vmtx`.
    vhea: Option<VheaTable>,
    /// `vmtx` — vertical metrics. Present only alongside `vhea`; parsed
    /// only when both tables exist (per spec they are co-required for
    /// vertical fonts).
    vmtx: Option<VmtxTable<'a>>,
    /// Optional per OpenType spec: every well-formed OpenType font
    /// carries `post`, but some real-world stripped-down fonts omit
    /// it. We tolerate absence rather than reject the whole font.
    post: Option<PostTable<'a>>,
    /// `OS/2 and Windows Metrics`. Required for OpenType but
    /// occasionally missing on stripped-down or legacy TrueType-only
    /// fonts; absence surfaces as `None` rather than rejecting the
    /// whole font.
    os2: Option<Os2Table>,
    /// `GDEF` — Glyph Definition Table. Optional per the OpenType
    /// spec: a font without GSUB / GPOS lookups doesn't need it. When
    /// present, surfaces per-glyph class data + ligature carets + the
    /// MarkAttachClassDef and MarkGlyphSets sub-tables that GSUB and
    /// GPOS shaping consult.
    gdef: Option<GdefTable<'a>>,
    /// `GSUB` — Glyph Substitution Table header view. Optional
    /// (a font that performs no glyph substitution omits the table).
    /// When present, surfaces the ScriptList / FeatureList /
    /// LookupList shape; per-lookup subtable decoding is deferred to a
    /// future round.
    gsub: Option<GsubTable<'a>>,
    /// `GPOS` — Glyph Positioning Table header view. Optional. Same
    /// header shape as `GSUB`; per-lookup positioning-subtable
    /// decoding is deferred.
    gpos: Option<GposTable<'a>>,
    /// `kern` — legacy kerning table. Optional. Modern fonts express
    /// kerning through GPOS pair adjustment, but many still ship a
    /// `kern` table for compatibility; we decode the OFF/Windows
    /// version-0 format (subtable formats 0 and 2).
    kern: Option<KernTable<'a>>,
    /// `fvar` — font variations table. Present in variable fonts;
    /// defines the design-space axes and named instances, and drives
    /// user→normalized coordinate normalization.
    fvar: Option<FvarTable>,
    /// `avar` — axis variations table. Optional; refines `fvar`'s
    /// default normalization with per-axis segment maps.
    avar: Option<AvarTable>,
    /// `STAT` — style attributes table. Required in variable fonts,
    /// optional otherwise; describes the family-relative design
    /// attributes and their `name`-table associations.
    stat: Option<StatTable>,
    /// `MVAR` — metrics variations. Optional; varies font-wide `OS/2` /
    /// `hhea` / `vhea` / `post` metrics per instance via an
    /// ItemVariationStore keyed by four-byte value tags.
    mvar: Option<MvarTable>,
    /// `HVAR` — horizontal metrics variations. Optional (required for
    /// CFF2 variable fonts with varying advance widths); per-glyph
    /// advance-width / side-bearing per-instance adjustments.
    hvar: Option<MetricsVariations>,
    /// `VVAR` — vertical metrics variations. Optional; per-glyph
    /// advance-height / side-bearing / vertical-origin adjustments.
    vvar: Option<MetricsVariations>,
    /// `BASE` — baseline table. Optional; per-axis, per-script baseline
    /// coordinates and min/max extents for multi-script alignment.
    base: Option<BaseTable>,
    /// `VORG` — vertical origin table. Optional CFF-OFF table giving the
    /// Y coordinate of each glyph's vertical origin directly.
    vorg: Option<VorgTable>,
    /// `COLR` — color table. Optional; version-0 layered color glyphs
    /// and/or the version-1 paint-graph color glyphs (with their
    /// embedded variation data).
    colr: Option<ColrTable<'a>>,
    /// `CPAL` — palette table. Required alongside `COLR` (it carries
    /// the colors the paint graph's palette indices select); optional
    /// alongside `SVG `.
    cpal: Option<CpalTable<'a>>,
    /// `sbix` — standard bitmap graphics. Optional; per-strike
    /// PNG/JPEG/TIFF glyph bitmaps.
    sbix: Option<SbixTable<'a>>,
    /// `EBLC` — embedded (monochrome / grayscale) bitmap locators.
    /// Optional; paired with `EBDT`.
    eblc: Option<BitmapLocationTable<'a>>,
    /// `CBLC` — color bitmap locators. Optional; paired with `CBDT`.
    /// Same structure as `EBLC` (major version 3).
    cblc: Option<BitmapLocationTable<'a>>,
    /// `EBDT` — embedded bitmap glyph data. Optional; paired with
    /// `EBLC`.
    ebdt: Option<BitmapDataTable<'a>>,
    /// `CBDT` — color bitmap glyph data. Optional; paired with
    /// `CBLC`.
    cbdt: Option<BitmapDataTable<'a>>,
    /// `EBSC` — embedded bitmap scaling: strikes defined as scaled
    /// versions of real `EBLC`/`EBDT` strikes. Optional.
    ebsc: Option<EbscTable>,
    /// `SVG ` — SVG glyph descriptions. Optional; color-variable
    /// values may come from `CPAL`.
    svg: Option<SvgTable<'a>>,
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

        // `vhea` / `vmtx` are co-required for vertical fonts and absent
        // from horizontal-only fonts. Parse `vmtx` only when `vhea`
        // supplied `numOfLongVerMetrics`; a `vhea` without `vmtx`
        // surfaces metrics-free vertical header data.
        let vhea = match dir.find(b"vhea", bytes) {
            Some(slice) => Some(VheaTable::parse(slice)?),
            None => None,
        };
        let vmtx = match (&vhea, dir.find(b"vmtx", bytes)) {
            (Some(vh), Some(slice)) => Some(VmtxTable::parse(
                slice,
                vh.num_long_ver_metrics,
                maxp.num_glyphs,
            )?),
            _ => None,
        };

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

        // `GDEF` is optional — a font without GSUB/GPOS lookups can
        // legitimately omit it. Parse if present.
        let gdef = match dir.find(b"GDEF", bytes) {
            Some(slice) => Some(GdefTable::parse(slice)?),
            None => None,
        };

        // `GSUB` and `GPOS` are both optional in OpenType: a
        // glyph-only font with neither substitution nor positioning
        // rules omits both.
        let gsub = match dir.find(b"GSUB", bytes) {
            Some(slice) => Some(GsubTable::parse(slice)?),
            None => None,
        };
        let gpos = match dir.find(b"GPOS", bytes) {
            Some(slice) => Some(GposTable::parse(slice)?),
            None => None,
        };

        // `kern` is optional and may use formats we don't decode; a
        // malformed `kern` shouldn't sink the whole font, so we tolerate
        // a parse failure by surfacing `None`.
        let kern = match dir.find(b"kern", bytes) {
            Some(slice) => KernTable::parse(slice).ok(),
            None => None,
        };

        // `fvar` / `avar` — variable-font axis definitions and the
        // normalization refinement. Both optional (present only in
        // variable fonts); `avar` only makes sense alongside `fvar`.
        let fvar = match dir.find(b"fvar", bytes) {
            Some(slice) => Some(FvarTable::parse(slice)?),
            None => None,
        };
        let avar = match (&fvar, dir.find(b"avar", bytes)) {
            (Some(_), Some(slice)) => AvarTable::parse(slice).ok(),
            _ => None,
        };
        // `STAT` is independent of fvar (allowed in non-variable fonts);
        // tolerate a malformed table by surfacing `None`.
        let stat = match dir.find(b"STAT", bytes) {
            Some(slice) => StatTable::parse(slice).ok(),
            None => None,
        };
        // `MVAR` is a variable-font table; tolerate a malformed table.
        let mvar = match dir.find(b"MVAR", bytes) {
            Some(slice) => MvarTable::parse(slice).ok(),
            None => None,
        };
        // `HVAR` / `VVAR` — per-glyph metrics variations. Variable-font
        // tables; tolerate malformed data.
        let hvar = match dir.find(b"HVAR", bytes) {
            Some(slice) => MetricsVariations::parse_hvar(slice).ok(),
            None => None,
        };
        let vvar = match dir.find(b"VVAR", bytes) {
            Some(slice) => MetricsVariations::parse_vvar(slice).ok(),
            None => None,
        };
        // `BASE` is optional and allowed in non-variable fonts; tolerate
        // a malformed table.
        let base = match dir.find(b"BASE", bytes) {
            Some(slice) => BaseTable::parse(slice).ok(),
            None => None,
        };
        // `VORG` is a CFF-OFF vertical-origin table; tolerate malformed.
        let vorg = match dir.find(b"VORG", bytes) {
            Some(slice) => VorgTable::parse(slice).ok(),
            None => None,
        };
        // `COLR` is optional; tolerate a malformed table (the font's
        // monochrome outlines remain usable without it).
        let colr = match dir.find(b"COLR", bytes) {
            Some(slice) => ColrTable::parse(slice).ok(),
            None => None,
        };
        // `CPAL` is required alongside `COLR` and optional alongside
        // `SVG `; same tolerance policy (a malformed palette table
        // degrades color-glyph rendering, not the whole font).
        let cpal = match dir.find(b"CPAL", bytes) {
            Some(slice) => CpalTable::parse(slice).ok(),
            None => None,
        };
        // `sbix` is optional; its per-strike offset arrays are sized
        // by `maxp.numGlyphs` (§5.6.7.4). Same tolerance policy.
        let sbix = match dir.find(b"sbix", bytes) {
            Some(slice) => SbixTable::parse(slice, maxp.num_glyphs).ok(),
            None => None,
        };
        // `EBLC` / `CBLC` — embedded-bitmap locators (monochrome /
        // color); optional, same tolerance policy.
        let eblc = match dir.find(b"EBLC", bytes) {
            Some(slice) => BitmapLocationTable::parse(slice).ok(),
            None => None,
        };
        let cblc = match dir.find(b"CBLC", bytes) {
            Some(slice) => BitmapLocationTable::parse(slice).ok(),
            None => None,
        };
        // `EBDT` / `CBDT` — the bitmap data the locators point into.
        let ebdt = match dir.find(b"EBDT", bytes) {
            Some(slice) => BitmapDataTable::parse(slice).ok(),
            None => None,
        };
        let cbdt = match dir.find(b"CBDT", bytes) {
            Some(slice) => BitmapDataTable::parse(slice).ok(),
            None => None,
        };
        // `EBSC` — scaled-strike definitions; optional.
        let ebsc = match dir.find(b"EBSC", bytes) {
            Some(slice) => EbscTable::parse(slice).ok(),
            None => None,
        };
        // `SVG ` — SVG glyph descriptions; optional.
        let svg = match dir.find(b"SVG ", bytes) {
            Some(slice) => SvgTable::parse(slice).ok(),
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
            vhea,
            vmtx,
            post,
            os2,
            gdef,
            gsub,
            gpos,
            kern,
            fvar,
            avar,
            stat,
            mvar,
            hvar,
            vvar,
            base,
            vorg,
            colr,
            cpal,
            sbix,
            eblc,
            cblc,
            ebdt,
            cbdt,
            ebsc,
            svg,
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
    /// count, FontDICT INDEX, per-glyph CharString bytes, and (for
    /// variable fonts) the parsed `ItemVariationStore`
    /// ([`Font::variation_store`]). Per-glyph outline decoding (the
    /// variation-aware `blend`/`vsindex` charstring interpreter) is
    /// available through [`Font::glyph_outline`] (default instance) and
    /// [`Font::glyph_outline_var`] (a caller-supplied instance).
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

    /// Borrow the CFF2 `ItemVariationStore` (§12) for a variable CFF2
    /// font, or `None` for non-variable CFF2 fonts and all CFF1 fonts.
    /// The store exposes the `VariationRegionList` (each region's
    /// per-axis `start`/`peak`/`end` F2DOT14 intervals) and the
    /// `ItemVariationData` subtables (`regionIndexes` selecting the
    /// active regions for `vsindex`); these are the inputs a future
    /// `blend` charstring pass needs.
    pub fn variation_store(&self) -> Option<&ItemVariationStore> {
        self.cff2_view().and_then(Cff2::variation_store)
    }

    // ---- glyph lookup ------------------------------------------------------

    /// Map a Unicode codepoint to its glyph id.
    pub fn glyph_index(&self, codepoint: char) -> Option<u16> {
        self.cmap.lookup(codepoint as u32)
    }

    /// Map a Unicode variation sequence — a `base` character followed by
    /// a `variation_selector` — to its glyph id, using the `cmap`
    /// format-14 (Unicode Variation Sequences) subtable.
    ///
    /// A non-default UVS yields its font-specified glyph; a default UVS
    /// resolves `base` through the base `cmap` ([`Font::glyph_index`]);
    /// an unsupported sequence (or a font with no format-14 subtable)
    /// yields `None`. Callers that want the "fall back to the base
    /// glyph for an unsupported selector" behaviour should try this
    /// first and then `glyph_index(base)`.
    pub fn glyph_index_variation(&self, base: char, variation_selector: char) -> Option<u16> {
        self.cmap
            .lookup_variation(base as u32, variation_selector as u32)
    }

    /// The `cmap` format-14 (Unicode Variation Sequences) view, if the
    /// font carries one. Lets callers enumerate the supported variation
    /// selectors and perform raw `(base, selector)` lookups.
    pub fn variation_sequences(&self) -> Option<Result<CmapUvs<'a>, Error>> {
        self.cmap.uvs()
    }

    /// Decode the cubic-Bezier outline for `glyph_id`.
    ///
    /// For a CFF1 font this runs the Type 2 charstring interpreter. For
    /// a CFF2 (variable) font this runs the variation-aware CFF2
    /// interpreter at the **default variation instance** (every region
    /// scalar `0`, so `blend` deltas contribute nothing and the result
    /// is the default design). Use [`Font::glyph_outline_var`] to
    /// decode a specific variation instance.
    pub fn glyph_outline(&self, glyph_id: u16) -> Result<CubicOutline, Error> {
        if glyph_id >= self.maxp.num_glyphs {
            return Err(Error::GlyphOutOfRange(glyph_id));
        }
        match &self.cff {
            CffFlavour::Cff1(c) => c.glyph_outline(glyph_id),
            CffFlavour::Cff2(c) => c.glyph_outline(glyph_id as u32),
        }
    }

    /// Decode the cubic-Bezier outline for `glyph_id` at the variation
    /// instance described by `region_scalars`.
    ///
    /// This is only meaningful for a CFF2 variable font: a `blend`
    /// operator's `j`-th delta is scaled by `region_scalars[j]` (the
    /// interpolation scalar of the `j`-th active variation region) and
    /// added to its default value. A caller derives the scalars from
    /// the font's `fvar`/`avar` axis settings via the OpenType *Font
    /// Variations Common Table Formats* region-scalar algorithm. An
    /// empty slice selects the default instance.
    ///
    /// For a CFF1 font `region_scalars` is ignored (CFF1 has no
    /// variation operators) and the static outline is returned.
    pub fn glyph_outline_var(
        &self,
        glyph_id: u16,
        region_scalars: &[f32],
    ) -> Result<CubicOutline, Error> {
        if glyph_id >= self.maxp.num_glyphs {
            return Err(Error::GlyphOutOfRange(glyph_id));
        }
        match &self.cff {
            CffFlavour::Cff1(c) => c.glyph_outline(glyph_id),
            CffFlavour::Cff2(c) => c.glyph_outline_var(glyph_id as u32, region_scalars),
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

    // ---- vertical metrics (vhea / vmtx) ------------------------------------

    /// Whether this font carries vertical layout metrics (`vhea` +
    /// `vmtx`). Typically true only for CJK fonts.
    pub fn has_vertical_metrics(&self) -> bool {
        self.vmtx.is_some()
    }

    /// Raw `vhea` view, if present.
    pub fn vhea(&self) -> Option<&VheaTable> {
        self.vhea.as_ref()
    }

    /// Vertical typographic ascender (`vhea.vertTypoAscender` /
    /// v1.0 `ascent`). `None` when the font has no `vhea`.
    pub fn vertical_ascent(&self) -> Option<i16> {
        self.vhea.as_ref().map(|v| v.ascent)
    }

    /// Vertical typographic descender (`vhea.vertTypoDescender` /
    /// v1.0 `descent`).
    pub fn vertical_descent(&self) -> Option<i16> {
        self.vhea.as_ref().map(|v| v.descent)
    }

    /// Vertical typographic line gap (`vhea.vertTypoLineGap` /
    /// v1.0 `lineGap`).
    pub fn vertical_line_gap(&self) -> Option<i16> {
        self.vhea.as_ref().map(|v| v.line_gap)
    }

    /// Per-glyph advance **height** in font units (`vmtx.advanceHeight`).
    /// `None` when the font carries no vertical metrics.
    pub fn glyph_advance_height(&self, glyph_id: u16) -> Option<u16> {
        self.vmtx.as_ref().map(|v| v.advance(glyph_id))
    }

    /// Per-glyph top side bearing in font units (`vmtx.topSideBearing`).
    /// `None` when the font carries no vertical metrics.
    pub fn glyph_tsb(&self, glyph_id: u16) -> Option<i16> {
        self.vmtx.as_ref().map(|v| v.top_side_bearing(glyph_id))
    }

    // ---- legacy kerning (kern) --------------------------------------------

    /// Raw `kern` table view, if present. Modern shaping uses GPOS pair
    /// adjustment (`Font::gpos`); `kern` is a legacy fallback.
    pub fn kern(&self) -> Option<&KernTable<'a>> {
        self.kern.as_ref()
    }

    /// Convenience: accumulated horizontal kerning adjustment for an
    /// ordered glyph pair from the legacy `kern` table, in font units.
    /// Returns 0 when the font has no `kern` table or the pair is
    /// uncovered. (See `KernView::kerning` for the additive semantics.)
    pub fn kern_pair(&self, left: u16, right: u16) -> i16 {
        self.kern
            .as_ref()
            .map(|k| k.kerning(left, right))
            .unwrap_or(0)
    }

    // ---- font variations (fvar / avar) ------------------------------------

    /// Whether this font carries an `fvar` table with at least one
    /// variation axis. (Distinct from [`Font::is_variable`], which keys
    /// off the CFF2 `VariationStoreOffset`; a font may have `fvar` axes
    /// for a TrueType-outline sibling while this CFF view is static.)
    pub fn has_variation_axes(&self) -> bool {
        self.fvar.as_ref().is_some_and(|f| f.axis_count() > 0)
    }

    /// The `fvar` table view, if present. Exposes the variation axes
    /// (tag / min / default / max / flags / name ID) and named instances.
    pub fn fvar(&self) -> Option<&FvarTable> {
        self.fvar.as_ref()
    }

    /// The `avar` table view, if present.
    pub fn avar(&self) -> Option<&AvarTable> {
        self.avar.as_ref()
    }

    /// The `COLR` color-table view, if present. Version-1 color glyphs
    /// resolve their variable paints against normalized instance
    /// coordinates — pass the result of [`Font::normalize_coords`] to
    /// [`tables::colr::ColrTable::paint`].
    pub fn colr(&self) -> Option<&ColrTable<'a>> {
        self.colr.as_ref()
    }

    /// The `CPAL` palette-table view, if present.
    pub fn cpal(&self) -> Option<&CpalTable<'a>> {
        self.cpal.as_ref()
    }

    /// The sRGB color record for `(palette_index, entry_index)` from
    /// the `CPAL` table, or `None` when the font has no `CPAL` table
    /// or either index is out of range. Palette 0 is the default
    /// palette. Note that the `COLR` foreground sentinel entry index
    /// (0xFFFF) intentionally resolves to `None` here — substitute the
    /// application-determined text foreground color instead.
    pub fn palette_color(&self, palette_index: u16, entry_index: u16) -> Option<ColorRecord> {
        self.cpal.as_ref()?.color(palette_index, entry_index)
    }

    /// The user-interface label for a palette, resolved through the
    /// `name` table from the `CPAL` version-1 Palette Label Array.
    /// `None` when there is no label or no matching `name` record.
    pub fn palette_label(&self, palette_index: u16) -> Option<&str> {
        let name_id = self.cpal.as_ref()?.palette_label(palette_index)?;
        self.name.find(name_id)
    }

    /// The user-interface label for a palette **entry** (shared by all
    /// palettes; e.g. "Outline", "Fill"), resolved through the `name`
    /// table from the `CPAL` version-1 Palette Entry Label Array.
    pub fn palette_entry_label(&self, entry_index: u16) -> Option<&str> {
        let name_id = self.cpal.as_ref()?.palette_entry_label(entry_index)?;
        self.name.find(name_id)
    }

    /// The version-0 `COLR` color glyph of `glyph_id` resolved to
    /// concrete colors: the bottom-up layer run as
    /// `(layer glyph ID, fill color)` pairs, with each layer's `CPAL`
    /// palette index resolved against palette number `palette` (0 =
    /// default) and the 0xFFFF sentinel resolved to `foreground`.
    /// `None` when the font lacks either table, the glyph has no
    /// version-0 color glyph, or a layer references an out-of-range
    /// palette entry.
    pub fn v0_layer_colors(
        &self,
        glyph_id: u16,
        palette: u16,
        foreground: ColorRecord,
    ) -> Option<Vec<(u16, ResolvedColor)>> {
        let colr = self.colr.as_ref()?;
        let cpal = self.cpal.as_ref()?;
        colr.v0_layers(glyph_id)?
            .iter()
            .map(|l| Some((l.glyph_id, l.resolve(cpal, palette, foreground)?)))
            .collect()
    }

    /// The `sbix` standard-bitmap-graphics view, if present.
    pub fn sbix(&self) -> Option<&SbixTable<'a>> {
        self.sbix.as_ref()
    }

    /// The `sbix` bitmap graphic for a glyph at a requested PPEM size:
    /// picks the best strike ([`tables::sbix::SbixTable::best_strike`]
    /// — exact match, else closest larger, else largest) and follows
    /// `'dupe'` redirects. `None` when the font has no `sbix` table,
    /// the strike has no data for the glyph, or the entry is
    /// malformed.
    pub fn sbix_glyph(&self, glyph_id: u16, ppem: u16) -> Option<GlyphGraphic<'a>> {
        self.sbix
            .as_ref()?
            .best_strike(ppem)?
            .glyph_graphic_resolved(glyph_id)
            .ok()?
    }

    /// The `EBLC` embedded-bitmap-locator view, if present.
    pub fn eblc(&self) -> Option<&BitmapLocationTable<'a>> {
        self.eblc.as_ref()
    }

    /// The `CBLC` color-bitmap-locator view, if present.
    pub fn cblc(&self) -> Option<&BitmapLocationTable<'a>> {
        self.cblc.as_ref()
    }

    /// The `EBDT` embedded-bitmap-data view, if present.
    pub fn ebdt(&self) -> Option<&BitmapDataTable<'a>> {
        self.ebdt.as_ref()
    }

    /// The `CBDT` color-bitmap-data view, if present.
    pub fn cbdt(&self) -> Option<&BitmapDataTable<'a>> {
        self.cbdt.as_ref()
    }

    /// The `EBSC` embedded-bitmap-scaling view, if present.
    pub fn ebsc(&self) -> Option<&EbscTable> {
        self.ebsc.as_ref()
    }

    /// The `SVG ` glyph-description view, if present.
    pub fn svg(&self) -> Option<&SvgTable<'a>> {
        self.svg.as_ref()
    }

    /// The SVG document describing `glyph_id`, if the font's `SVG `
    /// table covers it. The glyph's description is the element with
    /// id `glyph<ID>` inside [`SvgDocument::data`] (plain-text or
    /// gzip-encoded UTF-8 SVG — check [`SvgDocument::is_gzip`]).
    pub fn svg_document(&self, glyph_id: u16) -> Option<SvgDocument<'a>> {
        self.svg.as_ref()?.document_for_glyph(glyph_id)
    }

    /// One glyph's embedded **monochrome / grayscale** bitmap at a
    /// requested PPEM: picks the best `EBLC` strike, locates the
    /// glyph, and decodes the `EBDT` entry. Returns the strike's
    /// `BitmapSize` (for `bit_depth` and line metrics), the
    /// `BitmapLocation` (for index-table metrics), and the decoded
    /// data. `None` when either table is absent or the glyph has no
    /// bitmap in the chosen strike; `Some(Err)` when the tables are
    /// malformed.
    pub fn embedded_bitmap(
        &self,
        glyph_id: u16,
        ppem: u8,
    ) -> Option<Result<(BitmapSize, BitmapLocation, GlyphBitmapData<'a>), Error>> {
        Self::lookup_bitmap(self.eblc.as_ref()?, self.ebdt.as_ref()?, glyph_id, ppem)
    }

    /// One glyph's embedded **color** bitmap at a requested PPEM —
    /// the `CBLC` + `CBDT` counterpart of
    /// [`Font::embedded_bitmap`].
    pub fn color_bitmap(
        &self,
        glyph_id: u16,
        ppem: u8,
    ) -> Option<Result<(BitmapSize, BitmapLocation, GlyphBitmapData<'a>), Error>> {
        Self::lookup_bitmap(self.cblc.as_ref()?, self.cbdt.as_ref()?, glyph_id, ppem)
    }

    fn lookup_bitmap(
        loc_table: &BitmapLocationTable<'a>,
        data_table: &BitmapDataTable<'a>,
        glyph_id: u16,
        ppem: u8,
    ) -> Option<Result<(BitmapSize, BitmapLocation, GlyphBitmapData<'a>), Error>> {
        let size_index = loc_table.best_size(ppem)?;
        let size = loc_table.sizes()[size_index];
        let loc = match loc_table.locate(size_index, glyph_id) {
            Ok(Some(loc)) => loc,
            Ok(None) => return None,
            Err(e) => return Some(Err(e)),
        };
        Some(data_table.glyph_data(&loc).map(|d| (size, loc, d)))
    }

    /// Number of variation axes (`fvar.axisCount`); `0` for a
    /// non-variable font.
    pub fn axis_count(&self) -> usize {
        self.fvar.as_ref().map(|f| f.axis_count()).unwrap_or(0)
    }

    /// The variation axes in axis order (empty for a non-variable font).
    pub fn variation_axes(&self) -> &[VariationAxis] {
        self.fvar.as_ref().map(|f| f.axes()).unwrap_or(&[])
    }

    /// The named instances (empty for a non-variable font).
    pub fn named_instances(&self) -> &[NamedInstance] {
        self.fvar.as_ref().map(|f| f.instances()).unwrap_or(&[])
    }

    /// Normalize a user-scale axis-coordinate tuple to the `[-1, 1]`
    /// scale every variation table consumes. This is the **full**
    /// normalization pipeline (ISO/IEC 14496-22:2019 §7.3.1.1 +
    /// §7.3.1.3): `fvar` default normalization followed by the `avar`
    /// segment-map refinement (when an `avar` table is present).
    ///
    /// `user_coords` is matched positionally against the axes; a short
    /// slice fills remaining axes with their defaults, a long slice
    /// ignores the surplus. The result has `axis_count` entries (empty
    /// for a non-variable font).
    pub fn normalize_coords(&self, user_coords: &[f32]) -> Vec<f32> {
        let Some(fvar) = self.fvar.as_ref() else {
            return Vec::new();
        };
        let normalized = fvar.normalize_coords(user_coords);
        match self.avar.as_ref() {
            Some(avar) => avar.apply(&normalized),
            None => normalized,
        }
    }

    /// Decode a glyph outline for a specific variation instance,
    /// expressed in **user-scale axis coordinates** (e.g. `wght = 700`).
    ///
    /// This is the convenience that ties the variable-font tables
    /// together: it normalizes `user_coords` through `fvar` + `avar`,
    /// derives the per-region interpolation scalars from the CFF2
    /// `ItemVariationStore`'s default `ItemVariationData` (the algorithm
    /// of §7.1.7), and feeds them to the CFF2 variation-aware charstring
    /// interpreter. For a CFF1 (non-variable) font the static outline is
    /// returned and `user_coords` is ignored.
    ///
    /// Callers that already hold normalized region scalars can keep using
    /// the lower-level [`Font::glyph_outline_var`].
    pub fn glyph_outline_for_axes(
        &self,
        glyph_id: u16,
        user_coords: &[f32],
    ) -> Result<CubicOutline, Error> {
        if glyph_id >= self.maxp.num_glyphs {
            return Err(Error::GlyphOutOfRange(glyph_id));
        }
        match &self.cff {
            CffFlavour::Cff1(c) => c.glyph_outline(glyph_id),
            CffFlavour::Cff2(c) => {
                let normalized = self.normalize_coords(user_coords);
                // The default ItemVariationData (vsindex 0) drives the
                // region scalars `glyph_outline_var` expects. A CFF2 font
                // without a VariationStore is non-variable: pass an empty
                // scalar slice (default instance).
                let scalars = c
                    .variation_store()
                    .and_then(|ivs| ivs.region_scalars(0, &normalized))
                    .unwrap_or_default();
                c.glyph_outline_var(glyph_id as u32, &scalars)
            }
        }
    }

    // ---- style attributes (STAT) ------------------------------------------

    /// The `STAT` table view, if present. Exposes the design-axis
    /// records, the axis-value tables (formats 1-4), and the
    /// elided-fallback name ID.
    pub fn stat(&self) -> Option<&StatTable> {
        self.stat.as_ref()
    }

    /// `(major, minor)` version of the `STAT` table, if present.
    pub fn stat_version(&self) -> Option<(u16, u16)> {
        self.stat.as_ref().map(|s| s.version())
    }

    // ---- metrics variations (MVAR) ----------------------------------------

    /// The `MVAR` table view, if present. Exposes the value records and
    /// the ItemVariationStore that vary font-wide metrics.
    pub fn mvar(&self) -> Option<&MvarTable> {
        self.mvar.as_ref()
    }

    /// The per-instance adjustment for a font-wide metric value tag
    /// (e.g. `b"hasc"` = `OS/2.sTypoAscender`), given **user-scale** axis
    /// coordinates. Normalizes `user_coords` through `fvar`/`avar`, then
    /// resolves the `MVAR` delta. Returns `0.0` when there is no `MVAR`
    /// table, the tag is absent (constant metric), or the font has no
    /// axes. Add the result to the base metric to get the instance value.
    pub fn metric_variation(&self, tag: &[u8; 4], user_coords: &[f32]) -> f32 {
        let Some(mvar) = self.mvar.as_ref() else {
            return 0.0;
        };
        let normalized = self.normalize_coords(user_coords);
        mvar.metric_delta(tag, &normalized)
    }

    // ---- per-glyph metrics variations (HVAR / VVAR) -----------------------

    /// The `HVAR` table view, if present.
    pub fn hvar(&self) -> Option<&MetricsVariations> {
        self.hvar.as_ref()
    }

    /// The `VVAR` table view, if present.
    pub fn vvar(&self) -> Option<&MetricsVariations> {
        self.vvar.as_ref()
    }

    /// The per-instance **advance-width** adjustment for a glyph from
    /// `HVAR`, given user-scale axis coordinates. Returns `0.0` when
    /// there is no `HVAR` table. Add to the `hmtx` advance to get the
    /// instance advance width.
    pub fn advance_width_variation(&self, glyph_id: u16, user_coords: &[f32]) -> f32 {
        let Some(hvar) = self.hvar.as_ref() else {
            return 0.0;
        };
        let normalized = self.normalize_coords(user_coords);
        hvar.advance(glyph_id, &normalized)
    }

    /// The per-instance **advance-height** adjustment for a glyph from
    /// `VVAR`, given user-scale axis coordinates. Returns `0.0` when
    /// there is no `VVAR` table. Add to the `vmtx` advance height.
    pub fn advance_height_variation(&self, glyph_id: u16, user_coords: &[f32]) -> f32 {
        let Some(vvar) = self.vvar.as_ref() else {
            return 0.0;
        };
        let normalized = self.normalize_coords(user_coords);
        vvar.advance(glyph_id, &normalized)
    }

    // ---- baselines (BASE) -------------------------------------------------

    /// The `BASE` table view, if present.
    pub fn base(&self) -> Option<&BaseTable> {
        self.base.as_ref()
    }

    /// Convenience: the baseline coordinate (design units) for a given
    /// `(script_tag, baseline_tag)` on an axis, from the `BASE` table.
    /// `None` when there is no `BASE` table or the axis/script/baseline
    /// is absent.
    pub fn baseline_coord(
        &self,
        axis: BaseAxis,
        script_tag: &[u8; 4],
        baseline_tag: &[u8; 4],
    ) -> Option<i16> {
        self.base
            .as_ref()?
            .baseline_coord(axis, script_tag, baseline_tag)
    }

    // ---- vertical origin (VORG) -------------------------------------------

    /// The `VORG` table view, if present.
    pub fn vorg(&self) -> Option<&VorgTable> {
        self.vorg.as_ref()
    }

    /// The Y coordinate (design units) of a glyph's vertical origin from
    /// the `VORG` table, or `None` when the font has no `VORG`. (Without
    /// `VORG`, the vertical origin is the glyph bbox top plus the `vmtx`
    /// top side bearing.)
    pub fn vertical_origin_y(&self, glyph_id: u16) -> Option<i16> {
        self.vorg.as_ref().map(|v| v.vert_origin_y(glyph_id))
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

    /// Glyph name for `glyph_id` from the `post` table, as raw bytes.
    ///
    /// Resolves all three name-bearing `post` formats via
    /// [`PostTable::glyph_name`]: format 1.0 (glyph ID → standard
    /// Macintosh name), format 2.0 (`glyphNameIndex < 258` → standard
    /// name; `>= 258` → custom Pascal string), and format 2.5
    /// (`glyph_id + offset` → standard name). Standard names are
    /// returned as their UTF-8 byte view; custom names are the raw
    /// on-disk Pascal-string bytes (ASCII by convention, but not
    /// guaranteed UTF-8 by the spec).
    ///
    /// `None` for format 3.0 / `Other` `post` tables, when the table is
    /// absent, or when the glyph has no resolvable name. Callers
    /// wanting a typed split between standard and custom names should
    /// use [`Font::post`] + [`PostTable::glyph_name`].
    pub fn post_glyph_name(&self, glyph_id: u16) -> Option<&'a [u8]> {
        self.post
            .as_ref()?
            .glyph_name(glyph_id)
            .map(|n| n.as_bytes())
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

    // ---- `GDEF` table -----------------------------------------------------

    /// Borrow the parsed `GDEF` table, if present.
    ///
    /// GDEF is optional per the OpenType spec — a font without any
    /// GSUB / GPOS layout lookups can legitimately omit it, and many
    /// stripped-down system fonts do. Absence surfaces as `None`
    /// rather than rejecting the whole font.
    pub fn gdef(&self) -> Option<&GdefTable<'a>> {
        self.gdef.as_ref()
    }

    /// `GDEF` `(majorVersion, minorVersion)` pair (`(1, 0)`, `(1, 2)`,
    /// or `(1, 3)`), if the table is present.
    pub fn gdef_version(&self) -> Option<(u16, u16)> {
        self.gdef.as_ref().map(GdefTable::version)
    }

    /// Spec [`GlyphClass`] for `glyph_id`, from `GDEF.GlyphClassDef`.
    ///
    /// `None` when `GDEF` is absent, the GlyphClassDef sub-table is
    /// absent, or the glyph is unclassified (the spec's class-0 default
    /// for any glyph not covered by the on-disk records).
    pub fn glyph_class(&self, glyph_id: u16) -> Option<GlyphClass> {
        self.gdef.as_ref().and_then(|g| g.glyph_class(glyph_id))
    }

    /// Mark-attachment class for `glyph_id`, from
    /// `GDEF.MarkAttachClassDef`. Returns `0` if the table is absent,
    /// the sub-table is absent, or the glyph is unclassified — the
    /// "unfiltered" semantics `LookupFlag.markAttachmentType` uses.
    pub fn mark_attach_class(&self, glyph_id: u16) -> u16 {
        self.gdef
            .as_ref()
            .map(|g| g.mark_attach_class(glyph_id))
            .unwrap_or(0)
    }

    // ---- `GSUB` / `GPOS` layout tables ------------------------------------

    /// Borrow the parsed `GSUB` (Glyph Substitution Table) view, if
    /// present.
    ///
    /// GSUB is optional per the OpenType spec — a font that performs
    /// no glyph substitution legitimately omits it. The view surfaces
    /// the header (`majorVersion` + `minorVersion` +
    /// `featureVariationsOffset`) and `ScriptList` / `FeatureList` /
    /// `LookupList` walks. Decoding the per-lookup substitution
    /// subtable formats (GsubLookupType 1–8) is deferred to a future
    /// round.
    pub fn gsub(&self) -> Option<&GsubTable<'a>> {
        self.gsub.as_ref()
    }

    /// `GSUB` `(majorVersion, minorVersion)`, if the table is present.
    pub fn gsub_version(&self) -> Option<(u16, u16)> {
        self.gsub.as_ref().map(GsubTable::version)
    }

    /// Borrow the parsed `GPOS` (Glyph Positioning Table) view, if
    /// present.
    ///
    /// GPOS is optional per the OpenType spec — a font with no
    /// kerning or other positioning lookups legitimately omits it.
    /// The view surfaces the header and `ScriptList` / `FeatureList`
    /// / `LookupList` walks. Decoding the per-lookup positioning
    /// subtable formats (GposLookupType 1–9: SinglePos, PairPos,
    /// CursivePos, MarkBasePos, MarkLigPos, MarkMarkPos,
    /// ContextPos, ChainContextPos, Extension) is deferred to a
    /// future round.
    pub fn gpos(&self) -> Option<&GposTable<'a>> {
        self.gpos.as_ref()
    }

    /// `GPOS` `(majorVersion, minorVersion)`, if the table is present.
    pub fn gpos_version(&self) -> Option<(u16, u16)> {
        self.gpos.as_ref().map(GposTable::version)
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

    // ---- Adobe Glyph List (AGL) integration ------------------------------

    /// Resolve a PostScript glyph name to a glyph id by routing through
    /// the **Adobe Glyph List (AGL 2.0)** name → Unicode codepoint
    /// table (`crate::agl`) and then through the font's own `cmap`.
    ///
    /// This is the right tool when callers have a PostScript glyph
    /// name in hand (e.g. parsed from a PDF content stream, or from a
    /// `post`-format-2.0 Pascal-string entry) and need to map back to
    /// a glyph id without first decoding the name into a Unicode
    /// scalar.
    ///
    /// Two-step semantics:
    ///
    /// 1. Look up `name` in AGL via [`crate::agl::name_to_codepoint`].
    ///    `None` if the name isn't in AGL.
    /// 2. Map that codepoint to a glyph id via the font's `cmap`. `None`
    ///    if the font doesn't encode that codepoint.
    ///
    /// The AGL Specification's §6 component-name decomposition
    /// (`f_f_i` → `ffi`, `uniXXXX` → `U+XXXX`) is **not** applied —
    /// the AGL spec document itself is not staged under
    /// `docs/text/opentype/`. Callers that need the §6 algorithm can
    /// implement it in their own code on top of this exact-match
    /// lookup.
    pub fn glyph_id_from_agl_name(&self, name: &str) -> Option<u16> {
        let cp = agl::name_to_codepoint(name)?;
        self.glyph_index(cp)
    }

    /// Canonical Adobe Glyph List name for `glyph_id`, if any.
    ///
    /// Resolution order, mirroring "use the font's own knowledge
    /// first, then fall back to the standard":
    ///
    /// 1. The CFF charset → Strings name (the same lookup as
    ///    [`Font::glyph_name`]). For CFF1 fonts this surfaces the
    ///    font's authored PostScript name regardless of whether it
    ///    happens to be an AGL entry. Always `None` for CFF2 fonts
    ///    (CFF2 has no Charset / Strings).
    /// 2. The `post` table version-2.0 Pascal-string tail (the same
    ///    lookup as [`Font::post_glyph_name`]); decoded as UTF-8 and
    ///    returned only when the on-disk bytes are valid UTF-8.
    /// 3. The AGL reverse-lookup table — if the glyph is reachable
    ///    from a `cmap` entry, the AGL name of that codepoint.
    ///
    /// `None` only when none of the three sources have a name for
    /// this glyph.
    pub fn agl_glyph_name(&self, glyph_id: u16) -> Option<&str> {
        if glyph_id >= self.maxp.num_glyphs {
            return None;
        }
        // 1. CFF charset → Strings.
        if let Some(name) = self.glyph_name(glyph_id) {
            return Some(name);
        }
        // 2. post-format-2.0 Pascal-string tail (UTF-8-clean only).
        if let Some(bytes) = self.post_glyph_name(glyph_id) {
            if let Ok(s) = std::str::from_utf8(bytes) {
                return Some(s);
            }
        }
        // 3. AGL reverse lookup keyed on the glyph's `cmap`
        //    codepoint. The CmapTable doesn't expose a reverse
        //    iterator, so we walk the BMP only — the AGL itself is
        //    BMP-only (no astral entries), so any astral glyph would
        //    never match anyway.
        for cp in 0u32..0x1_0000 {
            if let Some(c) = char::from_u32(cp) {
                if self.cmap.lookup(cp) == Some(glyph_id) {
                    if let Some(name) = agl::codepoint_to_name(c) {
                        return Some(name);
                    }
                    // Found the codepoint but it's not in AGL; keep
                    // scanning in case another encoded codepoint maps
                    // to the same glyph and *is* in AGL.
                }
            }
        }
        None
    }
}
