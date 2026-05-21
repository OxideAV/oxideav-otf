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
  - Charset formats 0 / 1 / 2 (predefined ISOAdobe also recognised).
  - Encoding formats 0 / 1 (predefined Standard / Expert noted but not
    used — real lookup goes through the sfnt `cmap` table).
  - Private DICT including `defaultWidthX` / `nominalWidthX` and the
    Local Subrs INDEX offset.
- Type 2 charstring interpreter (Adobe TN5177):
  - Path: `rmoveto`, `hmoveto`, `vmoveto`, `rlineto`, `hlineto`,
    `vlineto`, `rrcurveto`, `hhcurveto`, `hvcurveto`, `vvcurveto`,
    `vhcurveto`, `rcurveline`, `rlinecurve`.
  - Flex: `flex`, `hflex`, `hflex1`, `flex1`.
  - Subroutines: `callsubr`, `callgsubr`, `return`, `endchar` with
    correct 107 / 1131 / 32768 bias formula.
  - Hints: `hstem`, `vstem`, `hstemhm`, `vstemhm`, `hintmask`,
    `cntrmask` — recorded for stack accounting; not enforced.
  - Width handling per TN5177 §4.7 (optional first-operand width
    delta vs `nominalWidthX` / `defaultWidthX`).
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

## Out of scope (round 3+)

- CFF2 (OpenType 1.8+ variation-aware variant — Adobe TN5174).
  Detected at parse time and reported as `Error::Cff2NotImplemented`.
- CIDFonts (FDArray / FDSelect / ROS) — detected and rejected.
- Hint enforcement (we anti-alias at >= 16 px, so hints are noise).
- Predefined Standard / Expert encoding lookup tables (legacy
  PostScript path; modern OpenType callers use the sfnt `cmap`
  table that we already implement).
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
