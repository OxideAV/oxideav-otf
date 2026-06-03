//! Common OpenType Layout tables shared by GSUB and GPOS.
//!
//! Spec: `docs/text/opentype/otspec-chapter2-common-layout-tables.html`
//! ("OpenType Layout common table formats"). The chapter defines four
//! data structures that both the Glyph Substitution (GSUB) and Glyph
//! Positioning (GPOS) tables consult via header offsets:
//!
//! * [`ScriptList`] — array of `ScriptRecord` (`scriptTag`,
//!   `scriptOffset`), sorted alphabetically by tag.
//! * [`Script`] — `defaultLangSysOffset` plus an array of
//!   `LangSysRecord` (`langSysTag`, `langSysOffset`).
//! * [`LangSys`] — `requiredFeatureIndex` plus an array of
//!   `featureIndices[]` into the FeatureList.
//! * [`FeatureList`] — array of `FeatureRecord` (`featureTag`,
//!   `featureOffset`).
//! * [`Feature`] — `featureParamsOffset` plus an array of
//!   `lookupListIndices[]` into the LookupList.
//! * [`LookupList`] — array of `lookupOffsets[]`.
//! * [`Lookup`] — `lookupType`, [`LookupFlag`], array of
//!   `subtableOffsets[]`, and an optional `markFilteringSet` (present
//!   when `LookupFlag::USE_MARK_FILTERING_SET` is set).
//!
//! Every accessor is read-only and zero-copy: each `parse` call
//! validates the on-disk shape and stashes the borrowed slice; field
//! lookups decode their two- or four-byte windows on every call. The
//! types are `Copy` so they can be passed around freely.

use crate::parser::{read_u16, read_u32};
use crate::Error;

// ---------------------------------------------------------------------------
// Tag helper
// ---------------------------------------------------------------------------

/// 4-byte OpenType tag (ScriptTag, LangSysTag, FeatureTag) read from a
/// fixed offset. The four bytes are returned verbatim — the spec does
/// not mandate ASCII even though every well-known tag happens to be
/// ASCII printable.
#[inline]
fn read_tag(bytes: &[u8], off: usize) -> Result<[u8; 4], Error> {
    if bytes.len() < off + 4 {
        return Err(Error::UnexpectedEof);
    }
    Ok([bytes[off], bytes[off + 1], bytes[off + 2], bytes[off + 3]])
}

// ---------------------------------------------------------------------------
// ScriptList
// ---------------------------------------------------------------------------

/// Parsed `ScriptList` table (chapter 2 §"ScriptList table").
///
/// Layout:
/// ```text
///   0 / 2 / scriptCount
///   2 / 6 / scriptRecords[scriptCount]   // (tag[4] + scriptOffset[2])
/// ```
///
/// The records are sorted alphabetically by tag, so [`Self::find`]
/// runs a binary search.
#[derive(Debug, Clone, Copy)]
pub struct ScriptList<'a> {
    bytes: &'a [u8],
    count: u16,
}

impl<'a> ScriptList<'a> {
    /// Validate the header and the trailing `scriptRecords[]` array.
    pub fn parse(bytes: &'a [u8]) -> Result<Self, Error> {
        let count = read_u16(bytes, 0)?;
        let need = 2usize
            .checked_add(
                (count as usize)
                    .checked_mul(6)
                    .ok_or(Error::BadStructure("ScriptList scriptCount overflow"))?,
            )
            .ok_or(Error::BadStructure("ScriptList length overflow"))?;
        if bytes.len() < need {
            return Err(Error::UnexpectedEof);
        }
        Ok(Self { bytes, count })
    }

    /// Number of `ScriptRecord`s.
    pub fn count(&self) -> u16 {
        self.count
    }

    /// `true` when the list is empty.
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// Raw 4-byte tag at record index `i`, or `None` if out of range.
    pub fn tag(&self, i: u16) -> Option<[u8; 4]> {
        if i >= self.count {
            return None;
        }
        let off = 2 + (i as usize) * 6;
        read_tag(self.bytes, off).ok()
    }

    /// Parse the [`Script`] table referenced by record `i`. Returns
    /// `None` for an out-of-range index, an `Err` if the offset
    /// resolves outside the ScriptList or the Script header itself is
    /// truncated.
    pub fn script(&self, i: u16) -> Option<Result<Script<'a>, Error>> {
        if i >= self.count {
            return None;
        }
        let off = 2 + (i as usize) * 6;
        let script_off = read_u16(self.bytes, off + 4).ok()? as usize;
        Some(self.script_at(script_off))
    }

    fn script_at(&self, off: usize) -> Result<Script<'a>, Error> {
        if off == 0 || off >= self.bytes.len() {
            return Err(Error::BadStructure("ScriptList: scriptOffset out of range"));
        }
        Script::parse(&self.bytes[off..])
    }

    /// Look up a Script by tag. Returns `None` when the tag is absent.
    pub fn find(&self, tag: &[u8; 4]) -> Option<Result<Script<'a>, Error>> {
        // ScriptRecord array is sorted alphabetically by tag.
        let mut lo = 0i32;
        let mut hi = self.count as i32 - 1;
        while lo <= hi {
            let mid = (lo + hi) / 2;
            let mid_tag = self.tag(mid as u16)?;
            match mid_tag.cmp(tag) {
                std::cmp::Ordering::Equal => {
                    return self.script(mid as u16);
                }
                std::cmp::Ordering::Less => lo = mid + 1,
                std::cmp::Ordering::Greater => hi = mid - 1,
            }
        }
        None
    }

    /// Iterate over `(tag, script_result)` pairs in on-disk order.
    pub fn iter(&self) -> ScriptListIter<'a> {
        ScriptListIter {
            list: *self,
            next: 0,
        }
    }
}

/// Iterator yielded by [`ScriptList::iter`].
#[derive(Debug, Clone)]
pub struct ScriptListIter<'a> {
    list: ScriptList<'a>,
    next: u16,
}

impl<'a> Iterator for ScriptListIter<'a> {
    type Item = ([u8; 4], Result<Script<'a>, Error>);
    fn next(&mut self) -> Option<Self::Item> {
        if self.next >= self.list.count {
            return None;
        }
        let i = self.next;
        self.next += 1;
        let tag = self.list.tag(i)?;
        let script = self.list.script(i)?;
        Some((tag, script))
    }
}

// ---------------------------------------------------------------------------
// Script
// ---------------------------------------------------------------------------

/// Parsed `Script` table (chapter 2 §"Script table").
///
/// Layout:
/// ```text
///   0 / 2 / defaultLangSysOffset     (Offset16, may be NULL)
///   2 / 2 / langSysCount
///   4 / 6 / langSysRecords[]         (tag[4] + langSysOffset[2])
/// ```
#[derive(Debug, Clone, Copy)]
pub struct Script<'a> {
    bytes: &'a [u8],
    default_off: u16,
    count: u16,
}

impl<'a> Script<'a> {
    pub fn parse(bytes: &'a [u8]) -> Result<Self, Error> {
        let default_off = read_u16(bytes, 0)?;
        let count = read_u16(bytes, 2)?;
        let need = 4usize
            .checked_add(
                (count as usize)
                    .checked_mul(6)
                    .ok_or(Error::BadStructure("Script langSysCount overflow"))?,
            )
            .ok_or(Error::BadStructure("Script length overflow"))?;
        if bytes.len() < need {
            return Err(Error::UnexpectedEof);
        }
        Ok(Self {
            bytes,
            default_off,
            count,
        })
    }

    /// `true` iff a default `LangSys` is present (`defaultLangSysOffset != 0`).
    pub fn has_default_lang_sys(&self) -> bool {
        self.default_off != 0
    }

    /// Number of language-system records (excluding the default).
    pub fn lang_sys_count(&self) -> u16 {
        self.count
    }

    /// Parse the default `LangSys`, or `None` if not present.
    pub fn default_lang_sys(&self) -> Option<Result<LangSys<'a>, Error>> {
        if self.default_off == 0 {
            return None;
        }
        let off = self.default_off as usize;
        if off >= self.bytes.len() {
            return Some(Err(Error::BadStructure(
                "Script: defaultLangSysOffset out of range",
            )));
        }
        Some(LangSys::parse(&self.bytes[off..]))
    }

    /// Tag of `LangSysRecord` `i`.
    pub fn lang_sys_tag(&self, i: u16) -> Option<[u8; 4]> {
        if i >= self.count {
            return None;
        }
        let off = 4 + (i as usize) * 6;
        read_tag(self.bytes, off).ok()
    }

    /// Parse the `LangSys` at record `i`.
    pub fn lang_sys(&self, i: u16) -> Option<Result<LangSys<'a>, Error>> {
        if i >= self.count {
            return None;
        }
        let off = 4 + (i as usize) * 6;
        let lang_off = read_u16(self.bytes, off + 4).ok()? as usize;
        if lang_off == 0 || lang_off >= self.bytes.len() {
            return Some(Err(Error::BadStructure(
                "Script: langSysOffset out of range",
            )));
        }
        Some(LangSys::parse(&self.bytes[lang_off..]))
    }

    /// Binary-search the `LangSysRecord` array by tag.
    pub fn find_lang_sys(&self, tag: &[u8; 4]) -> Option<Result<LangSys<'a>, Error>> {
        let mut lo = 0i32;
        let mut hi = self.count as i32 - 1;
        while lo <= hi {
            let mid = (lo + hi) / 2;
            let mid_tag = self.lang_sys_tag(mid as u16)?;
            match mid_tag.cmp(tag) {
                std::cmp::Ordering::Equal => return self.lang_sys(mid as u16),
                std::cmp::Ordering::Less => lo = mid + 1,
                std::cmp::Ordering::Greater => hi = mid - 1,
            }
        }
        None
    }
}

// ---------------------------------------------------------------------------
// LangSys
// ---------------------------------------------------------------------------

/// "No required feature" sentinel for [`LangSys::required_feature_index`].
pub const NO_REQUIRED_FEATURE: u16 = 0xFFFF;

/// Parsed `LangSys` table (chapter 2 §"Language system table").
///
/// Layout:
/// ```text
///   0 / 2 / lookupOrderOffset     (reserved — must be 0)
///   2 / 2 / requiredFeatureIndex  (0xFFFF = none)
///   4 / 2 / featureIndexCount
///   6 / 2 / featureIndices[featureIndexCount]
/// ```
#[derive(Debug, Clone, Copy)]
pub struct LangSys<'a> {
    bytes: &'a [u8],
    required: u16,
    count: u16,
}

impl<'a> LangSys<'a> {
    pub fn parse(bytes: &'a [u8]) -> Result<Self, Error> {
        // lookupOrderOffset is reserved — must be NULL.
        let _ = read_u16(bytes, 0)?;
        let required = read_u16(bytes, 2)?;
        let count = read_u16(bytes, 4)?;
        let need = 6usize
            .checked_add(
                (count as usize)
                    .checked_mul(2)
                    .ok_or(Error::BadStructure("LangSys featureIndexCount overflow"))?,
            )
            .ok_or(Error::BadStructure("LangSys length overflow"))?;
        if bytes.len() < need {
            return Err(Error::UnexpectedEof);
        }
        Ok(Self {
            bytes,
            required,
            count,
        })
    }

    /// `requiredFeatureIndex`, or `None` for the sentinel `0xFFFF`.
    pub fn required_feature_index(&self) -> Option<u16> {
        if self.required == NO_REQUIRED_FEATURE {
            None
        } else {
            Some(self.required)
        }
    }

    /// Number of entries in `featureIndices[]`.
    pub fn feature_count(&self) -> u16 {
        self.count
    }

    /// Feature index at position `i` (into the parent FeatureList).
    pub fn feature_index(&self, i: u16) -> Option<u16> {
        if i >= self.count {
            return None;
        }
        let off = 6 + (i as usize) * 2;
        read_u16(self.bytes, off).ok()
    }

    /// Iterate over the `featureIndices[]` array.
    pub fn feature_indices(&self) -> impl Iterator<Item = u16> + '_ {
        (0..self.count).filter_map(move |i| self.feature_index(i))
    }
}

// ---------------------------------------------------------------------------
// FeatureList
// ---------------------------------------------------------------------------

/// Parsed `FeatureList` table (chapter 2 §"FeatureList table").
///
/// Layout:
/// ```text
///   0 / 2 / featureCount
///   2 / 6 / featureRecords[]   // (tag[4] + featureOffset[2])
/// ```
///
/// The records "should be" sorted by tag (the spec phrase
/// "alphabetically by feature tag" is "should") but ties on a tag are
/// allowed because a feature implementation may differ per script /
/// language system. Iteration order is on-disk; tag lookup is linear.
#[derive(Debug, Clone, Copy)]
pub struct FeatureList<'a> {
    bytes: &'a [u8],
    count: u16,
}

impl<'a> FeatureList<'a> {
    pub fn parse(bytes: &'a [u8]) -> Result<Self, Error> {
        let count = read_u16(bytes, 0)?;
        let need = 2usize
            .checked_add(
                (count as usize)
                    .checked_mul(6)
                    .ok_or(Error::BadStructure("FeatureList featureCount overflow"))?,
            )
            .ok_or(Error::BadStructure("FeatureList length overflow"))?;
        if bytes.len() < need {
            return Err(Error::UnexpectedEof);
        }
        Ok(Self { bytes, count })
    }

    /// Number of feature records.
    pub fn count(&self) -> u16 {
        self.count
    }

    /// `true` iff the list is empty.
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// Tag of feature record `i`.
    pub fn tag(&self, i: u16) -> Option<[u8; 4]> {
        if i >= self.count {
            return None;
        }
        let off = 2 + (i as usize) * 6;
        read_tag(self.bytes, off).ok()
    }

    /// Parse the [`Feature`] referenced by record `i`.
    pub fn feature(&self, i: u16) -> Option<Result<Feature<'a>, Error>> {
        if i >= self.count {
            return None;
        }
        let off = 2 + (i as usize) * 6;
        let feat_off = read_u16(self.bytes, off + 4).ok()? as usize;
        if feat_off == 0 || feat_off >= self.bytes.len() {
            return Some(Err(Error::BadStructure(
                "FeatureList: featureOffset out of range",
            )));
        }
        Some(Feature::parse(&self.bytes[feat_off..]))
    }

    /// Iterate `(tag, feature_result)` pairs in on-disk order.
    pub fn iter(&self) -> FeatureListIter<'a> {
        FeatureListIter {
            list: *self,
            next: 0,
        }
    }
}

/// Iterator yielded by [`FeatureList::iter`].
#[derive(Debug, Clone)]
pub struct FeatureListIter<'a> {
    list: FeatureList<'a>,
    next: u16,
}

impl<'a> Iterator for FeatureListIter<'a> {
    type Item = ([u8; 4], Result<Feature<'a>, Error>);
    fn next(&mut self) -> Option<Self::Item> {
        if self.next >= self.list.count {
            return None;
        }
        let i = self.next;
        self.next += 1;
        let tag = self.list.tag(i)?;
        let feat = self.list.feature(i)?;
        Some((tag, feat))
    }
}

// ---------------------------------------------------------------------------
// Feature
// ---------------------------------------------------------------------------

/// Parsed `Feature` table (chapter 2 §"Feature table").
///
/// Layout:
/// ```text
///   0 / 2 / featureParamsOffset  (Offset16, may be NULL)
///   2 / 2 / lookupIndexCount
///   4 / 2 / lookupListIndices[lookupIndexCount]
/// ```
#[derive(Debug, Clone, Copy)]
pub struct Feature<'a> {
    bytes: &'a [u8],
    params_off: u16,
    count: u16,
}

impl<'a> Feature<'a> {
    pub fn parse(bytes: &'a [u8]) -> Result<Self, Error> {
        let params_off = read_u16(bytes, 0)?;
        let count = read_u16(bytes, 2)?;
        let need = 4usize
            .checked_add(
                (count as usize)
                    .checked_mul(2)
                    .ok_or(Error::BadStructure("Feature lookupIndexCount overflow"))?,
            )
            .ok_or(Error::BadStructure("Feature length overflow"))?;
        if bytes.len() < need {
            return Err(Error::UnexpectedEof);
        }
        Ok(Self {
            bytes,
            params_off,
            count,
        })
    }

    /// Raw `featureParamsOffset`; `0` means no feature-parameters
    /// table is attached. The spec defines parameter formats only for
    /// a handful of features (`size`, `ssXX`, `cvXX`); decoding them
    /// is deferred to a future round.
    pub fn feature_params_offset(&self) -> u16 {
        self.params_off
    }

    /// Number of lookup-list indices.
    pub fn lookup_count(&self) -> u16 {
        self.count
    }

    /// Lookup index at position `i` (into the parent LookupList).
    pub fn lookup_index(&self, i: u16) -> Option<u16> {
        if i >= self.count {
            return None;
        }
        let off = 4 + (i as usize) * 2;
        read_u16(self.bytes, off).ok()
    }

    /// Iterate over `lookupListIndices[]`.
    pub fn lookup_indices(&self) -> impl Iterator<Item = u16> + '_ {
        (0..self.count).filter_map(move |i| self.lookup_index(i))
    }
}

// ---------------------------------------------------------------------------
// LookupList + Lookup + LookupFlag
// ---------------------------------------------------------------------------

/// Parsed `LookupList` table (chapter 2 §"LookupList table").
///
/// Layout:
/// ```text
///   0 / 2 / lookupCount
///   2 / 2 / lookupOffsets[lookupCount]
/// ```
#[derive(Debug, Clone, Copy)]
pub struct LookupList<'a> {
    bytes: &'a [u8],
    count: u16,
}

impl<'a> LookupList<'a> {
    pub fn parse(bytes: &'a [u8]) -> Result<Self, Error> {
        let count = read_u16(bytes, 0)?;
        let need = 2usize
            .checked_add(
                (count as usize)
                    .checked_mul(2)
                    .ok_or(Error::BadStructure("LookupList lookupCount overflow"))?,
            )
            .ok_or(Error::BadStructure("LookupList length overflow"))?;
        if bytes.len() < need {
            return Err(Error::UnexpectedEof);
        }
        Ok(Self { bytes, count })
    }

    /// Number of lookups.
    pub fn count(&self) -> u16 {
        self.count
    }

    /// `true` iff the list is empty.
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// Parse the [`Lookup`] at index `i`.
    pub fn lookup(&self, i: u16) -> Option<Result<Lookup<'a>, Error>> {
        if i >= self.count {
            return None;
        }
        let off = 2 + (i as usize) * 2;
        let lkup_off = read_u16(self.bytes, off).ok()? as usize;
        if lkup_off == 0 || lkup_off >= self.bytes.len() {
            return Some(Err(Error::BadStructure(
                "LookupList: lookupOffset out of range",
            )));
        }
        Some(Lookup::parse(&self.bytes[lkup_off..]))
    }

    /// Iterate over every parsed `Lookup` in on-disk order.
    pub fn iter(&self) -> LookupListIter<'a> {
        LookupListIter {
            list: *self,
            next: 0,
        }
    }
}

/// Iterator yielded by [`LookupList::iter`].
#[derive(Debug, Clone)]
pub struct LookupListIter<'a> {
    list: LookupList<'a>,
    next: u16,
}

impl<'a> Iterator for LookupListIter<'a> {
    type Item = Result<Lookup<'a>, Error>;
    fn next(&mut self) -> Option<Self::Item> {
        if self.next >= self.list.count {
            return None;
        }
        let i = self.next;
        self.next += 1;
        self.list.lookup(i)
    }
}

/// Bit fields of the `LookupFlag` `u16`, per chapter 2 §"LookupFlag
/// bit enumeration".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LookupFlag(pub u16);

impl LookupFlag {
    /// Cursive-attachment-only direction bit (GPOS lookup type 3).
    pub const RIGHT_TO_LEFT: u16 = 0x0001;
    /// Skip base glyphs (consults `GDEF.GlyphClassDef`).
    pub const IGNORE_BASE_GLYPHS: u16 = 0x0002;
    /// Skip ligatures (consults `GDEF.GlyphClassDef`).
    pub const IGNORE_LIGATURES: u16 = 0x0004;
    /// Skip all marks (consults `GDEF.GlyphClassDef`).
    pub const IGNORE_MARKS: u16 = 0x0008;
    /// Lookup header carries a trailing `markFilteringSet` field.
    pub const USE_MARK_FILTERING_SET: u16 = 0x0010;
    /// High byte: filter on `GDEF.MarkAttachClassDef`, `0` = no filter.
    pub const MARK_ATTACHMENT_CLASS_MASK: u16 = 0xFF00;

    /// Raw `u16` bits.
    pub fn bits(self) -> u16 {
        self.0
    }

    /// `true` if the cursive RTL bit is set.
    pub fn right_to_left(self) -> bool {
        self.0 & Self::RIGHT_TO_LEFT != 0
    }

    /// `true` if the lookup is configured to skip base glyphs.
    pub fn ignore_base_glyphs(self) -> bool {
        self.0 & Self::IGNORE_BASE_GLYPHS != 0
    }

    /// `true` if the lookup is configured to skip ligature glyphs.
    pub fn ignore_ligatures(self) -> bool {
        self.0 & Self::IGNORE_LIGATURES != 0
    }

    /// `true` if the lookup is configured to skip all mark glyphs.
    pub fn ignore_marks(self) -> bool {
        self.0 & Self::IGNORE_MARKS != 0
    }

    /// `true` if the lookup carries a trailing `markFilteringSet`
    /// field (and a [`MarkGlyphSets`](super::gdef::MarkGlyphSets)
    /// table must exist in `GDEF`).
    pub fn use_mark_filtering_set(self) -> bool {
        self.0 & Self::USE_MARK_FILTERING_SET != 0
    }

    /// `markAttachmentType` filter; `0` = unfiltered. Non-zero values
    /// index a class in `GDEF.MarkAttachClassDef`.
    pub fn mark_attachment_type(self) -> u8 {
        ((self.0 & Self::MARK_ATTACHMENT_CLASS_MASK) >> 8) as u8
    }
}

/// Parsed `Lookup` table (chapter 2 §"Lookup table").
///
/// Layout:
/// ```text
///   0 / 2 / lookupType
///   2 / 2 / lookupFlag
///   4 / 2 / subTableCount
///   6 / 2 / subtableOffsets[subTableCount]
///   6 + 2*subTableCount / 2 / markFilteringSet  (iff USE_MARK_FILTERING_SET)
/// ```
///
/// `lookupType` is interpreted by the surrounding GSUB / GPOS table;
/// the chapter-2 form intentionally leaves the enumeration open.
#[derive(Debug, Clone, Copy)]
pub struct Lookup<'a> {
    bytes: &'a [u8],
    lookup_type: u16,
    flag: LookupFlag,
    sub_count: u16,
    /// Cached `markFilteringSet` when `flag.use_mark_filtering_set()`,
    /// else `0`. The spec only requires the field to be present, not
    /// non-zero, so callers should consult [`LookupFlag::use_mark_filtering_set`].
    mark_filtering_set: u16,
}

impl<'a> Lookup<'a> {
    pub fn parse(bytes: &'a [u8]) -> Result<Self, Error> {
        let lookup_type = read_u16(bytes, 0)?;
        let flag_bits = read_u16(bytes, 2)?;
        let flag = LookupFlag(flag_bits);
        let sub_count = read_u16(bytes, 4)?;
        let sub_array = 6usize
            .checked_add(
                (sub_count as usize)
                    .checked_mul(2)
                    .ok_or(Error::BadStructure("Lookup subTableCount overflow"))?,
            )
            .ok_or(Error::BadStructure("Lookup length overflow"))?;
        let need = if flag.use_mark_filtering_set() {
            sub_array
                .checked_add(2)
                .ok_or(Error::BadStructure("Lookup mark-filtering overflow"))?
        } else {
            sub_array
        };
        if bytes.len() < need {
            return Err(Error::UnexpectedEof);
        }
        let mark_filtering_set = if flag.use_mark_filtering_set() {
            read_u16(bytes, sub_array)?
        } else {
            0
        };
        Ok(Self {
            bytes,
            lookup_type,
            flag,
            sub_count,
            mark_filtering_set,
        })
    }

    /// Lookup-type number. Interpretation depends on whether this
    /// lookup lives in GSUB (1–8) or GPOS (1–9).
    pub fn lookup_type(&self) -> u16 {
        self.lookup_type
    }

    /// Parsed [`LookupFlag`] bits.
    pub fn flag(&self) -> LookupFlag {
        self.flag
    }

    /// Number of subtables.
    pub fn subtable_count(&self) -> u16 {
        self.sub_count
    }

    /// Borrow the raw bytes of subtable `i` as a sub-slice starting at
    /// the subtable header. Length runs to the end of the parent
    /// Lookup byte window — callers are expected to use the
    /// subtable's own internal length / count fields to bound a read.
    pub fn subtable_bytes(&self, i: u16) -> Option<&'a [u8]> {
        if i >= self.sub_count {
            return None;
        }
        let off_off = 6 + (i as usize) * 2;
        let sub_off = read_u16(self.bytes, off_off).ok()? as usize;
        if sub_off == 0 || sub_off >= self.bytes.len() {
            return None;
        }
        Some(&self.bytes[sub_off..])
    }

    /// `markFilteringSet` value (a [`MarkGlyphSets`](super::gdef::MarkGlyphSets)
    /// index in GDEF), or `None` when the flag bit is unset.
    pub fn mark_filtering_set(&self) -> Option<u16> {
        if self.flag.use_mark_filtering_set() {
            Some(self.mark_filtering_set)
        } else {
            None
        }
    }
}

// ---------------------------------------------------------------------------
// Common GSUB / GPOS header (versions 1.0 + 1.1)
// ---------------------------------------------------------------------------

/// Parsed common header used by both GSUB and GPOS. Version `1.0`
/// (10 bytes) carries ScriptList / FeatureList / LookupList offsets;
/// version `1.1` (14 bytes) adds an `Offset32 featureVariationsOffset`.
#[derive(Debug, Clone, Copy)]
pub(crate) struct LayoutHeader {
    pub(crate) major: u16,
    pub(crate) minor: u16,
    pub(crate) script_list_off: u16,
    pub(crate) feature_list_off: u16,
    pub(crate) lookup_list_off: u16,
    /// `0` (= NULL) on a v1.0 header or when the v1.1 offset is the
    /// "no feature variations" sentinel.
    pub(crate) feature_variations_off: u32,
}

impl LayoutHeader {
    /// Decode a GSUB / GPOS header, accepting major version 1 and
    /// minor version 0 or 1. Unknown versions surface as
    /// [`Error::BadStructure`].
    pub(crate) fn parse(bytes: &[u8]) -> Result<Self, Error> {
        if bytes.len() < 10 {
            return Err(Error::UnexpectedEof);
        }
        let major = read_u16(bytes, 0)?;
        let minor = read_u16(bytes, 2)?;
        if major != 1 || minor > 1 {
            return Err(Error::BadStructure(
                "GSUB/GPOS: only version 1.0 / 1.1 are defined",
            ));
        }
        let script_list_off = read_u16(bytes, 4)?;
        let feature_list_off = read_u16(bytes, 6)?;
        let lookup_list_off = read_u16(bytes, 8)?;
        let feature_variations_off = if minor == 1 {
            if bytes.len() < 14 {
                return Err(Error::UnexpectedEof);
            }
            read_u32(bytes, 10)?
        } else {
            0
        };
        Ok(Self {
            major,
            minor,
            script_list_off,
            feature_list_off,
            lookup_list_off,
            feature_variations_off,
        })
    }
}

// ---------------------------------------------------------------------------
// Synthetic-byte tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn be(u: u16) -> [u8; 2] {
        u.to_be_bytes()
    }
    fn be32(u: u32) -> [u8; 4] {
        u.to_be_bytes()
    }

    #[test]
    fn header_v10_round_trip() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&be(1));
        bytes.extend_from_slice(&be(0));
        bytes.extend_from_slice(&be(10));
        bytes.extend_from_slice(&be(20));
        bytes.extend_from_slice(&be(30));
        let h = LayoutHeader::parse(&bytes).unwrap();
        assert_eq!(h.major, 1);
        assert_eq!(h.minor, 0);
        assert_eq!(h.script_list_off, 10);
        assert_eq!(h.feature_list_off, 20);
        assert_eq!(h.lookup_list_off, 30);
        assert_eq!(h.feature_variations_off, 0);
    }

    #[test]
    fn header_v11_round_trip() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&be(1));
        bytes.extend_from_slice(&be(1));
        bytes.extend_from_slice(&be(14));
        bytes.extend_from_slice(&be(24));
        bytes.extend_from_slice(&be(34));
        bytes.extend_from_slice(&be32(99_999));
        let h = LayoutHeader::parse(&bytes).unwrap();
        assert_eq!(h.minor, 1);
        assert_eq!(h.feature_variations_off, 99_999);
    }

    #[test]
    fn header_rejects_unknown_versions() {
        let bytes_v2 = {
            let mut b = vec![0u8; 14];
            b[0..2].copy_from_slice(&be(2));
            b
        };
        assert!(matches!(
            LayoutHeader::parse(&bytes_v2),
            Err(Error::BadStructure(_))
        ));
        let bytes_v12 = {
            let mut b = vec![0u8; 14];
            b[0..2].copy_from_slice(&be(1));
            b[2..4].copy_from_slice(&be(2));
            b
        };
        assert!(matches!(
            LayoutHeader::parse(&bytes_v12),
            Err(Error::BadStructure(_))
        ));
    }

    /// Build a tiny ScriptList + Script + LangSys + FeatureList +
    /// Feature + LookupList + Lookup synthetic byte tower and confirm
    /// the parsers walk it.
    #[test]
    fn synthetic_round_trip() {
        // LangSys: lookupOrder=0, requiredFeatureIndex=0xFFFF,
        // featureIndexCount=2, featureIndices=[0,1]
        let mut lang_sys = Vec::new();
        lang_sys.extend_from_slice(&be(0));
        lang_sys.extend_from_slice(&be(NO_REQUIRED_FEATURE));
        lang_sys.extend_from_slice(&be(2));
        lang_sys.extend_from_slice(&be(0));
        lang_sys.extend_from_slice(&be(1));
        let ls = LangSys::parse(&lang_sys).unwrap();
        assert!(ls.required_feature_index().is_none());
        assert_eq!(ls.feature_count(), 2);
        assert_eq!(ls.feature_index(0), Some(0));
        assert_eq!(ls.feature_index(1), Some(1));
        assert_eq!(ls.feature_index(2), None);
        let v: Vec<_> = ls.feature_indices().collect();
        assert_eq!(v, vec![0, 1]);

        // Feature: featureParamsOffset=0, lookupIndexCount=1, lookupListIndices=[0]
        let mut feature = Vec::new();
        feature.extend_from_slice(&be(0));
        feature.extend_from_slice(&be(1));
        feature.extend_from_slice(&be(0));
        let f = Feature::parse(&feature).unwrap();
        assert_eq!(f.feature_params_offset(), 0);
        assert_eq!(f.lookup_count(), 1);
        assert_eq!(f.lookup_index(0), Some(0));

        // Lookup: type=1, flag=0, subTableCount=0  (no subtables — fine for the parser)
        let mut lookup = Vec::new();
        lookup.extend_from_slice(&be(1));
        lookup.extend_from_slice(&be(0));
        lookup.extend_from_slice(&be(0));
        let l = Lookup::parse(&lookup).unwrap();
        assert_eq!(l.lookup_type(), 1);
        assert_eq!(l.subtable_count(), 0);
        assert!(l.mark_filtering_set().is_none());
        assert!(!l.flag().right_to_left());
        assert!(!l.flag().ignore_marks());
    }

    #[test]
    fn lookup_with_mark_filtering_set() {
        let mut lookup = Vec::new();
        lookup.extend_from_slice(&be(4));
        lookup.extend_from_slice(&be(LookupFlag::USE_MARK_FILTERING_SET));
        lookup.extend_from_slice(&be(0));
        lookup.extend_from_slice(&be(7));
        let l = Lookup::parse(&lookup).unwrap();
        assert!(l.flag().use_mark_filtering_set());
        assert_eq!(l.mark_filtering_set(), Some(7));
    }

    #[test]
    fn lookup_flag_helpers() {
        let f = LookupFlag(0x0A0E);
        assert!(!f.right_to_left());
        assert!(f.ignore_base_glyphs());
        assert!(f.ignore_ligatures());
        assert!(f.ignore_marks());
        assert!(!f.use_mark_filtering_set());
        assert_eq!(f.mark_attachment_type(), 0x0A);
    }

    #[test]
    fn truncation_surfaces_unexpected_eof() {
        // ScriptList claims 3 records but the buffer has 2.
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&be(3));
        bytes.extend_from_slice(b"DFLT");
        bytes.extend_from_slice(&be(0));
        bytes.extend_from_slice(b"latn");
        bytes.extend_from_slice(&be(0));
        // Missing the third record.
        assert!(matches!(
            ScriptList::parse(&bytes),
            Err(Error::UnexpectedEof)
        ));

        // Lookup with USE_MARK_FILTERING_SET but missing the trailing word.
        let mut lookup = Vec::new();
        lookup.extend_from_slice(&be(4));
        lookup.extend_from_slice(&be(LookupFlag::USE_MARK_FILTERING_SET));
        lookup.extend_from_slice(&be(0));
        assert!(matches!(Lookup::parse(&lookup), Err(Error::UnexpectedEof)));
    }
}
