//! Synthetic byte-level tests for the `EBDT` / `CBDT` bitmap data
//! tables (ISO/IEC 14496-22:2019 §5.6.2 / §5.6.5): all supported
//! image formats (1, 2, 5, 6, 7, 8, 9, 17, 18, 19), the refusal of
//! obsolete/undefined formats 3 and 4, and bit-exact pixel unpacking
//! for the packed 1/2/4/8-bit layouts plus BGRA32.

use oxideav_otf::tables::ebdt::BitmapDataTable;
use oxideav_otf::tables::eblc::BitmapLocation;
use oxideav_otf::{unpack_bgra32, unpack_pixels, BitmapContent, GlyphMetrics};

/// SmallGlyphMetrics: height 3, width 5, bearingX 1, bearingY 3,
/// advance 6.
const SMALL: [u8; 5] = [3, 5, 1, 3, 6];
/// BigGlyphMetrics: 3x5, hbx 1, hby 3, hadv 6, vbx -1, vby -2, vadv 7.
const BIG: [u8; 8] = [3, 5, 1, 3, 6, 0xFF, 0xFE, 7];

/// Build an EBDT/CBDT table (4-byte header + blobs) and a location
/// pointing at the given blob.
fn table_with(major: u16, blob: &[u8], image_format: u16) -> (Vec<u8>, BitmapLocation) {
    let mut b = Vec::new();
    b.extend_from_slice(&major.to_be_bytes());
    b.extend_from_slice(&0u16.to_be_bytes());
    b.extend_from_slice(blob);
    let loc = BitmapLocation {
        image_format,
        offset: 4,
        length: blob.len() as u32,
        metrics: None,
    };
    (b, loc)
}

#[test]
fn formats_with_inline_metrics_and_alignment() {
    // Format 1: small metrics + byte-aligned rows.
    let mut blob = SMALL.to_vec();
    // Rows 10101 / 01010 / 11111, each padded to a byte.
    blob.extend_from_slice(&[0xA8, 0x50, 0xF8]);
    let (bytes, loc) = table_with(2, &blob, 1);
    let t = BitmapDataTable::parse(&bytes).unwrap();
    let g = t.glyph_data(&loc).unwrap();
    let Some(GlyphMetrics::Small(m)) = g.metrics else {
        panic!("expected small metrics");
    };
    assert_eq!((m.height, m.width, m.advance), (3, 5, 6));
    let BitmapContent::ByteAligned(img) = g.content else {
        panic!("expected byte-aligned");
    };
    // 5 px wide, 1-bit, byte-aligned: 1 byte per row.
    let px = unpack_pixels(img, 5, 3, 1, true).unwrap();
    assert_eq!(
        px,
        [1, 0, 1, 0, 1, /*row1*/ 0, 1, 0, 1, 0, /*row2*/ 1, 1, 1, 1, 1]
    );

    // Format 6: big metrics + byte-aligned.
    let mut blob = BIG.to_vec();
    blob.extend_from_slice(&[0xF8, 0x00, 0xF8]);
    let (bytes, loc) = table_with(2, &blob, 6);
    let t = BitmapDataTable::parse(&bytes).unwrap();
    let g = t.glyph_data(&loc).unwrap();
    let Some(GlyphMetrics::Big(m)) = g.metrics else {
        panic!("expected big metrics");
    };
    assert_eq!(
        (m.vert_bearing_x, m.vert_bearing_y, m.vert_advance),
        (-1, -2, 7)
    );
    let BitmapContent::ByteAligned(img) = g.content else {
        panic!("expected byte-aligned");
    };
    assert_eq!(
        unpack_pixels(img, 5, 3, 1, true).unwrap(),
        [1, 1, 1, 1, 1, 0, 0, 0, 0, 0, 1, 1, 1, 1, 1]
    );
}

#[test]
fn bit_aligned_rows_run_contiguously() {
    // 5x3 at 1 bit, bit-aligned: 15 bits packed into 2 bytes.
    // Rows: 10101 / 01010 / 11111 -> bits 10101_01010_11111(1 pad).
    let img = [0xAA, 0xBE];
    let px = unpack_pixels(&img, 5, 3, 1, false).unwrap();
    assert_eq!(px, [1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 1, 1, 1, 1]);

    // Same data through format 2 (small metrics + bit-aligned) and
    // format 7 (big metrics + bit-aligned) and format 5 (no metrics).
    let mut blob = SMALL.to_vec();
    blob.extend_from_slice(&img);
    let (bytes, loc) = table_with(2, &blob, 2);
    let t = BitmapDataTable::parse(&bytes).unwrap();
    let g = t.glyph_data(&loc).unwrap();
    assert!(matches!(g.content, BitmapContent::BitAligned(d) if d == img));

    let mut blob = BIG.to_vec();
    blob.extend_from_slice(&img);
    let (bytes, loc) = table_with(2, &blob, 7);
    let t = BitmapDataTable::parse(&bytes).unwrap();
    assert!(matches!(
        t.glyph_data(&loc).unwrap().content,
        BitmapContent::BitAligned(_)
    ));

    let (bytes, loc) = table_with(2, &img, 5);
    let t = BitmapDataTable::parse(&bytes).unwrap();
    let g = t.glyph_data(&loc).unwrap();
    // Format 5 carries no inline metrics (they live in EBLC).
    assert_eq!(g.metrics, None);
    assert!(matches!(g.content, BitmapContent::BitAligned(d) if d == img));
}

#[test]
fn multi_bit_depths_unpack_msb_first() {
    // 2-bit, 3x2, byte-aligned: row0 = 3,2,1 -> 11_10_01_00; row1 =
    // 0,1,2 -> 00_01_10_00.
    let img = [0xE4, 0x18];
    assert_eq!(
        unpack_pixels(&img, 3, 2, 2, true).unwrap(),
        [3, 2, 1, 0, 1, 2]
    );
    // 4-bit, 2x2, bit-aligned: 0xA, 0x5, 0xF, 0x0 packed contiguously.
    let img = [0xA5, 0xF0];
    assert_eq!(
        unpack_pixels(&img, 2, 2, 4, false).unwrap(),
        [0xA, 0x5, 0xF, 0x0]
    );
    // 8-bit passthrough.
    let img = [1, 2, 3, 4, 5, 6];
    assert_eq!(unpack_pixels(&img, 3, 2, 8, false).unwrap(), img);
    // Depths other than 1/2/4/8 are rejected.
    assert!(unpack_pixels(&img, 3, 2, 3, false).is_err());
    // Truncated image errors.
    assert!(unpack_pixels(&img[..5], 3, 2, 8, false).is_err());
}

#[test]
fn composite_formats_8_and_9() {
    // Format 8: small metrics + pad + 2 components.
    let mut blob = SMALL.to_vec();
    blob.push(0); // pad
    blob.extend_from_slice(&2u16.to_be_bytes());
    blob.extend_from_slice(&[0, 40, 0, 0]); // gid 40 at (0, 0)
    blob.extend_from_slice(&[0, 41, 3, 0xFB]); // gid 41 at (3, -5)
    let (bytes, loc) = table_with(2, &blob, 8);
    let t = BitmapDataTable::parse(&bytes).unwrap();
    let g = t.glyph_data(&loc).unwrap();
    let BitmapContent::Components(cs) = g.content else {
        panic!("expected components");
    };
    assert_eq!(cs.len(), 2);
    assert_eq!((cs[0].glyph_id, cs[0].x_offset, cs[0].y_offset), (40, 0, 0));
    assert_eq!(
        (cs[1].glyph_id, cs[1].x_offset, cs[1].y_offset),
        (41, 3, -5)
    );

    // Format 9: big metrics + components (no pad byte).
    let mut blob = BIG.to_vec();
    blob.extend_from_slice(&1u16.to_be_bytes());
    blob.extend_from_slice(&[0, 99, 0xFF, 1]); // gid 99 at (-1, 1)
    let (bytes, loc) = table_with(2, &blob, 9);
    let t = BitmapDataTable::parse(&bytes).unwrap();
    let BitmapContent::Components(cs) = t.glyph_data(&loc).unwrap().content else {
        panic!("expected components");
    };
    assert_eq!(
        (cs[0].glyph_id, cs[0].x_offset, cs[0].y_offset),
        (99, -1, 1)
    );
}

#[test]
fn cbdt_png_formats_17_18_19() {
    let png_payload = b"\x89PNG\r\n\x1a\nfakedata";

    // Format 17: small metrics + dataLen + PNG.
    let mut blob = SMALL.to_vec();
    blob.extend_from_slice(&(png_payload.len() as u32).to_be_bytes());
    blob.extend_from_slice(png_payload);
    let (bytes, loc) = table_with(3, &blob, 17);
    let t = BitmapDataTable::parse(&bytes).unwrap();
    assert_eq!(t.major_version(), 3);
    let g = t.glyph_data(&loc).unwrap();
    assert!(matches!(g.metrics, Some(GlyphMetrics::Small(_))));
    assert!(matches!(g.content, BitmapContent::Png(d) if d == png_payload));

    // Format 18: big metrics + dataLen + PNG.
    let mut blob = BIG.to_vec();
    blob.extend_from_slice(&(png_payload.len() as u32).to_be_bytes());
    blob.extend_from_slice(png_payload);
    let (bytes, loc) = table_with(3, &blob, 18);
    let t = BitmapDataTable::parse(&bytes).unwrap();
    let g = t.glyph_data(&loc).unwrap();
    assert!(matches!(g.metrics, Some(GlyphMetrics::Big(_))));
    assert!(matches!(g.content, BitmapContent::Png(d) if d == png_payload));

    // Format 19: dataLen + PNG only (metrics in CBLC).
    let mut blob = Vec::new();
    blob.extend_from_slice(&(png_payload.len() as u32).to_be_bytes());
    blob.extend_from_slice(png_payload);
    let (bytes, loc) = table_with(3, &blob, 19);
    let t = BitmapDataTable::parse(&bytes).unwrap();
    let g = t.glyph_data(&loc).unwrap();
    assert_eq!(g.metrics, None);
    assert!(matches!(g.content, BitmapContent::Png(d) if d == png_payload));

    // dataLen larger than the located entry is malformed.
    let mut blob = Vec::new();
    blob.extend_from_slice(&1000u32.to_be_bytes());
    blob.extend_from_slice(png_payload);
    let (bytes, loc) = table_with(3, &blob, 19);
    let t = BitmapDataTable::parse(&bytes).unwrap();
    assert!(t.glyph_data(&loc).is_err());
}

#[test]
fn unsupported_formats_and_versions_are_rejected() {
    let (bytes, mut loc) = table_with(2, &[0u8; 16], 3);
    let t = BitmapDataTable::parse(&bytes).unwrap();
    assert!(t.glyph_data(&loc).is_err()); // format 3 obsolete
    loc.image_format = 4;
    assert!(t.glyph_data(&loc).is_err()); // format 4 undefined in OFF
    loc.image_format = 20;
    assert!(t.glyph_data(&loc).is_err()); // unknown
                                          // Out-of-bounds location.
    loc.image_format = 5;
    loc.length = 10_000;
    assert!(t.glyph_data(&loc).is_err());
    // Bad major version.
    let mut bad = bytes.clone();
    bad[0..2].copy_from_slice(&9u16.to_be_bytes());
    assert!(BitmapDataTable::parse(&bad).is_err());
}

#[test]
fn bgra32_color_bitmaps() {
    // 2x1 image: full-green half-translucent (premultiplied per
    // §5.6.5.1: 00 80 00 80) and opaque red (00 00 FF FF).
    let img = [0x00, 0x80, 0x00, 0x80, 0x00, 0x00, 0xFF, 0xFF];
    let px = unpack_bgra32(&img, 2, 1).unwrap();
    assert_eq!(px, [[0x00, 0x80, 0x00, 0x80], [0x00, 0x00, 0xFF, 0xFF]]);
    assert!(unpack_bgra32(&img, 2, 2).is_err());
}
