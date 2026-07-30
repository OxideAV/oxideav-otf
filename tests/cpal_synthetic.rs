//! Synthetic byte-level tests for the `CPAL` palette table
//! (ISO/IEC 14496-22:2019 §5.7.12): version 0 and version 1 headers,
//! shared/overlapping color-record runs, the three version-1 optional
//! arrays (palette types, palette labels, palette entry labels), and
//! the parse-time validation rules.

use oxideav_otf::tables::cpal::CpalTable;
use oxideav_otf::{CPAL_USABLE_WITH_DARK_BACKGROUND, CPAL_USABLE_WITH_LIGHT_BACKGROUND};

fn u16b(v: u16) -> [u8; 2] {
    v.to_be_bytes()
}
fn u32b(v: u32) -> [u8; 4] {
    v.to_be_bytes()
}

/// Build a CPAL table.
///
/// * `version` — 0 or 1.
/// * `entries` — numPaletteEntries.
/// * `indices` — colorRecordIndices (one per palette).
/// * `records` — RGBA tuples, written to disk in BGRA order.
/// * v1 arrays — empty slice = array omitted (offset 0).
struct Builder<'x> {
    version: u16,
    entries: u16,
    indices: &'x [u16],
    records: &'x [[u8; 4]],
    types: &'x [u32],
    labels: &'x [u16],
    entry_labels: &'x [u16],
}

impl Builder<'_> {
    fn build(&self) -> Vec<u8> {
        let num_palettes = self.indices.len() as u16;
        let head_len = 12 + self.indices.len() * 2 + if self.version >= 1 { 12 } else { 0 };
        let records_at = head_len;
        let types_at = records_at + self.records.len() * 4;
        let labels_at = types_at + self.types.len() * 4;
        let entry_labels_at = labels_at + self.labels.len() * 2;

        let mut b = Vec::new();
        b.extend_from_slice(&u16b(self.version));
        b.extend_from_slice(&u16b(self.entries));
        b.extend_from_slice(&u16b(num_palettes));
        b.extend_from_slice(&u16b(self.records.len() as u16));
        b.extend_from_slice(&u32b(records_at as u32));
        for &i in self.indices {
            b.extend_from_slice(&u16b(i));
        }
        if self.version >= 1 {
            let opt = |at: usize, empty: bool| if empty { 0 } else { at as u32 };
            b.extend_from_slice(&u32b(opt(types_at, self.types.is_empty())));
            b.extend_from_slice(&u32b(opt(labels_at, self.labels.is_empty())));
            b.extend_from_slice(&u32b(opt(entry_labels_at, self.entry_labels.is_empty())));
        }
        for &[r, g, bl, a] in self.records {
            b.extend_from_slice(&[bl, g, r, a]); // BGRA on disk
        }
        for &t in self.types {
            b.extend_from_slice(&u32b(t));
        }
        for &l in self.labels {
            b.extend_from_slice(&u16b(l));
        }
        for &l in self.entry_labels {
            b.extend_from_slice(&u16b(l));
        }
        b
    }
}

const RED: [u8; 4] = [0xFF, 0x00, 0x00, 0xFF];
const GREEN: [u8; 4] = [0x00, 0xFF, 0x00, 0xFF];
const BLUE_HALF: [u8; 4] = [0x00, 0x00, 0xFF, 0x80];
const GRAY: [u8; 4] = [0x40, 0x40, 0x40, 0xC0];

#[test]
fn v1_full_surface() {
    let bytes = Builder {
        version: 1,
        entries: 2,
        indices: &[0, 2],
        records: &[RED, GREEN, BLUE_HALF, GRAY],
        types: &[
            CPAL_USABLE_WITH_LIGHT_BACKGROUND,
            CPAL_USABLE_WITH_LIGHT_BACKGROUND | CPAL_USABLE_WITH_DARK_BACKGROUND,
        ],
        labels: &[256, 0xFFFF],
        entry_labels: &[257, 0xFFFF],
    }
    .build();
    let t = CpalTable::parse(&bytes).expect("parse v1");

    assert_eq!(t.version(), 1);
    assert_eq!(t.num_palettes(), 2);
    assert_eq!(t.num_palette_entries(), 2);
    assert_eq!(t.num_color_records(), 4);
    assert_eq!(t.color_record_indices(), &[0, 2]);

    // Palette 0 = records 0..2, palette 1 = records 2..4.
    assert_eq!(t.color(0, 0).unwrap().rgba(), RED);
    assert_eq!(t.color(0, 1).unwrap().rgba(), GREEN);
    assert_eq!(t.color(1, 0).unwrap().rgba(), BLUE_HALF);
    assert_eq!(t.color(1, 1).unwrap().rgba(), GRAY);
    assert_eq!(
        t.palette(1)
            .unwrap()
            .iter()
            .map(|c| c.rgba())
            .collect::<Vec<_>>(),
        vec![BLUE_HALF, GRAY]
    );

    // Palette types.
    let p0 = t.palette_type(0).unwrap();
    assert!(p0.usable_with_light_background());
    assert!(!p0.usable_with_dark_background());
    let p1 = t.palette_type(1).unwrap();
    assert!(p1.usable_with_light_background());
    assert!(p1.usable_with_dark_background());
    assert_eq!(t.palette_type(2), None);

    // Labels: 0xFFFF is the no-label sentinel.
    assert_eq!(t.palette_label(0), Some(256));
    assert_eq!(t.palette_label(1), None);
    assert_eq!(t.palette_entry_label(0), Some(257));
    assert_eq!(t.palette_entry_label(1), None);
    assert_eq!(t.palette_entry_label(2), None);

    // Background selection: palette 1 carries both flags, so it
    // appears in both answers.
    assert_eq!(t.palettes_for_background(true), vec![0, 1]);
    assert_eq!(t.palettes_for_background(false), vec![1]);
}

#[test]
fn v1_arrays_all_omitted() {
    let bytes = Builder {
        version: 1,
        entries: 1,
        indices: &[0],
        records: &[RED],
        types: &[],
        labels: &[],
        entry_labels: &[],
    }
    .build();
    let t = CpalTable::parse(&bytes).expect("parse");
    assert_eq!(t.color(0, 0).unwrap().rgba(), RED);
    assert_eq!(t.palette_type(0), None);
    assert_eq!(t.palette_label(0), None);
    assert_eq!(t.palette_entry_label(0), None);
}

#[test]
fn foreground_sentinel_entry_is_never_a_cpal_color() {
    let bytes = Builder {
        version: 0,
        entries: 1,
        indices: &[0],
        records: &[RED],
        types: &[],
        labels: &[],
        entry_labels: &[],
    }
    .build();
    let t = CpalTable::parse(&bytes).expect("parse");
    // COLR's 0xFFFF paletteIndex means "text foreground"; it must not
    // resolve through CPAL.
    assert_eq!(t.color(0, 0xFFFF), None);
}

#[test]
fn v1_truncated_declared_arrays_are_rejected() {
    // Declare a palette-type array whose extent runs past the table
    // end: chop the last byte off an otherwise-valid table.
    let bytes = Builder {
        version: 1,
        entries: 1,
        indices: &[0],
        records: &[RED],
        types: &[CPAL_USABLE_WITH_DARK_BACKGROUND],
        labels: &[],
        entry_labels: &[],
    }
    .build();
    assert!(CpalTable::parse(&bytes[..bytes.len() - 1]).is_err());
    // The intact table parses.
    assert!(CpalTable::parse(&bytes).is_ok());
}

#[test]
fn record_count_must_cover_every_palette_run() {
    // Palette 1 starts at record 3 with 2 entries but only 4 records
    // exist: 3 + 2 > 4 must be rejected per §5.7.12.
    let bytes = Builder {
        version: 0,
        entries: 2,
        indices: &[0, 3],
        records: &[RED, GREEN, BLUE_HALF, GRAY],
        types: &[],
        labels: &[],
        entry_labels: &[],
    }
    .build();
    assert!(CpalTable::parse(&bytes).is_err());
}
