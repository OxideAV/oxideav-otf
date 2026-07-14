//! Exhaustive single-byte mutation + truncation robustness for the
//! round's color-font / embedded-bitmap tables — `CPAL`, `sbix`,
//! `EBLC`/`CBLC`, `EBDT`/`CBDT`, `EBSC`, and `SVG ` — mirroring the
//! `COLR` sweep in `colr_synthetic.rs`. Every mutant must either
//! fail to parse or survive full-surface queries with `Result` /
//! `Option` outcomes only — never a panic, hang, or runaway
//! allocation.

use oxideav_otf::tables::ebdt::{unpack_pixels, BitmapContent, BitmapDataTable, GlyphMetrics};
use oxideav_otf::tables::eblc::BitmapLocationTable;
use oxideav_otf::tables::ebsc::EbscTable;
use oxideav_otf::tables::sbix::SbixTable;
use oxideav_otf::tables::svg::SvgTable;
use oxideav_otf::tables::{colr::ColrTable, cpal::CpalTable};
use oxideav_otf::{resolve_paint_color, ColorRecord};

fn u16b(v: u16) -> [u8; 2] {
    v.to_be_bytes()
}
fn u32b(v: u32) -> [u8; 4] {
    v.to_be_bytes()
}

/// Run `exercise` over every single-byte mutant (three values per
/// position) and every truncation of `base`.
fn sweep(base: &[u8], exercise: &dyn Fn(&[u8])) {
    for i in 0..base.len() {
        for v in [0x00u8, 0xFF, base[i].wrapping_add(1)] {
            let mut m = base.to_vec();
            m[i] = v;
            exercise(&m);
        }
    }
    for len in 0..base.len() {
        exercise(&base[..len]);
    }
}

const FG: ColorRecord = ColorRecord {
    red: 1,
    green: 2,
    blue: 3,
    alpha: 255,
};

#[test]
fn cpal_mutation_robustness() {
    // v1 table with all three optional arrays present.
    let mut b = Vec::new();
    b.extend_from_slice(&u16b(1)); // version
    b.extend_from_slice(&u16b(2)); // numPaletteEntries
    b.extend_from_slice(&u16b(2)); // numPalettes
    b.extend_from_slice(&u16b(3)); // numColorRecords
    let records_at = 12 + 4 + 12;
    b.extend_from_slice(&u32b(records_at as u32));
    b.extend_from_slice(&u16b(0));
    b.extend_from_slice(&u16b(1));
    let types_at = records_at + 3 * 4;
    let labels_at = types_at + 2 * 4;
    let entry_labels_at = labels_at + 2 * 2;
    b.extend_from_slice(&u32b(types_at as u32));
    b.extend_from_slice(&u32b(labels_at as u32));
    b.extend_from_slice(&u32b(entry_labels_at as u32));
    b.extend_from_slice(&[0x10, 0x20, 0x30, 0xFF]);
    b.extend_from_slice(&[0x40, 0x50, 0x60, 0x80]);
    b.extend_from_slice(&[0x70, 0x80, 0x90, 0x40]);
    b.extend_from_slice(&u32b(1));
    b.extend_from_slice(&u32b(2));
    b.extend_from_slice(&u16b(256));
    b.extend_from_slice(&u16b(0xFFFF));
    b.extend_from_slice(&u16b(257));
    b.extend_from_slice(&u16b(0xFFFF));

    sweep(&b, &|bytes| {
        let Ok(t) = CpalTable::parse(bytes) else {
            return;
        };
        for palette in 0..=t.num_palettes() {
            for entry in [0u16, 1, 2, 0xFFFE, 0xFFFF] {
                let _ = t.color(palette, entry);
                let _ = resolve_paint_color(&t, palette, entry, 0.5, FG);
            }
            let _ = t.palette(palette);
            let _ = t.palette_type(palette);
            let _ = t.palette_label(palette);
        }
        for entry in 0..=t.num_palette_entries() {
            let _ = t.palette_entry_label(entry);
        }
    });
}

#[test]
fn sbix_mutation_robustness() {
    // 2 strikes x 3 glyphs: png, dupe chain, missing.
    let strike = |ppem: u16| -> Vec<u8> {
        let mut s = Vec::new();
        s.extend_from_slice(&u16b(ppem));
        s.extend_from_slice(&u16b(72));
        let offsets_at = s.len();
        s.resize(offsets_at + 4 * 4, 0);
        let mut set = |i: usize, v: usize| {
            let at = offsets_at + i * 4;
            let v = v as u32;
            s[at..at + 4].copy_from_slice(&u32b(v));
        };
        let g0 = 4 + 4 * 4;
        set(0, g0);
        let g1 = g0 + 8 + 4; // png record with 4 payload bytes
        set(1, g1);
        let g2 = g1 + 8 + 2; // dupe record
        set(2, g2);
        set(3, g2); // glyph 2 empty
        s.extend_from_slice(&[0, 1, 0, 2]);
        s.extend_from_slice(b"png ");
        s.extend_from_slice(&[9, 8, 7, 6]);
        s.extend_from_slice(&[0, 0, 0, 0]);
        s.extend_from_slice(b"dupe");
        s.extend_from_slice(&u16b(0));
        s
    };
    let s16 = strike(16);
    let s32 = strike(32);
    let mut b = Vec::new();
    b.extend_from_slice(&u16b(1));
    b.extend_from_slice(&u16b(3));
    b.extend_from_slice(&u32b(2));
    b.extend_from_slice(&u32b(16));
    b.extend_from_slice(&u32b(16 + s16.len() as u32));
    b.extend_from_slice(&s16);
    b.extend_from_slice(&s32);

    sweep(&b, &|bytes| {
        let Ok(t) = SbixTable::parse(bytes, 3) else {
            return;
        };
        let _ = t.draw_outlines();
        for i in 0..t.num_strikes() + 1 {
            let Some(s) = t.strike(i) else { continue };
            for gid in 0..4u16 {
                let _ = s.glyph_graphic(gid);
                let _ = s.glyph_graphic_resolved(gid);
            }
        }
        for ppem in [0u16, 16, 24, 32, 0xFFFF] {
            let _ = t.best_strike(ppem);
        }
    });
}

/// A small EBLC (one strike, index formats 2 + 3) and matching EBDT.
fn eblc_ebdt_pair() -> (Vec<u8>, Vec<u8>) {
    let mut sub2 = Vec::new();
    sub2.extend_from_slice(&u16b(2)); // index format 2
    sub2.extend_from_slice(&u16b(5)); // image format 5
    sub2.extend_from_slice(&u32b(4)); // imageDataOffset
    sub2.extend_from_slice(&u32b(2)); // imageSize
    sub2.extend_from_slice(&[3, 5, 1, 3, 6, 0xFF, 0xFE, 7]); // big metrics
    let mut sub3 = Vec::new();
    sub3.extend_from_slice(&u16b(3)); // index format 3
    sub3.extend_from_slice(&u16b(1)); // image format 1
    sub3.extend_from_slice(&u32b(8)); // imageDataOffset
    for off in [0u16, 8, 8, 16] {
        sub3.extend_from_slice(&u16b(off));
    }

    let ista_at = 8 + 48;
    let ista_len = 2 * 8;
    let mut b = Vec::new();
    b.extend_from_slice(&u16b(2)); // major
    b.extend_from_slice(&u16b(0));
    b.extend_from_slice(&u32b(1)); // numSizes
    b.extend_from_slice(&u32b(ista_at as u32));
    b.extend_from_slice(&u32b((ista_len + sub2.len() + sub3.len()) as u32));
    b.extend_from_slice(&u32b(2)); // numberOfIndexSubTables
    b.extend_from_slice(&u32b(0)); // colorRef
    b.extend_from_slice(&[0u8; 24]); // hori + vert line metrics
    b.extend_from_slice(&u16b(1)); // startGlyphIndex
    b.extend_from_slice(&u16b(6)); // endGlyphIndex
    b.extend_from_slice(&[16, 16, 1, 1]); // ppemX/Y, bitDepth, flags
                                          // IndexSubTableArray.
    b.extend_from_slice(&u16b(1));
    b.extend_from_slice(&u16b(2));
    b.extend_from_slice(&u32b(ista_len as u32));
    b.extend_from_slice(&u16b(4));
    b.extend_from_slice(&u16b(6));
    b.extend_from_slice(&u32b((ista_len + sub2.len()) as u32));
    b.extend_from_slice(&sub2);
    b.extend_from_slice(&sub3);

    let mut d = Vec::new();
    d.extend_from_slice(&u16b(2)); // major
    d.extend_from_slice(&u16b(0));
    d.extend_from_slice(&[0xAA; 40]); // bitmap bytes (format 5 + format 1 blobs)
    (b, d)
}

#[test]
fn eblc_ebdt_mutation_robustness() {
    let (loc_bytes, dat_bytes) = eblc_ebdt_pair();
    let dat = BitmapDataTable::parse(&dat_bytes).unwrap();

    // Mutate the location table against a fixed data table.
    sweep(&loc_bytes, &|bytes| {
        let Ok(t) = BitmapLocationTable::parse(bytes) else {
            return;
        };
        for size in 0..t.sizes().len() {
            for gid in 0..8u16 {
                let Ok(Some(loc)) = t.locate(size, gid) else {
                    continue;
                };
                let Ok(g) = dat.glyph_data(&loc) else {
                    continue;
                };
                let (w, h) = match (&g.metrics, &loc.metrics) {
                    (Some(GlyphMetrics::Small(m)), _) => (m.width, m.height),
                    (Some(GlyphMetrics::Big(m)), _) => (m.width, m.height),
                    (None, Some(m)) => (m.width, m.height),
                    (None, None) => continue,
                };
                match g.content {
                    BitmapContent::ByteAligned(img) => {
                        let _ = unpack_pixels(img, w as usize, h as usize, 1, true);
                    }
                    BitmapContent::BitAligned(img) => {
                        let _ = unpack_pixels(img, w as usize, h as usize, 1, false);
                    }
                    _ => {}
                }
            }
        }
        for ppem in [0u8, 16, 255] {
            let _ = t.best_size(ppem);
        }
    });

    // Mutate the data table against fixed locations.
    let loc = BitmapLocationTable::parse(&loc_bytes).unwrap();
    let locations: Vec<_> = (0..8u16)
        .filter_map(|gid| loc.locate(0, gid).ok().flatten())
        .collect();
    sweep(&dat_bytes, &|bytes| {
        let Ok(t) = BitmapDataTable::parse(bytes) else {
            return;
        };
        for l in &locations {
            let _ = t.glyph_data(l);
        }
    });
}

#[test]
fn ebsc_mutation_robustness() {
    let mut b = Vec::new();
    b.extend_from_slice(&u16b(2));
    b.extend_from_slice(&u16b(0));
    b.extend_from_slice(&u32b(2));
    for (px, py, sx, sy) in [(11u8, 11u8, 12u8, 12u8), (13, 14, 16, 16)] {
        b.extend_from_slice(&[0u8; 24]);
        b.extend_from_slice(&[px, py, sx, sy]);
    }
    sweep(&b, &|bytes| {
        let Ok(t) = EbscTable::parse(bytes) else {
            return;
        };
        for s in t.scales() {
            let _ = (s.hori, s.vert);
        }
        for ppem in [0u8, 11, 13, 255] {
            let _ = t.scale_for(ppem, ppem);
        }
    });
}

#[test]
fn svg_mutation_robustness() {
    let doc = b"<svg><g id=\"glyph1\"/></svg>";
    let mut b = Vec::new();
    b.extend_from_slice(&u16b(0));
    b.extend_from_slice(&u32b(10));
    b.extend_from_slice(&u32b(0));
    b.extend_from_slice(&u16b(2));
    let index_len = 2 + 2 * 12;
    b.extend_from_slice(&u16b(1));
    b.extend_from_slice(&u16b(2));
    b.extend_from_slice(&u32b(index_len as u32));
    b.extend_from_slice(&u32b(doc.len() as u32));
    b.extend_from_slice(&u16b(9));
    b.extend_from_slice(&u16b(9));
    b.extend_from_slice(&u32b(index_len as u32));
    b.extend_from_slice(&u32b(doc.len() as u32));
    b.extend_from_slice(doc);

    sweep(&b, &|bytes| {
        let Ok(t) = SvgTable::parse(bytes) else {
            return;
        };
        for gid in [0u16, 1, 2, 5, 9, 0xFFFF] {
            if let Some(d) = t.document_for_glyph(gid) {
                let _ = d.is_gzip();
            }
        }
        let _ = t.documents().count();
    });
}

/// Cross-table sweep: a COLR v0 + CPAL pair where the CPAL side is
/// mutated while resolution queries run against a fixed COLR.
#[test]
fn colr_cpal_cross_resolution_robustness() {
    let mut colr = Vec::new();
    colr.extend_from_slice(&u16b(0));
    colr.extend_from_slice(&u16b(1));
    colr.extend_from_slice(&u32b(14));
    colr.extend_from_slice(&u32b(20));
    colr.extend_from_slice(&u16b(2));
    colr.extend_from_slice(&u16b(7));
    colr.extend_from_slice(&u16b(0));
    colr.extend_from_slice(&u16b(2));
    colr.extend_from_slice(&u16b(20));
    colr.extend_from_slice(&u16b(0));
    colr.extend_from_slice(&u16b(21));
    colr.extend_from_slice(&u16b(0xFFFF));
    let colr = ColrTable::parse(&colr).unwrap();

    let mut cpal = Vec::new();
    cpal.extend_from_slice(&u16b(0));
    cpal.extend_from_slice(&u16b(1));
    cpal.extend_from_slice(&u16b(1));
    cpal.extend_from_slice(&u16b(1));
    cpal.extend_from_slice(&u32b(14));
    cpal.extend_from_slice(&u16b(0));
    cpal.extend_from_slice(&[0x10, 0x20, 0x30, 0xFF]);

    sweep(&cpal, &|bytes| {
        let Ok(t) = CpalTable::parse(bytes) else {
            return;
        };
        if let Some(layers) = colr.v0_layers(7) {
            for l in layers {
                let _ = l.resolve(&t, 0, FG);
                let _ = l.resolve(&t, 1, FG);
            }
        }
    });
}
