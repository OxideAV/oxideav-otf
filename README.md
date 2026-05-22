# oxideav-otf

Pure-Rust OpenType / CFF font parser for the
[oxideav](https://github.com/OxideAV) framework. Sibling to
[`oxideav-ttf`](https://github.com/OxideAV/oxideav-ttf): TTF handles
TrueType outlines (quadratic Beziers); OTF handles CFF outlines
(Type 2 charstrings → cubic Beziers).

## Round-1 scope (this release)

- sfnt + table directory walker (recognises `OTTO`, `0x00010000`, `true`).
- CFF (Adobe TN5176, version 1):
  - Header + Name INDEX + Top DICT + String INDEX + Global Subrs INDEX.
  - Charset formats 0 / 1 / 2 (predefined ISOAdobe also recognised),
    with `sid_of(gid)` *and* the reverse `gid_of_sid(sid)` lookup.
  - Encoding formats 0 / 1 plus predefined Standard Encoding
    (TN5176 Appendix B §1, full 256-entry `code → SID` table
    transcribed). Expert Encoding remains noted but not yet
    transcribed.
  - Private DICT including `defaultWidthX` / `nominalWidthX` and the
    Local Subrs INDEX offset.
- Type 2 charstring interpreter (Adobe TN5177):
  - Path: `rmoveto`, `hmoveto`, `vmoveto`, `rlineto`, `hlineto`,
    `vlineto`, `rrcurveto`, `hhcurveto`, `hvcurveto`, `vvcurveto`,
    `vhcurveto`, `rcurveline`, `rlinecurve`.
  - Flex: `flex`, `hflex`, `hflex1`, `flex1`.
  - Subroutines: `callsubr`, `callgsubr`, `return`, `endchar` with
    correct 107 / 1131 / 32768 bias formula.
  - Deprecated `endchar` four-operand form (TN5177 Appendix C / Type 1
    `seac`) — composes `bchar` + `achar` (resolved via Standard
    Encoding + the font's charset) with `(adx, ady)` translation of
    the accent component. Spec's nesting prohibition enforced.
  - Hints: `hstem`, `vstem`, `hstemhm`, `vstemhm`, `hintmask`,
    `cntrmask` — recorded for stack accounting; not enforced.
  - Width handling per TN5177 §4.7 (optional first-operand width
    delta vs `nominalWidthX` / `defaultWidthX`), including the
    5-operand seac form `[width?] adx ady bchar achar endchar`.
- Selected sfnt tables for metadata: `head`, `hhea`, `maxp`, `hmtx`,
  `cmap` (formats 0/4/6/12), `name`.

## Public API

```rust
use oxideav_otf::Font;

let bytes = std::fs::read("SourceSans3-Regular.otf")?;
let font  = Font::from_bytes(&bytes)?;

// Metadata.
let _ = font.family_name();         // Some("Source Sans 3")
let _ = font.full_name();
let _ = font.units_per_em();        // 1000 (CFF default)
let _ = font.glyph_count();
let _ = font.ps_name();             // PostScript name from CFF Name INDEX
let _ = font.ascent();
let _ = font.descent();
let _ = font.line_gap();

// CFF Top DICT metadata.
let _ = font.font_bbox();           // [xMin, yMin, xMax, yMax] in font units
let _ = font.italic_angle();        // degrees CCW from vertical (0 for upright)
let _ = font.underline_position();
let _ = font.underline_thickness();
let _ = font.is_fixed_pitch();
let _ = font.weight_name();         // Some("Regular"), etc.
let _ = font.notice();
let _ = font.copyright();
let _ = font.version_string();

// Table-directory enumeration.
for (tag, len) in font.table_tags() {
    println!("{:?}  {} bytes", std::str::from_utf8(&tag).unwrap(), len);
}
let _ = font.has_table(b"CFF ");
let _ = font.table_data(b"head");   // raw &[u8] for the head table

// Glyph lookup.
let gid = font.glyph_index('A').unwrap();
let _ = font.glyph_advance(gid);    // i16 advance width in font units
let _ = font.glyph_lsb(gid);
let _ = font.glyph_name(gid);       // "A" (via CFF charset → Strings)
let _ = font.glyph_bbox(gid)?;      // per-glyph bbox derived from charstring
let outline = font.glyph_outline(gid)?;

for contour in &outline.contours {
    for seg in &contour.segments {
        // CubicSegment::MoveTo / LineTo / CurveTo / ClosePath
        let _ = seg;
    }
}
```

## Round-2 additions (this push)

- CFF Top DICT metadata surfaced on the public `Font` API:
  `font_bbox` / `italic_angle` / `underline_position` /
  `underline_thickness` / `is_fixed_pitch` / `weight_name` /
  `notice` / `copyright` / `version_string` (all from already-parsed
  Top DICT operators, no extra spec material consumed).
- `Font::glyph_bbox(gid)` convenience that decodes the charstring
  and returns just the bounding box.
- Table-directory enumeration: `Font::table_tags()` /
  `Font::table_data(tag)` / `Font::has_table(tag)` expose the sfnt
  directory inventory directly to callers.
- `cff::TopMetadata` re-exported for callers that want to inspect
  the full pre-extracted metadata struct in one shot.

## Round-4 additions (this push)

CFF Type 2 charstring `seac` legacy composite + CFF Standard
Encoding lookup table (Adobe TN5176 Appendix B §1 + TN5177
Appendix C):

- A 256-entry Standard Encoding `code → SID` table is transcribed
  verbatim from TN5176 Appendix B §1 (the same table the Type 1
  `seac` and the deprecated 4-operand `endchar` form both
  reference for `bchar` / `achar` resolution). It is exposed as
  `cff::encoding::STANDARD_ENCODING` and also wired into
  `Encoding::Standard::lookup` so legacy Standard-encoded
  PostScript fonts now resolve `code → GID` directly through the
  charset, no sfnt-`cmap` round-trip needed.
- `Charset::gid_of_sid` reverse-lookup landed for ISOAdobe +
  Format 0 / 1 / 2 — the inverse of the existing `sid_of(gid)`.
- The Type 2 charstring interpreter detects an `endchar` whose
  stack carries 4 or 5 operands and runs the TN5177-Appendix-C
  seac path: resolve `bchar` and `achar` through Standard
  Encoding + the charset, recursively decode each component's
  charstring, translate the `achar` component by `(adx, ady)`, and
  merge both contour lists into the composite outline. Nested
  seac is rejected per spec; missing component glyphs surface as
  the new `Error::CharstringSeacBadComponent(u8)`; nested attempts
  surface as `Error::CharstringSeacNested`.

## Round-3 fixes (this push)

Type 2 charstring flex-operator opcode-dispatch correction (Adobe
TN5177 §4.6):

- `hflex` (12 34, 0x0C22), `flex` (12 35, 0x0C23), `hflex1` (12 36,
  0x0C24), `flex1` (12 37, 0x0C25) were previously routed to the
  wrong handlers — the dispatch table had every flex opcode
  shuffled by one slot. Real fonts using any of the four flex
  operators would have decoded with wrong arity expectations and
  produced incorrect outlines for affected glyphs. Source Sans 3
  Regular happens not to exercise the buggy path in any of our
  smoke-test glyphs, which is why the regression slipped through.
- `hflex1`'s second-curve `dyb` argument was `-dy2` (a copy-paste
  carry-over from `hflex`); spec says `dy5` (the operand actually
  on the stack). The closing `dy6 = -(dy1+dy2+dy5)` was correct.
- Added 10 hand-derived charstring fixtures (one per flex
  operator + arity-rejection tests + a routing sanity check) that
  re-derive the expected `CubicSegment` output from TN5177's
  operand expansion. These tests fail before the fix and pass
  after.

## Out of scope (round 3+)

- CFF2 (OpenType 1.8+ variation-aware variant — Adobe TN5174).
  Detected at parse time and reported as `Error::Cff2NotImplemented`.
- CIDFonts (FDArray / FDSelect / ROS) — detected and rejected.
- Hint enforcement (we anti-alias at >= 16 px, so hints are noise).
- Predefined Expert / ExpertSubset encoding lookup tables (TN5176
  Appendix B §2 + Appendix C §Expert) — Standard Encoding is
  transcribed as of round 4; Expert / ExpertSubset remain pending
  (they only matter for a vanishingly small set of legacy
  PostScript Expert fonts and modern OpenType callers route
  through the sfnt `cmap`).
- The Adobe Glyph List string → codepoint mapping (round 3+ if any
  consumer needs it).
- `OS/2`, `post`, `GSUB`, `GPOS`, `GDEF`, `kern` tables — blocked
  on docs gap #871 (OpenType + Adobe CFF specs not yet staged
  under `docs/text/opentype/`).

## Test fixture

`tests/fixtures/SourceSans3-Regular.otf` is Adobe Source Sans 3
Regular under the SIL Open Font License v1.1 (see
`tests/fixtures/SOURCE-SANS-LICENSE`). 335 KB, ~1900 glyphs,
exercises every common Type 2 operator including flex.

## License

MIT — see [`LICENSE`](LICENSE).
