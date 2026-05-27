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
  - Charset formats 0 / 1 / 2 plus all three predefined charsets
    (ISOAdobe, Expert, ExpertSubset — the Expert / ExpertSubset
    `GID → SID` lists transcribed from TN5176 Appendix C), with
    `sid_of(gid)` *and* the reverse `gid_of_sid(sid)` lookup.
  - Encoding formats 0 / 1 plus predefined Standard Encoding
    (TN5176 Appendix B §1) and predefined Expert Encoding
    (Appendix B §2) — both 256-entry `code → SID` tables
    transcribed in full.
  - Private DICT including `defaultWidthX` / `nominalWidthX` and the
    Local Subrs INDEX offset.
  - CID-keyed fonts (TN5176 §§18, 19): `ROS` detection, the `FDArray`
    Font DICT INDEX, and `FDSelect` formats 0 / 3 routing each glyph
    to its own Private DICT / Local Subrs / width defaults.
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

// CID-keyed fonts (TN5176 §18) — None / 0 on a plain CFF font.
let _ = font.is_cid();
let _ = font.cid_registry();        // Some("Adobe")
let _ = font.cid_ordering();        // Some("Japan1") / Some("Identity")
let _ = font.cid_supplement();      // Some(7)
let _ = font.cff_fd_count();        // number of FDArray Font DICTs

for contour in &outline.contours {
    for seg in &contour.segments {
        // CubicSegment::MoveTo / LineTo / CurveTo / ClosePath
        let _ = seg;
    }
}
```

## Round-171 additions (this push)

The remaining CFF predefined encoding — **Expert Encoding** (TN5176
Appendix B §2, Top DICT Encoding operand `1`) — is now resolved
instead of falling through to `None`. Before this push, a font
selecting predefined operand `1` parsed as `Encoding::Expert` but
`Encoding::lookup` returned `None` for every code, forcing callers to
detour through the sfnt `cmap` table.

The new 256-entry `EXPERT_ENCODING` table is transcribed verbatim from
Appendix B §2 (pages 40-43 of TN5176 4 Dec 03). 165 codes are
assigned, 91 are `.notdef` (matching the appendix's explicit gaps in
codes 0-31, 35, 64, 70-72, 74-75, 80-81, 85, 92, 127-160, 164-165,
171, 173-174, 176-177, 180-181, 185-187, 198-199). Every assigned SID
falls inside the predefined-strings range (max 378 = Ydieresissmall),
so `Font::glyph_index` resolves Expert-encoded codes through the same
Appendix A standard-strings table the rest of the CFF code uses,
without consulting the per-font String INDEX. Six new unit tests cover
the landmark codes, the standard-strings-only invariant, the
assigned-vs-unassigned count from the appendix, custom-charset
routing, the canonical Expert + Expert charset pair (where code 32 =
GID 1, code 255 = GID 165 = Ydieresissmall), and the
`Encoding::parse(_, 1)` dispatch.

This closes the last "noted but not transcribed" item on the round-115
add list and was the only remaining `Encoding::lookup` arm that
returned `None` unconditionally.

## Round-115 additions (previous push)

The two remaining predefined CFF charsets — **Expert** (Top DICT
charset operand 1) and **ExpertSubset** (operand 2) — are now
resolved instead of rejected. Before this push a font selecting
either was rejected at parse time with
`Cff("predefined Expert charset not implemented in round 1")`;
ISOAdobe (operand 0) was the only predefined charset handled.

Both are fixed `GID → SID` lists transcribed from Adobe TN5176
Appendix C in GID order beginning with GID 1 (`.notdef` is the
implicit GID 0). The appendix lays the entries out column-major
across three columns per page block; the new `EXPERT_SIDS` (165
entries → 166 glyphs) and `EXPERT_SUBSET_SIDS` (86 entries → 87
glyphs) arrays linearise them back into GID order. Every SID in
both tables is `<= 390`, i.e. a predefined standard string, so
`Font::glyph_name` resolves through the existing Appendix A
standard-strings table with no per-font String INDEX. Both
charsets implement the same `sid_of(gid)` / `gid_of_sid(sid)`
pair as the custom formats, so the `seac` component resolver and
the legacy-encoding `gid_of_sid` path work unchanged on
expert-charset fonts. Seven new unit tests cover the table
lengths, landmark GID↔SID mappings, a full GID round-trip for
every glyph in each charset, the standard-strings-resolvability
invariant, and the parse-time operand dispatch (1 → Expert, 2 →
ExpertSubset).

## Round-7 additions (this push)

The remaining four CFF Top DICT operators in TN5176 §9 Table 9 that
were already being parsed (the Dict layer kept them in its operand
table) but never surfaced are now exposed on the public `Font` API
and pre-extracted into `cff::TopMetadata`:

- **`FontMatrix`** (Top DICT op 12 07) — 6-element affine matrix
  `[a, b, c, d, tx, ty]` mapping glyph-space coordinates into
  PostScript user space. CFF's spec default is
  `[0.001, 0, 0, 0.001, 0, 0]` (the 1000-unit-em convention), and
  font-author overrides — common in CID fonts and high-resolution
  Type 1-derived fonts — are now visible to callers. Application:
  `x_user = a*x + c*y + tx`, `y_user = b*x + d*y + ty`. A
  non-conforming font emitting fewer than 6 operands is zero-filled
  rather than rejected (mirroring the existing FontBBox tolerance).
- **`PaintType`** (op 12 05) — 0 for filled outlines (every modern
  OpenType-CFF font), 2 for stroked outlines whose pen width is
  `StrokeWidth`. Default: 0.
- **`CharstringType`** (op 12 06) — the charstring format embedded
  in this font. Always 2 for OpenType-CFF; surfaced so callers can
  detect a malformed font carrying a legacy Type 1 charstring stream
  before the interpreter trips. Default: 2.
- **`StrokeWidth`** (op 12 08) — pen width applied when `PaintType
  == 2`, in font units. Default: 0.

`Font::font_matrix` / `paint_type` / `charstring_type` /
`stroke_width` are the new accessors. The numeric fields are also
added to the public `TopMetadata` struct (already re-exported at the
crate root). No new bytes are read from the font — all four
operators were being collected by the Dict parser since round 1 and
are now reached through the same `get_array` / `get_int` /
`get_number` calls the metadata-extraction routine already uses.
Three new unit tests cover defaults, populated values (FontMatrix
via two BCD-real entries + one i16, PaintType / CharstringType via
the 1-byte int form, StrokeWidth via the 1-byte int form), and the
zero-fill tolerance for an undersized FontMatrix; one new integration
test against the Source Sans 3 fixture asserts the surfaced matrix
scales to `1 / upem` along both axes.

## Round-6 additions (previous push)

Type 2 charstring arithmetic / storage / conditional operators (Adobe
TN5177 §§4.4–4.6). Before this push the interpreter rejected any of
these escape operators with `Error::CharstringUnsupportedOp`; fonts
that compute coordinates with them (or call subroutines whose return
value is selected via `ifelse`) now decode:

- **Arithmetic (§4.4):** `abs` (12 9), `add` (12 10), `sub` (12 11),
  `div` (12 12), `neg` (12 14), `mul` (12 24), `sqrt` (12 26),
  `random` (12 23). `div` by zero and `sqrt` of a negative both yield
  0 (the spec leaves them "undefined"; we pick a finite value so a
  malformed font can't inject NaN/Inf into pen coordinates). `random`
  is a deterministic LCG returning a value in (0, 1] — the spec only
  constrains the range, and determinism keeps outline decoding
  reproducible without a system-entropy dependency.
- **Stack (§4.4):** `drop` (12 18), `dup` (12 27), `exch` (12 28),
  `index` (12 29, negative `i` copies the top), `roll` (12 30,
  circular shift of the top N by J, positive = upward).
- **Storage (§4.5):** `put` (12 20) / `get` (12 21) over a 32-element
  transient array (the size fixed by TN5177 Appendix B). An
  out-of-range index surfaces as the new
  `Error::CharstringTransientIndex(i32)`; a `get` of an unwritten slot
  returns a defined 0.
- **Conditional (§4.6):** `and` (12 3), `or` (12 4), `not` (12 5),
  `eq` (12 15), `ifelse` (12 22, leaves `s1` if `v1 <= v2` else `s2`).

Unlike the path operators, these pop their inputs from the **top** of
the argument stack and push their result back, leaving the rest of the
stack intact (they never clear it). 18 new unit tests drive every
operator through a `rmoveto` so the resulting pen position proves the
computed value, plus underflow / out-of-range rejection paths.

## Round-5 additions (this push)

CID-keyed CFF support (Adobe TN5176 §§18, 19):

- A Top DICT beginning with `ROS` (op 12 30) is now recognised as a
  CID-keyed font. Such fonts have no top-level Private DICT; instead
  every glyph is routed through `FDSelect` (op 12 37) to one of the
  Font DICTs in the `FDArray` (op 12 36), and each Font DICT carries
  its own Private DICT (Local Subrs + width defaults). Before this
  push, any CID font was rejected at parse time with
  `Cff("Top DICT missing Private")`.
- `FDSelect` is implemented for both on-disk formats — format 0
  (a flat `Card8 fds[nGlyphs]` array) and format 3 (range-encoded
  `(first, fd)*` records + a sentinel GID), per TN5176 Tables 27-29.
- `Cff::glyph_outline` selects the per-glyph Private DICT, so glyphs
  in different FD groups decode with the correct subroutines and
  `defaultWidthX` / `nominalWidthX`.
- New public surface: `Font::is_cid` / `cid_registry` / `cid_ordering`
  / `cid_supplement` / `cff_fd_count`, plus `Cff::is_cid` /
  `registry_ordering` / `fd_count` and the re-exported
  `RegistryOrdering` type.
- A complete CID-keyed CFF (2 FDs, 3 glyphs, FDSelect format 3) is
  assembled byte-by-byte from the spec layout in the unit tests and
  parsed back, asserting ROS resolution, per-FD width routing, and
  outline decode for every glyph.

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
- Hint enforcement (we anti-alias at >= 16 px, so hints are noise).
- The Adobe Glyph List string → codepoint mapping (round 3+ if any
  consumer needs it).
- `OS/2`, `post`, `GSUB`, `GPOS`, `GDEF`, `kern` tables — the Adobe
  CFF / Type 2 / sfnt PDFs are now staged under
  `docs/text/opentype/spec/`, but the layout-table (GSUB/GPOS/GDEF)
  and `OS/2` / `post` definitions live in the Microsoft OpenType
  spec; only the HTML snapshot is staged so far.

## Test fixture

`tests/fixtures/SourceSans3-Regular.otf` is Adobe Source Sans 3
Regular under the SIL Open Font License v1.1 (see
`tests/fixtures/SOURCE-SANS-LICENSE`). 335 KB, ~1900 glyphs,
exercises every common Type 2 operator including flex.

## License

MIT — see [`LICENSE`](LICENSE).
