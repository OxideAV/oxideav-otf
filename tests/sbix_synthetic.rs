//! Synthetic byte-level tests for the `sbix` table (ISO/IEC
//! 14496-22:2019 §5.6.7): header + flags, per-strike glyph data
//! offsets, graphic types, `'dupe'` chains and cycles, and strike
//! selection.

use oxideav_otf::tables::sbix::SbixTable;
use oxideav_otf::{GraphicType, SBIX_FLAG_DRAW_OUTLINES};

fn u16b(v: u16) -> [u8; 2] {
    v.to_be_bytes()
}
fn u32b(v: u32) -> [u8; 4] {
    v.to_be_bytes()
}

/// One glyph's data blob: `(originX, originY, tag, payload)`.
type Glyph<'x> = Option<(i16, i16, [u8; 4], &'x [u8])>;

/// Build a strike blob for `num_glyphs` glyphs.
fn strike(ppem: u16, ppi: u16, glyphs: &[Glyph<'_>]) -> Vec<u8> {
    let mut b = Vec::new();
    b.extend_from_slice(&u16b(ppem));
    b.extend_from_slice(&u16b(ppi));
    let offsets_at = b.len();
    // Reserve numGlyphs + 1 offsets.
    b.resize(offsets_at + (glyphs.len() + 1) * 4, 0);
    for (i, g) in glyphs.iter().enumerate() {
        let here = b.len() as u32;
        b[offsets_at + i * 4..offsets_at + i * 4 + 4].copy_from_slice(&u32b(here));
        if let Some((ox, oy, tag, payload)) = g {
            b.extend_from_slice(&ox.to_be_bytes());
            b.extend_from_slice(&oy.to_be_bytes());
            b.extend_from_slice(tag);
            b.extend_from_slice(payload);
        }
    }
    let end = b.len() as u32;
    let last = offsets_at + glyphs.len() * 4;
    b[last..last + 4].copy_from_slice(&u32b(end));
    b
}

/// Assemble an sbix table from strike blobs.
fn sbix(flags: u16, strikes: &[Vec<u8>]) -> Vec<u8> {
    let mut b = Vec::new();
    b.extend_from_slice(&u16b(1)); // version
    b.extend_from_slice(&u16b(flags));
    b.extend_from_slice(&u32b(strikes.len() as u32));
    let mut at = 8 + strikes.len() * 4;
    for s in strikes {
        b.extend_from_slice(&u32b(at as u32));
        at += s.len();
    }
    for s in strikes {
        b.extend_from_slice(s);
    }
    b
}

const PNG_PAYLOAD: &[u8] = b"\x89PNG-not-really";

#[test]
fn header_strikes_and_glyph_data() {
    // 3 glyphs: gid0 = png, gid1 = none, gid2 = unknown tag.
    let s16 = strike(
        16,
        72,
        &[
            Some((1, -2, *b"png ", PNG_PAYLOAD)),
            None,
            Some((0, 0, *b"mask", b"xx")),
        ],
    );
    let bytes = sbix(SBIX_FLAG_DRAW_OUTLINES | 1, &[s16]);
    let t = SbixTable::parse(&bytes, 3).expect("parse");
    assert_eq!(t.version(), 1);
    assert!(t.draw_outlines());
    assert_eq!(t.num_strikes(), 1);

    let s = t.strike(0).unwrap();
    assert_eq!((s.ppem, s.ppi), (16, 72));

    let g = s.glyph_graphic(0).unwrap().unwrap();
    assert_eq!(g.origin_offset_x, 1);
    assert_eq!(g.origin_offset_y, -2);
    assert_eq!(g.graphic_type, GraphicType::Png);
    assert_eq!(g.data, PNG_PAYLOAD);

    // Zero-length entry = no bitmap.
    assert_eq!(s.glyph_graphic(1).unwrap(), None);

    // Unknown tags surface as-is (OFF does not support 'mask').
    let g = s.glyph_graphic(2).unwrap().unwrap();
    assert_eq!(g.graphic_type, GraphicType::Other(*b"mask"));

    // Out-of-range gid errors.
    assert!(s.glyph_graphic(3).is_err());
}

#[test]
fn dupe_chains_resolve_and_cycles_error() {
    // gid0 -> dupe(1), gid1 -> dupe(2), gid2 = jpg; gid3 -> dupe(3) self.
    let s = strike(
        32,
        96,
        &[
            Some((0, 0, *b"dupe", &u16b(1))),
            Some((0, 0, *b"dupe", &u16b(2))),
            Some((5, 6, *b"jpg ", b"JFIFdata")),
            Some((0, 0, *b"dupe", &u16b(3))),
        ],
    );
    let bytes = sbix(1, &[s]);
    let t = SbixTable::parse(&bytes, 4).expect("parse");
    let s = t.strike(0).unwrap();

    // Unresolved read surfaces the dupe record itself.
    let g = s.glyph_graphic(0).unwrap().unwrap();
    assert_eq!(g.graphic_type, GraphicType::Dupe);

    // Resolved read follows the chain to the JPEG.
    let g = s.glyph_graphic_resolved(0).unwrap().unwrap();
    assert_eq!(g.graphic_type, GraphicType::Jpg);
    assert_eq!(g.data, b"JFIFdata");
    assert_eq!((g.origin_offset_x, g.origin_offset_y), (5, 6));

    // Self-referencing dupe is a cycle.
    assert!(s.glyph_graphic_resolved(3).is_err());
}

#[test]
fn best_strike_prefers_exact_then_larger_then_largest() {
    let strikes = [
        strike(16, 72, &[None]),
        strike(32, 72, &[None]),
        strike(64, 72, &[None]),
    ];
    let bytes = sbix(1, &strikes);
    let t = SbixTable::parse(&bytes, 1).expect("parse");
    // Exact.
    assert_eq!(t.best_strike(32).unwrap().ppem, 32);
    // Between 16 and 32 -> closest larger.
    assert_eq!(t.best_strike(20).unwrap().ppem, 32);
    // Below all -> smallest (closest larger).
    assert_eq!(t.best_strike(10).unwrap().ppem, 16);
    // Above all -> largest available.
    assert_eq!(t.best_strike(100).unwrap().ppem, 64);
}

#[test]
fn best_strike_breaks_ppem_ties_by_ppi() {
    let strikes = [strike(32, 96, &[None]), strike(32, 192, &[None])];
    let bytes = sbix(1, &strikes);
    let t = SbixTable::parse(&bytes, 1).expect("parse");
    assert_eq!(t.best_strike(32).unwrap().ppi, 192);
}

#[test]
fn truncation_and_malformed_ranges_are_rejected() {
    let s = strike(16, 72, &[Some((0, 0, *b"png ", PNG_PAYLOAD))]);
    let bytes = sbix(1, &[s]);
    // Intact table parses.
    assert!(SbixTable::parse(&bytes, 1).is_ok());
    // Strike offsets array sized for more glyphs than the data holds.
    assert!(SbixTable::parse(&bytes, 200).is_err());
    // Truncated table: strike header out of bounds.
    assert!(SbixTable::parse(&bytes[..10], 1).is_err());

    // A non-empty glyph entry shorter than the 8-byte header is
    // malformed: build a strike whose only entry is 4 bytes long.
    let mut s = Vec::new();
    s.extend_from_slice(&u16b(16));
    s.extend_from_slice(&u16b(72));
    let base = 4 + 2 * 4; // header + 2 offsets
    s.extend_from_slice(&u32b(base as u32));
    s.extend_from_slice(&u32b(base as u32 + 4));
    s.extend_from_slice(&[0u8; 4]);
    let bytes = sbix(1, &[s]);
    let t = SbixTable::parse(&bytes, 1).unwrap();
    assert!(t.strike(0).unwrap().glyph_graphic(0).is_err());
}
