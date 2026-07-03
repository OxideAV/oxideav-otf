# oxideav-otf

[![CI](https://github.com/OxideAV/oxideav-otf/actions/workflows/ci.yml/badge.svg)](https://github.com/OxideAV/oxideav-otf/actions/workflows/ci.yml) [![crates.io](https://img.shields.io/crates/v/oxideav-otf.svg)](https://crates.io/crates/oxideav-otf) [![docs.rs](https://docs.rs/oxideav-otf/badge.svg)](https://docs.rs/oxideav-otf) [![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

Pure-Rust OpenType / CFF font parser for the
[oxideav](https://github.com/OxideAV) framework. Sibling to
[`oxideav-ttf`](https://github.com/OxideAV/oxideav-ttf): TTF handles
TrueType outlines (quadratic Beziers); OTF handles CFF outlines
(Type 2 charstrings → cubic Beziers).

## Capabilities

The crate parses an sfnt/OTF container into a `Font` and exposes
metadata, glyph metrics, glyph outlines (CFF Type 2 → cubic Beziers),
and typed views over the OpenType Layout tables. Highlights:

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
  `vhea` (v1.0 + v1.1), `vmtx`, `VORG` (CFF vertical origin), `cmap`
  (formats 0/4/6/12), `name`, `post` (every spec version), `OS/2`
  (versions 0..5, all six layouts), and the legacy `kern` table (OFF
  version-0; subtable formats 0 + 2).

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

// GPOS Lookup Type 1 — single adjustment positioning. The typed view
// decodes the ValueRecord/ValueFormat primitive and answers
// `value(glyph)` for each covered glyph (format 1 = one shared record;
// format 2 = a per-glyph array).
if let Some(g) = font.gpos() {
    for i in 0..g.lookup_count() {
        let l = g.lookup(i).unwrap();
        if l.lookup_type() != oxideav_otf::GPOS_LOOKUP_TYPE_SINGLE {
            continue;
        }
        for s in 0..l.subtable_count() {
            let sp = g.single_pos(i, s).unwrap()?;
            let _ = sp.format();              // 1 or 2
            let _ = sp.value_format().bits();  // which fields are present
            for (glyph, rec_res) in sp.iter() {
                let rec = rec_res?;
                // Apply as a shaper would: shift placement + advance.
                let _ = (glyph, rec.x_placement, rec.x_advance);
            }
        }
    }

    // GPOS Lookup Type 2 — pair adjustment positioning (kerning). The
    // typed view decodes both formats: format 1 (per-glyph PairSet
    // records) and format 2 (a class-pair matrix). `pair(first, second)`
    // returns the `PairValue { first, second }` adjustment for an ordered
    // glyph pair; `class_pair(c1, c2)` probes the format-2 matrix; and
    // `iter()` enumerates every `(first, second, PairValue)` of a
    // format-1 subtable.
    for i in 0..g.lookup_count() {
        let l = g.lookup(i).unwrap();
        if l.lookup_type() != oxideav_otf::GPOS_LOOKUP_TYPE_PAIR {
            continue;
        }
        for s in 0..l.subtable_count() {
            let pp = g.pair_pos(i, s).unwrap()?;
            let _ = pp.format();               // 1 or 2
            let _ = pp.value_format1().bits();  // first-glyph fields
            let _ = pp.value_format2().bits();  // second-glyph fields
            // Apply as a shaper would: adjust the cursor between a glyph
            // pair by the looked-up PairValue.
            if let Some(res) = pp.pair(/* first */ 0u16, /* second */ 0u16) {
                let pv = res?;
                let _ = (pv.first.x_advance, pv.second.x_advance);
            }
            // Format-1 subtables also enumerate their explicit pairs.
            for (first, second, val_res) in pp.iter() {
                let pv = val_res?;
                let _ = (first, second, pv.first.x_advance);
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

## OpenType Layout and advanced tables

Beyond the core CFF parsing and glyph outlines, the crate provides typed
views over:

- **`GDEF`** — glyph-class and mark-attach-class lookups
  (`glyph_class` / `mark_attach_class`); `itemVarStore` is surfaced as a
  raw offset only.
- **`GSUB`** — script / feature / lookup-list enumeration plus typed
  decoders for **every** lookup type: single (type 1), multiple (type 2),
  alternate (type 3), ligature (type 4), contextual (type 5), chained
  contextual (type 6), the extension (type 7) wrapper, and
  reverse-chaining contextual single substitution (type 8). Type 8
  (`ReverseChainSingleSubst`) decodes the input Coverage →
  `substituteGlyphIDs` mapping plus the backtrack / lookahead Coverage
  sequences that gate the match; `substitute(glyph)` answers the output
  glyph and `backtrack_coverage` / `lookahead_coverage` expose the
  context (it is reachable through `GsubTable::reverse_chain_single_subst`
  and the type-7 `ExtensionSubst::as_reverse_chain_single_subst`).
  Contextual (type 5) and chained contextual (type 6) reuse the same
  `tables::context` `SequenceContext` / `ChainedSequenceContext` decoders
  shared with GPOS types 7/8.
- **`GPOS`** — the same header enumeration plus typed decoders for
  single adjustment (type 1), pair adjustment (type 2), cursive
  attachment (type 3), mark-to-base attachment (type 4), mark-to-ligature
  attachment (type 5), mark-to-mark attachment (type 6), contextual
  positioning (type 7), chained contextual positioning (type 8), and the
  extension (type 9) lookups — every defined GPOS lookup type now has a
  typed decoder.
  Mark-to-base decodes the shared `Anchor` (formats 1/2/3) and
  MarkArray/MarkRecord primitives and answers `attachment(mark, base)`
  with the `(mark_anchor, base_anchor)` pair a shaper aligns to position
  a combining mark over its base glyph. Mark-to-mark reuses those same
  primitives (the `Mark2Array` mirrors the `BaseArray`) and answers
  `attachment(mark1, mark2)` with the `(mark1_anchor, mark2_anchor)` pair
  a shaper aligns to stack one combining mark over a preceding mark.
  Mark-to-ligature reuses the same MarkArray/`Anchor` primitives over a
  two-dimensional base side (the `LigatureArray` → `LigatureAttach` →
  per-component `ComponentRecord`); `attachment(mark, ligature,
  component)` selects the base anchor by both the mark's class and the
  caller-supplied ligature-component index, returning the
  `(mark_anchor, ligature_anchor)` pair a shaper aligns to position a
  mark over a specific ligature component.
  Cursive attachment decodes the
  CursivePosFormat1 EntryExit records (reusing the `Anchor` primitive)
  and answers `attachment(first, second)` with the `(exit_anchor,
  entry_anchor)` pair a shaper aligns to join adjacent cursive glyphs;
  either anchor of an EntryExit record may be NULL.
  Contextual (type 7) and chained contextual (type 8) positioning decode
  the shared `SequenceContext` / `ChainedSequenceContext` subtables
  (formats 1 glyph-based, 2 class-based, 3 coverage-based) from
  `tables::context`; these are identical on disk to the GSUB type-5/6
  contextual subtables. Each match surfaces its input (and, for chained,
  backtrack/lookahead) pattern plus the nested-lookup `SequenceLookupRecord`
  actions (`sequenceIndex` + `lookupListIndex`) a shaper applies — the
  pattern structure is decoded; nested-lookup resolution and glyph-buffer
  mutation remain the shaping client's responsibility.
- **Device / VariationIndex tables** — the per-ppem (non-variable) /
  per-instance (variable) correction tables referenced by GPOS
  `ValueRecord` `*DeviceOffset` fields, Anchor format-3
  `x`/`yDeviceOffset`, and GDEF CaretValue format-3 `deviceOffset`. A
  `Device` table (deltaFormat 1 / 2 / 3) decodes its packed 2- / 4- /
  8-bit signed deltas — `delta(ppem)` answers the signed pixel
  correction for a size — and a `VariationIndex` table (deltaFormat
  0x8000) surfaces its `(deltaSetOuterIndex, deltaSetInnerIndex)`
  delta-set index pair into the GDEF/BASE `ItemVariationStore`.
  Reachable through `Anchor::x_device` / `y_device`,
  `ValueRecord::{x,y}_{placement,advance}_device`, and
  `CaretValue::device`.
- **`cmap`** formats 0 / 2 / 4 / 6 / 12 / 13 / 14 (format 2 = the legacy
  high-byte mixed 8-/16-bit CJK mapping; format 13 = the "last resort"
  many-to-one constant-glyph ranges, ranked below every real-coverage
  format; format 14 = Unicode Variation Sequences,
  retained alongside the base subtable and queried via
  `Font::glyph_index_variation(base, selector)` /
  `Font::variation_sequences()` — a non-default UVS yields its explicit
  glyph, a default UVS resolves through the base cmap), `name`, `post`
  (every version, including the full 258-entry standard-Macintosh
  glyph-name set, so formats 1.0 / 2.0 / 2.5 resolve names end-to-end),
  and `OS/2` versions 0–5.
- **AGL** — the static Adobe Glyph List 2.0 table for glyph-name
  resolution (`agl_glyph_name`).
- **CFF2** (variable-font CFF, OpenType 1.9.1) — header, Top DICT,
  Global Subr / CharString / Font DICT INDEXes, the per-FontDICT
  PrivateDICT (default `vsindex` + LocalSubrINDEX), the FontDICTSelect
  (formats 0 / 3 / 4), and the `ItemVariationStore` are parsed, and a
  variation-aware Type 2 CharString interpreter decodes glyph outlines:
  it runs the path / hint / subroutine operators plus the two CFF2
  variation operators `vsindex` (select the active region list) and
  `blend` (interpolate `n` defaults with `n*k` deltas against `k`
  region scalars). CFF2's no-`endchar` termination, absent glyph-width
  prefix, and absent arithmetic operators are all handled.
  `Font::glyph_outline` decodes the **default variation instance**;
  `Font::glyph_outline_var(gid, &region_scalars)` decodes a specific
  instance from caller-supplied per-region scalars.

## Font variations (`fvar` / `avar`) and region-scalar derivation

The crate now decodes the variable-font axis-definition tables and ties
them to the CFF2 variation interpreter, so a caller can request a glyph
outline directly from **user-scale axis coordinates** (e.g. `wght = 700`):

- **`fvar`** (ISO/IEC 14496-22:2019 §7.3.3) — the design-space axes
  (tag / min / default / max / flags / `name` ID) and the named
  instances (subfamily + optional PostScript name ID, with the `0xFFFF`
  no-PS-name sentinel handled). Surfaced via `Font::fvar`,
  `variation_axes`, `named_instances`, `axis_count`,
  `has_variation_axes`. `VariationAxis::normalize` implements the
  §7.3.1.1 default normalization (default→0, min→-1, max→1, linear,
  clamped).
- **`avar`** (§7.3.1) — per-axis piecewise-linear segment maps that
  refine the default normalization (§7.3.1.3 processing; verified
  against the §7.3.1.4 worked example). Surfaced via `Font::avar`.
- **Region-scalar derivation** (§7.1.7) — `VariationRegion::scalar`
  computes a region's interpolation scalar from a normalized instance
  tuple (the product of per-axis triangular scalars, with the spec's
  three "ignore this axis" cases), and `ItemVariationStore::region_scalars`
  produces the per-region scalar vector for an `ItemVariationData`
  subtable. Validated against the §7.1.8 Skia two-axis example
  (instance `(0.2, 0.7)` → R1 `0.2`, R2 `0.7`, R3 `0.14`).
- **Delta-set `ItemVariationStore`** (§7.2.3) — the variation-data
  structure carrying the `itemCount × regionIndexCount` delta matrix
  (the metrics/positioning IVS, distinct from the CFF2 delta-free one);
  `delta(outer, inner, &normalized)` resolves a delta-set to a
  per-instance adjustment (`tables::ivs`).
- **`MVAR`** (§7.3.6) — metrics variations: value-tag → delta-set
  records + the IVS, resolving font-wide metric adjustments.
  `Font::metric_variation(tag, &user_coords)` (e.g. `b"hasc"` →
  `OS/2.sTypoAscender` delta).
- **`HVAR` / `VVAR`** (§7.3.5, §7.3.8) — per-glyph horizontal/vertical
  metrics variations via the IVS + `DeltaSetIndexMap` (advance, side
  bearings, and `VVAR`'s vertical origin; implicit glyph-ID index when
  no advance map is present). `Font::advance_width_variation` /
  `advance_height_variation(gid, &user_coords)`.
- **`STAT`** (§7.3.7) — the style attributes table: design-axis records
  (tag / name ID / ordering) and all four axis-value-table formats
  (1 single value, 2 value + range, 3 value + style-link, 4 multi-axis
  combination), with the older-sibling / elidable flags. Surfaced via
  `Font::stat` / `Font::stat_version`.
- **`BASE`** (§6.3) — the baseline table: per-axis baseline tag lists +
  per-script `BaseValues` resolving BaseCoord (formats 1/2/3) values.
  `Font::base` / `Font::baseline_coord(axis, script_tag, baseline_tag)`.
- **End-to-end glue** — `Font::normalize_coords(&user_coords)` runs the
  full `fvar` → `avar` pipeline, and `Font::glyph_outline_for_axes(gid,
  &user_coords)` chains normalization → region scalars → the CFF2
  variation-aware charstring interpreter, closing the gap that
  previously made region-scalar derivation the caller's job.

## Out of scope

- `GDEF.itemVarStore` is surfaced as a raw offset only (the metrics-/
  positioning-variation `ItemVariationStore` carrying its own delta
  sets — distinct from the CFF2 delta-free IVS — is not yet decoded).
- Hint enforcement (we anti-alias at >= 16 px, so hints are noise).
- The AGL Specification §6 component-name decomposition algorithm
  (`f_f_i` → `ffi`, `uniXXXX` → `U+XXXX`, etc.) — the static AGL 2.0
  table ships, but the §6 algorithm is not implemented (the spec
  document is not staged). `agl::name_to_codepoints` can absorb it
  without an API change once the spec is available.
- Every GSUB lookup type (1–8) now has a typed decoder, including
  reverse-chaining contextual single substitution (type 8,
  `ReverseChainSingleSubst`: input Coverage → `substituteGlyphIDs` with
  backtrack / lookahead Coverage context, reachable through
  `GsubTable::reverse_chain_single_subst` and the type-7 extension
  `ExtensionSubst::as_reverse_chain_single_subst`). The legacy `kern`
  table is now decoded separately (see the sfnt-metadata list above);
  modern fonts express kerning through GPOS pair adjustment. (Historical
  note: no `kern` chapter was staged under
  `docs/text/opentype/`).
- (none for `post` — the 258-entry standard-Macintosh glyph-name set is
  now staged and applied: `PostTable::glyph_name` resolves formats 1.0
  (glyph ID → standard name), 2.0 (`glyphNameIndex < 258` → standard,
  `>= 258` → custom Pascal string), and 2.5 (`glyph_id + offset` →
  standard); `Font::post_glyph_name` and the AGL fallback consume it.)

## Test fixture

`tests/fixtures/SourceSans3-Regular.otf` is Adobe Source Sans 3
Regular under the SIL Open Font License v1.1 (see
`tests/fixtures/SOURCE-SANS-LICENSE`). 335 KB, ~1900 glyphs,
exercises every common Type 2 operator including flex.

## License

MIT — see [`LICENSE`](LICENSE).
