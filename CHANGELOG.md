# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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
