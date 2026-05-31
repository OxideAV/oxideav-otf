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
  - Private DICT including `defaultWidthX` / `nominalWidthX`, the
    Local Subrs INDEX offset, and the full hint-zone vocabulary
    (`BlueValues` / `OtherBlues` / `FamilyBlues` /
    `FamilyOtherBlues` undeltified per TN5176 §4 Table 4 "delta"
    semantics; `StdHW` / `StdVW`; `StemSnapH` / `StemSnapV`;
    `BlueScale` / `BlueShift` / `BlueFuzz`; `ForceBold`;
    `LanguageGroup`; `ExpansionFactor`; `initialRandomSeed`).
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
  `cmap` (formats 0/4/6/12), `name`, `post` (every spec version), and
  `OS/2` (versions 0..5, all six layouts).

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
let _ = font.unique_id();           // Option<i32> — legacy PS Type 1 ID
let _ = font.xuid();                // &[i32] — extended unique ID array
let _ = font.synthetic_base();      // Option<i32> — Name-INDEX index
let _ = font.postscript();          // Option<&str> — embedded PS code
let _ = font.base_font_name();      // Option<&str> — MM master FontName
let _ = font.base_font_blend();     // &[f64] — undeltified UDV

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

// CFF Private DICT hint zones (TN5176 §15 Table 23).
let h = font.private_hints();
let _ = &h.blue_values;          // undeltified absolute y-coords
let _ = &h.other_blues;
let _ = h.std_hw;                // Option<f64>
let _ = h.std_vw;
let _ = &h.stem_snap_h;
let _ = h.blue_scale;            // 0.039625 default
let _ = h.force_bold;            // bool
let _ = h.language_group;        // 0 (Latin) / 1 (CJK)
let _ = font.glyph_private_hints(gid);  // CID-aware per-glyph routing

// CID-keyed fonts (TN5176 §18) — None / 0 on a plain CFF font.
let _ = font.is_cid();
let _ = font.cid_registry();        // Some("Adobe")
let _ = font.cid_ordering();        // Some("Japan1") / Some("Identity")
let _ = font.cid_supplement();      // Some(7)
let _ = font.cff_fd_count();        // number of FDArray Font DICTs

// OS/2 and Windows Metrics (spec versions 0..5, all supported).
let _ = font.os2_version();         // Some(3) on Source Sans 3
let _ = font.weight_class();        // Some(400) = Regular
let _ = font.width_class();         // Some(5) = Medium
let _ = font.width_class_percent(); // Some(100.0); maps 1..9 to spec %
let _ = font.fs_type();             // raw embedding-licensing bits
let _ = font.embedding_permission(); // Installable / RestrictedLicense / …
let _ = font.is_italic();
let _ = font.is_bold();
let _ = font.is_regular();
let _ = font.use_typo_metrics();    // fsSelection bit 7 (v4+)
let _ = font.is_oblique();          // fsSelection bit 9 (v4+)
let _ = font.vendor_id();           // achVendID as &str (e.g. "ADBO")
let _ = font.panose();              // &[u8; 10] PANOSE classification
let _ = font.typo_ascender();       // sTypoAscender (v0-full+)
let _ = font.typo_descender();
let _ = font.typo_line_gap();
let _ = font.win_ascent();          // usWinAscent (UFWORD)
let _ = font.win_descent();
let _ = font.x_height();            // sxHeight (v2+)
let _ = font.cap_height();          // sCapHeight (v2+)
let _ = font.default_char();
let _ = font.break_char();          // conventionally Some(0x20)
let _ = font.max_context();         // GSUB/GPOS max context length

for contour in &outline.contours {
    for seg in &contour.segments {
        // CubicSegment::MoveTo / LineTo / CurveTo / ClosePath
        let _ = seg;
    }
}
```

## Round-198 additions (this push)

The OpenType **`OS/2` and Windows Metrics table** is now parsed and
surfaced on the public `Font` API. Spec: Microsoft / ISO/IEC
14496-22 `OS/2` (`docs/text/opentype/otspec-os2.html`). Previously
the table was reachable only as raw bytes through
`Font::table_data(b"OS/2")`; the new `Os2Table` (re-exported from
the crate root via [`Font::os2`] plus a wide set of per-field
convenience getters) decodes every version 0..5 layout described in
the spec's *OS/2 Table Formats* preamble.

- **Six versions, all supported.** Version 0 in both the 68-byte
  "short" layout (Apple's TrueType Reference Manual variant — see
  `otspec-os2.html` "Some legacy TrueType fonts could have been
  built with a shortened version 0 OS/2 table" note) and the
  78-byte "full" Microsoft layout; v1 (86 bytes, adds
  `ulCodePageRange`); v2/v3/v4 (96 bytes, add `sxHeight` …
  `usMaxContext`, fsSelection bits 7–9 in v4); v5 (100 bytes, adds
  `usLower/UpperOpticalPointSize`).
- **Every header field decoded.** Weight class (`usWeightClass`),
  width class (`usWidthClass` + the spec's "% of normal" mapping
  table from 1..9 to 50/62.5/75/87.5/100/112.5/125/150/200),
  embedding-licensing bitfield (`fsType` + the decoded
  `EmbeddingPermission` enum covering Installable / Restricted
  License / Preview&Print / Editable plus the "no subsetting" and
  "bitmap embedding only" bits), subscript/superscript metrics, the
  strikeout pair, family-class split (`sFamilyClass` decomposed into
  `(class, subclass)`), 10-byte PANOSE classification, the four
  `ulUnicodeRange*` words plus a `has_unicode_range_bit(bit)`
  query, four-byte `achVendID` (raw plus best-effort UTF-8 / ASCII
  view), the 10 named `fsSelection` style bits (ITALIC, UNDERSCORE,
  NEGATIVE, OUTLINED, STRIKEOUT, BOLD, REGULAR, USE_TYPO_METRICS,
  WWS, OBLIQUE) exposed both as `FS_SELECTION_*` mask constants and
  per-bit predicate accessors, and the first / last char-index pair.
- **Version-gated tails.** Typographic / Windows metrics
  (`sTypoAscender`, `sTypoDescender`, `sTypoLineGap`, `usWinAscent`,
  `usWinDescent`) are reported via `Option<i16>` / `Option<u16>` and
  return `None` only on the legacy v0-short layout. Code-page range
  (`ulCodePageRange1` / `ulCodePageRange2` + a 64-bit
  `has_code_page_bit(bit)` query) is v1+. `sxHeight`, `sCapHeight`,
  `usDefaultChar`, `usBreakChar`, `usMaxContext` are v2+. Optical
  point-size range (`usLower/UpperOpticalPointSize`) is v5; the raw
  TWIPs values and the TWIPs/20 → points conversion are both
  exposed.
- **Truncation rejection.** A short v0 (< 68 bytes) is
  `Error::UnexpectedEof`; a v1+ table shorter than its declared
  layout is `Error::BadStructure`. The v0-short / v0-full
  distinction is purely table-length-driven per the spec's "check
  the table length before reading these fields" note.
- **`Font` integration.** [`Font::os2`] borrows the decoded table;
  for the most common consumer paths, lookup-free convenience
  getters on `Font` route the data: `Font::weight_class`,
  `Font::width_class`, `Font::width_class_percent`, `Font::fs_type`,
  `Font::embedding_permission`, `Font::is_italic`, `Font::is_bold`,
  `Font::is_regular`, `Font::use_typo_metrics`, `Font::is_oblique`,
  `Font::vendor_id`, `Font::panose`, `Font::typo_ascender`,
  `Font::typo_descender`, `Font::typo_line_gap`, `Font::win_ascent`,
  `Font::win_descent`, `Font::x_height`, `Font::cap_height`,
  `Font::default_char`, `Font::break_char`, `Font::max_context`.
  Absence of the table surfaces as `None` rather than rejecting the
  whole font (mirroring how the round-187 `post` integration treats
  optional tables).

Nineteen new unit tests in `src/tables/os2.rs` cover full v5 parse,
the v4 / v1 / v0-full / v0-short version drops, error paths for
short v0 (< 68 bytes) / version > 5 / truncated v1 tail (before
typo metrics, before code-page range) / truncated v2 tail / v5
without optical-size, the spec's nine-entry `usWidthClass`
percent-of-normal table, every `EmbeddingPermission` discriminant
(including the spec-reserved bit-0 case and the
multiple-bits-set legacy v0..v2 case), every named `fsSelection`
bit helper, non-ASCII `achVendID` fallback, walking
`has_unicode_range_bit` across all four words, the
`sFamilyClass` (class, subclass) split, and the TWIPs / points
optical-size conversion. One new integration test against the
Source Sans 3 fixture decodes its real-world v3 96-byte `OS/2`
table: version 3, exactly 96 bytes, weight 400, width 5 (100%),
`fsType = 0` (Installable embedding), `achVendID = "ADBO"`,
PANOSE family-type 2 (Latin Text), Basic-Latin Unicode-range bit 0
set, Latin-1 code-page bit 0 set, typo / win metrics positive and
mutually consistent, `usFirstCharIndex = 0x0020` and
`usLastCharIndex = 0xFFFF`, `usBreakChar = 0x0020`, no v5 optical-
size tail.

## Round-187 additions (previous push)

The OpenType **`post` table** (PostScript table) is now parsed and
surfaced on the public `Font` API. Spec: Microsoft / ISO/IEC 14496-22
`post` (`docs/text/opentype/otspec-post.html`). Previously the table
was reachable through the generic `Font::table_data(b"post")` bytes
accessor but never decoded; the new `PostTable` (and `PostFormat`
enum, both re-exported at the crate root) decode the 32-byte header
for every version and the version-2.0 / 2.5 tails.

- **Header (every version):** `italic_angle` (decoded from the on-disk
  16.16 `Fixed`), `underline_position` (FWORD = `i16`),
  `underline_thickness`, `is_fixed_pitch` (any non-zero on the
  `uint32` field rounds up to `true` per spec), and the four VM hint
  fields `min_mem_type42` / `max_mem_type42` / `min_mem_type1` /
  `max_mem_type1`.
- **Format 3.0** — header only; this is the format OpenType-CFF1
  fonts must use per the spec's "Versions" preamble. Source Sans 3
  Regular ships a 32-byte version-3.0 `post`; the new integration
  test asserts the exact 32-byte length, version `0x00030000`,
  zero italic angle, `isFixedPitch = false`, negative
  `underlinePosition`, and positive `underlineThickness` below
  `unitsPerEm`.
- **Format 2.0** — the header + a `numGlyphs` `u16` + a
  `glyphNameIndex[numGlyphs]` `u16` array + a Pascal-string
  `stringData` tail. `PostTable::name_index(gid)` returns the raw
  index; `name_string(pascal_index)` walks the Pascal-string list
  and returns the requested entry as a `&[u8]`. The two-half
  semantics from the spec (indices `0..258` = standard Mac glyph
  set; indices `258..65535` = `index − 258` into the Pascal list)
  are documented per-accessor.
- **Format 2.5** — the header + `numGlyphs` `u16` + `offset[numGlyphs]`
  signed-byte array; `PostTable::standard_offset(gid)` returns the
  raw `i8`. The format is flagged deprecated by both the spec and
  this implementation but still parsed for completeness.
- **Format 1.0** and any **`Other` Version16Dot16** value (e.g.
  Apple's 4.0 extension, "not supported in OpenType" per the spec)
  decode the header and skip the tail.

New on `Font`: `post()`, `post_format()`, `post_italic_angle()`,
`post_underline_position()`, `post_underline_thickness()`,
`post_is_fixed_pitch()`, and `post_glyph_name(gid)`. The latter
returns the per-glyph Pascal-style name for format 2.0 glyphs whose
`glyphNameIndex >= 258` (the non-standard half); for `< 258`
indices, the standard-Macintosh 258-entry list is referenced from
`otspec-post.html` but is not staged in
`docs/text/opentype/spec/` — only the Apple TrueType Reference
Manual's *table of contents* page is currently there. That
sub-feature is documented as a docs gap; callers wanting per-glyph
names that work universally for CFF1 fonts should keep using the
existing `Font::glyph_name` (CFF charset → strings) which has no
gap. The `post` table is treated as optional (it is one of OpenType's
nine required tables per `otff` spec, but real-world stripped-down
fonts sometimes omit it); a missing `post` parses fine and the
accessors return `None`.

Seventeen new unit tests in `src/tables/post.rs` cover the v1.0 /
v3.0 / v2.0 / v2.5 / `Other` header decodes, italic-angle
fractional decode, the `isFixedPitch` non-zero high-bit case, every
VM field, the v2.0 multi-Pascal-string round-trip with the spec's
worked example (glyph 408 → name index 262 → Pascal index 4), the
v2.5 worked example (`+36, +36, +36` for A/B/C at positions
37/38/39), truncation rejection paths, and the v2.0 Pascal-length
spec-defensive `None` return when the on-disk length walks past the
table tail. One new integration test against the Source Sans 3
fixture asserts format 3.0 + zero italic + proportional + plausible
underline.

## Round-183 additions (previous push)

CFF Private DICT hint zones (Adobe TN5176 §15 Table 23) are now
surfaced on the public `Font` API. Previously the Private DICT parser
extracted `defaultWidthX` / `nominalWidthX` / `Subrs` and silently
ignored every other operator; the new `PrivateHints` struct
(re-exported at the crate root) holds the full TN5176 §15 vocabulary
and exposes it through `Font::private_hints` and
`Font::glyph_private_hints`.

- **`BlueValues`** (op 6) / **`OtherBlues`** (op 7) /
  **`FamilyBlues`** (op 8) / **`FamilyOtherBlues`** (op 9) —
  alignment zones, each declared as the spec's "delta" operand type
  (§4 Table 4: first operand absolute, every subsequent operand is a
  difference from the running total). The accessors return the
  **undeltified** absolute y-coordinates. So TN5176's spec-worked
  raw stream `[-14, 14, 662, 14, -226, 10, 223, 0]` surfaces as
  `[-14, 0, 662, 676, 450, 460, 683, 683]`. Empty vectors when the
  operator is absent.
- **`StdHW`** (op 10) / **`StdVW`** (op 11) — dominant horizontal and
  vertical stem widths. `Option<f64>` so callers can distinguish
  "absent" from "zero" (TN5176 lists no default value for either).
- **`StemSnapH`** (op 12 12) / **`StemSnapV`** (op 12 13) —
  supplementary stem widths the rasterizer can snap stems to.
  Delta-encoded just like the blue-zone arrays; the accessor returns
  the running sums.
- **`BlueScale`** (op 12 9, default `0.039625`), **`BlueShift`**
  (op 12 10, default `7`), **`BlueFuzz`** (op 12 11, default `1`) —
  overshoot suppression tunables.
- **`ForceBold`** (op 12 14, default `false`) — Multiple Master
  synthetic-bold flag. Boolean operand decoded as `false` for `0`,
  `true` otherwise.
- **`LanguageGroup`** (op 12 17, default `0`) — `0` for Latin /
  Cyrillic / etc., `1` for CJK.
- **`ExpansionFactor`** (op 12 18, default `0.06`) — limit on the
  per-counter expansion allowed when forcing bold.
- **`initialRandomSeed`** (op 12 19, default `0`) — seed for the Type
  2 `random` operator.

CID-keyed fonts (TN5176 §18) carry one Private DICT per FDArray Font
DICT; `Font::private_hints` returns FDArray index 0 (matching the
glyph routing for FDSelect's first entry on most CID fonts), and
`Font::glyph_private_hints(gid)` routes through `FDSelect` per
TN5176 §19 to surface the correct per-FD hints. Callers iterating
the full FDArray can use `font.cff().private_hints_fd(i)` directly.

Hinting is still not *enforced* by the round-1 outline pipeline (we
anti-alias at >= 16 px); this surface is for callers inspecting font
metadata or implementing their own hinting downstream.

Eight new unit tests in `src/cff/private.rs` cover spec defaults,
delta-undeltification for every "delta"-typed operator, scalar
overrides, `ForceBold` boolean decode, and a worked TN5176 Appendix-D
Private DICT layout whose every field matches the spec's listed
bytes. One new integration test against the Source Sans 3 fixture
asserts BlueValues come in `(bottom, top)` pairs, are monotone
non-decreasing after undeltification, are font-unit integral;
`StdHW` / `StdVW` are positive; `BlueScale` / `BlueShift` /
`BlueFuzz` lie in plausible ranges; `LanguageGroup == 0` and
`ForceBold == false` for a Latin upright font; and that
`glyph_private_hints` on any in-range glyph returns the same struct
as `private_hints` (the non-CID invariant).

## Round-176 additions (previous push)

CFF Top DICT identity + synthetic-font operators (Adobe TN5176 §9
Tables 9 and 10) are now extracted into `TopMetadata` and surfaced on
the public `Font` API. Previously the Top DICT parser already
collected these into the raw entry list but the high-level metadata
struct only surfaced FontBBox / FontMatrix / paint / italic / underline
/ string-SID fields.

- **`UniqueID`** (op 13, "number") — `Font::unique_id() -> Option<i32>`.
  The legacy Adobe-assigned PostScript Type 1 unique identifier. Modern
  fonts prefer `XUID`; many recent OpenType-CFF fonts omit it.
- **`XUID`** (op 14, "array") — `Font::xuid() -> &[i32]`. Extended
  unique-identifier array; the spec leaves the length unconstrained
  beyond "array." Empty slice if absent. Deprecated in OpenType-CFF per
  TN5176 4 Dec 03 Appendix H but still emitted by older tooling.
- **`SyntheticBase`** (op 12 20, "number") —
  `Font::synthetic_base() -> Option<i32>`. The Name-INDEX index of the
  base font for synthetic fonts. Almost never present in shipping
  OpenType-CFF (OpenType is one-font-per-CFF) but spec-defined.
- **`PostScript`** (op 12 21, SID) — `Font::postscript() -> Option<&str>`.
  Embedded PostScript language code (TN5176 §9 Table 10), resolved
  through the CFF Strings table.
- **`BaseFontName`** (op 12 22, SID) —
  `Font::base_font_name() -> Option<&str>`. For multiple-master-derived
  synthetics, the FontName of the underlying master, SID-resolved.
- **`BaseFontBlend`** (op 12 23, "delta") —
  `Font::base_font_blend() -> &[f64]`. The User Design Vector for the
  master. The on-disk operands are *delta-encoded* per TN5176 §4
  Table 4 ("delta" type: first operand is absolute, each subsequent
  operand is the difference from the running total); the accessor
  returns the **undeltified** absolute values, so a raw stream of
  `[10, 5, -3, 2]` surfaces as `[10.0, 15.0, 12.0, 14.0]`. Empty
  slice if absent.

Six new unit tests in `src/cff/mod.rs` hand-encode a Top DICT carrying
each operator (including the spec's worked `UniqueID = 28416` example
from TN5176 §9 p. 19), plus an extended defaults test that asserts the
new fields default to `None` / empty for fonts that omit them.

The operator codes 12 20–23 are TN5176 §9 Table 10 escape operators;
the existing single-byte op 20 / 21 enum discriminants for the Private
DICT (`DefaultWidthX` / `NominalWidthX`) coexist cleanly because the
`Operator` enum is `#[repr(u16)]` and the escape form encodes as
`0x0C00 | sub` (e.g. `SyntheticBase = 0x0C14 ≠ DefaultWidthX = 0x14`).

## Round-171 additions (previous push)

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
- `GSUB`, `GPOS`, `GDEF`, `kern` tables — the Adobe CFF /
  Type 2 / sfnt PDFs are now staged under `docs/text/opentype/spec/`
  alongside the Microsoft per-table HTML snapshots
  (`otspec-gsub.html` / `otspec-gpos.html` / `otspec-gdef.html`),
  so future rounds can pick these up; round 187 took the `post`
  table off this list and round 198 took the `OS/2` table off it.
- Format-1.0 / 2.0 / 2.5 glyph-name lookups in `post` (the
  standard-Macintosh 258-entry list referenced from
  `otspec-post.html`). The list lives in Apple's TrueType Reference
  Manual chapter 6 and is not currently staged in
  `docs/text/opentype/`; only the manual's table-of-contents page is
  there. The non-standard Pascal-string half is fully resolvable.

## Test fixture

`tests/fixtures/SourceSans3-Regular.otf` is Adobe Source Sans 3
Regular under the SIL Open Font License v1.1 (see
`tests/fixtures/SOURCE-SANS-LICENSE`). 335 KB, ~1900 glyphs,
exercises every common Type 2 operator including flex.

## License

MIT — see [`LICENSE`](LICENSE).
