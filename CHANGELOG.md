# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- **GSUB Lookup Type 4 (ligature substitution) decoded** via a new
  `LigatureSubst` typed view, joining the round-247 `SingleSubst`
  work. Source: `docs/text/opentype/otspec-gsub.html` §"Lookup type 4
  subtable: ligature substitution"; the Coverage table is re-used
  from `tables::gdef::Coverage` (the shared common-layout primitive,
  per `otspec-chapter2-common-layout-tables.html`). `LigatureSubst`
  decodes the subtable header (`format`, `coverageOffset`,
  `ligatureSetCount`, `ligatureSetOffsets[]`); `LigatureSet` decodes
  the per-first-component `(ligatureCount, ligatureOffsets[])` pair;
  `Ligature` decodes `(ligatureGlyph, componentCount,
  componentGlyphIDs[componentCount - 1])`. The first component glyph
  is implicit (it's the Coverage entry that selected the
  LigatureSet), so the on-disk `componentGlyphIDs[]` array starts at
  the second component (input glyph sequence index = 1) per the
  spec. A `componentCount` of zero is rejected as `BadStructure` —
  the spec counts the first component, so zero leaves the
  first-component invariant unsatisfiable.
  `LigatureSubst::substitute(input: &[u16]) -> Option<(u16, u16)>`
  is the shaper-path entrypoint: the first glyph of `input` selects
  a LigatureSet via Coverage, the set is walked in array order
  (= preference order, "longer / preferred first" per spec), and the
  first Ligature whose `componentGlyphIDs[..]` matches the input
  tail wins — returning `(ligatureGlyph, componentCount)`, i.e. the
  substitute glyph and the total number of input glyphs consumed.
  `None` is returned for empty input, an uncovered first glyph, or
  no matching Ligature in the selected set. `LigatureSubst::iter()`
  yields `(coverage_glyph, LigatureSet)` pairs in ascending Coverage
  order; `LigatureSet::ligature(i)` borrows the Ligature at
  preference index `i`; `Ligature::component_glyphs()` yields the
  tail glyphs in input order. New convenience accessor
  `GsubTable::ligature_subst(lookup_i, sub_i)` mirrors
  `single_subst(...)`: `None` for out-of-range indices,
  `Some(Err(BadStructure))` when the referenced lookup is not
  declared as type 4. `LigatureSubst`, `LigatureSubstIter`,
  `LigatureSet`, `Ligature`, and `LigatureComponentIter` are
  re-exported at the crate root. Synthetic-byte unit tests cover the
  spec's worked Example 6 (Coverage = `{e, f}`, e-set = `[etc]`,
  f-set = `[ffi, fi]` with ffi preferred), the `format != 1`
  rejection, out-of-range `coverageOffset`, truncated
  `ligatureSetOffsets[]`, the `componentCount == 0` rejection, the
  `substitute()` first-match preference rule, the empty / uncovered /
  no-match paths, and an end-to-end GSUB byte tower whose only
  lookup is the same Example-6 subtable. A Source Sans 3 integration
  test walks every type-4 lookup, decodes every LigatureSet and
  Ligature, verifies Coverage iteration is ascending, every ligature
  glyph and every component glyph fits inside `maxp.numGlyphs`,
  `componentCount >= 1`, the tail iterator yields exactly
  `componentCount - 1` entries, and the first Ligature in each set
  round-trips through `substitute()` on its own canonical input.
  The other lookup types (2 Multiple, 3 Alternate, 5 Contextual,
  6 Chained-context, 7 Extension, 8 Reverse-chained-single) remain
  raw sub-slices pending dedicated rounds.

- **GSUB Lookup Type 1 (single substitution) decoded** via a new
  `SingleSubst` typed view — both on-disk subtable formats
  (`SingleSubstFormat1` with `deltaGlyphID` modulo-65536 wrap, and
  `SingleSubstFormat2` with the per-Coverage-index substitute
  array). Source: `docs/text/opentype/otspec-gsub.html` §"Lookup
  type 1 subtable: single substitution"; the Coverage table is
  re-used from `tables::gdef::Coverage` (shared with GPOS and the
  rest of GSUB per `otspec-chapter2-common-layout-tables.html`).
  `SingleSubst::substitute(input)` returns the rewritten glyph
  (`None` when uncovered), and `SingleSubst::iter()` yields
  `(input_glyph, output_glyph)` pairs in ascending input order.
  The format-1 path applies the spec's "addition is modulo 65536"
  and "if result < 0, add 65536" rules via `rem_euclid(65536)` on
  an `i32`. New convenience accessor `GsubTable::single_subst(
  lookup_i, sub_i)` returns `Option<Result<SingleSubst, Error>>`:
  `None` for out-of-range indices, `Some(Err(BadStructure))` when
  the referenced lookup is not declared as type 1. Also surfaced:
  named constants `GSUB_LOOKUP_TYPE_SINGLE` …
  `GSUB_LOOKUP_TYPE_REVERSE_CHAINED_SINGLE` (values 1 .. 8) for
  the `GsubLookupType` enumeration. Synthetic-byte unit tests
  cover format-1 (positive / negative modular-arithmetic wrap),
  format-2 (round-trip + glyph-count / Coverage-length disagreement
  rejection), unknown-format and truncation paths, and one
  end-to-end GSUB synthetic that walks the header → ScriptList →
  Lookup chain into a real SingleSubstFormat1 subtable. A Source
  Sans 3 integration test decodes all 57 of its type-1 lookups
  (12 SingleSubstFormat1 + 45 SingleSubstFormat2 subtables),
  verifies the Coverage iterator is ascending, that every
  `(input, output)` pair stays within `maxp.numGlyphs`, and that
  the iterator agrees with point lookups via
  `substitute(input)`. The other lookup types (2 Multiple,
  3 Alternate, 4 Ligature, 5 Contextual, 6 Chained-context,
  7 Extension, 8 Reverse-chained-single) remain raw sub-slices
  pending dedicated rounds.

- **`GSUB` and `GPOS` table headers parsed**, with the shared
  `ScriptList` / `Script` / `LangSys` / `FeatureList` / `Feature` /
  `LookupList` / `Lookup` / `LookupFlag` common-layout primitives.
  Sources: `docs/text/opentype/otspec-gsub.html`,
  `docs/text/opentype/otspec-gpos.html`, and
  `docs/text/opentype/otspec-chapter2-common-layout-tables.html`.
  Round 229 ships a new `tables::layout` module with the chapter-2
  primitives and `tables::gsub` / `tables::gpos` modules with the
  per-table header decoders. Both v1.0 (10-byte) and v1.1 (14-byte
  with `featureVariationsOffset`) headers are recognised; unknown
  versions and truncated v1.1 trailers are rejected with
  `Error::BadStructure` / `Error::UnexpectedEof`. The
  `LookupFlag` wrapper exposes the spec's bit vocabulary
  (`RIGHT_TO_LEFT`, `IGNORE_BASE_GLYPHS`, `IGNORE_LIGATURES`,
  `IGNORE_MARKS`, `USE_MARK_FILTERING_SET`, and the
  `MARK_ATTACHMENT_CLASS_FILTER` high-byte mask) with named
  boolean accessors. `Lookup` parses the conditionally-present
  `markFilteringSet` field (decoded iff
  `USE_MARK_FILTERING_SET` is set, per the spec's variable-length
  Lookup rule); per-subtable raw byte slices are exposed via
  `Lookup::subtable_bytes(i)` for downstream subtable-format work.
  New on `Font`: `gsub()`, `gsub_version()`, `gpos()`,
  `gpos_version()`. The view types are re-exported as `GsubView` /
  `GposView`; the chapter-2 primitives (`ScriptList`,
  `ScriptListIter`, `Script`, `LangSys`, `NO_REQUIRED_FEATURE`,
  `FeatureList`, `FeatureListIter`, `Feature`, `LookupList`,
  `LookupListIter`, `Lookup`, `LookupFlag`) are re-exported at
  the crate root. The per-lookup substitution / positioning
  subtable formats (GSUB lookup types 1–8, GPOS lookup types 1–9)
  themselves are surfaced only as raw byte slices; decoding their
  interiors (ligature sets, ValueRecords, Anchors, MarkArrays, …)
  is deferred to a future round, as are the FeatureVariations and
  feature-parameter (`'cv01'–'cv99'` / `'ss01'–'ss20'` / `'size'`)
  table formats.
- **`GDEF` Glyph Definition Table parsed**, source
  `docs/text/opentype/otspec-gdef.html` with the shared `Coverage` /
  `ClassDef` formats pulled from
  `docs/text/opentype/otspec-chapter2-common-layout-tables.html`.
  Round 222 ships a new `tables::gdef` module with `GdefTable`,
  `Coverage` (formats 1 + 2), `ClassDef` (formats 1 + 2), `AttachList`
  / `AttachPoint`, `LigCaretList` / `LigGlyph` / `CaretValue` (formats
  1 + 2 + 3), `MarkGlyphSets`, and a `GlyphClass` enum mapping the
  spec's GlyphClassDef numbers (1 = Base, 2 = Ligature, 3 = Mark,
  4 = Component). All three header versions are recognised — v1.0
  (12 bytes), v1.2 (14 bytes, `markGlyphSetsDefOffset`), and v1.3
  (18 bytes, `itemVarStoreOffset`); the ItemVariationStore itself is
  surfaced only as its raw offset (variation-store decoding is
  deferred). New on `Font`: `gdef()`, `gdef_version()`,
  `glyph_class(gid)`, and `mark_attach_class(gid)`. All accessors
  borrow zero-copy sub-slices against the original table bytes;
  `Coverage::index_of` and `ClassDef::class_of` binary-search the
  spec-sorted on-disk records. `Coverage` and the GDEF sub-table
  types are re-exported at the crate root.
- **Adobe Glyph List (AGL 2.0) name ↔ codepoint mapping**, source
  `docs/text/opentype/spec/agl-glyphlist.txt` (file format
  documented in `docs/text/opentype/spec/agl-aglfn-README.md`).
  Round 217 ships the table verbatim under `data/agl-glyphlist.txt`
  and exposes it through a new `agl` module:
  - `agl::name_to_codepoints(name) -> Option<Codepoints<'static>>`
    surfaces every AGL 2.0 entry — `Codepoints::Single(char)` for
    the 4200 single-codepoint entries, `Codepoints::Sequence(&[char])`
    for the 81 multi-codepoint entries (most are Hebrew base + vowel
    pointing combinations such as `dalethatafpatah → [U+05D3,
    U+05B2]`).
  - `agl::name_to_codepoint` is the common-case helper that returns
    `Some` only for single-codepoint entries.
  - `agl::codepoint_to_name(cp) -> Option<&'static str>` is the
    reverse lookup, keyed on a single Unicode scalar value. When
    multiple AGL aliases share a codepoint (e.g. ~17 Hebrew names
    aliasing U+05B8), the alphabetically-first name in AGL's on-disk
    order is returned.
  - `agl::entries`, `agl::entry_count` (= 4281), and
    `agl::distinct_codepoint_count` (= 3680) round out the
    introspection surface.
  - The AGL Specification §6 component-name decomposition algorithm
    (`f_f_i`, `uniXXXX`, `uXXXXX`) is **not** implemented because
    the AGL Specification document itself is not staged under
    `docs/text/opentype/`; only the raw table is. The current API
    accommodates a future §6 layer without changes.
- **`Font::glyph_id_from_agl_name(name) -> Option<u16>`** routes a
  PostScript glyph name through AGL then through the font's `cmap`,
  giving callers a one-call name → glyph-id resolver.
- **`Font::agl_glyph_name(gid) -> Option<&str>`** returns a canonical
  glyph name, preferring the font's authored CFF charset → Strings
  name; falling back to the `post` table version-2.0 Pascal-string
  tail (UTF-8-clean); finally falling back to the AGL reverse-lookup
  table keyed on whichever BMP codepoint the font's `cmap` routes to
  this glyph. CFF2 / TrueType-outline fonts now have a path to a
  PostScript name without a per-glyph CFF Strings table.

- **CFF2 (Compact Font Format Version 2) metadata parser**, spec
  `docs/text/opentype/otspec-cff2.html`. Round 211 lifts the
  blanket `Error::Cff2NotImplemented` rejection at parse time —
  CFF2-flavoured OpenType fonts (`OTTO` sfnt + `CFF2` table) now
  parse through to a fully populated `Font`. The previously-deferred
  Type 2 + `blend` + `vsindex` interpreter for variable-font
  outlines remains deferred (`Font::glyph_outline` on a CFF2 font
  still returns `Cff2NotImplemented`); everything else, including
  the new structural CFF2 view, is reachable.
  - **CFF2 header** (§6 Table 8) decoded into a public
    `Cff2Header { major, minor, header_size, top_dict_size }`. The
    spec's "`headerSize` must be used when locating the Top DICT"
    rule is honoured (the field exists to allow future versions to
    grow the header); `major != 2`, `header_size < 5`, and a
    declared header that exceeds the input buffer are all rejected.
    `top_dict_offset()` and `global_subr_index_offset()` accessors
    expose the spec-derived "start of TopDICT" and "start of
    GlobalSubrINDEX" offsets respectively.
  - **CFF2 INDEX format** (§6 "INDEX data") decoded into a public
    `Cff2Index<'a>` type with `uint32 count` (CFF1's `Card16` is
    the v1 form) and all four `offsetSize` widths 1 / 2 / 3 / 4 =
    Offset8 / 16 / 24 / 32. The empty-INDEX sentinel for CFF2 is
    the 4-byte `count = 0` form (CFF1 was 2 bytes).
    `Cff2Index::entry(i)` returns zero-copy slices; truncation,
    out-of-range `offsetSize`, and out-of-range `i` surface as
    `Error::Cff(...)` / `Error::UnexpectedEof`.
  - **CFF2 Top DICT** (§7) parsed into a public `Cff2TopDict` with
    all five spec-permitted operators: `CharStringINDEXOffset`
    (op 17, required), `VariationStoreOffset` (op 24, required iff
    variable), `FontDICTINDEXOffset` (op 12 36, required),
    `FontDICTSelectOffset` (op 12 37, optional, used when there is
    more than one Font DICT), `FontMatrix` (op 12 7, optional;
    restricted to `[s 0 0 s 0 0]` per spec note "only matrices with
    uniform horizontal and vertical scaling without translation are
    permitted"). The spec default `0.001 0 0 0.001 0 0` is
    substituted when `FontMatrix` is absent (re-exported as the
    crate-root constant `DEFAULT_FONT_MATRIX`).
    Required operators missing surface as `Error::Cff(...)` with a
    descriptive string; non-uniform / translated FontMatrix shapes
    surface as `Error::Cff("CFF2 Top DICT FontMatrix: ...")`;
    negative offsets surface as `Error::Cff("... negative
    offset")`; unrecognised operators are tolerated (CFF1-style).
  - **`Cff2::parse`** walks the spec's `Header → TopDICT →
    GlobalSubrINDEX → CharStringINDEX → FontDICTINDEX` chain,
    enforcing the §7.2 "FontDICTINDEX must contain at least one
    FontDICT" invariant. Public accessors: `header()`,
    `top_dict()`, `glyph_count()`, `font_dict_count()`,
    `global_subr_count()`, `is_variable()`, `charstring(gid)`,
    `font_dict(i)`, `global_subr(i)`, `bytes()`.
  - **`Font` integration:** new `Font::is_cff2()`,
    `Font::cff2() -> Option<&Cff2>`, `Font::cff2_header()`,
    `Font::cff2_top_dict()`, `Font::is_variable()` accessors.
    `Font::cff()` becomes `Option<&Cff>` (returns `None` for CFF2
    fonts). `Font::cff_fd_count()` returns the CFF2 FontDICTINDEX
    count for CFF2 fonts. `Font::font_matrix()` routes through the
    CFF2 Top DICT for CFF2 fonts.
  - **CFF1-only accessors fall back to spec defaults on CFF2 fonts**:
    `font_bbox()` → `[0; 4]`; `italic_angle()` → `0.0`;
    `underline_position()` → `-100.0`; `underline_thickness()` →
    `50.0`; `is_fixed_pitch()` → `false`; `paint_type()` → `0`;
    `charstring_type()` → `2`; `stroke_width()` → `0.0`;
    `weight_name()`, `notice()`, `copyright()`, `version_string()`,
    `postscript()`, `base_font_name()`, `glyph_name(gid)`,
    `ps_name()`, `cid_registry()`, `cid_ordering()`,
    `cid_supplement()`, `unique_id()`, `synthetic_base()` all
    return `None`; `xuid()`, `base_font_blend()` return empty
    slices. CFF2 callers wanting the equivalent identity / metric
    strings should consult the sfnt `name` and `post` tables
    instead (per the spec's "CFF2 reuses sfnt-level tables"
    design); each accessor's rustdoc names the alternative.
  - **DICT operator byte range widened** from `0..=21` to
    `0..=21 ∪ {24}` so the shared `Dict` parser can recognise the
    CFF2-specific `VariationStoreOffset` operator. The CFF1 spec
    leaves bytes 22, 23, 25–27 reserved (TN5176 §4 Table 3); a CFF1
    font using any of those was already malformed and stays so.
  - **`Error::Cff2NotImplemented`** rephrased: now signals only the
    deferred charstring decode (the previous parse-time rejection
    is gone).

- OpenType **`name` table version 1** support, spec Microsoft /
  ISO/IEC 14496-22 (`docs/text/opentype/otspec-name.html`). The
  existing parser accepted version-1 tables but silently ignored
  the `langTagCount` / `langTagRecord[]` trailer; this push adds
  full parsing of that block and a `NameTable::lang_tag(id)`
  accessor that resolves a name record's `languageID >= 0x8000` to
  its UTF-16BE BCP 47 language-tag string (e.g. `"en"`, `"fr-CA"`,
  `"zh-Hant-HK"`). IDs outside the spec-declared range
  `[0x8000, 0x8000 + langTagCount)` surface as `None` per spec
  ("the identity of the language is unknown; such name records
  should not be used"); IDs `< 0x8000` are platform-specific
  numeric LCIDs (not tags) and likewise return `None`. Version-0
  tables always return `None`.
  - Truncation / overlap rejection: a v1 table missing the
    `langTagCount` field or the declared `LangTagRecord[]` array
    is `Error::UnexpectedEof`; a v1 table whose `storageOffset`
    overlaps the `LangTagRecord[]` array surfaces as
    `Error::BadStructure("name.storageOffset overlaps langTagRecord
    array")`.
- **`NameId` enum** covering every spec-defined name ID 0..=25,
  with `NameId::Reserved15` included as a distinct variant so a
  record with the spec-reserved ID 15 is still representable.
  `NameId::from_raw(u16) -> Option<Self>` decodes a raw nameID;
  `NameId::to_raw(self) -> u16` is the inverse.
- **`NameRecord` struct** + **`NameTable::records()`** iterator
  surfacing every on-disk name record in spec-sorted (platformID,
  encodingID, languageID, nameID) order. `NameRecord::name_id()`
  returns the standard `NameId` when the raw value is 0..=25;
  `NameTable::record_value(rec)` decodes the on-disk bytes into
  an owned `String`.
- **`NameTable::get(NameId)`** typed lookup, **`version()` /
  `record_count()` / `lang_tag_count()`** header accessors.
- UTF-16BE decoder hardening: the shared decoder now rejects
  unpaired *low* surrogates (alongside the existing unpaired
  *high* surrogate rejection), since both are malformed UTF-16
  per TN5176 §H.4.
- New on `Font`: `name()`, `name_version()`, `name_lang_tag(id)`,
  `name_string(NameId)`, `designer()`, `manufacturer()`,
  `description()`, `vendor_url()`, `designer_url()`, `license()`,
  `license_url()`, `trademark()`, `sample_text()`,
  `typographic_family()`, `typographic_subfamily()`, `wws_family()`,
  `wws_subfamily()`, `variations_ps_name_prefix()`,
  `unique_font_id()` (the name-ID-3 string; distinct from the
  CFF-Top-DICT-sourced `Font::unique_id()` integer).
- `NameId` + `NameRecord` re-exported at the crate root.
- Sixteen new `tables::name` unit tests cover the v0 baseline
  preserved by `find()`, version-rejection above 1, every
  `NameId` round-trip across raw 0..=25 plus the reserved-15
  invariant, v1 parsing with the spec's worked `en` /
  `zh-Hant-HK` example, `lang_tag` in-range + out-of-range +
  numeric-LCID + v0-default-None paths, records iteration in
  on-disk order, truncation rejection at the `langTagCount` field
  and inside the `LangTagRecord` array, storage-overlap rejection,
  past-end storage offset rejection, truncated record-array
  rejection, UTF-16BE surrogate-pair acceptance (`U+1F600`),
  unpaired-low-surrogate rejection, Mac Roman ASCII subset
  decoding, and the existing Windows-beats-Mac priority in
  `find()`. One new integration test against Source Sans 3
  Regular asserts every newly-surfaced `Font::name_*` accessor
  resolves to the expected string (or `None` where the font omits
  a record), iterates the records and verifies spec sort order
  end-to-end, and exercises the v0 `lang_tag` invariant.

- OpenType **`OS/2` and Windows Metrics table** decoder, spec
  Microsoft / ISO/IEC 14496-22 (`docs/text/opentype/otspec-os2.html`).
  Previously the table was reachable only as raw bytes through the
  generic `Font::table_data(b"OS/2")` accessor; the new `Os2Table`
  (plus the `EmbeddingPermission` enum and the `FS_TYPE_*` /
  `FS_SELECTION_*` mask constants, all re-exported at the crate
  root) decodes every spec-defined version (0..5).
  - All six version layouts handled: v0-short (68 bytes, Apple's
    TrueType Reference Manual variant) and v0-full (78 bytes,
    Microsoft's final v0 spec), v1 (86 bytes, adds
    `ulCodePageRange1/2`), v2/v3/v4 (96 bytes, adds `sxHeight`,
    `sCapHeight`, `usDefaultChar`, `usBreakChar`, `usMaxContext`,
    plus fsSelection bits 7–9 in v4), and v5 (100 bytes, adds
    `usLowerOpticalPointSize` / `usUpperOpticalPointSize`).
  - Every spec-defined header field decoded. Weight class
    (`usWeightClass`), width class (with the spec's "% of normal"
    1..9 lookup), `fsType` (raw + decoded `EmbeddingPermission`
    plus the "no subsetting" / "bitmap-only" bit predicates),
    subscript / superscript metrics, the strikeout pair,
    `sFamilyClass` decomposed into `(class, subclass)`, 10-byte
    PANOSE, four `ulUnicodeRange*` words plus a
    `has_unicode_range_bit(bit)` query, 4-byte `achVendID`,
    `fsSelection` (10 named style bits with per-bit predicates),
    first / last char index.
  - Version-gated tails reported as `Option` so callers can detect
    legacy-format truncation: typo metrics (`sTypoAscender`,
    `sTypoDescender`, `sTypoLineGap`, `usWinAscent`, `usWinDescent`)
    on v0-full+, code-page range on v1+ (with
    `has_code_page_bit(bit)` query), v2 extension fields on v2+,
    optical point-size range on v5 (raw TWIPs + a TWIPs/20 → points
    conversion helper).
  - New on `Font`: `os2()`, `os2_version()`, `weight_class()`,
    `width_class()`, `width_class_percent()`, `fs_type()`,
    `embedding_permission()`, `is_italic()`, `is_bold()`,
    `is_regular()`, `use_typo_metrics()`, `is_oblique()`,
    `vendor_id()`, `panose()`, `typo_ascender()`,
    `typo_descender()`, `typo_line_gap()`, `win_ascent()`,
    `win_descent()`, `x_height()`, `cap_height()`, `default_char()`,
    `break_char()`, `max_context()`.
  - Truncation is rejected per spec: < 68 bytes →
    `Error::UnexpectedEof`; a v1+ declaration shorter than its
    layout → `Error::BadStructure`; version > 5 →
    `Error::BadStructure`.
  - Nineteen new `tables::os2` unit tests cover every version-tail
    drop, every error path, the `usWidthClass` spec-table lookup,
    every `EmbeddingPermission` discriminant (including the
    spec-reserved bit-0 legacy case), per-bit `fsSelection` helpers,
    `has_unicode_range_bit` walking all four 32-bit words, the
    `sFamilyClass` (class, subclass) split round-trip, and the
    optical-size TWIPs / points conversion. One new integration
    test against Source Sans 3 Regular asserts its real-world v3
    96-byte `OS/2`: version 3, weight 400, width 5, `fsType = 0`
    (Installable), `achVendID = "ADBO"`, PANOSE family-type 2,
    Basic-Latin and Latin-1 bits set, typo / win metrics
    mutually consistent, `usBreakChar = 0x0020`, no v5 optical-size
    tail.

### Changed

- `cff::strings::glyph_name_to_codepoint` — a `None`-returning stub
  since round 1 — now delegates to `agl::name_to_codepoint`. The
  legacy Standard-Encoding fallback hook in `cff::encoding` is
  therefore functional for the first time. No public API change.

## [0.1.2](https://github.com/OxideAV/oxideav-otf/compare/v0.1.1...v0.1.2) - 2026-05-29

### Other

- decode `post` (PostScript) table — header + v2.0/v2.5 tails
- surface Private DICT hint zones (TN5176 §15 Table 23)
- surface UniqueID / XUID / SyntheticBase / PostScript / BaseFontName / BaseFontBlend (TN5176 §9 Tables 9 + 10)
- transcribe predefined Expert Encoding table (TN5176 Appendix B §2)

### Added

- OpenType **`post` (PostScript) table** decoder, spec
  Microsoft / ISO/IEC 14496-22 (`docs/text/opentype/otspec-post.html`).
  Previously the table was reachable through the generic
  `Font::table_data(b"post")` bytes accessor but never decoded; the
  new `PostTable` (and `PostFormat` enum, both re-exported at the
  crate root) decode the 32-byte header for every version and the
  format-2.0 / 2.5 tails.
  - Header fields: italic-angle (decoded from the on-disk 16.16
    `Fixed` to `f64`), underline position / thickness (FWORD), the
    `isFixedPitch` flag (any non-zero on the `uint32` rounds to
    `true` per spec), and the four VM hint fields
    `minMemType42` / `maxMemType42` / `minMemType1` /
    `maxMemType1`.
  - Format 3.0 (header only, mandatory for OpenType-CFF1 fonts per
    the spec's "Versions" preamble) — handled directly.
  - Format 2.0 — `numGlyphs` u16 + `glyphNameIndex[numGlyphs]` u16
    + Pascal-string tail. `PostTable::name_index(gid)` returns the
    raw index, and `name_string(pascal_index)` walks the
    Pascal-string list returning the requested entry as `&[u8]`.
  - Format 2.5 (deprecated) — `numGlyphs` u16 + signed-byte offset
    array; `PostTable::standard_offset(gid)` returns the raw `i8`.
  - Format 1.0 and any `Other` Version16Dot16 value (e.g. Apple's
    4.0 extension that the spec marks "not supported in OpenType")
    decode the header and skip the tail.
  - New on `Font`: `post()`, `post_format()`, `post_italic_angle()`,
    `post_underline_position()`, `post_underline_thickness()`,
    `post_is_fixed_pitch()`, and `post_glyph_name(gid)`. The last
    resolves format-2.0 Pascal-string names (the
    `glyphNameIndex >= 258` half); the standard-Macintosh 258-entry
    name list (the `< 258` half) is referenced from
    `otspec-post.html` but lives in Apple's TrueType Reference
    Manual chapter 6, which is currently only staged at its
    table-of-contents level — see the round-187 docs gap.
  - The `post` table is treated as optional (it is one of
    OpenType's nine required tables per `otff` spec, but real-world
    stripped-down fonts sometimes omit it); a missing `post` parses
    fine and the accessors return `None`.

  Seventeen new unit tests in `src/tables/post.rs` cover every
  version path including the spec's worked v2.0 + v2.5 examples,
  truncation rejection, italic-angle fractional decode, the
  `isFixedPitch` non-zero high-bit case, and the `Other`
  Version16Dot16 fallback. One new integration test against the
  Source Sans 3 fixture asserts format 3.0 + zero italic +
  `isFixedPitch == false` + negative `underlinePosition` + positive
  `underlineThickness` below `unitsPerEm`.

- CFF Private DICT hint zones surfaced on the public API (Adobe TN5176
  §15 Table 23). Before this push the Private DICT parser only used
  `defaultWidthX` / `nominalWidthX` / `Subrs` and silently ignored
  every hint-related operator; the new `PrivateHints` struct holds the
  full vocabulary:
  - **`BlueValues`** (op 6), **`OtherBlues`** (op 7),
    **`FamilyBlues`** (op 8), **`FamilyOtherBlues`** (op 9) — primary
    and secondary alignment-zone tables; each is the "delta" operand
    type per TN5176 §4 Table 4 so the accessor returns the
    *undeltified* (running-sum) absolute y-coordinates. Empty when
    absent.
  - **`StdHW`** (op 10), **`StdVW`** (op 11) — dominant horizontal and
    vertical stem widths. `Option<f64>` so callers can distinguish
    "absent" from "zero" (TN5176 lists no default for either).
  - **`StemSnapH`** (op 12 12), **`StemSnapV`** (op 12 13) —
    supplementary stem widths the rasterizer snaps stems to within
    tolerance. Same delta-decoded semantics as `BlueValues`.
  - **`BlueScale`** (op 12 9, default `0.039625`), **`BlueShift`** (op
    12 10, default `7`), **`BlueFuzz`** (op 12 11, default `1`) —
    overshoot suppression and zone-fuzz tunables.
  - **`ForceBold`** (op 12 14, default `false`) — Multiple Master
    synthetic-bold flag. Boolean operand decoded as `false` for `0`,
    `true` otherwise.
  - **`LanguageGroup`** (op 12 17, default `0`) — `0` for Latin /
    Cyrillic etc., `1` for CJK.
  - **`ExpansionFactor`** (op 12 18, default `0.06`) — per-counter
    expansion limit when forcing bold.
  - **`initialRandomSeed`** (op 12 19, default `0`) — seed for the
    Type 2 `random` operator.

  Surfaced through three accessors so non-CID and CID-keyed fonts are
  uniform:
  - `Font::private_hints() -> &PrivateHints` — the font's primary
    Private DICT (FDArray index 0 on a CID-keyed font).
  - `Font::glyph_private_hints(gid) -> Option<&PrivateHints>` — routes
    through `FDSelect` (TN5176 §19) so a CID-keyed font's per-FD
    hints are reachable per glyph. `None` for an out-of-range glyph.
  - `Cff::private_hints_fd(fd_index) -> Option<&PrivateHints>` — direct
    FDArray indexing for callers iterating the full FDArray.

  Hinting is still not *enforced* by the round-1 outline pipeline
  (we anti-alias at >= 16 px); this surface is for callers inspecting
  font metadata or implementing their own hinting downstream.

  Eight new unit tests in `src/cff/private.rs` cover Table 23 defaults,
  delta-undeltification for each of the four blue-zone operators and
  for the stem-snap pair, scalar overrides for `BlueScale` /
  `BlueShift` / `BlueFuzz` / `ExpansionFactor` /
  `initialRandomSeed`, `ForceBold` boolean decode, and a full
  TN5176-Appendix-D worked Private DICT example whose every field
  matches the spec's listed bytes. One new integration test against
  the Source Sans 3 fixture asserts BlueValues come in pairs, are
  monotone-non-decreasing after undeltification, are integral; that
  `StdHW` and `StdVW` are positive; that `BlueScale` / `BlueShift` /
  `BlueFuzz` are in plausible ranges; that `LanguageGroup == 0` and
  `ForceBold == false` (Latin upright font); and that
  `Font::glyph_private_hints` of any in-range glyph routes back to the
  same struct (non-CID invariant).

- CFF Top DICT identity + synthetic-font operators (Adobe TN5176 §9
  Tables 9 and 10): **`UniqueID`** (op 13), **`XUID`** (op 14),
  **`SyntheticBase`** (op 12 20), **`PostScript`** (op 12 21),
  **`BaseFontName`** (op 12 22), **`BaseFontBlend`** (op 12 23).
  Previously the parser collected these into the raw DICT entry list
  but never surfaced them on `TopMetadata` or the public `Font` API.
  - `Font::unique_id() -> Option<i32>` (legacy PostScript Type 1 ID).
  - `Font::xuid() -> &[i32]` (extended unique-identifier array; empty
    slice if absent).
  - `Font::synthetic_base() -> Option<i32>` (Name-INDEX index of the
    base font for synthetics).
  - `Font::postscript() -> Option<&str>` (embedded PostScript code,
    resolved through the CFF Strings table).
  - `Font::base_font_name() -> Option<&str>` (multiple-master master
    font name, SID-resolved).
  - `Font::base_font_blend() -> &[f64]` (multiple-master User Design
    Vector — undeltified into absolute values per TN5176 §4 Table 4
    "delta" semantics; empty slice if absent).

  Six new unit tests in `src/cff/mod.rs` hand-encode a Top DICT
  carrying each operator, plus an extended defaults test that asserts
  the new fields default to `None` / empty for fonts that omit them.

## [0.1.1](https://github.com/OxideAV/oxideav-otf/compare/v0.1.0...v0.1.1) - 2026-05-24

### Other

- resolve predefined Expert / ExpertSubset charsets (TN5176 App. C)
- surface FontMatrix / PaintType / CharstringType / StrokeWidth (TN5176 §9 Table 9)
- Type 2 arithmetic / storage / conditional operators (TN5177 §§4.4-4.6)
- round 98: CFF CID-keyed fonts (ROS + FDArray + FDSelect)
- round 95: seac (deprecated 4-op endchar) + CFF Standard Encoding
- round 91: fix flex-family opcode dispatch + hflex1 dyb (TN5177 §4.6)
- round 83: surface CFF Top DICT metadata + sfnt directory enumeration
- release v0.1.0

### Added

- Predefined CFF **Expert Encoding** lookup table (Adobe TN5176
  Appendix B §2, Top DICT Encoding operand `1`). The new 256-entry
  `EXPERT_ENCODING` array maps `code: u8` → `SID: u16` and is wired
  into `Encoding::Expert::lookup`, so fonts that select predefined
  Encoding operand `1` now resolve `code → GID` directly through the
  per-font charset instead of returning `None` for every code. 165
  codes are assigned (matching the appendix's glyph count); 91 are
  `.notdef` (the spec's explicit gaps). Every assigned SID is `<= 378`,
  i.e. inside the predefined standard-strings range, so glyph-name
  resolution goes through the existing Appendix A standard-strings
  table without consulting the per-font String INDEX. This closes the
  last "noted but not transcribed" item on the round-115 add list —
  the only remaining `Encoding::lookup` arm that returned `None`
  unconditionally.

- Predefined CFF **Expert** and **ExpertSubset** charsets (Adobe
  TN5176 Appendix C, Top DICT charset operands 1 and 2). A font
  selecting either is now resolved instead of being rejected with
  `Cff("predefined Expert charset not implemented in round 1")`.
  Both are fixed `GID → SID` lists transcribed from the appendix in
  GID order (the appendix's column-major three-column layout
  linearised back into GID order): `EXPERT_SIDS` (165 entries → 166
  glyphs) and `EXPERT_SUBSET_SIDS` (86 entries → 87 glyphs). Every
  SID in both tables is `<= 390` (a predefined standard string), so
  `Font::glyph_name` resolves through the existing Appendix A
  standard-strings table without a per-font String INDEX. The new
  `Charset::Expert` / `Charset::ExpertSubset` variants implement the
  same `sid_of(gid)` / `gid_of_sid(sid)` pair as the custom formats,
  so the `seac` component resolver and legacy-encoding reverse
  lookup work unchanged on expert-charset fonts. ISOAdobe (operand
  0) was previously the only predefined charset handled.
- CFF Top DICT `FontMatrix` / `PaintType` / `CharstringType` /
  `StrokeWidth` operators (Adobe TN5176 §9 Table 9, ops 12 07 / 12 05
  / 12 06 / 12 08) surfaced on the public `Font` API. New accessors:
  `Font::font_matrix() -> [f64; 6]` (default
  `[0.001, 0, 0, 0.001, 0, 0]` per spec) returning the glyph→user-space
  affine matrix in spec order `[a, b, c, d, tx, ty]`;
  `Font::paint_type() -> i32` (default 0 = filled outline; 2 = stroked
  with `StrokeWidth` pen); `Font::charstring_type() -> i32` (default 2
  = Type 2 charstrings, the only value embedded in OpenType-CFF);
  `Font::stroke_width() -> f64` (default 0). The same four fields are
  added to the public `cff::TopMetadata` struct. A non-conforming font
  emitting fewer than 6 operands for FontMatrix is zero-filled rather
  than rejected, mirroring the existing FontBBox tolerance. No new
  bytes are read from the font — all four operators were being
  collected by the existing Dict parser since round 1 and are now
  reached through the same `get_array` / `get_int` / `get_number`
  calls the metadata-extraction routine already uses.
- Type 2 charstring arithmetic, stack, storage, and conditional
  operators (Adobe TN5177 §§4.4–4.6). The escape operators `abs`
  (12 9), `add` (12 10), `sub` (12 11), `div` (12 12), `neg` (12 14),
  `random` (12 23), `mul` (12 24), `sqrt` (12 26); the stack operators
  `drop` (12 18), `dup` (12 27), `exch` (12 28), `index` (12 29),
  `roll` (12 30); the storage operators `put` (12 20) / `get` (12 21)
  over a 32-element transient array (size per TN5177 Appendix B); and
  the conditional operators `and` (12 3), `or` (12 4), `not` (12 5),
  `eq` (12 15), `ifelse` (12 22) are now interpreted. Previously any
  of these surfaced as `Error::CharstringUnsupportedOp`. These
  operators pop their inputs from the top of the argument stack and
  push their result back without clearing it. `div`-by-zero and
  `sqrt` of a negative both yield a finite 0 (the spec leaves them
  "undefined") so a malformed font cannot inject NaN/Inf into pen
  coordinates; `random` is a deterministic LCG in (0, 1] for
  reproducible decoding.
- `Error::CharstringTransientIndex(i32)` for a `put` / `get` index
  outside the 0..32 transient-array range.
- CID-keyed CFF support (Adobe TN5176 §§18, 19). A CFF Top DICT that
  begins with the `ROS` operator (op 12 30) is now recognised as a
  CID-keyed font: instead of a single top-level Private DICT, each
  glyph selects its Font DICT through the `FDSelect` GID→FD-index map
  (formats 0 and 3, op 12 37) and the corresponding entry in the
  `FDArray` Font DICT INDEX (op 12 36), each of which carries its own
  Private DICT (Local Subrs + `defaultWidthX` / `nominalWidthX`).
  `Cff::glyph_outline` now routes through the per-glyph Private DICT,
  so glyphs in different FD groups decode with their own subroutines
  and width defaults. Previously any CID font was rejected at parse
  time with `Cff("Top DICT missing Private")`.
- New `cff::fdselect` module implementing FDSelect format 0
  (`Card8 fds[nGlyphs]`) and format 3 (range-encoded
  `(first, fd)*` + sentinel) per TN5176 Tables 27-29.
- `RegistryOrdering` (the `ROS` registry/ordering SIDs + supplement)
  is now a public type, re-exported from the crate root.
- Public CID accessors on `Font`: `is_cid()`, `cid_registry()`,
  `cid_ordering()`, `cid_supplement()`, `cff_fd_count()`; plus
  `Cff::is_cid()`, `Cff::registry_ordering()`, `Cff::fd_count()`.
- Type 2 charstring `endchar` deprecated four-operand `seac` form
  (Adobe TN5177 Appendix C / Type 1 `seac`): a charstring may now
  end with `[width?] adx ady bchar achar endchar` to compose a
  legacy accented glyph from two component glyphs. `bchar` and
  `achar` are CFF Standard Encoding codes (TN5176 Appendix B §1)
  that resolve through the per-font charset to component GIDs; the
  `achar` component is rendered with its pen translated by
  `(adx, ady)` and merged into the composite's contour list. The
  spec's nesting prohibition is enforced (`CharstringSeacNested`).
- CFF Standard Encoding lookup table (`STANDARD_ENCODING`,
  TN5176 Appendix B §1) — 256-entry `code → SID` map. Also wired
  into `Encoding::Standard::lookup` so legacy Standard-encoded
  fonts now resolve codepoint → GID without needing the sfnt
  `cmap`.
- `Charset::gid_of_sid(sid)` — reverse-direction sibling of
  `sid_of(gid)` for the seac and Standard-Encoding paths above.

### Fixed

- Type 2 charstring flex-operator dispatch was shuffled: every flex
  two-byte opcode (`hflex` 12 34, `flex` 12 35, `hflex1` 12 36,
  `flex1` 12 37 — Adobe TN5177 §4.6) routed to the wrong handler,
  producing incorrect arity checks and incorrect cubic-segment
  output for any glyph that used flex.
- `hflex1`'s second-curve mid-control y-delta used `-dy2` instead
  of the spec-mandated `dy5` (the operand on the stack at position
  s[7]). The closing dy6 was already correctly computed as
  `-(dy1 + dy2 + dy5)`.

### Added

- CFF Top DICT metadata accessors on `Font`: `font_bbox`,
  `italic_angle`, `underline_position`, `underline_thickness`,
  `is_fixed_pitch`, `weight_name`, `notice`, `copyright`,
  `version_string`. All values are pre-extracted into a new
  `cff::TopMetadata` struct (also re-exported at crate root) during
  `Font::from_bytes`, so accessors are O(1) with no extra parsing.
- `Font::glyph_bbox(gid)` — decodes the charstring and returns just
  the bounding box (convenience over `glyph_outline().bounds`).
- Sfnt-directory enumeration on `Font`: `table_tags()` iterates
  `(tag, length)` pairs in directory order, `table_data(tag)` borrows
  a table's raw bytes, `has_table(tag)` checks presence. Useful for
  diagnostics / inventory / picking a fallback table.
- Internal `Dict::get_number` helper for spec-typed "number" operands
  (italicAngle, underline metrics) that may be either int or BCD real.

### Tests

- 5 new unit tests in `cff::charstring` covering the seac path:
  two-component composition with a hand-derived offset
  (synthetic 3-glyph font fixture), seac-with-leading-width
  decoding, unresolved-`bchar` error, missing-resolver error, and
  nested-seac rejection. The composite outline's combined bounds
  + per-component MoveTo points are asserted bit-exact against the
  TN5177 Appendix C expansion.
- 2 new unit tests in `cff::encoding` covering the
  `STANDARD_ENCODING` landmark codes (space/A/Z/a/z/DEL/emdash/AE
  /ae/germandbls) and the Standard-encoding → charset GID round-trip.
- 3 new unit tests in `cff::charset` covering `gid_of_sid` for
  ISOAdobe + Format-0 + Format-1 charsets.
- 10 new unit tests in `cff::charstring` covering each flex
  operator's expanded cubic-segment output (hand-derived from the
  TN5177 §4.6 operand expansion), arity-rejection for each
  operator, and a routing sanity check that exercises every flex
  opcode against a stack count only its handler accepts. These
  tests fail against the pre-fix dispatch table.
- 4 new integration tests against the Source Sans 3 fixture
  exercising the new metadata + table-directory APIs.
- 2 new unit tests on `extract_top_metadata` covering the spec
  defaults (italicAngle=0, underline=-100/50, isFixedPitch=false,
  FontBBox=[0,0,0,0]) and a populated FontBBox / italicAngle /
  isFixedPitch scenario.

### Notes

- All work surfaces already-parsed CFF Top DICT data and the
  already-parsed sfnt table directory — no new spec material was
  consumed. Substantive new tables (OS/2, post, GSUB, GPOS) remain
  blocked on docs gap #871 (OpenType + Adobe CFF spec PDFs not yet
  staged under `docs/text/opentype/`); see that directory's
  README.md for the gap inventory.

## [0.1.0](https://github.com/OxideAV/oxideav-otf/compare/v0.0.2...v0.1.0) - 2026-05-03

### Other

- promote to 0.1

## [0.0.2](https://github.com/OxideAV/oxideav-otf/compare/v0.0.1...v0.0.2) - 2026-05-03

### Other

- drop duplicate semver_check key
- replace never-match regex with semver_check = false
- fix 6 lints (range_contains, div_ceil, mem::take, doc fmt, acronym)
- cargo fmt across CFF + tables modules

## [0.0.1] - 2026-05-03

### Added

- Initial round-1 release of the pure-Rust OpenType / CFF font parser.
- sfnt header + table directory walker recognising `OTTO`,
  `0x00010000`, and `true` magics with `CFF ` / `CFF2` table
  detection.
- CFF (Adobe TN5176 v1) parser: header, INDEX, DICT (with BCD-real
  operand handling), Charset (formats 0/1/2 + predefined ISOAdobe),
  Encoding (formats 0/1), Private DICT, Local + Global Subrs.
- Type 2 charstring interpreter (Adobe TN5177): every common path
  construction operator, the four flex variants, hint recording,
  subroutine resolution with the 107 / 1131 / 32768 bias formula,
  and TN5177 §4.7 width decoding.
- Selected sfnt metadata tables: `head`, `hhea`, `maxp`, `hmtx`,
  `cmap` (formats 0/4/6/12), `name`.
- Public glyph-lookup API: `glyph_index`, `glyph_outline` (cubic
  Bezier output), `glyph_advance`, `glyph_lsb`, `glyph_name`.
- Source Sans 3 Regular integration test fixture (SIL OFL v1.1).

### Deferred (round 2+)

- CFF2 (variation-aware) — detected and rejected for now.
- CIDFonts (FDArray / FDSelect / ROS).
- Hint enforcement (AA at >= 16 px renders without hints).
- Predefined Standard / Expert encoding lookup tables (sfnt `cmap`
  is the modern path).
- Adobe Glyph List name → codepoint mapping.
