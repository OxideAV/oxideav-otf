# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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
