//! `CPAL` — Palette Table (ISO/IEC 14496-22:2019 §5.7.12; the staged
//! chapter `docs/text/opentype/otspec-cpal.html`), versions 0 and 1.
//!
//! The palette table is a set of one or more palettes, each containing
//! the same number (`numPaletteEntries`) of color records with BGRA
//! values in the sRGB color space. All color records for all palettes
//! live in a single array; each palette is a contiguous run within it,
//! starting at `colorRecordIndices[paletteIndex]`. Runs may overlap and
//! multiple palettes may share a first record, so the number of
//! functionally-distinct palettes can be fewer than `numPalettes`.
//!
//! Version 1 appends three optional arrays:
//!
//! - **Palette Type Array** — per-palette 32-bit flags
//!   (usable-with-light-background / usable-with-dark-background).
//! - **Palette Label Array** — per-palette `name`-table IDs (0xFFFF =
//!   no label).
//! - **Palette Entry Label Array** — per-entry `name`-table IDs shared
//!   by all palettes (0xFFFF = no label).
//!
//! Palette index 0 is the default palette. Colors are referenced by
//! `(paletteIndex, paletteEntryIndex)`; the entry index 0xFFFF is
//! never a `CPAL` entry — in the `COLR` table it selects the
//! application-defined text foreground color (see
//! [`crate::tables::colr::COLR_FOREGROUND_PALETTE_INDEX`]).

use crate::parser::{read_u16, read_u32, read_u8};
use crate::Error;

/// `paletteTypes` bit 0: the palette is appropriate for display on a
/// light background such as white.
pub const CPAL_USABLE_WITH_LIGHT_BACKGROUND: u32 = 0x0001;
/// `paletteTypes` bit 1: the palette is appropriate for display on a
/// dark background such as black. Not mutually exclusive with the
/// light-background flag.
pub const CPAL_USABLE_WITH_DARK_BACKGROUND: u32 = 0x0002;
/// `paletteTypes` bits 2..: reserved for future use — set to 0 per the
/// `CPAL` chapter.
pub const CPAL_PALETTE_TYPE_RESERVED: u32 =
    !(CPAL_USABLE_WITH_LIGHT_BACKGROUND | CPAL_USABLE_WITH_DARK_BACKGROUND);

/// `paletteLabels` / `paletteEntryLabels` sentinel: no `name`-table ID
/// is provided for this palette / entry.
const NO_NAME_ID: u16 = 0xFFFF;

/// One color record. Stored on disk as BGRA byte order (blue first);
/// values are sRGB, **not** premultiplied, and the alpha is explicit
/// per entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ColorRecord {
    /// Red value (byte 2 on disk).
    pub red: u8,
    /// Green value (byte 1 on disk).
    pub green: u8,
    /// Blue value (byte 0 on disk).
    pub blue: u8,
    /// Alpha value (byte 3 on disk). 255 = fully opaque. Not
    /// premultiplied into the color channels.
    pub alpha: u8,
}

impl ColorRecord {
    /// The record as an `[r, g, b, a]` array.
    pub fn rgba(&self) -> [u8; 4] {
        [self.red, self.green, self.blue, self.alpha]
    }

    /// The record's alpha as a `0.0..=1.0` fraction (`alpha / 255`),
    /// the form the `COLR` paint alpha is multiplied with.
    pub fn alpha_f32(&self) -> f32 {
        self.alpha as f32 / 255.0
    }
}

/// Per-palette 32-bit `paletteTypes` flag field (version 1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PaletteType(pub u32);

impl PaletteType {
    /// Bit 0 — palette is appropriate on a light background.
    pub fn usable_with_light_background(self) -> bool {
        self.0 & CPAL_USABLE_WITH_LIGHT_BACKGROUND != 0
    }

    /// Bit 1 — palette is appropriate on a dark background.
    pub fn usable_with_dark_background(self) -> bool {
        self.0 & CPAL_USABLE_WITH_DARK_BACKGROUND != 0
    }

    /// The reserved flag bits (everything above bit 1), which the spec
    /// requires to be 0. Non-zero values come from a future spec
    /// revision (or a malformed font) and are surfaced rather than
    /// rejected.
    pub fn reserved_bits(self) -> u32 {
        self.0 & CPAL_PALETTE_TYPE_RESERVED
    }
}

/// A parsed `CPAL` table.
#[derive(Debug)]
pub struct CpalTable<'a> {
    data: &'a [u8],
    version: u16,
    num_palette_entries: u16,
    num_palettes: u16,
    num_color_records: u16,
    /// Offset from the start of the table to the first `ColorRecord`.
    first_color_record_offset: usize,
    /// Index of each palette's first color record in the records
    /// array.
    color_record_indices: Vec<u16>,
    /// Version-1 Palette Type Array offset (validated), when present.
    palette_types_offset: Option<usize>,
    /// Version-1 Palette Label Array offset (validated), when present.
    palette_labels_offset: Option<usize>,
    /// Version-1 Palette Entry Label Array offset (validated), when
    /// present.
    palette_entry_labels_offset: Option<usize>,
}

impl<'a> CpalTable<'a> {
    /// Parse a `CPAL` table. Versions above 1 parse with version-1
    /// structure (the version-1 header is a forward-compatible prefix).
    ///
    /// Enforced spec requirements: at least one palette with at least
    /// one entry (an empty `CPAL` table is not permitted),
    /// `numColorRecords >= max(colorRecordIndices) +
    /// numPaletteEntries`, and every declared array in bounds.
    pub fn parse(data: &'a [u8]) -> Result<Self, Error> {
        let version = read_u16(data, 0)?;
        let num_palette_entries = read_u16(data, 2)?;
        let num_palettes = read_u16(data, 4)?;
        let num_color_records = read_u16(data, 6)?;
        let first_color_record_offset = read_u32(data, 8)? as usize;

        // "An empty CPAL table, with no palettes and no color records
        // is not permitted."
        if num_palettes == 0 || num_palette_entries == 0 {
            return Err(Error::BadStructure(
                "CPAL: table must define at least one palette with at least one entry",
            ));
        }

        let mut color_record_indices = Vec::with_capacity(num_palettes as usize);
        for i in 0..num_palettes as usize {
            color_record_indices.push(read_u16(data, 12 + i * 2)?);
        }

        // "numColorRecords shall be greater than or equal to
        // max(colorRecordIndices) + numPaletteEntries."
        let max_index = color_record_indices.iter().copied().max().unwrap_or(0);
        let needed = max_index as u32 + num_palette_entries as u32;
        if (num_color_records as u32) < needed {
            return Err(Error::BadStructure(
                "CPAL: numColorRecords < max(colorRecordIndices) + numPaletteEntries",
            ));
        }

        // The whole color-records array must be in bounds.
        let records_end = first_color_record_offset
            .checked_add(num_color_records as usize * 4)
            .ok_or(Error::BadOffset)?;
        if first_color_record_offset == 0 || records_end > data.len() {
            return Err(Error::BadOffset);
        }

        // Version-1 trailing offsets. Each declared (non-zero) array
        // is bounds-checked here so accessors are infallible.
        let (mut palette_types_offset, mut palette_labels_offset, mut palette_entry_labels_offset) =
            (None, None, None);
        if version >= 1 {
            let after_indices = 12 + num_palettes as usize * 2;
            let types = read_u32(data, after_indices)? as usize;
            let labels = read_u32(data, after_indices + 4)? as usize;
            let entry_labels = read_u32(data, after_indices + 8)? as usize;
            if types != 0 {
                let end = types
                    .checked_add(num_palettes as usize * 4)
                    .ok_or(Error::BadOffset)?;
                if end > data.len() {
                    return Err(Error::BadOffset);
                }
                palette_types_offset = Some(types);
            }
            if labels != 0 {
                let end = labels
                    .checked_add(num_palettes as usize * 2)
                    .ok_or(Error::BadOffset)?;
                if end > data.len() {
                    return Err(Error::BadOffset);
                }
                palette_labels_offset = Some(labels);
            }
            if entry_labels != 0 {
                let end = entry_labels
                    .checked_add(num_palette_entries as usize * 2)
                    .ok_or(Error::BadOffset)?;
                if end > data.len() {
                    return Err(Error::BadOffset);
                }
                palette_entry_labels_offset = Some(entry_labels);
            }
        }

        Ok(Self {
            data,
            version,
            num_palette_entries,
            num_palettes,
            num_color_records,
            first_color_record_offset,
            color_record_indices,
            palette_types_offset,
            palette_labels_offset,
            palette_entry_labels_offset,
        })
    }

    /// The table version (0 or 1).
    pub fn version(&self) -> u16 {
        self.version
    }

    /// Number of palettes in the table (>= 1). Palette 0 is the
    /// default palette.
    pub fn num_palettes(&self) -> u16 {
        self.num_palettes
    }

    /// Number of entries in **each** palette (>= 1). Every palette has
    /// the same entry count.
    pub fn num_palette_entries(&self) -> u16 {
        self.num_palette_entries
    }

    /// Total number of color records, combined for all palettes (may
    /// be less than `num_palettes * num_palette_entries` when palettes
    /// share records).
    pub fn num_color_records(&self) -> u16 {
        self.num_color_records
    }

    /// The `colorRecordIndices` array: each palette's first record
    /// index into the color-records array.
    pub fn color_record_indices(&self) -> &[u16] {
        &self.color_record_indices
    }

    /// Read color record number `record_index` from the combined
    /// records array, or `None` when out of range.
    pub fn color_record(&self, record_index: u16) -> Option<ColorRecord> {
        if record_index >= self.num_color_records {
            return None;
        }
        let at = self.first_color_record_offset + record_index as usize * 4;
        // In bounds by the parse-time check.
        Some(ColorRecord {
            blue: read_u8(self.data, at).ok()?,
            green: read_u8(self.data, at + 1).ok()?,
            red: read_u8(self.data, at + 2).ok()?,
            alpha: read_u8(self.data, at + 3).ok()?,
        })
    }

    /// The color for `(palette_index, entry_index)` —
    /// `colorRecordIndices[paletteIndex] + paletteEntryIndex` into the
    /// records array — or `None` when either index is out of range.
    ///
    /// `entry_index` 0xFFFF (the `COLR` foreground sentinel) is out of
    /// range by construction (`numPaletteEntries <= 0xFFFF - 1`);
    /// resolve it to the application foreground color before calling.
    pub fn color(&self, palette_index: u16, entry_index: u16) -> Option<ColorRecord> {
        if entry_index >= self.num_palette_entries {
            return None;
        }
        let first = *self.color_record_indices.get(palette_index as usize)?;
        // first + entry <= max(indices) + numPaletteEntries - 1
        // < numColorRecords, guaranteed at parse time, but recompute
        // defensively in u32.
        let record = first as u32 + entry_index as u32;
        self.color_record(u16::try_from(record).ok()?)
    }

    /// All entries of one palette, in entry order, or `None` when
    /// `palette_index` is out of range.
    pub fn palette(&self, palette_index: u16) -> Option<Vec<ColorRecord>> {
        if palette_index >= self.num_palettes {
            return None;
        }
        (0..self.num_palette_entries)
            .map(|e| self.color(palette_index, e))
            .collect()
    }

    /// The version-1 `paletteTypes` flags for a palette. `None` when
    /// the table has no Palette Type Array or the index is out of
    /// range.
    pub fn palette_type(&self, palette_index: u16) -> Option<PaletteType> {
        if palette_index >= self.num_palettes {
            return None;
        }
        let base = self.palette_types_offset?;
        read_u32(self.data, base + palette_index as usize * 4)
            .ok()
            .map(PaletteType)
    }

    /// The `name`-table ID labeling a palette. `None` when the table
    /// has no Palette Label Array, the array holds the 0xFFFF
    /// no-label sentinel, or the index is out of range.
    pub fn palette_label(&self, palette_index: u16) -> Option<u16> {
        if palette_index >= self.num_palettes {
            return None;
        }
        let base = self.palette_labels_offset?;
        let id = read_u16(self.data, base + palette_index as usize * 2).ok()?;
        (id != NO_NAME_ID).then_some(id)
    }

    /// The `name`-table ID labeling a palette **entry** (e.g.
    /// "Outline", "Fill"); the set applies to all palettes. `None`
    /// when the table has no Palette Entry Label Array, the array
    /// holds the 0xFFFF no-label sentinel, or the index is out of
    /// range.
    pub fn palette_entry_label(&self, entry_index: u16) -> Option<u16> {
        if entry_index >= self.num_palette_entries {
            return None;
        }
        let base = self.palette_entry_labels_offset?;
        let id = read_u16(self.data, base + entry_index as usize * 2).ok()?;
        (id != NO_NAME_ID).then_some(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn u16b(v: u16) -> [u8; 2] {
        v.to_be_bytes()
    }
    fn u32b(v: u32) -> [u8; 4] {
        v.to_be_bytes()
    }

    /// Minimal v0 table: 2 palettes x 2 entries, 3 records with the
    /// second palette overlapping the first (indices 0 and 1).
    fn v0_table() -> Vec<u8> {
        let mut b = Vec::new();
        b.extend_from_slice(&u16b(0)); // version
        b.extend_from_slice(&u16b(2)); // numPaletteEntries
        b.extend_from_slice(&u16b(2)); // numPalettes
        b.extend_from_slice(&u16b(3)); // numColorRecords
        let records_at = 12 + 2 * 2;
        b.extend_from_slice(&u32b(records_at as u32));
        b.extend_from_slice(&u16b(0)); // palette 0 first record
        b.extend_from_slice(&u16b(1)); // palette 1 first record (overlap)
                                       // Records are BGRA on disk.
        b.extend_from_slice(&[0x01, 0x02, 0x03, 0xFF]); // record 0
        b.extend_from_slice(&[0x0A, 0x0B, 0x0C, 0x80]); // record 1
        b.extend_from_slice(&[0x10, 0x20, 0x30, 0x00]); // record 2
        b
    }

    #[test]
    fn v0_palette_lookup_and_overlap() {
        let bytes = v0_table();
        let t = CpalTable::parse(&bytes).expect("parse");
        assert_eq!(t.version(), 0);
        assert_eq!(t.num_palettes(), 2);
        assert_eq!(t.num_palette_entries(), 2);
        assert_eq!(t.num_color_records(), 3);
        // BGRA on disk -> RGBA out.
        assert_eq!(t.color(0, 0).unwrap().rgba(), [0x03, 0x02, 0x01, 0xFF]);
        assert_eq!(t.color(0, 1).unwrap().rgba(), [0x0C, 0x0B, 0x0A, 0x80]);
        // Palette 1 shares record 1 with palette 0.
        assert_eq!(t.color(1, 0), t.color(0, 1));
        assert_eq!(t.color(1, 1).unwrap().rgba(), [0x30, 0x20, 0x10, 0x00]);
        // Out-of-range entry / palette.
        assert_eq!(t.color(0, 2), None);
        assert_eq!(t.color(2, 0), None);
        // v0 has no v1 arrays.
        assert_eq!(t.palette_type(0), None);
        assert_eq!(t.palette_label(0), None);
        assert_eq!(t.palette_entry_label(0), None);
        // Whole-palette read.
        let p1 = t.palette(1).unwrap();
        assert_eq!(p1.len(), 2);
        assert_eq!(p1[0].alpha_f32(), 128.0 / 255.0);
    }

    #[test]
    fn rejects_empty_and_underdeclared_tables() {
        // numPalettes = 0.
        let mut b = v0_table();
        b[4..6].copy_from_slice(&u16b(0));
        assert!(CpalTable::parse(&b).is_err());
        // numPaletteEntries = 0.
        let mut b = v0_table();
        b[2..4].copy_from_slice(&u16b(0));
        assert!(CpalTable::parse(&b).is_err());
        // numColorRecords too small for palette 1's run.
        let mut b = v0_table();
        b[6..8].copy_from_slice(&u16b(2));
        assert!(CpalTable::parse(&b).is_err());
        // Records array out of bounds.
        let b = &v0_table()[..17];
        assert!(CpalTable::parse(b).is_err());
    }
}
