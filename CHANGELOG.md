# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Other

- **text shaping**: new `shape` module and `Font::shape(text,
  &ShapeOptions) -> Vec<ShapedGlyph>` — a full
  cmap → GSUB → GPOS pipeline per the OpenType Layout chapter-2
  processing model (ISO/IEC 14496-22:2019 §6.3): script/langsys
  resolution with `DFLT`→`latn` fallback, required-feature handling,
  default-enabled feature sets (GSUB `ccmp locl liga clig calt rlig
  rvrn`, GPOS `kern mark mkmk curs dist`; per-feature user override
  with 0=off / 1=on / ≥2=alternate index), lookups merged and applied
  in LookupList order, and LookupFlag skip filtering (GDEF glyph
  classes, mark-attachment-class filter, mark filtering sets, with
  the spec's supersession rules). GSUB application covers all eight
  lookup types (multiple-subst splicing, skip-aware ligature matching
  with cluster merge + ligature-component tracking, nested contextual
  records under their own flags with a depth cap, end-to-start
  reverse chaining); GPOS covers all nine (pair valueFormat2==0
  cursor rule, cursive exit/entry alignment with the RIGHT_TO_LEFT
  cross-stream rule, mark-to-base/-ligature/-mark anchor attachment,
  contextual nesting, extension wrappers). Legacy `kern` kicks in
  only when no GPOS `kern` feature exists for the resolved script.
  Fixture expectations (incl. a 53-glyph paragraph) validated against
  an independent system shaping binary (black-box); synthetic
  byte-built fonts cover the cursive / mark-to-ligature / contextual
  positioning / legacy-kern paths with hand-computed geometry.

- **variable-font shaping**: `ShapeOptions::coords` (user-scale fvar
  axis values) applies HVAR advance deltas at position init and
  resolves ValueRecord `*DeviceOffset` `VariationIndex` tables
  against the GDEF `ItemVariationStore` at the fvar→avar-normalized
  instance (§7.2.3; GPOS "GPOS table and OpenType Font Variations").
  `GdefTable::item_variation_store` now decodes the v1.3
  `itemVarStore` (previously a raw offset only).

- **FeatureVariations** (§6.3 "Feature variations"): new
  `FeatureVariations` / `FeatureTableSubstitution` decoders in
  `tables::layout` — record-ordered condition-set evaluation
  (ConditionFormat1 normalized axis ranges, inclusive bounds, AND
  conjunction, NULL set = universal, unknown formats fail the set,
  unsupported substitution versions reject the record), Offset32
  alternate feature tables. Exposed via
  `GsubTable::feature_variations` / `GposTable::feature_variations`
  and consumed by `Font::shape` (`rvrn` joins the default GSUB set).

- new supporting accessors: `SinglePos::raw`, `PairPos::raw`,
  `PairPos::value_device_base` (ValueRecord device offsets are
  PairSet-relative in PairPosFormat1 per the spec's "immediate
  parent" rule).

- support the **`VORG`** vertical origin table (ISO/IEC 14496-22:2019
  §5.4): the `defaultVertOriginY` plus the sorted
  `vertOriginYMetrics` array (glyph index → explicit vertical-origin Y).
  `VorgView::vert_origin_y(gid)` binary-searches the array and falls
  back to the default; surfaced via `Font::vorg` /
  `Font::vertical_origin_y`. Complements the `vmtx`/`VVAR` vertical
  metrics by giving CFF fonts the vertical origin directly instead of
  computing it from the charstring bbox top.

- support the **`BASE`** baseline table (ISO/IEC 14496-22:2019 §6.3):
  the v1.0/v1.1 header, the horizontal + vertical `Axis` tables, the
  `BaseTagList` (per-axis baseline tags), the `BaseScriptList` /
  `BaseScript` / `BaseValues` chain, and the `BaseCoord` formats 1/2/3
  (the design-unit coordinate; contour-point / Device refinements are
  not applied — the format-3 default-instance value is used). Surfaced
  via `Font::base` and the `Font::baseline_coord(axis, script_tag,
  baseline_tag)` convenience; the v1.1 `itemVarStoreOffset` is exposed
  as a raw offset (parseable with the delta-set ItemVariationStore).

- support the **`HVAR`** and **`VVAR`** per-glyph metrics variations
  tables (ISO/IEC 14496-22:2019 §7.3.5, §7.3.8): the IVS-offset +
  `DeltaSetIndexMap` offsets header (advance + side bearings, plus
  `VVAR`'s vertical-origin map) and the `DeltaSetIndexMap` itself (the
  packed `entryFormat` with inner-index bit count + entry size, glyph IDs
  past `mapCount` clamping to the last entry). `advance` resolves the
  per-glyph advance adjustment (implicit glyph-ID index when no advance
  map is present); the side-bearing / vertical-origin accessors do the
  same for their maps. Surfaced via `Font::hvar` / `vvar` /
  `advance_width_variation` / `advance_height_variation`.

- decode the **delta-set-storing `ItemVariationStore`** (ISO/IEC
  14496-22:2019 §7.2.3) — the variation-data structure shared by the
  metrics/positioning variation tables (distinct from the CFF2
  delta-free IVS where deltas live in CharStrings). It stores the
  `itemCount × regionIndexCount` delta-set matrix (first
  `shortDeltaCount` columns `int16`, the rest `int8`) and resolves a
  `(outer, inner)` index pair to a per-instance adjustment via the
  §7.1.7 region-scalar algorithm (`tables::ivs`).

- support the **`MVAR`** metrics variations table (§7.3.6): the value
  records mapping a four-byte value tag (e.g. `b"hasc"` =
  `OS/2.sTypoAscender`) to a delta-set index, plus the embedded
  ItemVariationStore. `MvarView::metric_delta` /
  `Font::metric_variation(tag, user_coords)` resolve a font-wide
  metric's per-instance adjustment (value records binary-searched by
  tag; absent tag ⇒ constant metric ⇒ 0).

- support the **`STAT`** style attributes table (ISO/IEC
  14496-22:2019 §7.3.7): the header (version 1.1/1.2, design-axis +
  axis-value arrays, `elidedFallbackNameID`), the design-axis records
  (`axisTag` / `axisNameID` / `axisOrdering`), and all four axis-value
  table formats — format 1 (single value), format 2 (nominal value +
  `[min, max]` range), format 3 (value + style-linked counterpart), and
  format 4 (multi-axis combination). The `OLDER_SIBLING_FONT_ATTRIBUTE`
  and `ELIDABLE_AXIS_VALUE_NAME` flags are exposed; NULL offsets and
  unrecognised formats are skipped per spec. Surfaced via `Font::stat`
  and `Font::stat_version`.

- support the variable-font axis-definition tables and connect them to
  the CFF2 variation interpreter (ISO/IEC 14496-22:2019 §7.1, §7.3):
  - **`fvar`** (§7.3.3) — decode the variation axes
    (tag / min / default / max / flags / `name` ID) and named instances
    (subfamily + optional PostScript name ID, `0xFFFF` no-name sentinel
    honoured). `VariationAxis::normalize` implements the §7.3.1.1
    default normalization. Surfaced via `Font::fvar` /
    `variation_axes` / `named_instances` / `axis_count` /
    `has_variation_axes`.
  - **`avar`** (§7.3.1) — decode the per-axis piecewise-linear segment
    maps and apply the §7.3.1.3 modified-normalization process
    (validated against the §7.3.1.4 worked example). Surfaced via
    `Font::avar`.
  - **region-scalar derivation** (§7.1.7) — `VariationRegion::scalar`
    computes a region's interpolation scalar (product of per-axis
    triangular scalars, with the three spec "ignore this axis" cases),
    and `ItemVariationStore::region_scalars` yields the per-region
    scalar vector for an `ItemVariationData` subtable (validated against
    the §7.1.8 Skia two-axis example).
  - **`Font::normalize_coords`** runs the full `fvar` → `avar`
    normalization pipeline, and **`Font::glyph_outline_for_axes`**
    chains user-scale axis coordinates → normalization → region scalars
    → the CFF2 charstring interpreter, so a glyph instance can now be
    decoded directly from axis values (previously the region-scalar
    step was the caller's responsibility).

- support the legacy **`kern`** table (ISO/IEC 14496-22:2019 §5.7.5),
  the OFF/Windows version-0 format: a 4-byte header (`version`,
  `nTables`) followed by subtables, each with its own `coverage`
  byte-field (horizontal / minimum / cross-stream / override flags + an
  8-bit format selector). Subtable **format 0** (sorted
  `(left, right, value)` pair list, binary-searched on the 32-bit
  `(left << 16) | right` key) and **format 2** (the two-dimensional
  class-kerning array with pre-multiplied left/right class-table
  offsets) are both decoded; reserved formats are skipped rather than
  rejected. `KernView::kerning(left, right)` accumulates the additive
  horizontal kerning value across applicable subtables (honouring the
  `override` flag), and the per-subtable `value` / coverage accessors
  let a shaper apply minimum / cross-stream / vertical subtables itself.
  Surfaced at the `Font` level via `kern()` and the
  `kern_pair(left, right)` convenience. (Modern fonts express kerning
  through GPOS pair adjustment, already supported; `kern` is the legacy
  fallback.) The Apple version-1.0 `kern` layout is intentionally not
  decoded (the OFF spec defines only version 0).

- support **`vhea` / `vmtx`** — the vertical header and vertical metrics
  tables (ISO/IEC 14496-22:2019 §§5.7.9–5.7.10). `vhea` decodes both
  v1.0 (`ascent`/`descent`/`lineGap`) and v1.1
  (`vertTypoAscender`/`vertTypoDescender`/`vertTypoLineGap`) — the two
  versions share an identical 36-byte layout, differing only in field
  names — surfacing `numOfLongVerMetrics` plus the caret slope / extent
  fields. `vmtx` mirrors `hmtx`: `numOfLongVerMetrics`
  `(advanceHeight, topSideBearing)` pairs followed by a bare
  `topSideBearing` tail, with tail glyphs inheriting the last full
  advance height (the spec's monospaced-run optimisation). Both tables
  are optional (present only in vertical / CJK fonts) and surfaced at
  the `Font` level via `has_vertical_metrics`, `vhea`,
  `vertical_ascent` / `vertical_descent` / `vertical_line_gap`,
  `glyph_advance_height`, and `glyph_tsb`.

- support **`cmap` subtable format 2** (high-byte mapping through table)
  — the legacy mixed 8-/16-bit encoding used by CJK code-page fonts. A
  code point's high byte selects a `SubHeader` via the
  `subHeaderKeys[256]` array (`subHeader 0` handles single-byte
  characters), and the low byte indexes that SubHeader's `glyphIdArray`
  sub-array through `firstCode` / `entryCount` / `idRangeOffset`, with
  `idDelta` applied modulo 65536 to a non-zero result. Ranked above the
  single-byte format 0 but below the Unicode formats.

- support **`cmap` subtable format 14** (Unicode Variation Sequences).
  A new `tables::cmap_uvs::CmapUvs` view decodes the format-14 subtable
  (VariationSelector records, DefaultUVS range tables, NonDefaultUVS
  mapping tables — all uint24-keyed and binary-searched). The
  `CmapTable` retains the format-14 subtable alongside its chosen base
  subtable (format 14 supplements rather than replaces the base cmap),
  exposing `CmapTable::uvs` and `CmapTable::lookup_variation`. At the
  `Font` level, `glyph_index_variation(base, selector)` resolves a
  variation sequence — a non-default UVS yields its explicit glyph, a
  default UVS resolves `base` through the base cmap, an unsupported
  sequence yields `None` — and `variation_sequences()` exposes the
  `CmapUvs` view for enumerating supported selectors.

- support **`cmap` subtable format 13** (many-to-one range mappings).
  Format 13 shares format 12's on-disk layout but maps every codepoint
  in a `ConstantMapGroup` `[startCharCode, endCharCode]` to the same
  `glyphID` (the "last resort" / fallback subtable). It is binary-
  searched like format 12 and ranked below every real-coverage format,
  so it only wins subtable selection when nothing better is present.

- decode **Device and VariationIndex tables** (`tables::device`). A
  `Device` table (deltaFormat 1 / 2 / 3) decodes its packed 2- / 4- /
  8-bit signed per-ppem deltas — `delta(ppem)` answers the signed pixel
  correction (spec's worked 4-bit example `{1,2,3,-1}` → `0x123F`) — and
  a `VariationIndex` table (deltaFormat 0x8000) surfaces its
  `(deltaSetOuterIndex, deltaSetInnerIndex)` delta-set index pair into
  the GDEF/BASE `ItemVariationStore`; `DeviceOrVariationIndex::parse`
  dispatches on the `deltaFormat` field. These were previously surfaced
  only as raw `Offset16` values: GPOS Anchor format-3 now decodes them
  via `Anchor::x_device` / `y_device`, GPOS `ValueRecord` via
  `x_placement_device` / `y_placement_device` / `x_advance_device` /
  `y_advance_device`, and GDEF CaretValue format-3 via
  `CaretValue::device` (plus a `CaretValue::coordinate` accessor). Each
  takes the slice whose byte 0 is the structure the offset is relative
  to (the Anchor table, the GPOS subtable, or the CaretValue table).

- decode GSUB Lookup Type 8 (reverse chaining contextual single
  substitution) as a typed `ReverseChainSingleSubst` view over
  ReverseChainSingleSubstFormat1 — the last GSUB lookup type without a
  typed decoder. It exposes the input `Coverage`, the backtrack /
  lookahead Coverage sequences (`backtrack_coverage(i)` /
  `lookahead_coverage(i)`), and the `substituteGlyphIDs` array;
  `substitute(glyph)` resolves a covered input to its single output
  glyph via the Coverage index, leaving the backtrack/lookahead context
  check to the caller (this is the one lookup applied in reverse order,
  so it invokes no nested lookups). Reachable through
  `GsubTable::reverse_chain_single_subst` and the type-7 extension
  `ExtensionSubst::as_reverse_chain_single_subst`. With this, every GSUB
  lookup type (1–8) has a typed decoder.

- decode **CFF2 variable-font glyph outlines**. A variation-aware CFF2
  Type 2 CharString interpreter (`cff2::Cff2Interpreter`) now runs the
  path / hint / subroutine operators plus the two CFF2 variation
  operators — `vsindex` (select the active `ItemVariationData`, hence the
  active region count `k`) and `blend` (pop `n + n*k + 1` operands and
  push `n` interpolated values `default[i] + Σ_j scalar[j]·delta[i*k+j]`).
  CFF2 differs from CFF1 in having no `endchar` (the CharString ends at
  end-of-stream), no glyph-width prefix, and no arithmetic / storage /
  conditional operators; all four are handled. The per-FontDICT
  PrivateDICT (default `vsindex` + LocalSubrINDEX, `cff2::Cff2Private`)
  and the FontDICT `Private` pointer (`cff2::Cff2FontDict`) are parsed,
  and a glyph is routed to its FontDICT through a new CFF2 FontDICTSelect
  (`cff2::Cff2FdSelect`, formats 0 / 3 / **4** — format 4 being the
  CFF2-only 32-bit-range variant for > 65,534 glyphs). `Font::glyph_outline`
  on a CFF2 font now decodes the **default variation instance** (every
  region scalar `0`) instead of returning `Error::Cff2NotImplemented`;
  the new `Font::glyph_outline_var(gid, &region_scalars)` decodes a
  specific instance from caller-supplied per-region interpolation scalars
  (the scalar derivation from `fvar`/`avar` axis settings — the OpenType
  *Font Variations Common Table Formats* region-scalar algorithm — is the
  shaping client's responsibility and is not staged in the CFF2 doc).
  `Error::Cff2NotImplemented` is retained but is no longer returned.

- decode GPOS Lookup Type 6 (mark-to-mark attachment positioning) as a
  typed `MarkMarkPos` view over MarkMarkPosFormat1. The structure mirrors
  mark-to-base: `mark1` plays the "mark" role and `mark2` plays the
  "base" role, with the `Mark2Array` laid out exactly like the
  `BaseArray`. `mark1_record(mark1)` resolves the attaching mark's class
  and `Anchor`; `mark2_anchor(mark2, class)` resolves the base-mark
  anchor for a class (`Ok(None)` for a NULL class offset); and
  `attachment(mark1, mark2)` returns the
  `MarkMarkAttachment { mark_class, mark1_anchor, mark2_anchor }` pair a
  shaper aligns to stack one combining mark over a preceding mark.
  Reachable directly via `GposTable::mark_mark_pos` and through the
  type-9 positioning extension (`ExtensionPos::as_mark_mark_pos`). Reuses
  the shared `Anchor` and MarkArray/MarkRecord primitives.

- decode GPOS Lookup Type 3 (cursive attachment positioning) as a typed
  `CursivePos` view over CursivePosFormat1. `entry_exit(glyph)` returns
  the glyph's `EntryExit` record (entry / exit `Anchor`, either of which
  may be NULL), and `attachment(first, second)` returns the
  `CursiveAttachment { exit_anchor, entry_anchor }` pair a shaper aligns
  to join adjacent cursive glyphs (the first glyph's exit anchor onto the
  second glyph's entry anchor); a NULL exit or entry anchor yields no
  adjustment. Reachable directly via `GposTable::cursive_pos` and through
  the type-9 positioning extension (`ExtensionPos::as_cursive_pos`).

- ship the 258-entry standard-Macintosh glyph-name set
  (`STANDARD_MAC_GLYPH_NAMES` / `standard_mac_glyph_name`) and apply it
  in `post`: `PostTable::glyph_name` now resolves format 1.0 (glyph ID
  → standard name), 2.0 (`glyphNameIndex < 258` → standard; `>= 258` →
  custom Pascal string), and 2.5 (`glyph_id + offset` → standard) via
  the new `PostGlyphName { Standard, Custom }` view. `Font::post_glyph_name`
  now resolves standard names for every format instead of returning
  `None` for `glyphNameIndex < 258`.
- decode GPOS Lookup Type 4 (mark-to-base attachment positioning) as a
  typed `MarkBasePos` view, with the shared `Anchor` (formats 1/2/3) and
  MarkArray/MarkRecord primitives; `attachment(mark, base)` returns the
  `(mark_anchor, base_anchor)` pair for combining-mark placement.
  Reachable directly and through the type-9 positioning extension
  (`ExtensionPos::as_mark_base_pos`).

## [0.1.3](https://github.com/OxideAV/oxideav-otf/compare/v0.1.2...v0.1.3) - 2026-06-15

### Other

- GPOS Lookup Type 9 (positioning subtable extension) decoded
- decode Lookup Type 2 (pair adjustment positioning)
- parse CFF2 ItemVariationStore (§12) for variable fonts
- decode ValueRecord/ValueFormat + Lookup Type 1 single adjustment
- decode Lookup Type 7 (substitution extension) as typed ExtensionSubst view
- decode Lookup Type 3 (alternate substitution) as typed AlternateSubst view
- decode Lookup Type 2 (multiple substitution) as typed MultipleSubst view
- drop release-plz.toml — use release-plz defaults across the workspace
- decode Lookup Type 4 (ligature substitution) as a typed LigatureSubst view
- GSUB Lookup Type 1 (single substitution) decoded as typed SingleSubst view
- decode headers + ScriptList/FeatureList/LookupList primitives
- decode GDEF + Coverage + ClassDef common-layout primitives
- ship Adobe Glyph List 2.0 + Font name↔gid accessors
- avoid clippy 1.96 doc_lazy_continuation in module-level CFF2 line
- parse header, Top DICT, GlobalSubrINDEX, CharStringINDEX, FontDICTINDEX
- parse table version 1 + langTagRecord array + NameId enum
- decode OS/2 and Windows Metrics table — versions 0..5

### Added

- **GPOS Lookup Type 9 (positioning subtable extension) decoded** — the
  `ExtensionPos` typed view, mirroring the GSUB type-7 extension. It
  decodes the 8-byte `PosExtensionFormat1` header (`format`,
  `extensionLookupType`, `Offset32 extensionOffset`) and resolves the
  32-bit indirection to the wrapped subtable. `extension_subtable_bytes()`
  surfaces the wrapped bytes raw; `as_single_pos()` / `as_pair_pos()`
  decode the wrapped positioning types this crate already handles (1 / 2).
  `GposTable::extension_pos(lookup_i, sub_i)` mirrors the `single_pos` /
  `pair_pos` accessors (wrong-type lookups → `BadStructure`). Parse-time
  validation enforces `format == 1`, `extensionLookupType` in `1..=8`
  (never 9 — the spec forbids an extension pointing at another
  extension), and a non-NULL in-range `extensionOffset`. This is the
  format-extension mechanism behind Source Sans 3's kerning, which is now
  reachable through the typed path. `ExtensionPos` is re-exported at the
  crate root. Source: `docs/text/opentype/otspec-gpos.html` §"Lookup
  type 9 subtable: positioning subtable extension".

- **GPOS Lookup Type 2 (pair adjustment positioning) decoded** — the
  `PairPos` typed view joins the existing `SinglePos` (type 1). Both
  on-disk formats are supported: format 1 (per-glyph `PairSet` /
  `PairValue` records, binary-searched by `secondGlyph`) and format 2
  (the `class1Count × class2Count` matrix keyed through two `ClassDef`
  tables). `PairPos::pair(first, second)` returns the
  `PairValue { first, second }` adjustment; `class_pair(c1, c2)` is the
  direct format-2 matrix probe; `iter()` enumerates every explicit
  `(first, second, PairValue)` triple of a format-1 subtable.
  `GposTable::pair_pos(lookup_i, sub_i)` mirrors the `single_pos`
  accessor (wrong-type lookups → `BadStructure`). Both `valueFormat1` /
  `valueFormat2` reserved-bit, range, count-mismatch, and matrix-overrun
  checks are enforced at parse time. Source:
  `docs/text/opentype/otspec-gpos.html` §"Lookup type 2 subtable: pair
  adjustment positioning" (+ the shared `ValueRecord` / `Coverage` /
  `ClassDef` primitives). `PairPos`, `PairPosIter`, and `PairValue` are
  re-exported at the crate root.

- **CFF2 `ItemVariationStore` (§12) parsed for variable fonts** — the
  `VariationStore` block pointed at by the Top DICT
  `VariationStoreOffset` operator is now decoded into a typed
  `ItemVariationStore`: the `VariationRegionList` (every
  `VariationRegion`'s per-axis `RegionAxisCoordinates` `start`/`peak`/
  `end`, each an F2DOT14 value normalized to `[-1.0, 1.0]`) plus the
  array of `ItemVariationData` subtables (`itemCount`,
  `shortDeltaCount`, and the `regionIndexes` array that gives a
  `blend`'s active-region count `k`). The format-1 check, declared IVS
  `length` extent confinement, and `regionIndex < regionCount` bounds
  are enforced. Source: `docs/text/opentype/otspec-cff2.html` §12
  "VariationStore data contents" + the worked "Example CFF2 table" byte
  trace (CFF2 offsets 0x10–0x37). Exposed via `Cff2::variation_store()`
  and `Font::variation_store()`; the spec's worked example round-trips
  bit-exactly. The per-glyph `blend`/`vsindex` charstring math (which
  combines these regions with instance axis settings via the per-region
  scalar algorithm) remains deferred — that algorithm lives in the
  OpenType *Font Variations Common Table Formats* chapter, not in the
  staged CFF2 doc.
- **GPOS `ValueRecord` / `ValueFormat` primitive and Lookup Type 1
  (single adjustment positioning) decoded** — the GPOS table's first
  typed lookup. Source: `docs/text/opentype/otspec-gpos.html`
  §"ValueRecord" and §"Lookup type 1 subtable: single adjustment
  positioning". `ValueFormat` exposes predicate accessors for the eight
  defined flag bits, an `is_valid()` reserved-bit check, and
  `record_size()` = `2 × popcount(definedBits)`. `ValueRecord` decodes
  the placement/advance design-unit values plus the four raw
  Device/VariationIndex `Offset16`s, reading only the fields the
  `ValueFormat` declares, in the spec's fixed flag-bit order; undeclared
  fields read back as `0`. `SinglePos` decodes both on-disk formats
  (format 1 = one shared `ValueRecord`; format 2 = a per-glyph array
  indexed by Coverage Index), re-uses the shared `Coverage` primitive,
  and answers `value(glyph)` / iterates `(glyph_id, ValueRecord)`.
  `GposTable::single_pos(lookup_i, sub_i)` mirrors the GSUB
  `single_subst` accessor (`None` out of range, `Some(Err)` on a
  non-type-1 lookup). The full `GPOS_LOOKUP_TYPE_*` constant set (1..9),
  `ValueFormat`, `ValueRecord`, `SinglePos`, and `SinglePosIter` are
  re-exported at the crate root. Synthetic byte-tower unit tests cover
  the format/size math, the field-order/empty-record rules, both
  `SinglePos` formats, and every error path; one Source Sans 3
  integration test documents the fixture's (legitimate) absence of
  type-1 lookups and the accessor's wrong-type rejection.
- **GSUB Lookup Type 7 (substitution extension) decoded** via a new
  `ExtensionSubst` typed view, joining the round-247 `SingleSubst`,
  round-254 `LigatureSubst`, round-262 `MultipleSubst`, and round-270
  `AlternateSubst` work. Source: `docs/text/opentype/otspec-gsub.html`
  §"Lookup type 7 subtable: substitution subtable extension". This
  lookup type is a *format extension mechanism*, not a substitution
  action: it reaches the real subtable through a 32-bit offset for
  fonts whose accumulated subtable sizes exceed 16-bit offsets.
  `ExtensionSubst` decodes the one defined on-disk format —
  `(format, extensionLookupType, Offset32 extensionOffset)` — and
  validates at parse time that `format == 1`, that
  `extensionLookupType` is a defined GsubLookupType (`1..=8`) other
  than 7 (the spec forbids an extension pointing at another
  extension), and that `extensionOffset` lands inside the subtable's
  byte window. The wrapped subtable is surfaced raw via
  `extension_subtable_bytes()` and through typed resolvers for the
  lookup types this crate already decodes: `as_single_subst()` /
  `as_multiple_subst()` / `as_alternate_subst()` /
  `as_ligature_subst()` (each rejects with `BadStructure` when the
  declared `extensionLookupType` disagrees). New convenience accessor
  `GsubTable::extension_subst(lookup_i, sub_i)` mirrors the existing
  type-1..4 accessors: `None` for out-of-range indices,
  `Some(Err(BadStructure))` when the referenced lookup is not declared
  as type 7. `ExtensionSubst` is re-exported at the crate root.
  Synthetic-byte unit tests cover round-trips wrapping a
  `SingleSubstFormat1` and the Example-6 ligature subtable, the
  raw-bytes path for a not-yet-typed wrapped type (8), rejection of
  `format != 1`, of `extensionLookupType == 7`, of out-of-vocabulary
  types (0 / 9 / 0xFFFF), of NULL and out-of-range `extensionOffset`,
  truncated headers, the wrong-type accessor rejection, and an
  end-to-end GSUB byte tower whose only lookup is a type-7 extension
  wrapping a single substitution. A Source Sans 3 integration test
  walks every lookup, decodes any type-7 subtables (validating the
  spec's "all extension subtables of one Lookup share the same
  extensionLookupType" rule and resolving wrapped types 1..4 through
  the typed views), and pins the accessor semantics on a real non-7
  lookup. The remaining lookup types (5 Contextual, 6 Chained-context,
  8 Reverse-chained-single) remain raw sub-slices pending dedicated
  rounds.

- **GSUB Lookup Type 3 (alternate substitution) decoded** via a new
  `AlternateSubst` typed view, joining the round-247 `SingleSubst`,
  round-254 `LigatureSubst`, and round-262 `MultipleSubst` work.
  Source: `docs/text/opentype/otspec-gsub.html` §"Lookup type 3
  subtable: alternate substitution"; the Coverage table is re-used
  from `tables::gdef::Coverage` (the shared common-layout primitive,
  per `otspec-chapter2-common-layout-tables.html`). `AlternateSubst`
  decodes the one defined on-disk format — `(format, coverageOffset,
  alternateSetCount, alternateSetOffsets[alternateSetCount])` — and
  validates the spec's "ordered by Coverage index" invariant
  (`alternateSetCount == coverage.len()`) at parse time; `AlternateSet`
  decodes the per-input `(glyphCount, alternateGlyphIDs[glyphCount])`
  payload. Unlike `MultipleSubst`, the spec sets no lower bound on an
  AlternateSet's `glyphCount`, so an empty AlternateSet (no
  alternatives) is accepted rather than rejected. The alternates are
  "in arbitrary order" per spec — index 0 is not privileged.
  `AlternateSubst::substitute(input: u16) -> Option<AlternateSet>` is
  the shaper-path entrypoint: the covered input glyph selects an
  AlternateSet via Coverage Index, returned as a zero-copy view over
  the on-disk `alternateGlyphIDs[]` bytes. It does **not** itself
  choose an alternate — selection is a higher-layer (feature / UI)
  decision per spec. `AlternateSubst::iter()` yields
  `(coverage_glyph, AlternateSet)` pairs in ascending Coverage order;
  `AlternateSet::glyph(i)` borrows the alternate at index `i`;
  `AlternateSet::glyphs()` yields every alternate in on-disk order.
  New convenience accessor `GsubTable::alternate_subst(lookup_i,
  sub_i)` mirrors `single_subst(...)` / `multiple_subst(...)` /
  `ligature_subst(...)`: `None` for out-of-range indices,
  `Some(Err(BadStructure))` when the referenced lookup is not declared
  as type 3. `AlternateSubst`, `AlternateSubstIter`, `AlternateSet`,
  and `AlternateGlyphIter` are re-exported at the crate root.
  Synthetic-byte unit tests cover the spec's worked Example 5 (default
  ampersand glyph `0x003A` mapping to alternatives `[0x00C9,
  0x00CA]`), Coverage iteration with two covered glyphs (ascending
  order), the `format != 1` rejection, out-of-range `coverageOffset`,
  the `alternateSetCount != coverage.len()` rejection, truncated
  `alternateSetOffsets[]`, the accepted empty-AlternateSet case, the
  rejection of a non-type-3 lookup on the accessor, and an end-to-end
  GSUB byte tower whose only lookup is the same Example-5 subtable. A
  Source Sans 3 integration test walks every type-3 lookup (the font
  ships one — a single subtable with ~210 AlternateSet tables for its
  `aalt` feature), decodes every `AlternateSubst` and every per-input
  `AlternateSet`, verifies Coverage iteration is ascending, every
  alternate glyph fits inside `maxp.numGlyphs`, the `glyph(k)`
  point-lookup agrees with the `glyphs()` iterator, and
  `substitute(input)` agrees with the iter/set path. The remaining
  lookup types (5 Contextual, 6 Chained-context, 7 Extension,
  8 Reverse-chained-single) remain raw sub-slices pending dedicated
  rounds.

- **GSUB Lookup Type 2 (multiple substitution) decoded** via a new
  `MultipleSubst` typed view, joining the round-247 `SingleSubst` and
  round-254 `LigatureSubst` work. Source:
  `docs/text/opentype/otspec-gsub.html` §"Lookup type 2 subtable:
  multiple substitution"; the Coverage table is re-used from
  `tables::gdef::Coverage` (the shared common-layout primitive, per
  `otspec-chapter2-common-layout-tables.html`). `MultipleSubst` decodes
  the one defined on-disk format — `(format, coverageOffset,
  sequenceCount, sequenceOffsets[sequenceCount])` — and validates the
  spec's `sequenceCount == coverage.len()` invariant at parse time;
  `Sequence` decodes the per-input `(glyphCount,
  substituteGlyphIDs[glyphCount])` payload. A `glyphCount` of zero is
  rejected as `BadStructure`: the spec explicitly prohibits using
  multiple substitution as a deletion ("The glyphCount value must
  always be greater than 0"). `MultipleSubst::substitute(input: u16)
  -> Option<Sequence>` is the shaper-path entrypoint: the covered
  input glyph selects a Sequence via Coverage Index, which is returned
  as a zero-copy view over the on-disk `substituteGlyphIDs[]` bytes.
  `MultipleSubst::iter()` yields `(coverage_glyph, Sequence)` pairs in
  ascending Coverage order; `Sequence::glyph(i)` borrows the
  substitute glyph at output index `i`; `Sequence::glyphs()` yields
  the full output sequence in order. New convenience accessor
  `GsubTable::multiple_subst(lookup_i, sub_i)` mirrors
  `single_subst(...)` / `ligature_subst(...)`: `None` for out-of-range
  indices, `Some(Err(BadStructure))` when the referenced lookup is not
  declared as type 2. `MultipleSubst`, `MultipleSubstIter`,
  `Sequence`, and `SequenceGlyphIter` are re-exported at the crate
  root. Synthetic-byte unit tests cover the spec's worked Example 4
  (ffi-ligature glyph `0x00F1` decomposed into `[f=0x1A, f=0x1A,
  i=0x1D]`), Coverage iteration with two covered glyphs (ascending
  order), the `format != 1` rejection, out-of-range `coverageOffset`,
  the `sequenceCount != coverage.len()` rejection, truncated
  `sequenceOffsets[]`, the `glyphCount == 0` (deletion) rejection,
  the rejection of a non-type-2 lookup on the accessor, and an
  end-to-end GSUB byte tower whose only lookup is the same Example-4
  subtable. A Source Sans 3 integration test walks every type-2
  lookup (the font ships two — one ~407-sequence mark-decomposition
  subtable plus a smaller secondary subtable), decodes every
  `MultipleSubst` and every per-input `Sequence`, verifies Coverage
  iteration is ascending, every substitute glyph fits inside
  `maxp.numGlyphs`, every `glyphCount >= 1`, the `glyph(k)`
  point-lookup agrees with the `glyphs()` iterator, and
  `substitute(input)` agrees with the iter/sequence path. The other
  lookup types (3 Alternate, 5 Contextual, 6 Chained-context,
  7 Extension, 8 Reverse-chained-single) remain raw sub-slices
  pending dedicated rounds.

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
