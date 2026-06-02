//! Integration test for CFF2 (OpenType 1.9.1 variation-aware CFF
//! flavour).
//!
//! We don't ship a CFF2 font fixture (would be a binary artefact under
//! redistribution-ambiguous licensing) — instead this test builds a
//! synthetic-byte CFF2-flavoured OpenType font from scratch with the
//! six required tables (`head`, `hhea`, `maxp`, `cmap`, `hmtx`,
//! `name`) plus the `CFF2` table, parses it through the public
//! `Font::from_bytes` API, and exercises every public CFF2 accessor.
//!
//! Spec: `docs/text/opentype/otspec-cff2.html` for the CFF2 table,
//! `docs/text/opentype/otspec-otff.html` for the sfnt directory.

use oxideav_otf::{Error, Font};

/// Build a synthetic OpenType/CFF2 font with the minimum required
/// sfnt tables + a 1-glyph CFF2 table (no variations, no global
/// subrs). The output passes `Font::from_bytes` and exposes a CFF2
/// header + Top DICT through the public API.
fn build_minimal_cff2_font() -> Vec<u8> {
    // --- Construct each table's payload first, then assemble the
    //     sfnt header + directory once we know their sizes. -----

    // head: 54 bytes per Microsoft head spec. We don't use any of
    // the values downstream beyond `unitsPerEm`, so most fields are
    // zero. `magicNumber = 0x5F0F3CF5` and `unitsPerEm = 1000` are
    // the only meaningful bits.
    let mut head = vec![0u8; 54];
    head[0..4].copy_from_slice(&0x00010000u32.to_be_bytes()); // majorVersion / minorVersion
    head[4..8].copy_from_slice(&0u32.to_be_bytes()); // fontRevision
    head[8..12].copy_from_slice(&0u32.to_be_bytes()); // checkSumAdjustment
    head[12..16].copy_from_slice(&0x5F0F3CF5u32.to_be_bytes()); // magicNumber
    head[16..18].copy_from_slice(&0u16.to_be_bytes()); // flags
    head[18..20].copy_from_slice(&1000u16.to_be_bytes()); // unitsPerEm
                                                          // 20..28 created, 28..36 modified — leave zero.
    head[36..38].copy_from_slice(&0i16.to_be_bytes()); // xMin
    head[38..40].copy_from_slice(&0i16.to_be_bytes()); // yMin
    head[40..42].copy_from_slice(&1000i16.to_be_bytes()); // xMax
    head[42..44].copy_from_slice(&1000i16.to_be_bytes()); // yMax
                                                          // 44..46 macStyle, 46..48 lowestRecPPEM, 48..50 fontDirectionHint.
    head[50..52].copy_from_slice(&0i16.to_be_bytes()); // indexToLocFormat
    head[52..54].copy_from_slice(&0i16.to_be_bytes()); // glyphDataFormat

    // hhea: 36 bytes per Microsoft hhea spec.
    let mut hhea = vec![0u8; 36];
    hhea[0..4].copy_from_slice(&0x00010000u32.to_be_bytes());
    hhea[4..6].copy_from_slice(&800i16.to_be_bytes()); // ascender
    hhea[6..8].copy_from_slice(&(-200i16).to_be_bytes()); // descender
    hhea[8..10].copy_from_slice(&100i16.to_be_bytes()); // lineGap
    hhea[10..12].copy_from_slice(&1000u16.to_be_bytes()); // advanceWidthMax
                                                          // 12..32 zero
    hhea[34..36].copy_from_slice(&1u16.to_be_bytes()); // numberOfHMetrics

    // maxp version 0.5 (the CFF flavour — 6 bytes).
    let mut maxp = vec![0u8; 6];
    maxp[0..4].copy_from_slice(&0x00005000u32.to_be_bytes());
    maxp[4..6].copy_from_slice(&1u16.to_be_bytes()); // numGlyphs

    // hmtx: 1 longHorMetric = (advanceWidth: u16, lsb: i16) = 4 bytes.
    let mut hmtx = vec![0u8; 4];
    hmtx[0..2].copy_from_slice(&500u16.to_be_bytes());
    hmtx[2..4].copy_from_slice(&0i16.to_be_bytes());

    // cmap: minimal format-0 subtable (262 bytes) inside a 4-byte
    // header + 8-byte encoding record.
    let cmap = build_minimal_cmap();

    // name: empty v0 table (6-byte header, 0 records).
    let mut name = vec![0u8; 6];
    name[0..2].copy_from_slice(&0u16.to_be_bytes()); // version
    name[2..4].copy_from_slice(&0u16.to_be_bytes()); // count
    name[4..6].copy_from_slice(&6u16.to_be_bytes()); // storageOffset

    // CFF2 table.
    let cff2 = build_minimal_cff2_table();

    // --- Assemble sfnt directory + tables -------------------------
    // Tables are addressed by 4-byte tag; the directory must be sorted
    // ascending by tag per the spec.
    let mut tables: Vec<(&[u8; 4], Vec<u8>)> = vec![
        (b"CFF2", cff2),
        (b"cmap", cmap),
        (b"head", head),
        (b"hhea", hhea),
        (b"hmtx", hmtx),
        (b"maxp", maxp),
        (b"name", name),
    ];
    tables.sort_by(|a, b| a.0.cmp(b.0));

    let n = tables.len() as u16;
    let header_size = 12 + 16 * n as usize;
    let mut data_offsets: Vec<usize> = Vec::with_capacity(tables.len());
    let mut cursor = header_size;
    for (_tag, payload) in &tables {
        data_offsets.push(cursor);
        cursor += payload.len();
        // sfnt table-record offsets must be 4-byte aligned per spec;
        // we pad with zeros.
        while cursor % 4 != 0 {
            cursor += 1;
        }
    }

    let mut out = Vec::with_capacity(cursor);
    // sfnt header: OTTO + numTables + binary-search hints (we leave
    // searchRange/entrySelector/rangeShift = 0; the parser doesn't
    // validate them).
    out.extend_from_slice(&0x4F54544Fu32.to_be_bytes());
    out.extend_from_slice(&n.to_be_bytes());
    out.extend_from_slice(&0u16.to_be_bytes()); // searchRange
    out.extend_from_slice(&0u16.to_be_bytes()); // entrySelector
    out.extend_from_slice(&0u16.to_be_bytes()); // rangeShift

    // Directory entries.
    for (i, (tag, payload)) in tables.iter().enumerate() {
        out.extend_from_slice(*tag);
        out.extend_from_slice(&0u32.to_be_bytes()); // checksum (ignored)
        out.extend_from_slice(&(data_offsets[i] as u32).to_be_bytes());
        out.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    }

    // Table payloads, padded to 4-byte boundaries.
    for (i, (_tag, payload)) in tables.iter().enumerate() {
        while out.len() < data_offsets[i] {
            out.push(0);
        }
        out.extend_from_slice(payload);
    }
    out
}

/// Build a minimal cmap with one Format-0 subtable mapping every
/// byte codepoint to GID 0 except `A` → GID 0 (the parser doesn't
/// care which characters map; we just need the table to parse).
fn build_minimal_cmap() -> Vec<u8> {
    let mut t = Vec::new();
    // Header: version=0, numTables=1.
    t.extend_from_slice(&0u16.to_be_bytes());
    t.extend_from_slice(&1u16.to_be_bytes());
    // Encoding record: platformID=0 (Unicode), encodingID=0
    // (default), offset=12 (after header + 1 record).
    t.extend_from_slice(&0u16.to_be_bytes());
    t.extend_from_slice(&0u16.to_be_bytes());
    t.extend_from_slice(&12u32.to_be_bytes());
    // Format 0 subtable: format=0, length=262, language=0, then
    // 256-byte glyphIdArray (all zero).
    t.extend_from_slice(&0u16.to_be_bytes()); // format
    t.extend_from_slice(&262u16.to_be_bytes()); // length
    t.extend_from_slice(&0u16.to_be_bytes()); // language
    t.extend(std::iter::repeat(0u8).take(256));
    t
}

/// Build the minimal CFF2 table that `Cff2::parse` will accept.
#[allow(clippy::vec_init_then_push)]
fn build_minimal_cff2_table() -> Vec<u8> {
    // CharStringINDEXOffset = 14, FontDICTINDEXOffset = 22. Layout
    // identical to the unit test in `crate::cff2::tests` —
    // synthesised here so the integration test doesn't depend on
    // private test helpers.
    let cs_off = 14u32;
    let fd_off = 22u32;
    let mut v = Vec::new();
    // Header (5 bytes).
    v.push(2); // major
    v.push(0); // minor
    v.push(5); // headerSize
    v.push(0); // topDICTSize hi
    v.push(5); // topDICTSize lo

    // Top DICT (5 bytes): two `(operand, operator)` pairs.
    v.push((cs_off + 139) as u8); // operand cs_off
    v.push(17); // CharStringINDEXOffset
    v.push((fd_off + 139) as u8); // operand fd_off
    v.extend_from_slice(&[12, 36]); // FontDICTINDEXOffset

    // GlobalSubrINDEX (empty, 4 bytes).
    v.extend_from_slice(&[0, 0, 0, 0]);

    assert_eq!(v.len(), cs_off as usize);

    // CharStringINDEX: 1 entry, payload single byte 0x0E
    // (Type 2 charstring `endchar`) — we don't decode it this round,
    // it just needs to be addressable.
    v.extend_from_slice(&[0, 0, 0, 1]); // count
    v.push(1); // offSize
    v.extend_from_slice(&[1, 2]); // offsets
    v.push(0x0E); // endchar

    assert_eq!(v.len(), fd_off as usize);

    // FontDICTINDEX: 1 entry, single byte 0xFF (we don't yet decode
    // Font DICT contents).
    v.extend_from_slice(&[0, 0, 0, 1]);
    v.push(1);
    v.extend_from_slice(&[1, 2]);
    v.push(0xFF);

    v
}

#[test]
fn parses_synthetic_cff2_font() {
    let bytes = build_minimal_cff2_font();
    let f = Font::from_bytes(&bytes).expect("CFF2 font parses");

    assert!(f.is_cff2(), "Font::is_cff2 should be true for CFF2 fonts");
    assert!(!f.is_variable(), "no VariationStoreOffset in this fixture");
    assert!(!f.is_cid(), "synthetic font is not CID-keyed");

    // Header surface.
    let hdr = f.cff2_header().expect("cff2 header");
    assert_eq!(hdr.major, 2);
    assert_eq!(hdr.minor, 0);
    assert_eq!(hdr.header_size, 5);
    assert_eq!(hdr.top_dict_size, 5);
    assert_eq!(hdr.top_dict_offset(), 5);
    // GlobalSubrINDEX immediately follows the Top DICT, so its
    // offset is `headerSize + topDICTSize = 10`.
    assert_eq!(hdr.global_subr_index_offset(), 10);

    // Top DICT surface.
    let td = f.cff2_top_dict().expect("cff2 top dict");
    assert_eq!(td.charstring_index_offset, 14);
    assert_eq!(td.font_dict_index_offset, 22);
    assert!(td.font_dict_select_offset.is_none());
    assert!(td.variation_store_offset.is_none());
    // FontMatrix is absent on disk, so the spec default applies.
    assert!(!td.has_font_matrix);
    assert_eq!(td.font_matrix, [0.001, 0.0, 0.0, 0.001, 0.0, 0.0]);

    // `font_matrix` / `units_per_em` / `glyph_count` accessors
    // route through the CFF2 view (FontMatrix default) and the
    // sfnt-level `head` / `maxp`.
    assert_eq!(f.units_per_em(), 1000);
    assert_eq!(f.glyph_count(), 1);
    assert_eq!(f.font_matrix(), [0.001, 0.0, 0.0, 0.001, 0.0, 0.0]);
}

#[test]
fn glyph_outline_returns_cff2_not_implemented() {
    let bytes = build_minimal_cff2_font();
    let f = Font::from_bytes(&bytes).expect("CFF2 font parses");
    let err = f
        .glyph_outline(0)
        .expect_err("CFF2 outline decode is deferred");
    assert!(matches!(err, Error::Cff2NotImplemented));
}

#[test]
fn cff1_accessors_return_defaults_on_cff2() {
    let bytes = build_minimal_cff2_font();
    let f = Font::from_bytes(&bytes).expect("CFF2 font parses");

    // None of the CFF1-only accessors should surface non-defaults.
    assert_eq!(f.font_bbox(), [0.0; 4]);
    assert_eq!(f.italic_angle(), 0.0);
    assert_eq!(f.underline_position(), -100.0);
    assert_eq!(f.underline_thickness(), 50.0);
    assert!(!f.is_fixed_pitch());
    assert_eq!(f.paint_type(), 0);
    assert_eq!(f.charstring_type(), 2);
    assert_eq!(f.stroke_width(), 0.0);
    assert!(f.weight_name().is_none());
    assert!(f.notice().is_none());
    assert!(f.copyright().is_none());
    assert!(f.version_string().is_none());
    assert!(f.postscript().is_none());
    assert!(f.base_font_name().is_none());
    assert!(f.unique_id().is_none());
    assert!(f.xuid().is_empty());
    assert!(f.synthetic_base().is_none());
    assert!(f.base_font_blend().is_empty());
    assert!(f.cff().is_none(), "Font::cff returns None on CFF2");
    assert!(f.ps_name().is_none(), "CFF2 has no Name INDEX");
    assert!(f.glyph_name(0).is_none(), "CFF2 has no charset/strings");
    assert!(f.cid_registry().is_none());
    assert!(f.cid_ordering().is_none());
    assert!(f.cid_supplement().is_none());
    // FontDICTINDEX count is exposed through `cff_fd_count`.
    assert_eq!(f.cff_fd_count(), 1);
}

#[test]
fn cff2_view_exposes_raw_charstrings_and_font_dicts() {
    let bytes = build_minimal_cff2_font();
    let f = Font::from_bytes(&bytes).expect("CFF2 font parses");
    let c = f.cff2().expect("cff2 view");

    assert_eq!(c.glyph_count(), 1);
    assert_eq!(c.font_dict_count(), 1);
    assert_eq!(c.global_subr_count(), 0);
    // CharString bytes are reachable for inspection even though the
    // full decoder is deferred.
    assert_eq!(c.charstring(0).unwrap(), &[0x0Eu8][..]);
    assert_eq!(c.font_dict(0).unwrap(), &[0xFFu8][..]);
    // Out-of-range queries surface as Error::Cff(...).
    assert!(c.charstring(1).is_err());
    assert!(c.font_dict(1).is_err());
}
