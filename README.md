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

// GDEF — Glyph Definition Table (optional; None for fonts without
// GSUB / GPOS layout lookups).
let _ = font.gdef();                // Option<&GdefTable>
let _ = font.gdef_version();        // Some((1, 0)) on Source Sans 3
let _ = font.glyph_class(gid);      // Some(GlyphClass::Base | …) / None
let _ = font.mark_attach_class(gid); // mark-attach class number; 0 = unclassified

// GSUB / GPOS — header views (optional; both None for fonts without
// substitution or positioning rules).
let _ = font.gsub();                // Option<&GsubTable>
let _ = font.gsub_version();        // Some((1, 0)) on Source Sans 3
let _ = font.gpos();                // Option<&GposTable>
let _ = font.gpos_version();        // Some((1, 0)) on Source Sans 3
if let Some(g) = font.gsub() {
    let scripts = g.script_list()?;
    let dflt = g.find_script(b"DFLT");
    let _ = g.feature_count();      // number of FeatureRecords
    let _ = g.lookup_count();
    let _ = scripts;
    let _ = dflt;
    for (tag, feat) in g.feature_list()?.iter() {
        // tag = b"liga" / b"kern" / b"calt" / …
        let _ = (tag, feat);
    }
    for lookup in g.lookup_list()?.iter() {
        let l = lookup?;
        let _ = l.lookup_type();    // 1..=8 for GSUB, 1..=9 for GPOS
        let _ = l.flag().ignore_marks();
        let _ = l.mark_filtering_set();
    }

    // GSUB Lookup Type 1 — single substitution. The typed view
    // decodes both on-disk subtable formats and answers
    // `substitute(input)` / iterates `(input, output)` pairs.
    for i in 0..g.lookup_count() {
        let l = g.lookup(i).unwrap();
        if l.lookup_type() != oxideav_otf::GSUB_LOOKUP_TYPE_SINGLE {
            continue;
        }
        for s in 0..l.subtable_count() {
            let ss = g.single_subst(i, s).unwrap()?;
            let _ = ss.format();          // 1 or 2
            let _ = ss.substitute(42);    // Option<u16>; None when uncovered
            for (input, output) in ss.iter() {
                // Apply the substitution as a shaper would.
                let _ = (input, output);
            }
        }
    }

    // GSUB Lookup Type 2 — multiple substitution (one → many). The
    // typed view decodes Coverage + Sequence tables and answers
    // `substitute(input)` returning a borrowed `Sequence` whose
    // `glyphs()` iterator yields the output glyph sequence. Per spec,
    // every Sequence has glyphCount >= 1 (the standard prohibits
    // using Multiple substitution as a deletion).
    for i in 0..g.lookup_count() {
        let l = g.lookup(i).unwrap();
        if l.lookup_type() != oxideav_otf::GSUB_LOOKUP_TYPE_MULTIPLE {
            continue;
        }
        for s in 0..l.subtable_count() {
            let ms = g.multiple_subst(i, s).unwrap()?;
            // Walk every (input_glyph, Sequence) pair.
            for (input, seq_res) in ms.iter() {
                let seq = seq_res?;
                let _ = seq.glyph_count();              // always >= 1
                let _: Vec<u16> = seq.glyphs().collect();
                let _ = input;
            }
            // Apply as a shaper would: replace the input glyph with
            // its sequence (if covered) and advance the cursor by
            // `seq.glyph_count()` output positions.
            if let Some(seq) = ms.substitute(/* current_glyph */ 0u16) {
                let _ = seq.glyphs();
            }
        }
    }

    // GSUB Lookup Type 3 — alternate substitution (one → choice of
    // many). The typed view decodes Coverage + AlternateSet tables and
    // answers `substitute(input)` returning a borrowed `AlternateSet`
    // whose `glyphs()` iterator yields the aesthetic alternatives (in
    // arbitrary order per spec — picking one is a higher-layer decision).
    for i in 0..g.lookup_count() {
        let l = g.lookup(i).unwrap();
        if l.lookup_type() != oxideav_otf::GSUB_LOOKUP_TYPE_ALTERNATE {
            continue;
        }
        for s in 0..l.subtable_count() {
            let alt = g.alternate_subst(i, s).unwrap()?;
            for (input, set_res) in alt.iter() {
                let set = set_res?;
                let _ = set.glyph_count();
                let _: Vec<u16> = set.glyphs().collect();
                let _ = input;
            }
            // Apply as a shaper would: the covered input glyph offers a
            // set of equivalents; the client substitutes one of them.
            if let Some(set) = alt.substitute(/* current_glyph */ 0u16) {
                let _ = set.glyphs();
            }
        }
    }

    // GSUB Lookup Type 4 — ligature substitution (many → one). The
    // typed view decodes Coverage + LigatureSet + Ligature tables and
    // answers `substitute(input)` returning `(ligature_glyph,
    // components_consumed)` for the first matching Ligature in the
    // set (spec: array order = preference order).
    for i in 0..g.lookup_count() {
        let l = g.lookup(i).unwrap();
        if l.lookup_type() != oxideav_otf::GSUB_LOOKUP_TYPE_LIGATURE {
            continue;
        }
        for s in 0..l.subtable_count() {
            let ls = g.ligature_subst(i, s).unwrap()?;
            // Walk every (first_component_glyph, LigatureSet) pair.
            for (first_glyph, set_res) in ls.iter() {
                let set = set_res?;
                for j in 0..set.ligature_count() {
                    let lig = set.ligature(j).unwrap()?;
                    let _ = lig.ligature_glyph();
                    let _ = lig.component_count();   // includes first_glyph
                    let _: Vec<u16> = lig.component_glyphs().collect();
                    let _ = first_glyph;
                }
            }
            // Apply as a shaper would: feed the current input slice
            // starting at the candidate first_glyph; on success, advance
            // the shaper's cursor by `components` glyphs.
            let input: &[u16] = &[/* current_glyph, next_glyph, ... */];
            if let Some((out_glyph, components)) = ls.substitute(input) {
                let _ = (out_glyph, components);
            }
        }
    }

    // GSUB Lookup Type 7 — substitution extension (32-bit-offset
    // indirection wrapping a subtable of any other lookup type). The
    // typed view validates the header and resolves the wrapped
    // subtable; per spec, process as though the extension subtable
    // replaced the type-7 subtable that referenced it.
    for i in 0..g.lookup_count() {
        let l = g.lookup(i).unwrap();
        if l.lookup_type() != oxideav_otf::GSUB_LOOKUP_TYPE_EXTENSION {
            continue;
        }
        for s in 0..l.subtable_count() {
            let ext = g.extension_subst(i, s).unwrap()?;
            let _ = ext.format();                   // always 1
            let _ = ext.extension_lookup_type();    // 1..=8, never 7
            let _ = ext.extension_offset();         // Offset32
            let _ = ext.extension_subtable_bytes(); // raw wrapped bytes
            // Typed resolution for the already-decoded wrapped types.
            match ext.extension_lookup_type() {
                oxideav_otf::GSUB_LOOKUP_TYPE_SINGLE => {
                    let ss = ext.as_single_subst()?;
                    let _ = ss.substitute(42);
                }
                oxideav_otf::GSUB_LOOKUP_TYPE_LIGATURE => {
                    let ls = ext.as_ligature_subst()?;
                    let _ = ls.substitute(&[1, 2]);
                }
                _ => { /* as_multiple_subst / as_alternate_subst, or raw */ }
            }
        }
    }
}

for contour in &outline.contours {
    for seg in &contour.segments {
        // CubicSegment::MoveTo / LineTo / CurveTo / ClosePath
        let _ = seg;
    }
}
```

## Round-277 additions (this push)

GSUB **Lookup Type 7 (substitution extension)** is now decoded as a
typed view, joining Type 1 (single, round 247), Type 2 (multiple,
round 262), Type 3 (alternate, round 270), and Type 4 (ligature,
round 248). Spec: `docs/text/opentype/otspec-gsub.html` §"Lookup
type 7 subtable: substitution subtable extension". Type 7 is a
*format extension mechanism*, not a substitution action: it lets a
Lookup reach its real subtable through a 32-bit offset, for fonts
whose accumulated subtable sizes exceed what 16-bit offsets can
address. One on-disk format is defined (`SubstExtensionFormat1`,
8 bytes).

- **`ExtensionSubst<'a>`** decodes `(format, extensionLookupType,
  Offset32 extensionOffset)`. Parse-time validation: `format == 1`;
  `extensionLookupType` must be a defined GsubLookupType (`1..=8`)
  **other than 7** — the spec forbids an extension pointing at
  another extension; and `extensionOffset` (relative to the start of
  the ExtensionSubstFormat1 subtable, per spec) must be non-NULL and
  land inside the subtable's byte window.
- **`ExtensionSubst::extension_subtable_bytes()`** surfaces the
  wrapped ("extension") subtable as a zero-copy byte window starting
  at `extensionOffset` — the spec's processing model is to proceed
  as though each extension subtable replaced the type-7 subtable
  that referenced it, with the Lookup's effective type being
  `extensionLookupType`.
- **Typed resolvers** for the wrapped types this crate already
  decodes: `as_single_subst()` / `as_multiple_subst()` /
  `as_alternate_subst()` / `as_ligature_subst()`. Each checks the
  declared `extensionLookupType` first and rejects a mismatch with
  `BadStructure`; wrapped types 5 / 6 / 8 stay reachable through the
  raw-bytes window.
- **`GsubTable::extension_subst(lookup_i, sub_i)`** is the
  convenience accessor mirroring the existing type-1..4 accessors:
  `None` for out-of-range indices, `Some(Err(BadStructure))` when
  the referenced lookup is not declared as
  `GSUB_LOOKUP_TYPE_EXTENSION` (= 7).

The `ExtensionSubst` type is re-exported at the crate root. The
remaining GSUB lookup types (5 Contextual, 6 Chained-context,
8 Reverse-chained-single) remain raw byte slices via
`Lookup::subtable_bytes(i)`.

Synthetic-byte unit tests cover round-trips wrapping a
`SingleSubstFormat1` (delta substitution resolves through the
indirection) and the spec's Example-6 ligature subtable, the
raw-bytes path for a not-yet-typed wrapped type (8), every error
path (`format != 1`, the spec-forbidden `extensionLookupType == 7`,
out-of-vocabulary types 0 / 9 / 0xFFFF, NULL and out-of-range
`extensionOffset`, truncated headers), the wrong-type resolver and
accessor rejections, and an end-to-end GSUB byte tower whose only
lookup is a type-7 extension wrapping a single substitution. One new
integration test against the Source Sans 3 fixture walks every
lookup, decodes any type-7 subtables — validating the spec's "all
extension subtables of one Lookup must have the same
extensionLookupType" rule and resolving wrapped types 1..4 through
the typed views — and pins the accessor semantics on a real
non-type-7 lookup (the fixture is small enough that its GSUB does
not need the 32-bit indirection, so the walk also documents that
absence is legitimate).

## Round-270 additions (previous push)

GSUB **Lookup Type 3 (alternate substitution)** is now decoded as a
typed view, joining Type 1 (single substitution, round 247), Type 2
(multiple substitution, round 262), and Type 4 (ligature
substitution, round 248). Spec:
`docs/text/opentype/otspec-gsub.html` §"Lookup type 3 subtable:
alternate substitution". One on-disk format is defined
(`AlternateSubstFormat1`); the typed view exposes every field down to
the per-AlternateSet `alternateGlyphIDs[]` array.

- **`AlternateSubst<'a>`** decodes the subtable header (`format`,
  `coverageOffset`, `alternateSetCount`, `alternateSetOffsets[]`). The
  Coverage table is re-used from `tables::gdef::Coverage` (the same
  shared common-layout primitive that GPOS, GDEF, and GSUB types
  1 / 2 / 4 / 5 / 6 / 8 read). Parse-time validates the spec's
  "ordered by Coverage index" rule
  (`alternateSetCount == coverage.len()`) alongside the usual range
  checks on `coverageOffset` and `alternateSetOffsets[]`.
- **`AlternateSet<'a>`** decodes the per-input
  `(glyphCount, alternateGlyphIDs[glyphCount])` payload. Unlike
  `Sequence` (Type 2), the spec sets no lower bound on `glyphCount`,
  so an empty AlternateSet (no alternatives) is accepted, not
  rejected. The alternatives are "in arbitrary order" per spec —
  index 0 is not privileged.
- **`AlternateSubst::substitute(input: u16) -> Option<AlternateSet>`**
  is the shaper-path entrypoint. The input glyph is looked up in
  Coverage; the corresponding AlternateSet is returned as a zero-copy
  view over the on-disk `alternateGlyphIDs[]` bytes. It does **not**
  itself pick an alternate — selection is a higher-layer (feature /
  UI) decision per spec.
- **`AlternateSubst::iter()`** yields every `(input_glyph,
  AlternateSet)` pair in ascending Coverage order;
  **`AlternateSet::glyph(i)`** borrows the alternate at index `i`; and
  **`AlternateSet::glyphs()`** yields every alternate in on-disk order.
- **`GsubTable::alternate_subst(lookup_i, sub_i)`** is the convenience
  accessor that walks the lookup chain and confirms the lookup type is
  `GSUB_LOOKUP_TYPE_ALTERNATE` (= 3) before parsing. Returns `None`
  for out-of-range indices and `Some(Err(BadStructure))` when the
  referenced lookup is the wrong type. Mirrors the existing
  `single_subst(...)` / `multiple_subst(...)` / `ligature_subst(...)`
  accessors on the same type.

The `AlternateSubst`, `AlternateSubstIter`, `AlternateSet`, and
`AlternateGlyphIter` types are re-exported at the crate root. The
remaining GSUB lookup types (5 Contextual, 6 Chained-context,
7 Extension, 8 Reverse-chained-single) remain raw byte slices via
`Lookup::subtable_bytes(i)`.

Synthetic-byte unit tests cover the spec's worked Example 5 (default
ampersand glyph `0x003A` mapping to alternatives `[0x00C9, 0x00CA]`),
Coverage iteration across two covered glyphs in ascending order, every
error path (`format != 1`, out-of-range `coverageOffset`,
`alternateSetCount != coverage.len()`, truncated
`alternateSetOffsets[]`), the accepted empty-AlternateSet case, and
the out-of-range / wrong-type accessor returns. One new integration
test against the Source Sans 3 fixture walks every type-3 lookup (the
font ships one — a single subtable with ~210 AlternateSet tables for
its `aalt` feature), decodes every `AlternateSubst` and every
per-input `AlternateSet`, verifies (a) Coverage iteration is
ascending, (b) every alternate glyph fits inside `maxp.numGlyphs`,
(c) the `glyph(k)` point-lookup agrees with the `glyphs()` iterator,
and (d) `substitute(input)` agrees with the iter/set path.

## Round-262 additions (previous push)

GSUB **Lookup Type 2 (multiple substitution)** is now decoded as a
typed view, joining Type 1 (single substitution, round 247) and
Type 4 (ligature substitution, round 248). Spec:
`docs/text/opentype/otspec-gsub.html` §"Lookup type 2 subtable:
multiple substitution". One on-disk format is defined
(`MultipleSubstFormat1`); the typed view exposes every field down to
the per-Sequence `substituteGlyphIDs[]` array.

- **`MultipleSubst<'a>`** decodes the subtable header (`format`,
  `coverageOffset`, `sequenceCount`, `sequenceOffsets[]`). The
  Coverage table is re-used from `tables::gdef::Coverage` (the same
  shared common-layout primitive that GPOS, GDEF, and GSUB types
  1 / 4 / 5 / 6 / 8 read). Parse-time validates the spec's
  `sequenceCount == coverage.len()` invariant alongside the usual
  range checks on `coverageOffset` and `sequenceOffsets[]`.
- **`Sequence<'a>`** decodes the per-input
  `(glyphCount, substituteGlyphIDs[glyphCount])` payload. A
  `glyphCount` of zero is rejected as `BadStructure`: the spec
  explicitly prohibits using Multiple substitution as a deletion
  ("The glyphCount value must always be greater than 0").
- **`MultipleSubst::substitute(input: u16) -> Option<Sequence>`** is
  the shaper-path entrypoint. The input glyph is looked up in
  Coverage; the corresponding Sequence is returned as a zero-copy
  view over the on-disk `substituteGlyphIDs[]` bytes. `None` for an
  uncovered input or a malformed inner Sequence.
- **`MultipleSubst::iter()`** yields every `(input_glyph, Sequence)`
  pair in ascending Coverage order; **`Sequence::glyph(i)`** borrows
  the substitute glyph at output index `i`; and
  **`Sequence::glyphs()`** yields the full output sequence in order.
- **`GsubTable::multiple_subst(lookup_i, sub_i)`** is the convenience
  accessor that walks the lookup chain and confirms the lookup type
  is `GSUB_LOOKUP_TYPE_MULTIPLE` (= 2) before parsing. Returns
  `None` for out-of-range indices and `Some(Err(BadStructure))` when
  the referenced lookup is the wrong type. Mirrors the existing
  `single_subst(...)` / `ligature_subst(...)` accessors on the same
  type.

The `MultipleSubst`, `MultipleSubstIter`, `Sequence`, and
`SequenceGlyphIter` types are re-exported at the crate root. The
other GSUB lookup types (3 Alternate, 5 Contextual, 6
Chained-context, 7 Extension, 8 Reverse-chained-single) remain raw
byte slices via `Lookup::subtable_bytes(i)`.

Synthetic-byte unit tests cover the spec's worked Example 4
(ffi-ligature glyph `0x00F1` decomposed into `[f=0x1A, f=0x1A,
i=0x1D]`), Coverage iteration across two covered glyphs in ascending
order, every error path (`format != 1`, out-of-range
`coverageOffset`, `sequenceCount != coverage.len()`, truncated
`sequenceOffsets[]`, the spec-prohibited `glyphCount == 0`
deletion), and the out-of-range / wrong-type accessor returns. One
new integration test against the Source Sans 3 fixture walks every
type-2 lookup (the font ships two type-2 subtables — one ~407-
sequence mark-decomposition subtable plus a smaller secondary
subtable), decodes every `MultipleSubst` and every per-input
`Sequence`, verifies (a) Coverage iteration is ascending, (b) every
substitute glyph fits inside `maxp.numGlyphs`, (c) every
`glyphCount >= 1`, (d) the `glyph(k)` point-lookup agrees with the
`glyphs()` iterator, and (e) `substitute(input)` agrees with the
iter/sequence path.

## Round-248 additions (previous push)

GSUB **Lookup Type 4 (ligature substitution)** is now decoded as a
typed view, joining Type 1 (single substitution) from the previous
push. Spec: `docs/text/opentype/otspec-gsub.html` §"Lookup type 4
subtable: ligature substitution". One on-disk format is defined
(`LigatureSubstFormat1`); the typed view exposes every field down to
the per-Ligature `componentGlyphIDs[]` array.

- **`LigatureSubst<'a>`** decodes the subtable header (`format`,
  `coverageOffset`, `ligatureSetCount`, `ligatureSetOffsets[]`). The
  Coverage table is re-used from `tables::gdef::Coverage` (the same
  shared common-layout primitive that GPOS, GDEF, and GSUB types
  1 / 5 / 6 / 8 read).
- **`LigatureSet<'a>`** decodes the per-first-component
  `(ligatureCount, ligatureOffsets[])` pair. Ligature offsets are
  measured from the start of the LigatureSet, per spec; per-Ligature
  byte windows are validated when accessed.
- **`Ligature<'a>`** decodes `(ligatureGlyph, componentCount,
  componentGlyphIDs[componentCount - 1])`. The first component glyph
  is implicit — it's the Coverage entry that selected the
  LigatureSet — so the on-disk `componentGlyphIDs[]` array starts at
  the *second* component (input glyph sequence index = 1). A
  `componentCount` of zero is rejected at parse time (the spec count
  includes the first component; zero leaves the first-component
  invariant unsatisfiable).
- **`LigatureSubst::substitute(input: &[u16]) -> Option<(u16, u16)>`**
  is the shaper-path entrypoint. The first glyph of `input` is looked
  up in Coverage; the corresponding LigatureSet is walked in spec
  order (= preference order, "longer / preferred ligatures first");
  the first Ligature whose `componentGlyphIDs[..]` matches the input
  tail wins and the call returns `(ligatureGlyph, componentCount)` —
  the substitute glyph plus the total number of input glyphs the
  ligature consumed. `None` for: empty input, uncovered first glyph,
  or no matching Ligature in the selected set.
- **`LigatureSubst::iter()`** yields every `(coverage_glyph,
  LigatureSet)` pair in ascending Coverage order; **`LigatureSet
  ::ligature(i)`** borrows the Ligature at preference index `i`; and
  **`Ligature::component_glyphs()`** yields the tail glyphs (the
  second-and-beyond components) in input order.
- **`GsubTable::ligature_subst(lookup_i, sub_i)`** is the convenience
  accessor that walks the lookup chain and confirms the lookup type
  is `GSUB_LOOKUP_TYPE_LIGATURE` (= 4) before parsing. Returns
  `None` for out-of-range indices and `Some(Err(BadStructure))` when
  the referenced lookup is the wrong type. Mirrors the existing
  `single_subst(...)` accessor on the same type.

The `LigatureSubst`, `LigatureSubstIter`, `LigatureSet`, `Ligature`,
and `LigatureComponentIter` types are re-exported at the crate root.
The other GSUB lookup types (2 Multiple, 3 Alternate, 5 Contextual,
6 Chained-context, 7 Extension, 8 Reverse-chained-single) remain raw
byte slices via `Lookup::subtable_bytes(i)`.

Synthetic-byte unit tests cover the spec's worked Example 6
(Coverage = `{e, f}`, e-set = `[etc]`, f-set = `[ffi, fi]` with ffi
preferred), every error path (`format != 1`, out-of-range
`coverageOffset`, truncated `ligatureSetOffsets[]`, `componentCount
== 0`), the `substitute()` first-match preference rule, and the
out-of-range / wrong-type accessor returns. One new integration test
against the Source Sans 3 fixture walks every type-4 lookup, decodes
every LigatureSet and Ligature, verifies (a) Coverage iteration is
ascending, (b) every ligature glyph and every component glyph fits
inside `maxp.numGlyphs`, (c) `componentCount >= 1`, (d) the tail
iterator returns exactly `componentCount - 1` entries, and (e) the
first Ligature in each set round-trips through `substitute()` on its
own canonical input.

## Round-229 additions (previous push)

The OpenType **`GSUB` and `GPOS` headers** are now parsed and
surfaced on the public `Font` API, together with the shared chapter-2
common-layout primitives: `ScriptList`, `Script`, `LangSys`,
`FeatureList`, `Feature`, `LookupList`, `Lookup`, and `LookupFlag`.
Spec: `docs/text/opentype/otspec-gsub.html` and
`docs/text/opentype/otspec-gpos.html` (headers); the inner
ScriptList / FeatureList / LookupList / Lookup / LookupFlag formats
are pulled from
`docs/text/opentype/otspec-chapter2-common-layout-tables.html`.
Before this push the GSUB / GPOS tables were reachable only as raw
bytes via `Font::table_data(b"GSUB")` / `b"GPOS"`; the round-222
`GDEF` work landed the per-glyph metadata side of the shaper plumbing
but the per-feature / per-lookup half stayed opaque.

- **Both header versions decoded.** Version 1.0 (10 bytes,
  `majorVersion` + `minorVersion` + `scriptListOffset` +
  `featureListOffset` + `lookupListOffset`); version 1.1 (14 bytes,
  adds an `Offset32 featureVariationsOffset`). Unknown versions and
  truncation at the v1.1 trailer are rejected
  (`Error::BadStructure` and `Error::UnexpectedEof` respectively).
  Out-of-range header offsets are rejected at parse time.
- **`ScriptList`** decodes `scriptCount` plus the
  `scriptRecords[scriptCount]` (`(Tag[4], Offset16)`) array. The
  records are spec-sorted alphabetically by tag, so
  `find(tag)` binary-searches in `O(log n)`; `iter()` walks the
  list in on-disk order.
- **`Script`** decodes `defaultLangSysOffset` plus the
  `langSysRecords[langSysCount]` array, with binary-search
  `find_lang_sys(tag)` and a `default_lang_sys()` accessor for the
  common-case fallback.
- **`LangSys`** decodes the spec's three core fields:
  `lookupOrderOffset` (reserved — must be NULL),
  `requiredFeatureIndex` (with `NO_REQUIRED_FEATURE = 0xFFFF`
  surfaced as `None`), and the `featureIndices[featureIndexCount]`
  array that points at the parent FeatureList. Iteration is
  zero-copy.
- **`FeatureList`** decodes `featureCount` + `featureRecords[]`;
  `tag(i)` and `feature(i)` walk the array. The spec wording is
  "should be sorted by tag" rather than "must" (because a tag may
  legally appear more than once for distinct scripts / language
  systems), so look-up is linear via `iter()` rather than binary
  search.
- **`Feature`** decodes `featureParamsOffset` (surfaced as a raw
  `u16`; the `'cv01'–'cv99'` / `'ss01'–'ss20'` / `'size'`
  parameter formats are deferred) plus
  `lookupListIndices[lookupIndexCount]` — the indices into the
  parent LookupList that implement the feature.
- **`LookupList`** decodes `lookupCount` + `lookupOffsets[]`;
  `lookup(i)` parses the `Lookup` at the indexed offset and
  `iter()` walks every lookup in on-disk order.
- **`Lookup`** decodes `lookupType` + `lookupFlag` +
  `subTableCount` + `subtableOffsets[]`, plus the conditionally
  present `markFilteringSet` field (decoded iff
  `LookupFlag::USE_MARK_FILTERING_SET` is set, per the spec's
  variable-length Lookup rule). Per-subtable raw byte slices are
  exposed via `Lookup::subtable_bytes(i)`; the per-lookup-type
  subtable formats (GSUB types 1–8, GPOS types 1–9) are surfaced as
  raw bytes only, with decoding deferred to a future round.
- **`LookupFlag`** wraps the 16-bit flag word with the spec's bit
  vocabulary: `RIGHT_TO_LEFT` (`0x0001`, cursive-attachment-only),
  `IGNORE_BASE_GLYPHS` (`0x0002`), `IGNORE_LIGATURES` (`0x0004`),
  `IGNORE_MARKS` (`0x0008`), `USE_MARK_FILTERING_SET` (`0x0010`),
  and the high-byte `MARK_ATTACHMENT_CLASS_FILTER` mask (`0xFF00`).
  Each flag has a boolean accessor; the high byte is also exposed
  as `mark_attachment_type() -> u8`. The reserved-zero bits
  (`0x00E0`) are ignored on read, per the spec's "set to zero".
- **`Font` integration.** `Font::gsub()` and `Font::gpos()` borrow
  the parsed views; `Font::gsub_version()` and
  `Font::gpos_version()` return the header version pair without
  going through the full view. Absence of either table surfaces as
  `None` rather than rejecting the whole font (a glyph-only font
  with no substitution or positioning rules can legitimately omit
  both).

Re-exported at the crate root: `ScriptList`, `ScriptListIter`,
`Script`, `LangSys`, `NO_REQUIRED_FEATURE`, `FeatureList`,
`FeatureListIter`, `Feature`, `LookupList`, `LookupListIter`,
`Lookup`, `LookupFlag`. The GSUB / GPOS view types are also
re-exported as `GsubView` / `GposView` so callers that want a
top-level alias do not have to dive into `oxideav_otf::tables`.

Twelve new unit tests in `src/tables/layout.rs`,
`src/tables/gsub.rs`, and `src/tables/gpos.rs` cover header round
trips for both v1.0 and v1.1, rejection of unknown major / minor
versions, rejection of truncated trailers, the `LookupFlag` bit
helpers across the full mask space, a synthetic ScriptList +
Script + LangSys + FeatureList + Feature + LookupList + Lookup
byte tower, a minimal `GSUB` `liga` byte tower, a minimal `GPOS`
`kern` byte tower, and rejection of the
`USE_MARK_FILTERING_SET`-with-missing-trailer-word edge case. Six
new integration tests in `tests/source_sans.rs` exercise the
real-font path against Source Sans 3: the GSUB header parses as
v1.0 with no feature variations, the GSUB ScriptList exposes both
`DFLT` and `latn`, every parsed Feature record yields lookup-list
indices within the lookup count, the GPOS header parses as v1.0,
every GPOS lookup has a type in the spec's `1..=9` range with a
self-consistent `markFilteringSet` presence, and the `latn` GPOS
script resolves through to a default-LangSys with at least one
feature index but no required feature.

## Round-236 additions (this push)

The first **GSUB lookup-type subtable decoder** lands —
`GsubLookupType = 1`, **single substitution**, surfaced as the typed
[`SingleSubst`] view. Spec:
`docs/text/opentype/otspec-gsub.html` §"Lookup type 1 subtable:
single substitution". The chapter-2 Coverage table is re-used from
`tables::gdef::Coverage` (it is shared between GSUB, GPOS and the
rest of the layout tables per
`docs/text/opentype/otspec-chapter2-common-layout-tables.html`).

- **Format 1 (`SingleSubstFormat1`, 6 bytes)** decoded —
  `(format, coverageOffset, deltaGlyphID)` — with the spec's
  modular-arithmetic semantics on the output: "Addition of
  `deltaGlyphID` is modulo 65536" and "If the result after adding
  `deltaGlyphID` to the input glyph index is less than zero, add
  65536". Both are implemented via `rem_euclid(65536)` on an `i32`
  sum so the wrap-around is symmetrical.
- **Format 2 (`SingleSubstFormat2`)** decoded —
  `(format, coverageOffset, glyphCount, substituteGlyphIDs[])`.
  The spec's invariant "The `substituteGlyphIDs` array must
  contain the same number of glyph indices as the Coverage table"
  is enforced at `parse()` time: a `glyphCount`-vs-Coverage-length
  mismatch returns `Error::BadStructure`.
- **`SingleSubst::substitute(input)`** returns `Option<u16>`:
  `None` when the input glyph is not in the Coverage set, `Some`
  with the rewritten glyph otherwise. **`SingleSubst::iter()`**
  yields every `(input_glyph, output_glyph)` pair in ascending
  input-glyph order — convenient for offline subset / shape audits.
- **`GsubTable::single_subst(lookup_i, sub_i)`** is the convenience
  accessor: it walks the LookupList, asserts the lookup is declared
  as `GSUB_LOOKUP_TYPE_SINGLE`, slices the indexed subtable, and
  parses it. `None` is reserved for genuinely missing
  indices; a type-mismatch surfaces as `Some(Err(BadStructure))` so
  callers can distinguish the two failure shapes.
- **`GsubLookupType` constants exposed**: `GSUB_LOOKUP_TYPE_SINGLE`
  (1), `MULTIPLE` (2), `ALTERNATE` (3), `LIGATURE` (4), `CONTEXT`
  (5), `CHAINED_CONTEXT` (6), `EXTENSION` (7),
  `REVERSE_CHAINED_SINGLE` (8). Decoders for types 2..8 remain
  follow-up work — `Lookup::subtable_bytes(i)` continues to expose
  them as raw sub-slices.

Twelve new unit tests cover format-1 round-trip + positive- and
negative-delta modular wrap (input `5` + delta `-10` ↦ `65531`;
input `65530` + delta `+10` ↦ `4`), format-2 round-trip,
rejection of a `glyphCount` that disagrees with the Coverage,
rejection of an unknown subtable format, rejection of truncated
trailers, and one end-to-end synthetic that walks the GSUB
header → ScriptList → FeatureList → LookupList → Lookup → typed
`SingleSubst` chain. One Source Sans 3 integration test decodes
**every type-1 lookup in the font** — 57 lookups, split between
12 `SingleSubstFormat1` and 45 `SingleSubstFormat2` subtables —
verifying each Coverage iterator is strictly ascending, every
`(input, output)` pair stays within `maxp.numGlyphs`, the
iterator agrees with `substitute(input)` point lookups, and a
synthetic out-of-range glyph (numerically equal to `numGlyphs`)
correctly returns `None`.

Re-exported at the crate root: `SingleSubst`, `SingleSubstIter`,
and the eight `GSUB_LOOKUP_TYPE_*` constants.

## Round-222 additions (previous push)

The OpenType **`GDEF` Glyph Definition Table** is now parsed and
surfaced on the public `Font` API, along with the shared
**Coverage** and **ClassDef** common-layout primitives. Spec:
Microsoft / ISO/IEC 14496-22 `GDEF`
(`docs/text/opentype/otspec-gdef.html`) with the Coverage / ClassDef
formats pulled from
`docs/text/opentype/otspec-chapter2-common-layout-tables.html`.
Before this push the table was reachable only as raw bytes through
`Font::table_data(b"GDEF")`; GSUB / GPOS shaping (a future-round
target) could not consult `LookupFlag.ignoreMarks` /
`ignoreLigatures` / `markAttachmentType` / `useMarkFilteringSet`
without it.

- **All three header versions decoded.** Version 1.0 (12 bytes,
  `GlyphClassDef` + `AttachList` + `LigCaretList` +
  `MarkAttachClassDef`); version 1.2 (14 bytes, adds
  `MarkGlyphSetsDef`); version 1.3 (18 bytes, adds a `uint32`
  `itemVarStoreOffset`). The spec-undefined `minorVersion = 1` is
  rejected with `Error::BadStructure`; truncation at the v1.2 / v1.3
  trailers surfaces as `Error::UnexpectedEof`.
- **GlyphClassDef** routes through the generic [`ClassDef`] parser
  and returns the spec's four-class enumeration: `1 = Base`,
  `2 = Ligature`, `3 = Mark`, `4 = Component`. Glyphs not covered by
  the on-disk records implicitly belong to class 0 and surface as
  `None` from `Font::glyph_class(gid)`.
- **AttachList** decodes the `coverageOffset` + `glyphCount` +
  `attachPointOffsets[glyphCount]` header; per-glyph `AttachPoint`
  records expose the contour-point index array with `len()` /
  `get(i)` / `iter()`. Walked from a glyph ID via the embedded
  Coverage table.
- **LigCaretList** decodes the same Coverage-keyed shape plus the
  three `CaretValueFormat` variants: format 1 (`coordinate: i16` in
  design units), format 2 (`contourPointIndex: u16`), and format 3
  (`coordinate: i16` plus a `deviceOffset: u16` to a Device or
  VariationIndex table — the offset itself is surfaced raw; Device /
  VariationIndex decoding is deferred). `LigGlyph::caret_count()` /
  `caret_value(i)` walk the records in spec-sorted increasing-
  coordinate order.
- **MarkAttachClassDef** is the same `ClassDef` shape as
  `GlyphClassDef`; `Font::mark_attach_class(gid)` returns the class
  number (`0` for unclassified — the "unfiltered" semantics
  `LookupFlag.markAttachmentType` uses).
- **MarkGlyphSets** (v1.2+) decodes the `format = 1` + `setCount` +
  `coverageOffsets[setCount]` table; the spec's "uses Offset32, not
  Offset16" note for the offsets is honoured. `MarkGlyphSets::set(i)`
  returns a [`Coverage`] view; `contains(i, glyph_id)` is the
  per-glyph membership query that `LookupFlag.useMarkFilteringSet`
  needs.
- **ItemVariationStore** (v1.3+) is surfaced only as its raw
  `itemVarStoreOffset` byte offset
  (`GdefTable::item_var_store_offset()`). Decoding the store itself
  (the variation-data adjustment-delta records) is deferred to the
  same future round that lands variable-font CFF2 charstring
  decoding.
- **Coverage** common-layout table (chapter 2). Both spec formats
  decoded: format 1 (sorted `glyphArray[]`) and format 2
  (sorted `(start, end, startCoverageIndex)` ranges). Both
  `index_of(glyph_id)` paths binary-search the sorted on-disk
  records; format 2 then computes the dense Coverage Index using the
  spec's `startCoverageIndex + g - startGlyphID` formula. `iter()`
  walks every `(glyph_id, coverage_index)` pair in spec order;
  `contains(glyph_id)` is the common-case shortcut.
- **ClassDef** common-layout table (chapter 2). Format 1
  (`startGlyphID` + dense `classValues[glyphCount]` array) and
  format 2 (sorted `(start, end, class)` ranges) both decoded;
  `class_of(glyph_id)` returns the assigned class number or `0` for
  any glyph not covered (the spec's "implicit class 0" default).
- **`Font` integration.** [`Font::gdef`] borrows the parsed table;
  the lookup-free convenience routes [`Font::gdef_version`],
  [`Font::glyph_class`], and [`Font::mark_attach_class`] cover the
  common queries shapers actually run. Absence of the table surfaces
  as `None` rather than rejecting the whole font (a font with no
  GSUB / GPOS lookups can legitimately omit `GDEF`).

Re-exported at the crate root: `Coverage`, `CoverageIter`,
`ClassDef`, `GlyphClass`, `AttachList`, `AttachPoint`,
`LigCaretList`, `LigGlyph`, `CaretValue`, `MarkGlyphSets`.

Sixteen new unit tests in `src/tables/gdef.rs` cover Coverage
format 1 + 2 round-trips (lookup + iter), Coverage rejection paths
(unknown format, truncation), ClassDef format 1 default-zero outside
the dense range, ClassDef format 2 with the spec's Example-2
`GlyphClassDef` shape (one range per spec glyph class), ClassDef
rejection paths, full v1.0 / v1.2 / v1.3 header round-trips,
rejection of `majorVersion != 1` and the spec-undefined
`minorVersion = 1`, truncation rejection at the v1.2 + v1.3
trailers, the spec's Example-3 AttachList shape (two glyphs,
1 + 2 attach points), a hand-built LigCaretList exercising all
three `CaretValueFormat` variants, and the `GlyphClass::from_raw`
round-trip across `0..=5` plus the `0xFFFF` defence. Two new
integration tests in `tests/source_sans.rs` exercise the new
accessors against the real Source Sans 3 fixture: the font ships a
v1.0 GDEF with a format-2 GlyphClassDef + MarkAttachClassDef (no
AttachList / LigCaretList), every ASCII letter classifies as
`GlyphClass::Base`, every spec class is represented at least once
across the full 1900-glyph repertoire, and a synthetic Coverage
table round-trips through the public re-export.

## Round-217 additions (earlier)

The **Adobe Glyph List (AGL 2.0)** — the canonical PostScript
glyph-name to Unicode-scalar-value mapping — is now shipped in-tree
and exposed through a dedicated `agl` module plus two new `Font`
accessors. Source: `data/agl-glyphlist.txt`, a verbatim copy of the
AGL 2.0 table (September 20, 2002) staged under
`docs/text/opentype/spec/agl-glyphlist.txt`; the format is described
in the companion `agl-aglfn-README.md`. Before this push the
crate-internal `cff::strings::glyph_name_to_codepoint` stub returned
`None` for every input; round-1 deferred AGL on the grounds that the
sfnt `cmap` always wins for codepoint→GID. Round 217 fills the
opposite direction: callers that have a **PostScript glyph name** in
hand (PDF content streams, `post`-format-2.0 Pascal-string tails,
TeX font-encoding files) can now route it back to a glyph id without
implementing their own AGL.

- **`agl` module** (re-exported at the crate root via `oxideav_otf::agl`):
  - `name_to_codepoints(name) -> Option<Codepoints<'static>>` — full
    AGL lookup. `Codepoints::Single(char)` for the 4200
    single-codepoint entries; `Codepoints::Sequence(&[char])` for
    the 81 multi-codepoint entries (mostly Hebrew base + vowel-
    pointing combinations like `dalethatafpatah → [U+05D3, U+05B2]`).
  - `name_to_codepoint(name) -> Option<char>` — common-case helper
    that returns `Some` only for single-codepoint entries.
  - `codepoint_to_name(cp) -> Option<&'static str>` — reverse lookup
    keyed on a single Unicode scalar value. When multiple AGL aliases
    share a codepoint (most common case: ~17 Hebrew names aliasing
    U+05B8 HEBREW POINT QAMATS, plus the `Acutesmall` /
    `acutesmall` PUA pairs), the alphabetically-first name in
    AGL's on-disk order is returned. Multi-codepoint sequence
    entries do not participate in the reverse path (a single `char`
    can't disambiguate them).
  - `entries() -> impl Iterator<Item = (&'static str, Codepoints<'static>)>`
    — iterates every AGL pair in on-disk ASCII-sorted-by-name order.
  - `entry_count() -> usize` — `4281` for AGL 2.0
    (4200 single-codepoint + 81 sequence entries).
  - `distinct_codepoint_count() -> usize` — `3680` distinct
    codepoints reachable via the reverse path (the gap between
    4200 single-codepoint entries and 3680 distinct codepoints is
    the legacy alias families).
- **`Font::glyph_id_from_agl_name(name) -> Option<u16>`** — two-step
  resolver: look up `name` in AGL, then look up the resulting
  codepoint in the font's `cmap`. The right tool for callers who
  have a PostScript glyph name and need a glyph id without first
  decoding it themselves. The AGL Specification's §6 component-name
  decomposition algorithm (`f_f_i` → `ffi`, `uniXXXX` → `U+XXXX`)
  is **not** applied because that document is not staged under
  `docs/text/opentype/` — only the raw AGL 2.0 table and its
  `aglfn-README.md` companion are.
- **`Font::agl_glyph_name(gid) -> Option<&str>`** — canonical AGL
  name for a glyph, with a three-step resolution order tuned for
  the "use the font's own knowledge first" convention: (1) the CFF
  charset → Strings name (the font's authored PostScript name);
  (2) the `post` table version-2.0 Pascal-string tail (UTF-8-clean);
  (3) the AGL reverse table, keyed on whichever BMP codepoint the
  font's `cmap` routes to this glyph. CFF1 fonts almost always
  surface from step 1; the `post` fallback is for CFF2 / TrueType-
  outline mixed cases.

A new `cff::strings::glyph_name_to_codepoint` body — previously a
`None`-returning stub kept alive only so `encoding.rs` compiled —
now delegates to `agl::name_to_codepoint`. No API surface changed,
but the legacy Standard-Encoding fallback hook is now functional
for the first time.

Sixteen new unit tests in `src/agl.rs` cover: AGL 2.0 entry count
landmarks (4281 total / 4200 single-codepoint / 3680 distinct
reachable codepoints / 81 sequence entries); full ASCII letter and
digit round-trip; common-punctuation PostScript-name landmarks;
PUA small-cap landmarks (`Acutesmall = U+F7B4`,
`Asmall = U+F761`, `AEsmall = U+F7E6`); BMP-ligature spec-worked
entries (`AE`, `ae`, `OE`, `oe`, `ffi`); the canonical
multi-codepoint Hebrew example (`dalethatafpatah`); CJK kana spot
checks; the "alphabetically-first alias wins" reverse-lookup
property on U+05B8 with 17 sharing names; ASCII-alphanumeric
defence on every parsed glyph name; surrogate / astral-plane
absence in AGL 2.0; the `Codepoints` accessor helpers; and the
"sequence entries don't participate in reverse lookup" invariant.
Four new integration tests in `tests/source_sans.rs` exercise the
new `Font` accessors against the Source Sans 3 fixture: ASCII-letter
round trip via `glyph_id_from_agl_name`; the "CFF charset wins over
AGL fallback" priority on every alphabetic glyph; missing-name
rejection; and out-of-range glyph rejection on `agl_glyph_name`.

## Round-211 additions (previous push)

CFF2 (Compact Font Format Version 2 — OpenType 1.9.1 `CFF2` table,
spec `docs/text/opentype/otspec-cff2.html`) is now parsed for its
header, Top DICT, and structural INDEXes. Before this push the parser
rejected every CFF2 font outright with `Error::Cff2NotImplemented`;
post-push the entire metadata surface (`head` / `hhea` / `cmap` /
`name` / `OS/2` / `post`) plus a structural CFF2 view is reachable,
and only `Font::glyph_outline` still returns `Cff2NotImplemented`
(the Type 2 + `blend` + `vsindex` interpreter for variable-font
outlines stays deferred).

- **CFF2 header (§6 Table 8, "headerFormat")** decoded into a new
  `Cff2Header { major, minor, header_size, top_dict_size }` (5 bytes,
  `uint16` `topDICTSize` instead of CFF1's `Card8 offSize`). The
  parser honours the spec's "headerSize must be used when locating
  the Top DICT" rule (the field exists so future versions can grow
  the header), rejects `major != 2`, rejects `header_size < 5`, and
  rejects a declared header that exceeds the table buffer. The
  derived `top_dict_offset()` (== `header_size`) and
  `global_subr_index_offset()` (== `header_size + top_dict_size`,
  per spec §6) are exposed for callers walking the table.
- **CFF2 INDEX format (§6 "INDEX data")** decoded into a new
  `Cff2Index<'a>` type whose `count` field is `uint32` (vs. CFF1's
  `Card16`) and whose empty-INDEX sentinel is the 4-byte `(count=0)`
  form (vs. CFF1's 2-byte `Card16(0)`). All four spec-allowed
  `offsetSize` values (1, 2, 3, 4 = Offset8 / 16 / 24 / 32) are
  supported; `entry(i)` returns zero-copy slices. The truncation,
  out-of-range, and `offsetSize ∉ 1..=4` error paths are
  rejected with `Error::Cff(...)` and `Error::UnexpectedEof`.
- **CFF2 Top DICT (§7)** parsed into `Cff2TopDict` with all five
  spec-permitted operators:
  - `CharStringINDEXOffset` (0x11, required) — offset to the
    CharStringINDEX from the CFF2 table start.
  - `VariationStoreOffset` (0x18, required iff the font has
    variations) — offset to the OpenType `ItemVariationStore`.
  - `FontDICTINDEXOffset` (0x0c24, required) — offset to the
    FontDICT INDEX.
  - `FontDICTSelectOffset` (0x0c25, optional) — present only when
    the font has more than one FontDICT.
  - `FontMatrix` (0x0c07, optional) — the spec-restricted
    `[s 0 0 s 0 0]` form (`a == d`, all other entries zero) is
    enforced; the default `0.001 0 0 0.001 0 0` (`DEFAULT_FONT_MATRIX`,
    re-exported at the crate root) is substituted when the operator
    is absent (i.e. for the `unitsPerEm == 1000` case the spec
    recommends).
  The two required operators are rejected at parse time when
  missing; non-uniform / translated FontMatrix shapes are rejected
  with descriptive `Error::Cff(...)` strings; unknown operators are
  silently skipped (CFF1-style tolerance).
- **`Cff2` struct** drives the four-step walk `Header → Top DICT →
  GlobalSubrINDEX → CharStringINDEX → FontDICTINDEX`. The required
  "non-empty FontDICTINDEX" invariant (§7.2) is enforced.
  Accessors: `header()`, `top_dict()`, `glyph_count()`,
  `font_dict_count()`, `global_subr_count()`, `is_variable()`,
  `charstring(gid)` (raw bytes for later decoding),
  `font_dict(i)`, `global_subr(i)`, `bytes()`.
- **`Font` integration.** New accessors on the public `Font` API:
  - `is_cff2()` — true for CFF2 fonts.
  - `cff2() -> Option<&Cff2>` — borrows the parsed CFF2 view; `None`
    for CFF1 fonts (the existing `cff()` is now `Option<&Cff>` for
    symmetry, `None` for CFF2 fonts).
  - `cff2_header()` / `cff2_top_dict()` — convenience views.
  - `is_variable()` — true when the CFF2 Top DICT carries a
    `VariationStoreOffset` operator.
  - `cff_fd_count()` now routes through CFF2's FontDICTINDEX count
    on CFF2 fonts (in addition to the CFF1 FDArray count behaviour).
- **CFF1-only accessors return spec defaults on CFF2 fonts** instead
  of panicking or producing garbage. `font_bbox()` returns
  `[0; 4]`; `italic_angle()` returns `0.0`; `underline_position()` /
  `underline_thickness()` return CFF's `-100` / `50` spec defaults;
  `paint_type()`, `charstring_type()`, `stroke_width()` return their
  CFF spec defaults; `weight_name()` / `notice()` / `copyright()` /
  `version_string()` / `postscript()` / `base_font_name()` /
  `glyph_name(gid)` / `ps_name()` / `cid_registry()` /
  `cid_ordering()` / `cid_supplement()` / `unique_id()` /
  `synthetic_base()` all return `None`; `xuid()` / `base_font_blend()`
  return empty slices. The doc on each accessor calls out the CFF2
  fallback explicitly so callers know which alternative tables
  (`name` table for textual identity, `post` for italic/underline
  metrics) to consult.
- **CFF1 charstring expansion still benefits.** As a side effect of
  letting the shared DICT parser handle CFF2 byte `0x18`
  (VariationStoreOffset operator), the operator-byte range is now
  `0..=21 ∪ {24}` instead of `0..=21`. The CFF1 spec leaves bytes
  22, 23, 25–27 reserved (per TN5176 §4 Table 3); a CFF1 font using
  any of those was already malformed and stays so.
- **`font_matrix()` on `Font`** now reads from the CFF2 Top DICT
  for CFF2 fonts (vs. previously returning a stale CFF1 default).

Sixteen new unit tests in `src/cff2/{header.rs, index.rs,
top_dict.rs, mod.rs}` cover: header parse + format-detect, header
rejection paths (wrong major, header_size < 5, truncated buffer);
CFF2 INDEX parse with `offSize` of 1 / 2 / 3 / 4, empty CFF2 INDEX
= 4 bytes, truncation rejection, out-of-range `offSize`; Top DICT
parse with each operator, missing-required rejection,
non-uniform/translated FontMatrix rejection, negative offset
rejection, unknown-operator tolerance; full `Cff2::parse` of a
minimal hand-assembled table, empty-FontDICTINDEX rejection,
variable-font detection, truncated Top DICT and out-of-range
CharString offset rejection. Four new integration tests in
`tests/cff2_synthetic.rs` build a complete synthetic
OpenType/CFF2 font (`head` / `hhea` / `cmap` / `hmtx` / `maxp` /
`name` / `CFF2`) and exercise every new `Font` accessor end-to-end.

## Round-204 additions (previous push)

The OpenType **`name` table** is now version-1 aware, with full
language-tag record support and the complete spec-defined name-ID
catalogue surfaced as a typed enum. Spec: Microsoft / ISO/IEC
14496-22 `name` (`docs/text/opentype/otspec-name.html`). Previously
the parser accepted version-1 tables but silently ignored the
`langTagCount` / `langTagRecord[]` trailer; version-1-only language
IDs `>= 0x8000` would have been treated as platform-specific numeric
IDs and surfaced as garbage.

- **Version 1 trailer parsed.** A version-1 `name` table now decodes
  the `uint16 langTagCount` + `LangTagRecord[langTagCount]` block per
  the spec's "Naming table version 1" layout. `LangTagRecord` is the
  spec's `(length, langTagOffset)` pair pointing into the storage
  area. The parser rejects a v1 table whose declared `storageOffset`
  overlaps the LangTagRecord array (`Error::BadStructure(
  "name.storageOffset overlaps langTagRecord array")`) and a v1 table
  that is too short to carry either the `langTagCount` field or the
  declared array (`Error::UnexpectedEof`).
- **`NameTable::lang_tag(language_id)`** resolves a name record's
  `languageID >= 0x8000` to its UTF-16BE BCP 47 language-tag string.
  Per the spec's worked example, a font with `langTagRecord[0] =
  "en"` and `langTagRecord[1] = "zh-Hant-HK"` maps language ID
  `0x8000 → "en"` and `0x8001 → "zh-Hant-HK"`. IDs outside `[0x8000,
  0x8000 + langTagCount)` return `None` per spec ("the identity of
  the language is unknown; such name records should not be used");
  IDs `< 0x8000` are platform-specific numeric LCIDs (not language
  tags) and also return `None`. Version-0 tables always return
  `None`.
- **`NameId` enum** for the 26 spec-defined name IDs 0..=25 with
  per-variant documentation (Copyright, FontFamily, FontSubfamily,
  UniqueId, FullName, Version, PostScript, Trademark, Manufacturer,
  Designer, Description, VendorUrl, DesignerUrl, License, LicenseUrl,
  Reserved15, TypographicFamily, TypographicSubfamily, CompatibleFull,
  SampleText, PostScriptCidFindfont, WwsFamily, WwsSubfamily,
  LightBackgroundPalette, DarkBackgroundPalette, VariationsPsNamePrefix).
  `NameId::Reserved15` is included as a distinct variant so a font
  that emits a record with the spec-reserved ID 15 is still
  representable. `NameId::from_raw(u16) -> Option<Self>` decodes a raw
  ID into the typed enum; `to_raw(self) -> u16` is the inverse.
  Re-exported at the crate root.
- **`NameRecord` struct** + **`NameTable::records()`** iterator over
  every on-disk record in spec-sorted (platform, encoding, language,
  nameID) order. Each `NameRecord` carries the raw 6-tuple
  `(platform_id, encoding_id, language_id, name_id_raw, length,
  string_offset)`; `name_id()` returns the standard `NameId` when the
  raw value is `0..=25`; `record_value(rec)` decodes the on-disk bytes
  into an owned `String` using the same platform / encoding rules as
  `find()`. Re-exported at the crate root.
- **`NameTable::get(NameId)`** typed alternative to `find(u16)`.
- **`NameTable::version()` / `record_count()` / `lang_tag_count()`**
  surface the on-disk header fields.
- **UTF-16BE decoder hardening.** The shared decoder now rejects
  unpaired *low* surrogates (the existing code already rejected
  unpaired high surrogates) — both are malformed UTF-16 and per
  TN5176 §H.4 should be rejected rather than silently mojibake'd.

New on `Font` (all consult the parsed `name` table; absence of the
relevant record surfaces as `None`): `name()`, `name_version()`,
`name_lang_tag(id)`, `name_string(NameId)`, `designer()`,
`manufacturer()`, `description()`, `vendor_url()`, `designer_url()`,
`license()`, `license_url()`, `trademark()`, `sample_text()`,
`typographic_family()`, `typographic_subfamily()`, `wws_family()`,
`wws_subfamily()`, `variations_ps_name_prefix()`, `unique_font_id()`.
The last is distinct from the CFF-Top-DICT-sourced `Font::unique_id()`
(which is a 32-bit integer); `unique_font_id()` is the name-ID-3
human-readable string.

Sixteen new unit tests in `src/tables/name.rs` cover: the v0
baseline preserved by `find()`; rejection of version > 1; the
`NameId` ↔ raw round-trip across the entire 0..=25 range plus the
"reserved 15 is still distinct" property; v1 parsing with two
language-tag records (the spec-worked `en` / `zh-Hant-HK` example);
`lang_tag` resolution including the spec's "should not be used"
out-of-range case + numeric (`< 0x8000`) rejection; v0 always-`None`
behaviour for `lang_tag`; records iteration with on-disk-order
guarantees; truncation rejection at the langTagCount field and
inside the langTagRecord array; storage-overlap rejection;
past-end string-offset rejection; truncated record-array
rejection; UTF-16BE surrogate-pair acceptance (`U+1F600`); unpaired
low-surrogate rejection; Mac Roman ASCII subset; and the existing
Windows-beats-Mac priority. One new integration test against the
Source Sans 3 fixture asserts every newly-surfaced `Font` accessor
returns the expected string (or `None` where the font omits a
record), iterates the records and confirms spec sort order, and
exercises the v0 `lang_tag` invariant.

## Round-198 additions (previous push)

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

## Out of scope (round 218+)

- CFF2 Type 2 + `blend` + `vsindex` charstring decoder (OpenType 1.9.1
  CFF2 spec §9). The CFF2 header, Top DICT, GlobalSubrINDEX,
  CharStringINDEX, and FontDICTINDEX are now parsed (round 211);
  `Font::glyph_outline` on a CFF2 font still returns
  `Error::Cff2NotImplemented` until the variation-aware charstring
  interpreter and the `ItemVariationStore` region-blend resolver
  land.
- Hint enforcement (we anti-alias at >= 16 px, so hints are noise).
- The AGL Specification §6 component-name decomposition algorithm
  (`f_f_i` → `ffi`, `uniXXXX` → `U+XXXX`, `uXXXXX` → astral
  scalar values, etc.). Round 217 ships the static AGL 2.0 table
  but not the §6 algorithm — the AGL Specification document itself
  is not staged under `docs/text/opentype/`; only the raw glyph-list
  table and its `aglfn-README.md` companion are. Once the spec is
  staged, `agl::name_to_codepoints` can absorb the algorithm
  without an API change.
- `GSUB`, `GPOS`, `kern` tables — the Adobe CFF / Type 2 / sfnt
  PDFs are now staged under `docs/text/opentype/spec/` alongside the
  Microsoft per-table HTML snapshots (`otspec-gsub.html` /
  `otspec-gpos.html`), so future rounds can pick these up; round 187
  took the `post` table off this list, round 198 took the `OS/2`
  table off it, and round 222 took the `GDEF` table off it.
  `GDEF.itemVarStore` decoding (the variation-data delta store
  shared with GPOS / JSTF) is still deferred; only the raw offset
  is surfaced.
- Format-1.0 / 2.0 / 2.5 glyph-name lookups in `post` (the
  standard-Macintosh 258-entry list referenced from
  `otspec-post.html`). The list lives in Apple's TrueType Reference
  Manual chapter 6 and is not currently staged in
  `docs/text/opentype/`; only the manual's table-of-contents page is
  there. The non-standard Pascal-string half is fully resolvable
  through `post_glyph_name` and now also via `agl_glyph_name`'s
  step-3 AGL fallback.

## Test fixture

`tests/fixtures/SourceSans3-Regular.otf` is Adobe Source Sans 3
Regular under the SIL Open Font License v1.1 (see
`tests/fixtures/SOURCE-SANS-LICENSE`). 335 KB, ~1900 glyphs,
exercises every common Type 2 operator including flex.

## License

MIT — see [`LICENSE`](LICENSE).
