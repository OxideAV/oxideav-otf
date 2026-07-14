//! `SVG ` — the SVG glyph-outline table
//! (ISO/IEC 14496-22:2019 §5.5).
//!
//! Carries SVG 1.1 documents describing some or all glyphs (color,
//! gradients, animation); every SVG glyph must still have a
//! corresponding `CFF `/`glyf` description in the font. The table is
//! a small header pointing at an **SVG Document Index**: sorted,
//! non-overlapping `[startGlyphID, endGlyphID]` ranges, each with the
//! offset/length of its document. Multiple index entries may share
//! one document; within a document, the glyph description for glyph
//! N is the element with id `glyph<N>` (non-zero-padded decimal).
//!
//! Documents are UTF-8, either plain text or gzip-encoded (RFC 1952;
//! `svgDocLength` counts the **encoded** bytes) —
//! [`SvgDocument::is_gzip`] answers which. This crate surfaces the
//! raw document bytes; XML parsing / decompression / rendering belong
//! to higher layers.
//!
//! Color handling (§5.5.2): documents may use color variables
//! `--color<i>` bound from a `CPAL` palette (entry `i` of the
//! selected palette, `i < numPaletteEntries`); the text foreground
//! color is expressed through the `context-fill` /
//! `context-fill-opacity` attributes. The default (first) palette's
//! colors must equal the documents' default variable values.

use crate::parser::{read_u16, read_u32};
use crate::Error;

/// One SVG Document Index entry, resolved to its document bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SvgDocument<'a> {
    /// First glyph ID the document covers.
    pub start_glyph_id: u16,
    /// Last glyph ID the document covers (inclusive).
    pub end_glyph_id: u16,
    /// The document bytes (plain-text or gzip-encoded UTF-8 SVG).
    pub data: &'a [u8],
}

impl SvgDocument<'_> {
    /// Whether the document is gzip-encoded (RFC 1952 magic
    /// `1F 8B`). Plain-text documents return `false`.
    pub fn is_gzip(&self) -> bool {
        self.data.starts_with(&[0x1F, 0x8B])
    }

    /// The element id that carries `glyph_id`'s description inside
    /// this document: `glyph<ID>` with the ID as a non-zero-padded
    /// decimal number.
    pub fn glyph_element_id(glyph_id: u16) -> String {
        format!("glyph{glyph_id}")
    }
}

/// An index entry: a glyph range and its document's byte range
/// (absolute within the `SVG ` table).
#[derive(Debug, Clone, Copy)]
struct DocRecord {
    start_glyph_id: u16,
    end_glyph_id: u16,
    doc_start: usize,
    doc_len: usize,
}

/// A parsed `SVG ` table.
#[derive(Debug)]
pub struct SvgTable<'a> {
    data: &'a [u8],
    version: u16,
    entries: Vec<DocRecord>,
}

impl<'a> SvgTable<'a> {
    /// Parse an `SVG ` table, enforcing the §5.5.1 index invariants:
    /// non-zero entry count, ascending `startGlyphID` with
    /// `start > previous end` (no overlap), `endGlyphID >=
    /// startGlyphID`, and non-zero, in-bounds document offsets and
    /// lengths.
    pub fn parse(data: &'a [u8]) -> Result<Self, Error> {
        let version = read_u16(data, 0)?;
        let doc_index_offset = read_u32(data, 2)? as usize;
        // reserved u32 at offset 6 — "Set to 0", not validated (a
        // reader must tolerate unknown future use).
        if doc_index_offset == 0 {
            return Err(Error::BadStructure(
                "SVG: svgDocIndexOffset must be non-zero",
            ));
        }
        let num_entries = read_u16(data, doc_index_offset)? as usize;
        if num_entries == 0 {
            return Err(Error::BadStructure("SVG: document index must be non-empty"));
        }
        if num_entries > data.len() / 12 {
            return Err(Error::BadStructure("SVG: numEntries exceeds table size"));
        }
        let mut entries = Vec::with_capacity(num_entries);
        let mut prev_end: Option<u16> = None;
        for i in 0..num_entries {
            let at = doc_index_offset + 2 + i * 12;
            let start_glyph_id = read_u16(data, at)?;
            let end_glyph_id = read_u16(data, at + 2)?;
            let doc_offset = read_u32(data, at + 4)? as usize;
            let doc_len = read_u32(data, at + 8)? as usize;
            if end_glyph_id < start_glyph_id {
                return Err(Error::BadStructure("SVG: endGlyphID < startGlyphID"));
            }
            if let Some(prev) = prev_end {
                if start_glyph_id <= prev {
                    return Err(Error::BadStructure(
                        "SVG: index entries must ascend without overlap",
                    ));
                }
            }
            prev_end = Some(end_glyph_id);
            if doc_offset == 0 || doc_len == 0 {
                return Err(Error::BadStructure(
                    "SVG: svgDocOffset and svgDocLength must be non-zero",
                ));
            }
            // svgDocOffset is relative to the SVG Document Index.
            let doc_start = doc_index_offset
                .checked_add(doc_offset)
                .ok_or(Error::BadOffset)?;
            let doc_end = doc_start.checked_add(doc_len).ok_or(Error::BadOffset)?;
            if doc_end > data.len() {
                return Err(Error::BadOffset);
            }
            entries.push(DocRecord {
                start_glyph_id,
                end_glyph_id,
                doc_start,
                doc_len,
            });
        }
        Ok(Self {
            data,
            version,
            entries,
        })
    }

    /// The table version (0).
    pub fn version(&self) -> u16 {
        self.version
    }

    /// Number of SVG Document Index entries.
    pub fn num_entries(&self) -> usize {
        self.entries.len()
    }

    /// All index entries resolved to documents, in index order.
    pub fn documents(&self) -> impl Iterator<Item = SvgDocument<'a>> + '_ {
        self.entries.iter().map(|e| self.resolve(e))
    }

    /// The document covering `glyph_id`, or `None` when no index
    /// entry's range contains it. (Entries are sorted by
    /// `startGlyphID`, so this is a binary search.)
    pub fn document_for_glyph(&self, glyph_id: u16) -> Option<SvgDocument<'a>> {
        let i = match self
            .entries
            .binary_search_by_key(&glyph_id, |e| e.start_glyph_id)
        {
            Ok(i) => i,
            Err(0) => return None,
            Err(i) => i - 1,
        };
        let e = &self.entries[i];
        (glyph_id >= e.start_glyph_id && glyph_id <= e.end_glyph_id).then(|| self.resolve(e))
    }

    /// Whether the table carries an SVG description for `glyph_id`.
    pub fn has_glyph(&self, glyph_id: u16) -> bool {
        self.document_for_glyph(glyph_id).is_some()
    }

    fn resolve(&self, e: &DocRecord) -> SvgDocument<'a> {
        SvgDocument {
            start_glyph_id: e.start_glyph_id,
            end_glyph_id: e.end_glyph_id,
            // In bounds by the parse-time check.
            data: &self.data[e.doc_start..e.doc_start + e.doc_len],
        }
    }
}
