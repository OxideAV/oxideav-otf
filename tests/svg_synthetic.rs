//! Synthetic byte-level tests for the `SVG ` table (ISO/IEC
//! 14496-22:2019 §5.5): the header + document index, shared
//! documents across ranges, gzip detection, glyph element ids, and
//! the index-ordering / non-zero-field invariants.

use oxideav_otf::tables::svg::SvgTable;
use oxideav_otf::SvgDocument;

fn u16b(v: u16) -> [u8; 2] {
    v.to_be_bytes()
}
fn u32b(v: u32) -> [u8; 4] {
    v.to_be_bytes()
}

/// Build an SVG table: entries are `(start, end, doc index)` over the
/// `docs` list; documents are laid out after the index. `svgDocOffset`
/// is relative to the document index per §5.5.1.
fn svg_table(entries: &[(u16, u16, usize)], docs: &[&[u8]]) -> Vec<u8> {
    const HDR: usize = 10;
    let index_len = 2 + entries.len() * 12;
    // Per-document offsets relative to the doc index start.
    let mut doc_offsets = Vec::new();
    let mut cursor = index_len;
    for d in docs {
        doc_offsets.push(cursor);
        cursor += d.len();
    }
    let mut b = Vec::new();
    b.extend_from_slice(&u16b(0)); // version
    b.extend_from_slice(&u32b(HDR as u32)); // svgDocIndexOffset
    b.extend_from_slice(&u32b(0)); // reserved
    b.extend_from_slice(&u16b(entries.len() as u16));
    for &(start, end, doc) in entries {
        b.extend_from_slice(&u16b(start));
        b.extend_from_slice(&u16b(end));
        b.extend_from_slice(&u32b(doc_offsets[doc] as u32));
        b.extend_from_slice(&u32b(docs[doc].len() as u32));
    }
    for d in docs {
        b.extend_from_slice(d);
    }
    b
}

const PLAIN: &[u8] = b"<svg><g id=\"glyph95\"/><g id=\"glyph96\"/></svg>";
// RFC 1952 gzip magic + arbitrary tail (not a real deflate stream;
// the table layer only carries the bytes).
const GZ: &[u8] = &[0x1F, 0x8B, 0x08, 0x00, 0xAA, 0xBB];

#[test]
fn index_lookup_and_shared_documents() {
    // Two ranges share document 0 (the spec's own example layout);
    // a third range has its own gzip document.
    let bytes = svg_table(&[(95, 96, 0), (98, 98, 0), (99, 99, 1)], &[PLAIN, GZ]);
    let t = SvgTable::parse(&bytes).expect("parse");
    assert_eq!(t.version(), 0);
    assert_eq!(t.num_entries(), 3);

    let d = t.document_for_glyph(95).unwrap();
    assert_eq!((d.start_glyph_id, d.end_glyph_id), (95, 96));
    assert_eq!(d.data, PLAIN);
    assert!(!d.is_gzip());
    // Same document via the second range.
    let d2 = t.document_for_glyph(98).unwrap();
    assert_eq!(d2.data, PLAIN);
    assert_eq!((d2.start_glyph_id, d2.end_glyph_id), (98, 98));

    let g = t.document_for_glyph(99).unwrap();
    assert_eq!(g.data, GZ);
    assert!(g.is_gzip());

    // Gaps and out-of-range IDs.
    assert!(t.document_for_glyph(94).is_none());
    assert!(t.document_for_glyph(97).is_none());
    assert!(t.document_for_glyph(100).is_none());
    assert!(t.has_glyph(96));
    assert!(!t.has_glyph(97));

    // Iteration covers every entry in order.
    let starts: Vec<u16> = t.documents().map(|d| d.start_glyph_id).collect();
    assert_eq!(starts, [95, 98, 99]);

    // Element id convention: non-zero-padded decimal.
    assert_eq!(SvgDocument::glyph_element_id(96), "glyph96");
    assert_eq!(SvgDocument::glyph_element_id(7), "glyph7");
}

#[test]
fn ordering_and_range_invariants_are_enforced() {
    // startGlyphID not ascending past the previous end (overlap).
    let bytes = svg_table(&[(10, 20, 0), (20, 25, 0)], &[PLAIN]);
    assert!(SvgTable::parse(&bytes).is_err());
    // Descending start.
    let bytes = svg_table(&[(30, 31, 0), (10, 11, 0)], &[PLAIN]);
    assert!(SvgTable::parse(&bytes).is_err());
    // endGlyphID < startGlyphID.
    let bytes = svg_table(&[(21, 20, 0)], &[PLAIN]);
    assert!(SvgTable::parse(&bytes).is_err());
    // Adjacent-but-disjoint ranges are fine.
    let bytes = svg_table(&[(10, 20, 0), (21, 25, 0)], &[PLAIN]);
    assert!(SvgTable::parse(&bytes).is_ok());
}

#[test]
fn nonzero_and_bounds_invariants_are_enforced() {
    // Zero svgDocIndexOffset.
    let mut bytes = svg_table(&[(1, 1, 0)], &[PLAIN]);
    bytes[2..6].copy_from_slice(&u32b(0));
    assert!(SvgTable::parse(&bytes).is_err());
    // Zero numEntries.
    let mut bytes = svg_table(&[(1, 1, 0)], &[PLAIN]);
    bytes[10..12].copy_from_slice(&u16b(0));
    assert!(SvgTable::parse(&bytes).is_err());
    // Zero svgDocLength.
    let mut bytes = svg_table(&[(1, 1, 0)], &[PLAIN]);
    bytes[20..24].copy_from_slice(&u32b(0));
    assert!(SvgTable::parse(&bytes).is_err());
    // Document range past the table end.
    let bytes = svg_table(&[(1, 1, 0)], &[PLAIN]);
    assert!(SvgTable::parse(&bytes[..bytes.len() - 1]).is_err());
}
