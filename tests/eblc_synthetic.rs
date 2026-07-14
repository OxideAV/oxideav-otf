//! Synthetic byte-level tests for the `EBLC` / `CBLC` bitmap location
//! tables (ISO/IEC 14496-22:2019 §5.6.3 / §5.6.6): the BitmapSize
//! strike records with their SbitLineMetrics, the IndexSubTableArray
//! range walk, and all five IndexSubTable formats.

use oxideav_otf::tables::eblc::BitmapLocationTable;
use oxideav_otf::BITMAP_FLAG_HORIZONTAL_METRICS;

fn u16b(v: u16) -> [u8; 2] {
    v.to_be_bytes()
}
fn u32b(v: u32) -> [u8; 4] {
    v.to_be_bytes()
}

/// Shared BigGlyphMetrics used by index formats 2 / 5:
/// 7x5 bitmap, hbx 1, hby 6, hadv 6, vbx -2, vby -1, vadv 8.
const BIG_METRICS: [u8; 8] = [7, 5, 1, 6, 6, 0xFE, 0xFF, 8];

/// An SbitLineMetrics blob (10 metric bytes + 2 pads).
fn line_metrics(ascender: i8, descender: i8) -> [u8; 12] {
    let mut b = [0u8; 12];
    b[0] = ascender as u8;
    b[1] = descender as u8;
    b[2] = 9; // widthMax
    b[3] = 1; // caretSlopeNumerator
    b[4] = 1; // caretSlopeDenominator
    b[5] = 0; // caretOffset
    b[6] = 0x7F; // minOriginSB
    b[7] = 0x7F; // minAdvanceSB
    b[8] = ascender as u8; // maxBeforeBL
    b[9] = descender as u8; // minAfterBL
    b
}

/// One strike (BitmapSize) + its IndexSubTableArray + subtables.
///
/// `ranges` = `(first, last, subtable bytes)`.
fn build_table(major: u16, ranges: &[(u16, u16, Vec<u8>)], ppem: u8, bit_depth: u8) -> Vec<u8> {
    const HDR: usize = 8;
    const SIZE_LEN: usize = 48;
    let ista_at = HDR + SIZE_LEN; // one strike only
    let ista_len = ranges.len() * 8;

    // Layout subtables after the array.
    let mut sub_offsets = Vec::new();
    let mut cursor = ista_len;
    for (_, _, sub) in ranges {
        sub_offsets.push(cursor);
        cursor += sub.len();
    }
    let index_tables_size = cursor as u32;

    let mut b = Vec::new();
    b.extend_from_slice(&u16b(major)); // majorVersion
    b.extend_from_slice(&u16b(0)); // minorVersion
    b.extend_from_slice(&u32b(1)); // numSizes
                                   // BitmapSize.
    b.extend_from_slice(&u32b(ista_at as u32));
    b.extend_from_slice(&u32b(index_tables_size));
    b.extend_from_slice(&u32b(ranges.len() as u32));
    b.extend_from_slice(&u32b(0)); // colorRef
    b.extend_from_slice(&line_metrics(12, -4)); // hori
    b.extend_from_slice(&line_metrics(6, -6)); // vert
    let first = ranges.iter().map(|r| r.0).min().unwrap_or(0);
    let last = ranges.iter().map(|r| r.1).max().unwrap_or(0);
    b.extend_from_slice(&u16b(first));
    b.extend_from_slice(&u16b(last));
    b.push(ppem); // ppemX
    b.push(ppem); // ppemY
    b.push(bit_depth);
    b.push(BITMAP_FLAG_HORIZONTAL_METRICS);
    assert_eq!(b.len(), ista_at);
    // IndexSubTableArray.
    for ((first, last, _), sub_off) in ranges.iter().zip(&sub_offsets) {
        b.extend_from_slice(&u16b(*first));
        b.extend_from_slice(&u16b(*last));
        b.extend_from_slice(&u32b(*sub_off as u32));
    }
    for (_, _, sub) in ranges {
        b.extend_from_slice(sub);
    }
    b
}

/// IndexSubHeader bytes.
fn header(index_format: u16, image_format: u16, image_data_offset: u32) -> Vec<u8> {
    let mut b = Vec::new();
    b.extend_from_slice(&u16b(index_format));
    b.extend_from_slice(&u16b(image_format));
    b.extend_from_slice(&u32b(image_data_offset));
    b
}

#[test]
fn strike_metadata_and_line_metrics() {
    let sub = {
        let mut b = header(1, 1, 100);
        for off in [0u32, 10, 30] {
            b.extend_from_slice(&u32b(off));
        }
        b
    };
    let bytes = build_table(2, &[(4, 5, sub)], 24, 8);
    let t = BitmapLocationTable::parse(&bytes).expect("parse");
    assert_eq!(t.major_version(), 2);
    assert_eq!(t.minor_version(), 0);
    assert_eq!(t.sizes().len(), 1);
    let s = &t.sizes()[0];
    assert_eq!((s.ppem_x, s.ppem_y, s.bit_depth), (24, 24, 8));
    assert!(s.horizontal_metrics());
    assert!(!s.vertical_metrics());
    assert_eq!(s.hori.ascender, 12);
    assert_eq!(s.hori.descender, -4);
    assert_eq!(s.hori.width_max, 9);
    assert_eq!(s.vert.ascender, 6);
    assert_eq!((s.start_glyph_index, s.end_glyph_index), (4, 5));
}

#[test]
fn index_format_1_variable_offset32() {
    // Glyphs 4..=5: offsets 0, 10, 30 (glyph 4 = 10 bytes at 100,
    // glyph 5 = 20 bytes at 110).
    let sub = {
        let mut b = header(1, 6, 100);
        for off in [0u32, 10, 30] {
            b.extend_from_slice(&u32b(off));
        }
        b
    };
    let bytes = build_table(2, &[(4, 5, sub)], 24, 1);
    let t = BitmapLocationTable::parse(&bytes).unwrap();

    let loc = t.locate(0, 4).unwrap().unwrap();
    assert_eq!((loc.image_format, loc.offset, loc.length), (6, 100, 10));
    assert_eq!(loc.metrics, None);
    let loc = t.locate(0, 5).unwrap().unwrap();
    assert_eq!((loc.offset, loc.length), (110, 20));
    // Outside every range.
    assert_eq!(t.locate(0, 3).unwrap(), None);
    assert_eq!(t.locate(0, 6).unwrap(), None);
    // Bad size index errors.
    assert!(t.locate(1, 4).is_err());
}

#[test]
fn index_format_2_constant_metrics() {
    let sub = {
        let mut b = header(2, 5, 400);
        b.extend_from_slice(&u32b(9)); // imageSize
        b.extend_from_slice(&BIG_METRICS);
        b
    };
    let bytes = build_table(2, &[(10, 12, sub)], 16, 1);
    let t = BitmapLocationTable::parse(&bytes).unwrap();

    for (gid, expect_off) in [(10u16, 400u32), (11, 409), (12, 418)] {
        let loc = t.locate(0, gid).unwrap().unwrap();
        assert_eq!(
            (loc.image_format, loc.offset, loc.length),
            (5, expect_off, 9)
        );
        let m = loc.metrics.unwrap();
        assert_eq!((m.height, m.width), (7, 5));
        assert_eq!((m.hori_bearing_x, m.hori_bearing_y), (1, 6));
        assert_eq!((m.vert_bearing_x, m.vert_bearing_y), (-2, -1));
        assert_eq!((m.hori_advance, m.vert_advance), (6, 8));
    }
}

#[test]
fn index_format_3_variable_offset16_and_zero_length_gaps() {
    // Glyph 21 has a zero-length entry (missing) between 20 and 22.
    let sub = {
        let mut b = header(3, 1, 50);
        for off in [0u16, 8, 8, 20] {
            b.extend_from_slice(&u16b(off));
        }
        b
    };
    let bytes = build_table(2, &[(20, 22, sub)], 12, 2);
    let t = BitmapLocationTable::parse(&bytes).unwrap();

    let loc = t.locate(0, 20).unwrap().unwrap();
    assert_eq!((loc.offset, loc.length), (50, 8));
    assert_eq!(t.locate(0, 21).unwrap(), None); // zero-size gap
    let loc = t.locate(0, 22).unwrap().unwrap();
    assert_eq!((loc.offset, loc.length), (58, 12));
}

#[test]
fn index_format_4_sparse_pairs() {
    // Sparse glyphs 7 and 100 (plus the closing sentinel pair).
    let sub = {
        let mut b = header(4, 2, 1000);
        b.extend_from_slice(&u32b(2)); // numGlyphs
        for (gid, off) in [(7u16, 0u16), (100, 24), (0xFFFF, 60)] {
            b.extend_from_slice(&u16b(gid));
            b.extend_from_slice(&u16b(off));
        }
        b
    };
    let bytes = build_table(2, &[(7, 100, sub)], 20, 4);
    let t = BitmapLocationTable::parse(&bytes).unwrap();

    let loc = t.locate(0, 7).unwrap().unwrap();
    assert_eq!((loc.image_format, loc.offset, loc.length), (2, 1000, 24));
    let loc = t.locate(0, 100).unwrap().unwrap();
    assert_eq!((loc.offset, loc.length), (1024, 36));
    // In the range but not in the sparse array.
    assert_eq!(t.locate(0, 50).unwrap(), None);
}

#[test]
fn index_format_5_constant_metrics_sparse_ids() {
    let sub = {
        let mut b = header(5, 17, 2000);
        b.extend_from_slice(&u32b(64)); // imageSize
        b.extend_from_slice(&BIG_METRICS);
        b.extend_from_slice(&u32b(3)); // numGlyphs
        for gid in [30u16, 33, 40] {
            b.extend_from_slice(&u16b(gid));
        }
        b.extend_from_slice(&u16b(0)); // pad to uint32 alignment
        b
    };
    let bytes = build_table(3, &[(30, 40, sub)], 32, 32);
    let t = BitmapLocationTable::parse(&bytes).unwrap();
    assert_eq!(t.major_version(), 3); // CBLC flavour
    assert_eq!(t.sizes()[0].bit_depth, 32); // color strike

    let loc = t.locate(0, 33).unwrap().unwrap();
    assert_eq!((loc.image_format, loc.offset, loc.length), (17, 2064, 64));
    assert!(loc.metrics.is_some());
    let loc = t.locate(0, 40).unwrap().unwrap();
    assert_eq!(loc.offset, 2128);
    assert_eq!(t.locate(0, 31).unwrap(), None);
}

#[test]
fn multiple_ranges_route_to_their_subtables() {
    let sub_a = {
        let mut b = header(1, 1, 0);
        for off in [0u32, 4] {
            b.extend_from_slice(&u32b(off));
        }
        b
    };
    let sub_b = {
        let mut b = header(2, 5, 500);
        b.extend_from_slice(&u32b(16));
        b.extend_from_slice(&BIG_METRICS);
        b
    };
    let bytes = build_table(2, &[(1, 1, sub_a), (9, 9, sub_b)], 10, 1);
    let t = BitmapLocationTable::parse(&bytes).unwrap();
    assert_eq!(t.locate(0, 1).unwrap().unwrap().image_format, 1);
    assert_eq!(t.locate(0, 9).unwrap().unwrap().image_format, 5);
    assert_eq!(t.locate(0, 5).unwrap(), None);
}

#[test]
fn best_size_selection_and_bad_versions() {
    // best_size: exact, else closest larger, else largest.
    let sub = |img_off: u32| {
        let mut b = header(2, 1, img_off);
        b.extend_from_slice(&u32b(4));
        b.extend_from_slice(&BIG_METRICS);
        b
    };
    // Build a 2-strike table by hand: reuse build_table twice and
    // splice? Simpler: single-strike tables checked separately.
    let t16 = build_table(2, &[(1, 1, sub(0))], 16, 1);
    let t = BitmapLocationTable::parse(&t16).unwrap();
    assert_eq!(t.best_size(16), Some(0));
    assert_eq!(t.best_size(64), Some(0));

    // Version guard: major must be 2 or 3.
    let mut bad = t16.clone();
    bad[0..2].copy_from_slice(&u16b(1));
    assert!(BitmapLocationTable::parse(&bad).is_err());

    // Unknown index format errors.
    let mut sub9 = header(9, 1, 0);
    sub9.extend_from_slice(&u32b(4));
    let bytes = build_table(2, &[(1, 1, sub9)], 16, 1);
    let t = BitmapLocationTable::parse(&bytes).unwrap();
    assert!(t.locate(0, 1).is_err());

    // Truncated table: numSizes says 1 but the record is cut off.
    assert!(BitmapLocationTable::parse(&t16[..20]).is_err());
}
