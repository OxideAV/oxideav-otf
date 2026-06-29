//! Integration test for the variable-font metadata tables (`fvar` +
//! `avar`) wired through the public `Font` API.
//!
//! We don't ship a variable-font binary fixture; instead this builds a
//! synthetic OTF carrying the sfnt required tables plus a tiny `CFF `
//! table and hand-assembled `fvar` / `avar` tables, then drives the
//! public `Font::from_bytes` -> `variation_axes` / `named_instances` /
//! `normalize_coords` surface.
//!
//! Spec: ISO/IEC 14496-22:2019 §7.3.3 (fvar), §7.3.1 (avar);
//! sfnt directory per `docs/text/opentype/otspec-otff.html`.

use oxideav_otf::Font;

fn fixed(v: f32) -> [u8; 4] {
    ((v * 65536.0) as i32).to_be_bytes()
}

fn f2dot14(v: f32) -> [u8; 2] {
    ((v * 16384.0).round() as i16).to_be_bytes()
}

/// fvar with two axes (wght 100/400/900, wdth 75/100/125) and two named
/// instances (one with a PS name ID).
fn build_fvar() -> Vec<u8> {
    let mut b = Vec::new();
    b.extend_from_slice(&1u16.to_be_bytes()); // majorVersion
    b.extend_from_slice(&0u16.to_be_bytes()); // minorVersion
    b.extend_from_slice(&16u16.to_be_bytes()); // axesArrayOffset
    b.extend_from_slice(&2u16.to_be_bytes()); // reserved
    b.extend_from_slice(&2u16.to_be_bytes()); // axisCount
    b.extend_from_slice(&20u16.to_be_bytes()); // axisSize
    b.extend_from_slice(&2u16.to_be_bytes()); // instanceCount
    b.extend_from_slice(&14u16.to_be_bytes()); // instanceSize = 2*4 + 4 + 2
                                               // axis 0: wght
    b.extend_from_slice(b"wght");
    b.extend_from_slice(&fixed(100.0));
    b.extend_from_slice(&fixed(400.0));
    b.extend_from_slice(&fixed(900.0));
    b.extend_from_slice(&0u16.to_be_bytes()); // flags
    b.extend_from_slice(&256u16.to_be_bytes()); // axisNameID
                                                // axis 1: wdth
    b.extend_from_slice(b"wdth");
    b.extend_from_slice(&fixed(75.0));
    b.extend_from_slice(&fixed(100.0));
    b.extend_from_slice(&fixed(125.0));
    b.extend_from_slice(&0u16.to_be_bytes());
    b.extend_from_slice(&257u16.to_be_bytes());
    // instance 0: Regular (400, 100), PS name 300
    b.extend_from_slice(&17u16.to_be_bytes()); // subfamilyNameID
    b.extend_from_slice(&0u16.to_be_bytes()); // flags
    b.extend_from_slice(&fixed(400.0));
    b.extend_from_slice(&fixed(100.0));
    b.extend_from_slice(&300u16.to_be_bytes()); // postScriptNameID
                                                // instance 1: Bold (700, 100), PS name = none (0xFFFF)
    b.extend_from_slice(&258u16.to_be_bytes());
    b.extend_from_slice(&0u16.to_be_bytes());
    b.extend_from_slice(&fixed(700.0));
    b.extend_from_slice(&fixed(100.0));
    b.extend_from_slice(&0xFFFFu16.to_be_bytes());
    b
}

/// avar that warps the wght axis (spec §7.3.1.4 example) and leaves
/// wdth as the identity (empty segment map).
fn build_avar() -> Vec<u8> {
    let mut b = Vec::new();
    b.extend_from_slice(&1u16.to_be_bytes()); // majorVersion
    b.extend_from_slice(&0u16.to_be_bytes()); // minorVersion
    b.extend_from_slice(&0u16.to_be_bytes()); // reserved
    b.extend_from_slice(&2u16.to_be_bytes()); // axisCount
                                              // axis 0 segment map (the spec example).
    let maps: &[(f32, f32)] = &[
        (-1.0, -1.0),
        (-0.75, -0.5),
        (0.0, 0.0),
        (0.4, 0.4),
        (0.6, 0.9),
        (1.0, 1.0),
    ];
    b.extend_from_slice(&(maps.len() as u16).to_be_bytes());
    for (from, to) in maps {
        b.extend_from_slice(&f2dot14(*from));
        b.extend_from_slice(&f2dot14(*to));
    }
    // axis 1 segment map: empty (identity).
    b.extend_from_slice(&0u16.to_be_bytes());
    b
}

/// Minimal static `CFF ` table with one empty glyph, enough for
/// `Cff::parse` to succeed. Mirrors the crate's own minimal CFF
/// fixtures: header + Name INDEX + Top DICT INDEX + String INDEX +
/// Global Subr INDEX + CharStrings INDEX + Private DICT + charset.
fn build_minimal_cff() -> Vec<u8> {
    // Layout offsets are computed as we go.
    let mut v = Vec::new();
    // Header: major=1, minor=0, hdrSize=4, offSize=1.
    v.extend_from_slice(&[1, 0, 4, 1]);
    // Name INDEX: 1 name "A".
    v.extend_from_slice(&[0, 1]); // count
    v.push(1); // offSize
    v.extend_from_slice(&[1, 2]); // offsets
    v.push(b'A');
    // Top DICT INDEX: 1 dict. We need CharStrings (op 17) and charset
    // (op 15) and Private (op 18) offsets — fill them after we know the
    // total layout. To keep this simple we point CharStrings at a
    // known offset and provide a 1-glyph CharStrings INDEX.
    // We build the Top DICT body referencing absolute offsets, so we
    // assemble the trailing structures first to learn their offsets.

    // Reserve: compute offsets relative to start of CFF table.
    // Strategy: place CharStrings INDEX, Private DICT, and charset after
    // the Top DICT INDEX + String INDEX + Global Subr INDEX. We hand-
    // compute by constructing tail blobs, then size the Top DICT.

    // --- tail blobs ---
    // CharStrings INDEX: 1 entry = single byte 0x0E (endchar).
    let mut charstrings = Vec::new();
    charstrings.extend_from_slice(&[0, 1]); // count
    charstrings.push(1); // offSize
    charstrings.extend_from_slice(&[1, 2]); // offsets
    charstrings.push(0x0E); // endchar

    // Private DICT: empty (size 0). The Top DICT Private operator is
    // `size offset 18`; size 0 means no Private DICT body.
    // charset: format 0 with 0 entries (only .notdef, implicit).
    let charset = vec![0u8]; // format 0, no SID entries (nGlyphs-1 = 0)

    // We need: Top DICT INDEX, String INDEX (empty), Global Subr INDEX
    // (empty), then charstrings, charset.
    // Build the empty INDEXes.
    let string_index = vec![0u8, 0u8]; // count 0
    let gsubr_index = vec![0u8, 0u8]; // count 0

    // Compute where the Top DICT INDEX starts: right after Name INDEX.
    let name_index_end = v.len();
    // We'll build the Top DICT body with placeholder integer operands
    // encoded as 5-byte (29 + i32) so the size is fixed regardless of
    // value, making offset math stable.
    // Operators we emit: CharStrings (17), charset (15), Private (18).
    // Private takes two operands (size, offset).
    // Each integer operand is 5 bytes (0x1D + 4 bytes).
    // Top DICT body length:
    //   charstrings: 5 (operand) + 1 (op 17)              = 6
    //   charset:     5 (operand) + 1 (op 15)              = 6
    //   private:     5 + 5 (two operands) + 1 (op 18)     = 11
    // total = 23
    let top_dict_len = 23usize;
    // Top DICT INDEX wrapper: count(2)+offSize(1)+offsets(2*2)+data.
    let top_dict_index_len = 2 + 1 + 4 + top_dict_len;

    let top_dict_index_start = name_index_end;
    let string_index_start = top_dict_index_start + top_dict_index_len;
    let gsubr_index_start = string_index_start + string_index.len();
    let charstrings_start = gsubr_index_start + gsubr_index.len();
    let charset_start = charstrings_start + charstrings.len();
    let private_start = charset_start + charset.len();
    let private_size = 0usize;

    // Emit Top DICT INDEX.
    v.extend_from_slice(&[0, 1]); // count
    v.push(2); // offSize = 2
    v.extend_from_slice(&1u16.to_be_bytes()); // offset[0] = 1
    v.extend_from_slice(&((top_dict_len + 1) as u16).to_be_bytes()); // offset[1]
                                                                     // Top DICT body.
    let int5 = |val: i32| {
        let mut o = vec![0x1Du8];
        o.extend_from_slice(&val.to_be_bytes());
        o
    };
    v.extend_from_slice(&int5(charstrings_start as i32));
    v.push(17); // CharStrings
    v.extend_from_slice(&int5(charset_start as i32));
    v.push(15); // charset
    v.extend_from_slice(&int5(private_size as i32));
    v.extend_from_slice(&int5(private_start as i32));
    v.push(18); // Private

    // String INDEX (empty), Global Subr INDEX (empty), CharStrings,
    // charset, (no Private body since size 0).
    v.extend_from_slice(&string_index);
    v.extend_from_slice(&gsubr_index);
    v.extend_from_slice(&charstrings);
    v.extend_from_slice(&charset);

    v
}

fn build_font(with_avar: bool) -> Vec<u8> {
    // head (54 bytes), unitsPerEm 1000, magic, bbox.
    let mut head = vec![0u8; 54];
    head[0..4].copy_from_slice(&0x00010000u32.to_be_bytes());
    head[12..16].copy_from_slice(&0x5F0F3CF5u32.to_be_bytes());
    head[18..20].copy_from_slice(&1000u16.to_be_bytes());
    head[50..52].copy_from_slice(&2i16.to_be_bytes()); // indexToLocFormat (unused)

    // hhea: ascent/descent + numberOfHMetrics = 1.
    let mut hhea = vec![0u8; 36];
    hhea[0..4].copy_from_slice(&0x00010000u32.to_be_bytes());
    hhea[4..6].copy_from_slice(&800i16.to_be_bytes());
    hhea[6..8].copy_from_slice(&(-200i16).to_be_bytes());
    hhea[34..36].copy_from_slice(&1u16.to_be_bytes());

    // maxp v0.5 (CFF) — version 0x00005000, numGlyphs = 1.
    let mut maxp = vec![0u8; 6];
    maxp[0..4].copy_from_slice(&0x00005000u32.to_be_bytes());
    maxp[4..6].copy_from_slice(&1u16.to_be_bytes());

    // hmtx: 1 long metric.
    let mut hmtx = vec![0u8; 4];
    hmtx[0..2].copy_from_slice(&500u16.to_be_bytes());

    // cmap: format-0 subtable (all → 0).
    let mut cmap = Vec::new();
    cmap.extend_from_slice(&0u16.to_be_bytes());
    cmap.extend_from_slice(&1u16.to_be_bytes());
    cmap.extend_from_slice(&0u16.to_be_bytes());
    cmap.extend_from_slice(&0u16.to_be_bytes());
    cmap.extend_from_slice(&12u32.to_be_bytes());
    cmap.extend_from_slice(&0u16.to_be_bytes()); // format
    cmap.extend_from_slice(&262u16.to_be_bytes()); // length
    cmap.extend_from_slice(&0u16.to_be_bytes()); // language
    cmap.extend(std::iter::repeat(0u8).take(256));

    // name: empty.
    let mut name = vec![0u8; 6];
    name[4..6].copy_from_slice(&6u16.to_be_bytes());

    let cff = build_minimal_cff();
    let fvar = build_fvar();

    let mut tables: Vec<(&[u8; 4], Vec<u8>)> = vec![
        (b"CFF ", cff),
        (b"cmap", cmap),
        (b"fvar", fvar),
        (b"head", head),
        (b"hhea", hhea),
        (b"hmtx", hmtx),
        (b"maxp", maxp),
        (b"name", name),
    ];
    if with_avar {
        tables.push((b"avar", build_avar()));
    }
    tables.sort_by(|a, b| a.0.cmp(b.0));

    let n = tables.len() as u16;
    let header_size = 12 + 16 * n as usize;
    let mut data_offsets: Vec<usize> = Vec::with_capacity(tables.len());
    let mut cursor = header_size;
    for (_t, payload) in &tables {
        data_offsets.push(cursor);
        cursor += payload.len();
        while cursor % 4 != 0 {
            cursor += 1;
        }
    }

    let mut out = Vec::with_capacity(cursor);
    out.extend_from_slice(&0x4F54544Fu32.to_be_bytes()); // OTTO
    out.extend_from_slice(&n.to_be_bytes());
    out.extend_from_slice(&0u16.to_be_bytes());
    out.extend_from_slice(&0u16.to_be_bytes());
    out.extend_from_slice(&0u16.to_be_bytes());
    for (i, (tag, payload)) in tables.iter().enumerate() {
        out.extend_from_slice(*tag);
        out.extend_from_slice(&0u32.to_be_bytes());
        out.extend_from_slice(&(data_offsets[i] as u32).to_be_bytes());
        out.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    }
    for (i, (_t, payload)) in tables.iter().enumerate() {
        while out.len() < data_offsets[i] {
            out.push(0);
        }
        out.extend_from_slice(payload);
    }
    out
}

#[test]
fn fvar_axes_and_instances_surface() {
    let bytes = build_font(false);
    let f = Font::from_bytes(&bytes).expect("font with fvar parses");
    assert!(f.has_variation_axes());
    assert_eq!(f.axis_count(), 2);

    let axes = f.variation_axes();
    assert_eq!(&axes[0].tag, b"wght");
    assert_eq!(axes[0].min, 100.0);
    assert_eq!(axes[0].default, 400.0);
    assert_eq!(axes[0].max, 900.0);
    assert_eq!(&axes[1].tag, b"wdth");

    let inst = f.named_instances();
    assert_eq!(inst.len(), 2);
    assert_eq!(inst[0].subfamily_name_id, 17);
    assert_eq!(inst[0].postscript_name_id, Some(300));
    assert_eq!(inst[1].subfamily_name_id, 258);
    assert_eq!(inst[1].postscript_name_id, None); // 0xFFFF sentinel
    assert_eq!(inst[1].coordinates, vec![700.0, 100.0]);
}

#[test]
fn normalize_without_avar_is_default_normalization() {
    let bytes = build_font(false);
    let f = Font::from_bytes(&bytes).expect("parses");
    assert!(f.avar().is_none());
    // wght 700: (700-400)/(900-400) = 0.6; wdth default → 0.0.
    let n = f.normalize_coords(&[700.0, 100.0]);
    assert!((n[0] - 0.6).abs() < 1e-5);
    assert!((n[1] - 0.0).abs() < 1e-5);
    // extremes.
    assert_eq!(f.normalize_coords(&[100.0, 75.0]), vec![-1.0, -1.0]);
    assert_eq!(f.normalize_coords(&[900.0, 125.0]), vec![1.0, 1.0]);
}

#[test]
fn normalize_with_avar_applies_segment_map() {
    let bytes = build_font(true);
    let f = Font::from_bytes(&bytes).expect("parses");
    assert!(f.avar().is_some());
    // wght 700 default-normalizes to 0.6, which the avar example maps
    // to 0.9 (record (0.6, 0.9)). wdth identity → 0.0.
    let n = f.normalize_coords(&[700.0, 100.0]);
    assert!((n[0] - 0.9).abs() < 0.01, "got {}", n[0]);
    assert!((n[1] - 0.0).abs() < 1e-5);
    // A wght that default-normalizes to 0.5 maps to 0.65 per the example.
    // 0.5 = (w-400)/500 → w = 650.
    let n2 = f.normalize_coords(&[650.0, 100.0]);
    assert!((n2[0] - 0.65).abs() < 0.01, "got {}", n2[0]);
}
