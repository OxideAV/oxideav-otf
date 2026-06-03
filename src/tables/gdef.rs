//! `GDEF` — Glyph Definition Table.
//!
//! Spec: Microsoft / ISO/IEC 14496-22 OpenType `GDEF` table
//! (`docs/text/opentype/otspec-gdef.html`), with the [`Coverage`] and
//! [`ClassDef`] structures pulled from
//! `docs/text/opentype/otspec-chapter2-common-layout-tables.html`.
//!
//! GDEF carries per-glyph metadata that GSUB / GPOS lookups consult
//! when shaping a glyph sequence:
//!
//! * **GlyphClassDef** — assigns every glyph to one of base / ligature /
//!   mark / component (the spec's four glyph-class numbers; absent
//!   glyphs fall into class 0). Used by GSUB / GPOS to honour
//!   `LookupFlag` bits that ignore marks / ligatures / etc.
//! * **AttachList** — caches the contour-point indices that GPOS
//!   attachment lookups would otherwise have to re-derive on every
//!   text run.
//! * **LigCaretList** — per-ligature caret X / Y coordinates for
//!   selection and hit-testing within a ligature.
//! * **MarkAttachClassDef** — partitions mark glyphs into mutually
//!   exclusive classes that `LookupFlag.markAttachmentType` filters
//!   on.
//! * **MarkGlyphSets** (v1.2+) — overlapping mark-glyph sets that
//!   `LookupFlag.useMarkFilteringSet` filters on.
//! * **ItemVariationStore** (v1.3+) — variation deltas for caret X / Y
//!   coordinates in variable fonts (and, via the same store, the
//!   GPOS and JSTF variation data). This implementation surfaces only
//!   the raw offset; decoding the store itself is deferred.
//!
//! Header layout (per the spec's three "GDEF Header" tables):
//!
//! ```text
//!   0  / 2 / majorVersion (= 1)
//!   2  / 2 / minorVersion (= 0, 2, or 3)
//!   4  / 2 / glyphClassDefOffset   (Offset16, may be NULL)
//!   6  / 2 / attachListOffset      (Offset16, may be NULL)
//!   8  / 2 / ligCaretListOffset    (Offset16, may be NULL)
//!  10  / 2 / markAttachClassDefOffset (Offset16, may be NULL)
//!  -- v1.2+ --
//!  12  / 2 / markGlyphSetsDefOffset (Offset16, may be NULL)
//!  -- v1.3+ --
//!  14  / 4 / itemVarStoreOffset    (Offset32, may be NULL)
//! ```
//!
//! Every "offset" field is measured from the start of the GDEF table
//! and may be NULL (`0`) to indicate the sub-table is absent.
//!
//! This module is **read-only** — every accessor walks the borrowed
//! `&[u8]` once and returns owned primitives or sub-slices.

use crate::parser::{read_i16, read_u16, read_u32};
use crate::Error;

// -- Coverage table (shared with GSUB / GPOS) -----------------------------

/// Parsed `Coverage` table (chapter 2, Common Layout Table Formats).
///
/// A Coverage table maps an arbitrary set of glyph IDs to a dense
/// [`Coverage Index`](Self::index_of) starting at zero. The dense index
/// is the position the rest of the lookup uses to look up per-glyph
/// data. Two on-disk formats are defined:
///
/// * **Format 1** — a sorted list of individual glyph IDs.
///   `Coverage Index = position in the list`.
/// * **Format 2** — a sorted list of `(startGlyphID, endGlyphID,
///   startCoverageIndex)` triples. For a glyph `g` in `[start, end]`,
///   `Coverage Index = startCoverageIndex + g - startGlyphID`.
///
/// Both forms are kept as borrowed byte slices and decoded on every
/// query so the type stays cheap to copy.
#[derive(Debug, Clone, Copy)]
pub struct Coverage<'a> {
    inner: CoverageInner<'a>,
}

#[derive(Debug, Clone, Copy)]
enum CoverageInner<'a> {
    /// Sorted list of `glyphCount` `u16` glyph IDs.
    Format1 { glyphs: &'a [u8] },
    /// Sorted list of `rangeCount` 6-byte `(start, end, startIdx)` records.
    Format2 { ranges: &'a [u8] },
}

impl<'a> Coverage<'a> {
    /// Parse a Coverage table from a buffer whose first byte is the
    /// format identifier.
    pub fn parse(data: &'a [u8]) -> Result<Self, Error> {
        let format = read_u16(data, 0)?;
        match format {
            1 => {
                let count = read_u16(data, 2)? as usize;
                let need = 4usize.checked_add(count * 2).ok_or(Error::BadStructure(
                    "GDEF/Coverage format 1 length overflow",
                ))?;
                if data.len() < need {
                    return Err(Error::UnexpectedEof);
                }
                Ok(Self {
                    inner: CoverageInner::Format1 {
                        glyphs: &data[4..need],
                    },
                })
            }
            2 => {
                let count = read_u16(data, 2)? as usize;
                let need = 4usize.checked_add(count * 6).ok_or(Error::BadStructure(
                    "GDEF/Coverage format 2 length overflow",
                ))?;
                if data.len() < need {
                    return Err(Error::UnexpectedEof);
                }
                Ok(Self {
                    inner: CoverageInner::Format2 {
                        ranges: &data[4..need],
                    },
                })
            }
            _ => Err(Error::BadStructure(
                "GDEF/Coverage table has unknown format",
            )),
        }
    }

    /// Coverage format discriminant (1 = glyph list, 2 = range list).
    pub fn format(&self) -> u16 {
        match self.inner {
            CoverageInner::Format1 { .. } => 1,
            CoverageInner::Format2 { .. } => 2,
        }
    }

    /// Total number of glyph IDs covered by the table. Equivalent to
    /// `1 + max(coverage_index)` over every glyph the table covers.
    pub fn len(&self) -> usize {
        match self.inner {
            CoverageInner::Format1 { glyphs } => glyphs.len() / 2,
            CoverageInner::Format2 { ranges } => {
                let mut total = 0usize;
                for chunk in ranges.chunks_exact(6) {
                    let start = u16::from_be_bytes([chunk[0], chunk[1]]) as i32;
                    let end = u16::from_be_bytes([chunk[2], chunk[3]]) as i32;
                    if end >= start {
                        total = total.saturating_add((end - start + 1) as usize);
                    }
                }
                total
            }
        }
    }

    /// `true` if the table covers zero glyphs.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Coverage Index for `glyph_id`, or `None` if not covered.
    ///
    /// Format 1 binary-searches a sorted `u16` array; format 2
    /// binary-searches a sorted range list and computes the index
    /// arithmetically per the spec's `startCoverageIndex + g - startGlyphID`
    /// formula.
    pub fn index_of(&self, glyph_id: u16) -> Option<u16> {
        match self.inner {
            CoverageInner::Format1 { glyphs } => {
                let n = glyphs.len() / 2;
                let mut lo = 0usize;
                let mut hi = n;
                while lo < hi {
                    let mid = (lo + hi) / 2;
                    let g = u16::from_be_bytes([glyphs[mid * 2], glyphs[mid * 2 + 1]]);
                    match g.cmp(&glyph_id) {
                        core::cmp::Ordering::Equal => return Some(mid as u16),
                        core::cmp::Ordering::Less => lo = mid + 1,
                        core::cmp::Ordering::Greater => hi = mid,
                    }
                }
                None
            }
            CoverageInner::Format2 { ranges } => {
                let n = ranges.len() / 6;
                let mut lo = 0usize;
                let mut hi = n;
                while lo < hi {
                    let mid = (lo + hi) / 2;
                    let off = mid * 6;
                    let start = u16::from_be_bytes([ranges[off], ranges[off + 1]]);
                    let end = u16::from_be_bytes([ranges[off + 2], ranges[off + 3]]);
                    if glyph_id < start {
                        hi = mid;
                    } else if glyph_id > end {
                        lo = mid + 1;
                    } else {
                        let start_idx = u16::from_be_bytes([ranges[off + 4], ranges[off + 5]]);
                        return Some(start_idx + (glyph_id - start));
                    }
                }
                None
            }
        }
    }

    /// `true` if `glyph_id` is covered by the table.
    pub fn contains(&self, glyph_id: u16) -> bool {
        self.index_of(glyph_id).is_some()
    }

    /// Iterate every covered glyph ID in sorted (ascending) order.
    ///
    /// The iterator yields `(glyph_id, coverage_index)` pairs. For a
    /// well-formed Coverage table the second element of consecutive
    /// pairs increases by exactly 1.
    pub fn iter(&self) -> CoverageIter<'a> {
        CoverageIter {
            inner: self.inner,
            pos: 0,
            sub: 0,
        }
    }
}

/// Iterator over the `(glyph_id, coverage_index)` pairs of a [`Coverage`].
#[derive(Debug, Clone)]
pub struct CoverageIter<'a> {
    inner: CoverageInner<'a>,
    pos: usize,
    sub: u16,
}

impl<'a> Iterator for CoverageIter<'a> {
    type Item = (u16, u16);
    fn next(&mut self) -> Option<Self::Item> {
        match self.inner {
            CoverageInner::Format1 { glyphs } => {
                let n = glyphs.len() / 2;
                if self.pos >= n {
                    return None;
                }
                let g = u16::from_be_bytes([glyphs[self.pos * 2], glyphs[self.pos * 2 + 1]]);
                let idx = self.pos as u16;
                self.pos += 1;
                Some((g, idx))
            }
            CoverageInner::Format2 { ranges } => loop {
                let n = ranges.len() / 6;
                if self.pos >= n {
                    return None;
                }
                let off = self.pos * 6;
                let start = u16::from_be_bytes([ranges[off], ranges[off + 1]]);
                let end = u16::from_be_bytes([ranges[off + 2], ranges[off + 3]]);
                let start_idx = u16::from_be_bytes([ranges[off + 4], ranges[off + 5]]);
                let g = start + self.sub;
                if g > end {
                    self.pos += 1;
                    self.sub = 0;
                    continue;
                }
                let idx = start_idx + self.sub;
                self.sub += 1;
                return Some((g, idx));
            },
        }
    }
}

// -- ClassDef table (shared with GSUB / GPOS) -----------------------------

/// Parsed `ClassDef` table (chapter 2, Common Layout Table Formats).
///
/// A ClassDef table assigns each glyph in the font to one of several
/// integer classes. Any glyph not covered by the on-disk records is
/// implicitly class 0. Two on-disk formats are defined:
///
/// * **Format 1** — `(startGlyphID, glyphCount, classValues[glyphCount])`.
/// * **Format 2** — sorted, non-overlapping
///   `(startGlyphID, endGlyphID, class)` triples.
#[derive(Debug, Clone, Copy)]
pub struct ClassDef<'a> {
    inner: ClassDefInner<'a>,
}

#[derive(Debug, Clone, Copy)]
enum ClassDefInner<'a> {
    /// Format 1: dense `classValues` array starting at `start`.
    Format1 { start: u16, values: &'a [u8] },
    /// Format 2: sorted list of 6-byte `(start, end, class)` records.
    Format2 { ranges: &'a [u8] },
}

impl<'a> ClassDef<'a> {
    /// Parse a ClassDef table.
    pub fn parse(data: &'a [u8]) -> Result<Self, Error> {
        let format = read_u16(data, 0)?;
        match format {
            1 => {
                let start = read_u16(data, 2)?;
                let count = read_u16(data, 4)? as usize;
                let need = 6usize.checked_add(count * 2).ok_or(Error::BadStructure(
                    "GDEF/ClassDef format 1 length overflow",
                ))?;
                if data.len() < need {
                    return Err(Error::UnexpectedEof);
                }
                Ok(Self {
                    inner: ClassDefInner::Format1 {
                        start,
                        values: &data[6..need],
                    },
                })
            }
            2 => {
                let count = read_u16(data, 2)? as usize;
                let need = 4usize.checked_add(count * 6).ok_or(Error::BadStructure(
                    "GDEF/ClassDef format 2 length overflow",
                ))?;
                if data.len() < need {
                    return Err(Error::UnexpectedEof);
                }
                Ok(Self {
                    inner: ClassDefInner::Format2 {
                        ranges: &data[4..need],
                    },
                })
            }
            _ => Err(Error::BadStructure(
                "GDEF/ClassDef table has unknown format",
            )),
        }
    }

    /// ClassDef format discriminant (1 = dense array, 2 = sorted range list).
    pub fn format(&self) -> u16 {
        match self.inner {
            ClassDefInner::Format1 { .. } => 1,
            ClassDefInner::Format2 { .. } => 2,
        }
    }

    /// Class number assigned to `glyph_id`. Glyphs not covered by the
    /// on-disk records implicitly belong to class 0.
    pub fn class_of(&self, glyph_id: u16) -> u16 {
        match self.inner {
            ClassDefInner::Format1 { start, values } => {
                if glyph_id < start {
                    return 0;
                }
                let off = (glyph_id - start) as usize;
                let count = values.len() / 2;
                if off >= count {
                    return 0;
                }
                u16::from_be_bytes([values[off * 2], values[off * 2 + 1]])
            }
            ClassDefInner::Format2 { ranges } => {
                let n = ranges.len() / 6;
                let mut lo = 0usize;
                let mut hi = n;
                while lo < hi {
                    let mid = (lo + hi) / 2;
                    let off = mid * 6;
                    let start = u16::from_be_bytes([ranges[off], ranges[off + 1]]);
                    let end = u16::from_be_bytes([ranges[off + 2], ranges[off + 3]]);
                    if glyph_id < start {
                        hi = mid;
                    } else if glyph_id > end {
                        lo = mid + 1;
                    } else {
                        return u16::from_be_bytes([ranges[off + 4], ranges[off + 5]]);
                    }
                }
                0
            }
        }
    }
}

// -- GDEF table -----------------------------------------------------------

/// The four standard glyph classes assigned by GDEF's `GlyphClassDef`
/// table (spec's "GlyphClassDef enumeration"). Other class numbers are
/// reserved.
#[repr(u16)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GlyphClass {
    /// Class 1 — single-character, spacing base glyph.
    Base = 1,
    /// Class 2 — multi-character spacing ligature glyph.
    Ligature = 2,
    /// Class 3 — non-spacing combining mark glyph.
    Mark = 3,
    /// Class 4 — part of a single character (a sub-glyph component
    /// used inside ligatures).
    Component = 4,
}

impl GlyphClass {
    /// Decode the spec's GlyphClassDef enumeration into a typed enum.
    /// Returns `None` for class 0 (the "unclassified" default) and for
    /// any class number outside the spec-defined `1..=4`.
    pub fn from_raw(raw: u16) -> Option<Self> {
        match raw {
            1 => Some(Self::Base),
            2 => Some(Self::Ligature),
            3 => Some(Self::Mark),
            4 => Some(Self::Component),
            _ => None,
        }
    }
}

/// One caret position for a ligature glyph (LigCaretList).
///
/// The spec defines three formats; format 1 carries a design-unit
/// coordinate, format 2 references a contour-point index, format 3
/// carries a design-unit coordinate plus an offset to a Device or
/// VariationIndex table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaretValue {
    /// CaretValueFormat1 — `(coordinate)`. X or Y value in design units.
    DesignUnits(i16),
    /// CaretValueFormat2 — `(contour_point_index)`. Index into the
    /// glyph's contour-point list whose final hinted position the
    /// rasterizer uses as the caret coordinate.
    ContourPoint(u16),
    /// CaretValueFormat3 — `(coordinate, device_offset)`. Design-unit
    /// value plus a Device-or-VariationIndex offset from the start of
    /// the CaretValue table itself. The device-table offset may be 0
    /// to indicate "no device adjustment / use the bare coordinate."
    DesignUnitsWithDevice { coordinate: i16, device_offset: u16 },
}

/// Parsed `GDEF` table.
///
/// All offsets in the original header are kept as borrowed sub-slices
/// against the table buffer so per-sub-table accessors are cheap.
#[derive(Debug)]
pub struct GdefTable<'a> {
    bytes: &'a [u8],
    major: u16,
    minor: u16,
    glyph_class_def_off: u16,
    attach_list_off: u16,
    lig_caret_list_off: u16,
    mark_attach_class_def_off: u16,
    mark_glyph_sets_def_off: u16,
    item_var_store_off: u32,
}

impl<'a> GdefTable<'a> {
    /// Parse a `GDEF` table from a borrowed byte slice.
    pub fn parse(bytes: &'a [u8]) -> Result<Self, Error> {
        if bytes.len() < 12 {
            return Err(Error::UnexpectedEof);
        }
        let major = read_u16(bytes, 0)?;
        let minor = read_u16(bytes, 2)?;
        if major != 1 {
            return Err(Error::BadStructure("GDEF: majorVersion != 1"));
        }
        if minor != 0 && minor != 2 && minor != 3 {
            return Err(Error::BadStructure("GDEF: minorVersion not in {0, 2, 3}"));
        }
        let glyph_class_def_off = read_u16(bytes, 4)?;
        let attach_list_off = read_u16(bytes, 6)?;
        let lig_caret_list_off = read_u16(bytes, 8)?;
        let mark_attach_class_def_off = read_u16(bytes, 10)?;

        let mut mark_glyph_sets_def_off = 0u16;
        let mut item_var_store_off = 0u32;

        if minor >= 2 {
            if bytes.len() < 14 {
                return Err(Error::UnexpectedEof);
            }
            mark_glyph_sets_def_off = read_u16(bytes, 12)?;
        }
        if minor >= 3 {
            if bytes.len() < 18 {
                return Err(Error::UnexpectedEof);
            }
            item_var_store_off = read_u32(bytes, 14)?;
        }

        // Bounds-check every non-NULL offset against the table buffer.
        for &off in &[
            glyph_class_def_off,
            attach_list_off,
            lig_caret_list_off,
            mark_attach_class_def_off,
            mark_glyph_sets_def_off,
        ] {
            if off != 0 && (off as usize) >= bytes.len() {
                return Err(Error::BadOffset);
            }
        }
        if item_var_store_off != 0 && (item_var_store_off as usize) >= bytes.len() {
            return Err(Error::BadOffset);
        }

        Ok(Self {
            bytes,
            major,
            minor,
            glyph_class_def_off,
            attach_list_off,
            lig_caret_list_off,
            mark_attach_class_def_off,
            mark_glyph_sets_def_off,
            item_var_store_off,
        })
    }

    /// `majorVersion.minorVersion` as a `(u16, u16)` pair.
    pub fn version(&self) -> (u16, u16) {
        (self.major, self.minor)
    }

    /// `true` if the table is at least version 1.2 (has the
    /// `markGlyphSetsDefOffset` field).
    pub fn has_mark_glyph_sets(&self) -> bool {
        self.minor >= 2
    }

    /// `true` if the table is at least version 1.3 (has the
    /// `itemVarStoreOffset` field).
    pub fn has_item_var_store(&self) -> bool {
        self.minor >= 3
    }

    /// Raw byte offset within the GDEF table for the item-variation
    /// store; 0 if absent or pre-v1.3. The store itself is left raw —
    /// `ItemVariationStore` decoding lives in a future round.
    pub fn item_var_store_offset(&self) -> u32 {
        self.item_var_store_off
    }

    // -- GlyphClassDef sub-table ------------------------------------------

    /// Parsed `GlyphClassDef` sub-table, if present.
    ///
    /// The sub-table is a generic [`ClassDef`] keyed on glyph ID; the
    /// class values it returns are the spec's GlyphClassDef enumeration
    /// (1 = base, 2 = ligature, 3 = mark, 4 = component, 0 = unassigned).
    pub fn glyph_class_def(&self) -> Option<ClassDef<'a>> {
        if self.glyph_class_def_off == 0 {
            return None;
        }
        ClassDef::parse(&self.bytes[self.glyph_class_def_off as usize..]).ok()
    }

    /// Spec [`GlyphClass`] for `glyph_id`, or `None` for an
    /// unclassified glyph (class 0 — the default) and for any class
    /// number outside the spec-defined `1..=4` range.
    pub fn glyph_class(&self, glyph_id: u16) -> Option<GlyphClass> {
        GlyphClass::from_raw(self.glyph_class_def()?.class_of(glyph_id))
    }

    // -- MarkAttachClassDef sub-table -------------------------------------

    /// Parsed `MarkAttachClassDef` sub-table, if present.
    ///
    /// Same on-disk format as any [`ClassDef`]; the class values are
    /// font-author defined (no spec enumeration). `LookupFlag.
    /// markAttachmentType` filters on the class number returned here.
    pub fn mark_attach_class_def(&self) -> Option<ClassDef<'a>> {
        if self.mark_attach_class_def_off == 0 {
            return None;
        }
        ClassDef::parse(&self.bytes[self.mark_attach_class_def_off as usize..]).ok()
    }

    /// Convenience: mark-attach class number for `glyph_id` (0 if
    /// unclassified or if the table is absent).
    pub fn mark_attach_class(&self, glyph_id: u16) -> u16 {
        self.mark_attach_class_def()
            .map(|c| c.class_of(glyph_id))
            .unwrap_or(0)
    }

    // -- AttachList sub-table ---------------------------------------------

    /// Parsed `AttachList` sub-table, if present.
    pub fn attach_list(&self) -> Option<AttachList<'a>> {
        if self.attach_list_off == 0 {
            return None;
        }
        AttachList::parse(&self.bytes[self.attach_list_off as usize..]).ok()
    }

    // -- LigCaretList sub-table -------------------------------------------

    /// Parsed `LigCaretList` sub-table, if present.
    pub fn lig_caret_list(&self) -> Option<LigCaretList<'a>> {
        if self.lig_caret_list_off == 0 {
            return None;
        }
        LigCaretList::parse(&self.bytes[self.lig_caret_list_off as usize..]).ok()
    }

    // -- MarkGlyphSets sub-table (v1.2+) ----------------------------------

    /// Parsed `MarkGlyphSets` sub-table, if present. Returns `None`
    /// for pre-v1.2 tables and for v1.2+ tables whose offset is NULL.
    pub fn mark_glyph_sets(&self) -> Option<MarkGlyphSets<'a>> {
        if self.mark_glyph_sets_def_off == 0 {
            return None;
        }
        MarkGlyphSets::parse(&self.bytes[self.mark_glyph_sets_def_off as usize..]).ok()
    }
}

// -- AttachList ----------------------------------------------------------

/// Parsed `AttachList` sub-table.
///
/// The on-disk layout is
/// `(coverageOffset, glyphCount, attachPointOffsets[glyphCount])`,
/// every offset measured from the start of the AttachList itself. Each
/// `attachPointOffsets[i]` points at a tiny AttachPoint record listing
/// the contour-point indices for the glyph whose Coverage Index is `i`.
#[derive(Debug, Clone)]
pub struct AttachList<'a> {
    bytes: &'a [u8],
    coverage_off: u16,
    glyph_count: u16,
}

impl<'a> AttachList<'a> {
    fn parse(data: &'a [u8]) -> Result<Self, Error> {
        if data.len() < 4 {
            return Err(Error::UnexpectedEof);
        }
        let coverage_off = read_u16(data, 0)?;
        let glyph_count = read_u16(data, 2)?;
        let need = 4usize
            .checked_add(glyph_count as usize * 2)
            .ok_or(Error::BadStructure("GDEF/AttachList length overflow"))?;
        if data.len() < need {
            return Err(Error::UnexpectedEof);
        }
        if coverage_off != 0 && (coverage_off as usize) >= data.len() {
            return Err(Error::BadOffset);
        }
        Ok(Self {
            bytes: data,
            coverage_off,
            glyph_count,
        })
    }

    /// Number of glyphs that carry attachment points.
    pub fn glyph_count(&self) -> u16 {
        self.glyph_count
    }

    /// Coverage table listing the glyphs that carry attachment points.
    /// Returns `None` if the on-disk `coverageOffset` is NULL (a
    /// well-formed font never emits this, but the spec uses
    /// "may be NULL" wording for every offset).
    pub fn coverage(&self) -> Option<Coverage<'a>> {
        if self.coverage_off == 0 {
            return None;
        }
        Coverage::parse(&self.bytes[self.coverage_off as usize..]).ok()
    }

    /// Borrow the attach-point record for `glyph_id` (resolved via the
    /// Coverage table). Returns `None` if the glyph isn't covered.
    pub fn attach_points(&self, glyph_id: u16) -> Option<AttachPoint<'a>> {
        let idx = self.coverage()?.index_of(glyph_id)?;
        self.attach_points_by_index(idx)
    }

    /// Borrow the attach-point record at Coverage Index `idx`.
    pub fn attach_points_by_index(&self, idx: u16) -> Option<AttachPoint<'a>> {
        if idx >= self.glyph_count {
            return None;
        }
        let off_base = 4 + (idx as usize) * 2;
        let off = u16::from_be_bytes([self.bytes[off_base], self.bytes[off_base + 1]]) as usize;
        if off == 0 {
            return None;
        }
        let buf = self.bytes.get(off..)?;
        AttachPoint::parse(buf).ok()
    }
}

/// Borrowed view of a single AttachPoint table.
#[derive(Debug, Clone, Copy)]
pub struct AttachPoint<'a> {
    indices: &'a [u8],
}

impl<'a> AttachPoint<'a> {
    fn parse(data: &'a [u8]) -> Result<Self, Error> {
        if data.len() < 2 {
            return Err(Error::UnexpectedEof);
        }
        let count = read_u16(data, 0)? as usize;
        let need = 2usize
            .checked_add(count * 2)
            .ok_or(Error::BadStructure("GDEF/AttachPoint length overflow"))?;
        if data.len() < need {
            return Err(Error::UnexpectedEof);
        }
        Ok(Self {
            indices: &data[2..need],
        })
    }

    /// Number of attachment points on this glyph.
    pub fn len(&self) -> usize {
        self.indices.len() / 2
    }

    /// `true` if the glyph has no attachment points (the on-disk
    /// `pointCount` is zero).
    pub fn is_empty(&self) -> bool {
        self.indices.is_empty()
    }

    /// `i`-th contour-point index in spec-sorted (increasing) order.
    pub fn get(&self, i: usize) -> Option<u16> {
        let off = i.checked_mul(2)?;
        let bytes = self.indices.get(off..off + 2)?;
        Some(u16::from_be_bytes([bytes[0], bytes[1]]))
    }

    /// Iterate every contour-point index in spec-sorted order.
    pub fn iter(&self) -> impl Iterator<Item = u16> + '_ {
        self.indices
            .chunks_exact(2)
            .map(|c| u16::from_be_bytes([c[0], c[1]]))
    }
}

// -- LigCaretList --------------------------------------------------------

/// Parsed `LigCaretList` sub-table.
#[derive(Debug, Clone)]
pub struct LigCaretList<'a> {
    bytes: &'a [u8],
    coverage_off: u16,
    lig_glyph_count: u16,
}

impl<'a> LigCaretList<'a> {
    fn parse(data: &'a [u8]) -> Result<Self, Error> {
        if data.len() < 4 {
            return Err(Error::UnexpectedEof);
        }
        let coverage_off = read_u16(data, 0)?;
        let lig_glyph_count = read_u16(data, 2)?;
        let need = 4usize
            .checked_add(lig_glyph_count as usize * 2)
            .ok_or(Error::BadStructure("GDEF/LigCaretList length overflow"))?;
        if data.len() < need {
            return Err(Error::UnexpectedEof);
        }
        if coverage_off != 0 && (coverage_off as usize) >= data.len() {
            return Err(Error::BadOffset);
        }
        Ok(Self {
            bytes: data,
            coverage_off,
            lig_glyph_count,
        })
    }

    /// Number of ligature glyphs with caret data.
    pub fn lig_glyph_count(&self) -> u16 {
        self.lig_glyph_count
    }

    /// Coverage table listing the ligature glyphs that have caret data.
    pub fn coverage(&self) -> Option<Coverage<'a>> {
        if self.coverage_off == 0 {
            return None;
        }
        Coverage::parse(&self.bytes[self.coverage_off as usize..]).ok()
    }

    /// Borrow the LigGlyph record for `glyph_id`, or `None` if the
    /// ligature isn't covered.
    pub fn lig_glyph(&self, glyph_id: u16) -> Option<LigGlyph<'a>> {
        let idx = self.coverage()?.index_of(glyph_id)?;
        self.lig_glyph_by_index(idx)
    }

    /// Borrow the LigGlyph at Coverage Index `idx`.
    pub fn lig_glyph_by_index(&self, idx: u16) -> Option<LigGlyph<'a>> {
        if idx >= self.lig_glyph_count {
            return None;
        }
        let off_base = 4 + (idx as usize) * 2;
        let off = u16::from_be_bytes([self.bytes[off_base], self.bytes[off_base + 1]]) as usize;
        if off == 0 {
            return None;
        }
        let buf = self.bytes.get(off..)?;
        LigGlyph::parse(buf).ok()
    }
}

/// Borrowed view of a single LigGlyph table.
#[derive(Debug, Clone, Copy)]
pub struct LigGlyph<'a> {
    /// Backing bytes start at the LigGlyph table itself.
    bytes: &'a [u8],
    caret_count: u16,
}

impl<'a> LigGlyph<'a> {
    fn parse(data: &'a [u8]) -> Result<Self, Error> {
        if data.len() < 2 {
            return Err(Error::UnexpectedEof);
        }
        let caret_count = read_u16(data, 0)?;
        let need = 2usize
            .checked_add(caret_count as usize * 2)
            .ok_or(Error::BadStructure("GDEF/LigGlyph length overflow"))?;
        if data.len() < need {
            return Err(Error::UnexpectedEof);
        }
        Ok(Self {
            bytes: data,
            caret_count,
        })
    }

    /// Number of caret-value records for this ligature
    /// (= ligature component count − 1).
    pub fn caret_count(&self) -> u16 {
        self.caret_count
    }

    /// Decode the `i`-th caret value (in spec-sorted, increasing-
    /// coordinate order).
    pub fn caret_value(&self, i: usize) -> Option<CaretValue> {
        if i >= self.caret_count as usize {
            return None;
        }
        let off_base = 2 + i * 2;
        let off = u16::from_be_bytes([self.bytes[off_base], self.bytes[off_base + 1]]) as usize;
        if off == 0 {
            return None;
        }
        let buf = self.bytes.get(off..)?;
        parse_caret_value(buf).ok()
    }
}

fn parse_caret_value(data: &[u8]) -> Result<CaretValue, Error> {
    if data.len() < 4 {
        return Err(Error::UnexpectedEof);
    }
    let format = read_u16(data, 0)?;
    match format {
        1 => Ok(CaretValue::DesignUnits(read_i16(data, 2)?)),
        2 => Ok(CaretValue::ContourPoint(read_u16(data, 2)?)),
        3 => {
            if data.len() < 6 {
                return Err(Error::UnexpectedEof);
            }
            Ok(CaretValue::DesignUnitsWithDevice {
                coordinate: read_i16(data, 2)?,
                device_offset: read_u16(data, 4)?,
            })
        }
        _ => Err(Error::BadStructure(
            "GDEF/CaretValue table has unknown format",
        )),
    }
}

// -- MarkGlyphSets (v1.2+) -----------------------------------------------

/// Parsed `MarkGlyphSets` sub-table (v1.2+).
#[derive(Debug, Clone)]
pub struct MarkGlyphSets<'a> {
    bytes: &'a [u8],
    set_count: u16,
}

impl<'a> MarkGlyphSets<'a> {
    fn parse(data: &'a [u8]) -> Result<Self, Error> {
        if data.len() < 4 {
            return Err(Error::UnexpectedEof);
        }
        let format = read_u16(data, 0)?;
        if format != 1 {
            return Err(Error::BadStructure(
                "GDEF/MarkGlyphSets unknown format (expected 1)",
            ));
        }
        let set_count = read_u16(data, 2)?;
        // Per the spec note, the coverage-offset array uses Offset32.
        let need = 4usize
            .checked_add(set_count as usize * 4)
            .ok_or(Error::BadStructure("GDEF/MarkGlyphSets length overflow"))?;
        if data.len() < need {
            return Err(Error::UnexpectedEof);
        }
        Ok(Self {
            bytes: data,
            set_count,
        })
    }

    /// Number of mark glyph sets defined.
    pub fn set_count(&self) -> u16 {
        self.set_count
    }

    /// Borrow the [`Coverage`] table for set index `i`.
    pub fn set(&self, i: usize) -> Option<Coverage<'a>> {
        if i >= self.set_count as usize {
            return None;
        }
        let off_base = 4 + i * 4;
        let off = u32::from_be_bytes([
            self.bytes[off_base],
            self.bytes[off_base + 1],
            self.bytes[off_base + 2],
            self.bytes[off_base + 3],
        ]) as usize;
        if off == 0 {
            return None;
        }
        let buf = self.bytes.get(off..)?;
        Coverage::parse(buf).ok()
    }

    /// `true` if `glyph_id` is a member of mark glyph set `i`.
    pub fn contains(&self, i: usize, glyph_id: u16) -> bool {
        self.set(i).map(|c| c.contains(glyph_id)).unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // -- helpers ----------------------------------------------------------

    fn be16(v: u16) -> [u8; 2] {
        v.to_be_bytes()
    }
    fn be32(v: u32) -> [u8; 4] {
        v.to_be_bytes()
    }

    fn build_coverage_fmt1(glyphs: &[u16]) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(&be16(1));
        v.extend_from_slice(&be16(glyphs.len() as u16));
        for g in glyphs {
            v.extend_from_slice(&be16(*g));
        }
        v
    }

    fn build_coverage_fmt2(ranges: &[(u16, u16, u16)]) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(&be16(2));
        v.extend_from_slice(&be16(ranges.len() as u16));
        for (s, e, idx) in ranges {
            v.extend_from_slice(&be16(*s));
            v.extend_from_slice(&be16(*e));
            v.extend_from_slice(&be16(*idx));
        }
        v
    }

    fn build_classdef_fmt1(start: u16, values: &[u16]) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(&be16(1));
        v.extend_from_slice(&be16(start));
        v.extend_from_slice(&be16(values.len() as u16));
        for cv in values {
            v.extend_from_slice(&be16(*cv));
        }
        v
    }

    fn build_classdef_fmt2(ranges: &[(u16, u16, u16)]) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(&be16(2));
        v.extend_from_slice(&be16(ranges.len() as u16));
        for (s, e, c) in ranges {
            v.extend_from_slice(&be16(*s));
            v.extend_from_slice(&be16(*e));
            v.extend_from_slice(&be16(*c));
        }
        v
    }

    // -- Coverage --------------------------------------------------------

    #[test]
    fn coverage_format1_lookup_and_iter() {
        let raw = build_coverage_fmt1(&[3, 7, 9, 42]);
        let c = Coverage::parse(&raw).unwrap();
        assert_eq!(c.format(), 1);
        assert_eq!(c.len(), 4);
        assert!(!c.is_empty());
        assert_eq!(c.index_of(3), Some(0));
        assert_eq!(c.index_of(7), Some(1));
        assert_eq!(c.index_of(9), Some(2));
        assert_eq!(c.index_of(42), Some(3));
        assert_eq!(c.index_of(0), None);
        assert_eq!(c.index_of(8), None);
        assert_eq!(c.index_of(43), None);
        assert!(c.contains(42));
        assert!(!c.contains(43));
        let items: Vec<_> = c.iter().collect();
        assert_eq!(items, vec![(3, 0), (7, 1), (9, 2), (42, 3)]);
    }

    #[test]
    fn coverage_format2_lookup_and_iter() {
        // Two ranges: [10..=12] -> Coverage 0..=2, [20..=21] -> 3..=4.
        let raw = build_coverage_fmt2(&[(10, 12, 0), (20, 21, 3)]);
        let c = Coverage::parse(&raw).unwrap();
        assert_eq!(c.format(), 2);
        assert_eq!(c.len(), 5);
        assert_eq!(c.index_of(10), Some(0));
        assert_eq!(c.index_of(11), Some(1));
        assert_eq!(c.index_of(12), Some(2));
        assert_eq!(c.index_of(20), Some(3));
        assert_eq!(c.index_of(21), Some(4));
        assert_eq!(c.index_of(9), None);
        assert_eq!(c.index_of(13), None);
        assert_eq!(c.index_of(22), None);
        let items: Vec<_> = c.iter().collect();
        assert_eq!(items, vec![(10, 0), (11, 1), (12, 2), (20, 3), (21, 4)]);
    }

    #[test]
    fn coverage_rejects_unknown_format_and_truncation() {
        let mut bad = vec![0u8; 4];
        bad[0..2].copy_from_slice(&be16(3));
        assert!(matches!(Coverage::parse(&bad), Err(Error::BadStructure(_))));
        let mut trunc = build_coverage_fmt1(&[1, 2, 3]);
        trunc.truncate(trunc.len() - 1);
        assert!(matches!(Coverage::parse(&trunc), Err(Error::UnexpectedEof)));
    }

    // -- ClassDef --------------------------------------------------------

    #[test]
    fn classdef_format1_default_zero_outside_range() {
        let raw = build_classdef_fmt1(10, &[1, 2, 0, 3]);
        let c = ClassDef::parse(&raw).unwrap();
        assert_eq!(c.format(), 1);
        assert_eq!(c.class_of(9), 0); // below start
        assert_eq!(c.class_of(10), 1);
        assert_eq!(c.class_of(11), 2);
        assert_eq!(c.class_of(12), 0); // explicit zero
        assert_eq!(c.class_of(13), 3);
        assert_eq!(c.class_of(14), 0); // past end
        assert_eq!(c.class_of(1000), 0);
    }

    #[test]
    fn classdef_format2_default_zero_between_ranges() {
        // Spec Example 2 GlyphClassDef shape.
        let raw = build_classdef_fmt2(&[
            (0x24, 0x24, 1),
            (0x58, 0x58, 3),
            (0x9F, 0x9F, 2),
            (0x18F, 0x18F, 4),
        ]);
        let c = ClassDef::parse(&raw).unwrap();
        assert_eq!(c.format(), 2);
        assert_eq!(c.class_of(0x24), 1);
        assert_eq!(c.class_of(0x58), 3);
        assert_eq!(c.class_of(0x9F), 2);
        assert_eq!(c.class_of(0x18F), 4);
        assert_eq!(c.class_of(0x23), 0);
        assert_eq!(c.class_of(0x59), 0);
        assert_eq!(c.class_of(0xA0), 0);
        assert_eq!(c.class_of(0xFFFF), 0);
    }

    #[test]
    fn classdef_rejects_unknown_format_and_truncation() {
        let mut bad = vec![0u8; 4];
        bad[0..2].copy_from_slice(&be16(5));
        assert!(matches!(ClassDef::parse(&bad), Err(Error::BadStructure(_))));
        let mut trunc = build_classdef_fmt1(0, &[1, 2, 3]);
        trunc.truncate(trunc.len() - 1);
        assert!(matches!(ClassDef::parse(&trunc), Err(Error::UnexpectedEof)));
    }

    // -- GdefTable header ------------------------------------------------

    fn build_gdef_v10(
        glyph_class_def: Option<Vec<u8>>,
        attach_list: Option<Vec<u8>>,
        lig_caret_list: Option<Vec<u8>>,
        mark_attach: Option<Vec<u8>>,
    ) -> Vec<u8> {
        let header_len = 12usize;
        let mut blocks = Vec::new();
        let mut offsets = [0u16; 4];
        let mut cursor = header_len;
        for (i, blk) in [
            &glyph_class_def,
            &attach_list,
            &lig_caret_list,
            &mark_attach,
        ]
        .iter()
        .enumerate()
        {
            if let Some(b) = blk {
                offsets[i] = cursor as u16;
                blocks.push(b.clone());
                cursor += b.len();
            }
        }
        let mut v = Vec::new();
        v.extend_from_slice(&be16(1));
        v.extend_from_slice(&be16(0));
        for o in &offsets {
            v.extend_from_slice(&be16(*o));
        }
        for b in blocks {
            v.extend_from_slice(&b);
        }
        v
    }

    fn build_gdef_v12_with_mgs(mgs: Vec<u8>) -> Vec<u8> {
        let header_len = 14usize;
        let mut v = Vec::new();
        v.extend_from_slice(&be16(1));
        v.extend_from_slice(&be16(2));
        for _ in 0..4 {
            v.extend_from_slice(&be16(0));
        }
        v.extend_from_slice(&be16(header_len as u16));
        v.extend_from_slice(&mgs);
        v
    }

    fn build_gdef_v13_minimal() -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(&be16(1));
        v.extend_from_slice(&be16(3));
        for _ in 0..4 {
            v.extend_from_slice(&be16(0));
        }
        v.extend_from_slice(&be16(0)); // markGlyphSets
        v.extend_from_slice(&be32(0)); // itemVarStore
        v
    }

    #[test]
    fn gdef_v10_parses_glyph_class_def_only() {
        let cd = build_classdef_fmt2(&[(2, 2, 1), (5, 7, 3)]);
        let table = build_gdef_v10(Some(cd), None, None, None);
        let g = GdefTable::parse(&table).unwrap();
        assert_eq!(g.version(), (1, 0));
        assert!(!g.has_mark_glyph_sets());
        assert!(!g.has_item_var_store());
        let class_def = g.glyph_class_def().expect("GlyphClassDef");
        assert_eq!(class_def.class_of(2), 1);
        assert_eq!(class_def.class_of(6), 3);
        assert_eq!(g.glyph_class(2), Some(GlyphClass::Base));
        assert_eq!(g.glyph_class(5), Some(GlyphClass::Mark));
        assert_eq!(g.glyph_class(0), None);
        assert_eq!(g.glyph_class(8), None);
    }

    #[test]
    fn gdef_v12_parses_mark_glyph_sets() {
        // 1 set, covering glyphs 0x40..=0x42.
        let cov = build_coverage_fmt2(&[(0x40, 0x42, 0)]);
        let mut mgs = Vec::new();
        mgs.extend_from_slice(&be16(1)); // format
        mgs.extend_from_slice(&be16(1)); // setCount
        mgs.extend_from_slice(&be32(4 + 4)); // offset to cov (relative to mgs start)
        mgs.extend_from_slice(&cov);
        let table = build_gdef_v12_with_mgs(mgs);
        let g = GdefTable::parse(&table).unwrap();
        assert_eq!(g.version(), (1, 2));
        assert!(g.has_mark_glyph_sets());
        assert!(!g.has_item_var_store());
        let mgs = g.mark_glyph_sets().expect("MarkGlyphSets");
        assert_eq!(mgs.set_count(), 1);
        assert!(mgs.contains(0, 0x41));
        assert!(!mgs.contains(0, 0x50));
    }

    #[test]
    fn gdef_v13_minimal_parses_item_var_store_offset() {
        let table = build_gdef_v13_minimal();
        let g = GdefTable::parse(&table).unwrap();
        assert_eq!(g.version(), (1, 3));
        assert!(g.has_mark_glyph_sets());
        assert!(g.has_item_var_store());
        assert_eq!(g.item_var_store_offset(), 0);
    }

    #[test]
    fn gdef_rejects_bad_versions() {
        // Wrong major.
        let mut bad = vec![0u8; 12];
        bad[0..2].copy_from_slice(&be16(2));
        assert!(matches!(
            GdefTable::parse(&bad),
            Err(Error::BadStructure(_))
        ));
        // Unknown minor (1 isn't defined).
        let mut bad = vec![0u8; 12];
        bad[0..2].copy_from_slice(&be16(1));
        bad[2..4].copy_from_slice(&be16(1));
        assert!(matches!(
            GdefTable::parse(&bad),
            Err(Error::BadStructure(_))
        ));
    }

    #[test]
    fn gdef_rejects_truncation_at_v12_field() {
        // v1.2 header is 14 bytes; pass only 12.
        let mut bad = vec![0u8; 12];
        bad[0..2].copy_from_slice(&be16(1));
        bad[2..4].copy_from_slice(&be16(2));
        assert!(matches!(GdefTable::parse(&bad), Err(Error::UnexpectedEof)));
    }

    #[test]
    fn gdef_rejects_truncation_at_v13_field() {
        // v1.3 header is 18 bytes; pass only 14.
        let mut bad = vec![0u8; 14];
        bad[0..2].copy_from_slice(&be16(1));
        bad[2..4].copy_from_slice(&be16(3));
        assert!(matches!(GdefTable::parse(&bad), Err(Error::UnexpectedEof)));
    }

    // -- AttachList -------------------------------------------------------

    #[test]
    fn attach_list_round_trip() {
        // Spec Example 3 shape: two glyphs in Coverage (GID 0x10, 0x20)
        // with 1 + 2 attachment points respectively.
        let cov = build_coverage_fmt1(&[0x10, 0x20]);
        let glyph_count = 2u16;
        // AttachList layout:
        //   0/2  coverageOffset
        //   2/2  glyphCount
        //   4/4  attachPointOffsets[2]
        //   8..  Coverage table
        //   8+cov.len() AttachPoint[0]
        //   ...           AttachPoint[1]
        let ap0 = {
            let mut v = Vec::new();
            v.extend_from_slice(&be16(1)); // pointCount
            v.extend_from_slice(&be16(7));
            v
        };
        let ap1 = {
            let mut v = Vec::new();
            v.extend_from_slice(&be16(2));
            v.extend_from_slice(&be16(3));
            v.extend_from_slice(&be16(11));
            v
        };
        let cov_off = 4 + 2 * 2;
        let ap0_off = cov_off + cov.len();
        let ap1_off = ap0_off + ap0.len();

        let mut al = Vec::new();
        al.extend_from_slice(&be16(cov_off as u16));
        al.extend_from_slice(&be16(glyph_count));
        al.extend_from_slice(&be16(ap0_off as u16));
        al.extend_from_slice(&be16(ap1_off as u16));
        al.extend_from_slice(&cov);
        al.extend_from_slice(&ap0);
        al.extend_from_slice(&ap1);

        let parsed = AttachList::parse(&al).unwrap();
        assert_eq!(parsed.glyph_count(), 2);
        let p0 = parsed.attach_points(0x10).unwrap();
        assert_eq!(p0.len(), 1);
        assert_eq!(p0.get(0), Some(7));
        let p1 = parsed.attach_points(0x20).unwrap();
        assert_eq!(p1.len(), 2);
        let v: Vec<_> = p1.iter().collect();
        assert_eq!(v, vec![3, 11]);
        assert!(parsed.attach_points(0x30).is_none());
    }

    // -- LigCaretList -----------------------------------------------------

    #[test]
    fn lig_caret_list_all_three_formats() {
        let cov = build_coverage_fmt1(&[0xAA]);
        // LigGlyph with 3 carets, one of each format.
        let cv1 = {
            let mut v = Vec::new();
            v.extend_from_slice(&be16(1)); // format
            v.extend_from_slice(&(-50i16).to_be_bytes());
            v
        };
        let cv2 = {
            let mut v = Vec::new();
            v.extend_from_slice(&be16(2));
            v.extend_from_slice(&be16(42)); // contour point index
            v
        };
        let cv3 = {
            let mut v = Vec::new();
            v.extend_from_slice(&be16(3));
            v.extend_from_slice(&(200i16).to_be_bytes());
            v.extend_from_slice(&be16(0)); // device offset NULL
            v
        };
        // LigGlyph layout:
        //   0/2 caretCount
        //   2/2 caretValueOffsets[caretCount]
        //   6   cv1
        //   6+cv1 cv2
        //   ... cv3
        let cv1_off = 2 + 3 * 2;
        let cv2_off = cv1_off + cv1.len();
        let cv3_off = cv2_off + cv2.len();
        let mut lg = Vec::new();
        lg.extend_from_slice(&be16(3));
        lg.extend_from_slice(&be16(cv1_off as u16));
        lg.extend_from_slice(&be16(cv2_off as u16));
        lg.extend_from_slice(&be16(cv3_off as u16));
        lg.extend_from_slice(&cv1);
        lg.extend_from_slice(&cv2);
        lg.extend_from_slice(&cv3);

        let cov_off = 4 + 2;
        let lg_off = cov_off + cov.len();
        let mut lcl = Vec::new();
        lcl.extend_from_slice(&be16(cov_off as u16));
        lcl.extend_from_slice(&be16(1));
        lcl.extend_from_slice(&be16(lg_off as u16));
        lcl.extend_from_slice(&cov);
        lcl.extend_from_slice(&lg);

        let parsed = LigCaretList::parse(&lcl).unwrap();
        assert_eq!(parsed.lig_glyph_count(), 1);
        let lg = parsed.lig_glyph(0xAA).unwrap();
        assert_eq!(lg.caret_count(), 3);
        assert_eq!(lg.caret_value(0), Some(CaretValue::DesignUnits(-50)));
        assert_eq!(lg.caret_value(1), Some(CaretValue::ContourPoint(42)));
        assert_eq!(
            lg.caret_value(2),
            Some(CaretValue::DesignUnitsWithDevice {
                coordinate: 200,
                device_offset: 0
            })
        );
        assert_eq!(lg.caret_value(3), None);
        assert!(parsed.lig_glyph(0xBB).is_none());
    }

    #[test]
    fn glyph_class_from_raw_round_trip() {
        assert_eq!(GlyphClass::from_raw(0), None);
        assert_eq!(GlyphClass::from_raw(1), Some(GlyphClass::Base));
        assert_eq!(GlyphClass::from_raw(2), Some(GlyphClass::Ligature));
        assert_eq!(GlyphClass::from_raw(3), Some(GlyphClass::Mark));
        assert_eq!(GlyphClass::from_raw(4), Some(GlyphClass::Component));
        assert_eq!(GlyphClass::from_raw(5), None);
        assert_eq!(GlyphClass::from_raw(0xFFFF), None);
    }
}
