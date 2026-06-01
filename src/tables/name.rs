//! `name` — Naming table (OpenType / ISO 14496-22 §"name table").
//!
//! Spec: `docs/text/opentype/otspec-name.html` (Microsoft / ISO/IEC
//! 14496-22). Two versions are defined by the spec:
//!
//! - **Version 0** — `uint16 version + uint16 count + Offset16
//!   storageOffset + NameRecord[count] + storage[]`. Language IDs are
//!   platform-specific numeric values, always < 0x8000.
//! - **Version 1** — version 0 plus a trailing
//!   `uint16 langTagCount + LangTagRecord[langTagCount]` block before
//!   the string storage. Language IDs >= 0x8000 index the
//!   `langTagRecord` array (offset `lang_id - 0x8000`); each entry
//!   references a UTF-16BE BCP 47 language-tag string in the storage
//!   area (e.g. `"en"`, `"fr-CA"`, `"zh-Hant-HK"`). A name record
//!   referencing a language ID outside `0x8000 .. 0x8000 +
//!   langTagCount` "should not be used" per spec, and we surface that
//!   condition as `lang_tag(lang_id) -> None`.
//!
//! Selection priority for [`NameTable::find`]: Windows / Unicode BMP
//! English (3, 1, 0x409), then any Windows English, then Windows /
//! Unicode any, then Windows / Unicode UCS-4, then Mac Roman English,
//! then Unicode-platform records, then anything else.

use crate::parser::read_u16;
use crate::Error;

/// Length of the on-disk fixed-size header (`version` + `count` +
/// `storageOffset`).
const NAME_HEADER_LEN: usize = 6;
/// Length of a single `NameRecord` (platformID + encodingID +
/// languageID + nameID + length + offset).
const NAME_RECORD_LEN: usize = 12;
/// Length of a single `LangTagRecord` (length + langTagOffset).
const LANG_TAG_RECORD_LEN: usize = 4;

/// Standard `name` table name IDs (0..=25) per OpenType `otspec-name.html`
/// "Name IDs". Each variant is the spec-defined logical string
/// category; values 26..=255 are reserved for future standard names
/// and values 256..=32767 are reserved for font-specific names (e.g.
/// GSUB feature parameter strings). `Reserved15` is included because
/// the spec explicitly lists name ID 15 as reserved — the variant
/// exists so a caller iterating `NameId::from_raw` doesn't lose
/// information about a font that emits a record with ID 15.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NameId {
    /// 0 — Copyright notice.
    Copyright,
    /// 1 — Font Family name (the 4-style style-linking family,
    /// constrained to at most four members `{regular, italic, bold,
    /// bold italic}` per spec recommendation).
    FontFamily,
    /// 2 — Font Subfamily name (one of `Regular` / `Italic` / `Bold`
    /// / `Bold Italic` for fonts respecting the 4-style grouping).
    FontSubfamily,
    /// 3 — Unique font identifier (e.g. `"Monotype: Times New Roman
    /// Bold: 1990"`).
    UniqueId,
    /// 4 — Full font name (typically family + subfamily; the
    /// human-readable display name applications show in font menus).
    FullName,
    /// 5 — Version string; begins with `"Version X.Y"` per spec.
    Version,
    /// 6 — PostScript name (ASCII, length <= 63, restricted character
    /// set; the name used to invoke the font through PostScript).
    PostScript,
    /// 7 — Trademark.
    Trademark,
    /// 8 — Manufacturer.
    Manufacturer,
    /// 9 — Designer name.
    Designer,
    /// 10 — Description of the typeface.
    Description,
    /// 11 — URL of the vendor.
    VendorUrl,
    /// 12 — URL of the designer.
    DesignerUrl,
    /// 13 — License description.
    License,
    /// 14 — License info URL.
    LicenseUrl,
    /// 15 — Reserved by the spec. Variant exists so `from_raw(15)`
    /// can distinguish a reserved-bin record from a missing name ID
    /// (the spec advises against emitting this but doesn't forbid it).
    Reserved15,
    /// 16 — Typographic Family name (a.k.a. "Preferred Family" in
    /// older spec text; an extended family grouping that escapes the
    /// 4-style cap of name ID 1).
    TypographicFamily,
    /// 17 — Typographic Subfamily name (a.k.a. "Preferred Subfamily").
    TypographicSubfamily,
    /// 18 — Compatible Full name (Macintosh only).
    CompatibleFull,
    /// 19 — Sample text.
    SampleText,
    /// 20 — PostScript CID `findfont` name; presence implies name ID
    /// 6 should be used with `composefont` instead.
    PostScriptCidFindfont,
    /// 21 — WWS (weight / width / slope) family name. Used when the
    /// typographic family includes attributes other than weight / width
    /// / slope; see `OS/2.fsSelection` bit 8.
    WwsFamily,
    /// 22 — WWS subfamily name.
    WwsSubfamily,
    /// 23 — Light Background Palette name (paired with `CPAL`).
    LightBackgroundPalette,
    /// 24 — Dark Background Palette name (paired with `CPAL`).
    DarkBackgroundPalette,
    /// 25 — Variations PostScript Name Prefix (variable fonts; see
    /// Adobe Technical Note #5902 referenced from `otspec-name.html`).
    VariationsPsNamePrefix,
}

impl NameId {
    /// Decode a raw `nameID` field into a `NameId` if it is one of the
    /// 26 standard values 0..=25, else `None`. Callers that need to
    /// see custom IDs (256..=32767) should iterate
    /// [`NameTable::records`] and read [`NameRecord::name_id_raw`].
    pub fn from_raw(name_id: u16) -> Option<Self> {
        Some(match name_id {
            0 => Self::Copyright,
            1 => Self::FontFamily,
            2 => Self::FontSubfamily,
            3 => Self::UniqueId,
            4 => Self::FullName,
            5 => Self::Version,
            6 => Self::PostScript,
            7 => Self::Trademark,
            8 => Self::Manufacturer,
            9 => Self::Designer,
            10 => Self::Description,
            11 => Self::VendorUrl,
            12 => Self::DesignerUrl,
            13 => Self::License,
            14 => Self::LicenseUrl,
            15 => Self::Reserved15,
            16 => Self::TypographicFamily,
            17 => Self::TypographicSubfamily,
            18 => Self::CompatibleFull,
            19 => Self::SampleText,
            20 => Self::PostScriptCidFindfont,
            21 => Self::WwsFamily,
            22 => Self::WwsSubfamily,
            23 => Self::LightBackgroundPalette,
            24 => Self::DarkBackgroundPalette,
            25 => Self::VariationsPsNamePrefix,
            _ => return None,
        })
    }

    /// The raw 16-bit nameID this variant corresponds to.
    pub fn to_raw(self) -> u16 {
        match self {
            Self::Copyright => 0,
            Self::FontFamily => 1,
            Self::FontSubfamily => 2,
            Self::UniqueId => 3,
            Self::FullName => 4,
            Self::Version => 5,
            Self::PostScript => 6,
            Self::Trademark => 7,
            Self::Manufacturer => 8,
            Self::Designer => 9,
            Self::Description => 10,
            Self::VendorUrl => 11,
            Self::DesignerUrl => 12,
            Self::License => 13,
            Self::LicenseUrl => 14,
            Self::Reserved15 => 15,
            Self::TypographicFamily => 16,
            Self::TypographicSubfamily => 17,
            Self::CompatibleFull => 18,
            Self::SampleText => 19,
            Self::PostScriptCidFindfont => 20,
            Self::WwsFamily => 21,
            Self::WwsSubfamily => 22,
            Self::LightBackgroundPalette => 23,
            Self::DarkBackgroundPalette => 24,
            Self::VariationsPsNamePrefix => 25,
        }
    }
}

/// One row of the `name` table's `NameRecord` array, before string
/// decoding. The `value()` accessor decodes the on-disk bytes into a
/// Rust `String` using the platform / encoding pair (UTF-16BE for
/// Microsoft / Unicode platforms, an ASCII subset for Mac Roman). Use
/// [`NameTable::records`] to iterate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NameRecord {
    /// `platformID` per `otspec-name.html` "Platform IDs" — `0`
    /// Unicode, `1` Macintosh, `2` ISO (deprecated), `3` Windows, `4`
    /// Custom.
    pub platform_id: u16,
    /// `encodingID` — platform-specific. For Windows the common values
    /// are 1 (Unicode BMP / UCS-2) and 10 (Unicode UCS-4); for
    /// Macintosh 0 is Roman.
    pub encoding_id: u16,
    /// `languageID`. < 0x8000 is platform-specific (Windows uses LCIDs,
    /// e.g. `0x0409` = English-US). >= 0x8000 indexes the
    /// `langTagRecord` array on a version-1 table (offset `lang_id -
    /// 0x8000`); see [`NameTable::lang_tag`].
    pub language_id: u16,
    /// Raw `nameID`. Decode through [`NameId::from_raw`] for the
    /// 0..=25 standard categories; values 256..=32767 are reserved for
    /// font-specific layout-feature parameter strings.
    pub name_id_raw: u16,
    /// String length on disk, in bytes.
    pub length: u16,
    /// String offset, in bytes, from the start of the string-storage
    /// area (which is the table's `storageOffset` field).
    pub string_offset: u16,
}

impl NameRecord {
    /// Decode the standard `NameId` for this record (if it is one of
    /// the 26 spec-defined values 0..=25).
    pub fn name_id(&self) -> Option<NameId> {
        NameId::from_raw(self.name_id_raw)
    }
}

#[derive(Debug, Clone)]
pub struct NameTable<'a> {
    bytes: &'a [u8],
    /// Table version (0 or 1).
    version: u16,
    /// Number of `NameRecord`s.
    count: u16,
    /// Offset of the string-storage area from the start of the table.
    string_offset: u16,
    /// Number of language-tag records (always 0 on version-0 tables).
    lang_tag_count: u16,
    /// Byte offset of the start of the `langTagRecord` array from the
    /// start of the table, or 0 when there are no records.
    lang_tag_array_offset: usize,
    _phantom: core::marker::PhantomData<&'a ()>,
}

impl<'a> NameTable<'a> {
    pub fn parse(bytes: &'a [u8]) -> Result<Self, Error> {
        if bytes.len() < NAME_HEADER_LEN {
            return Err(Error::UnexpectedEof);
        }
        let version = read_u16(bytes, 0)?;
        if version > 1 {
            return Err(Error::BadStructure("name.version > 1"));
        }
        let count = read_u16(bytes, 2)?;
        let string_offset = read_u16(bytes, 4)?;
        let records_end = NAME_HEADER_LEN
            .checked_add(
                (count as usize)
                    .checked_mul(NAME_RECORD_LEN)
                    .ok_or(Error::BadStructure("name.count overflow"))?,
            )
            .ok_or(Error::BadStructure("name.records overflow"))?;
        if bytes.len() < records_end {
            return Err(Error::UnexpectedEof);
        }

        let (lang_tag_count, lang_tag_array_offset) = if version == 1 {
            // Version 1 has `uint16 langTagCount` immediately after
            // the NameRecord array, followed by `LangTagRecord[]`.
            if bytes.len() < records_end + 2 {
                return Err(Error::UnexpectedEof);
            }
            let n = read_u16(bytes, records_end)?;
            let array_off = records_end + 2;
            let array_end = array_off
                .checked_add(
                    (n as usize)
                        .checked_mul(LANG_TAG_RECORD_LEN)
                        .ok_or(Error::BadStructure("name.langTagCount overflow"))?,
                )
                .ok_or(Error::BadStructure("name.langTagRecord overflow"))?;
            if bytes.len() < array_end {
                return Err(Error::UnexpectedEof);
            }
            // The string-storage region must lie at or past the end of
            // the LangTagRecord array per the spec's table layout.
            if (string_offset as usize) < array_end {
                return Err(Error::BadStructure(
                    "name.storageOffset overlaps langTagRecord array",
                ));
            }
            (n, array_off)
        } else {
            (0u16, 0usize)
        };

        if (string_offset as usize) > bytes.len() {
            return Err(Error::BadOffset);
        }

        Ok(Self {
            bytes,
            version,
            count,
            string_offset,
            lang_tag_count,
            lang_tag_array_offset,
            _phantom: core::marker::PhantomData,
        })
    }

    /// Table version (`0` or `1`).
    pub fn version(&self) -> u16 {
        self.version
    }

    /// Number of NameRecords (`count` field).
    pub fn record_count(&self) -> u16 {
        self.count
    }

    /// Number of LangTagRecords (`langTagCount` field). Always `0` on
    /// a version-0 table.
    pub fn lang_tag_count(&self) -> u16 {
        self.lang_tag_count
    }

    /// Iterate every `NameRecord` in directory order. The on-disk
    /// records are spec-sorted (`platformID`, `encodingID`,
    /// `languageID`, `nameID` ascending), so this iteration is also
    /// sorted.
    pub fn records(&self) -> impl Iterator<Item = NameRecord> + '_ {
        (0..self.count as usize).map(move |i| {
            let off = NAME_HEADER_LEN + i * NAME_RECORD_LEN;
            // These reads are bounds-checked at parse time, but
            // `read_u16` still does its own check; on the unhappy
            // path we surface a default-zero record which a caller's
            // `value()` lookup would then fail on cleanly.
            let platform_id = read_u16(self.bytes, off).unwrap_or(0);
            let encoding_id = read_u16(self.bytes, off + 2).unwrap_or(0);
            let language_id = read_u16(self.bytes, off + 4).unwrap_or(0);
            let name_id_raw = read_u16(self.bytes, off + 6).unwrap_or(0);
            let length = read_u16(self.bytes, off + 8).unwrap_or(0);
            let string_offset = read_u16(self.bytes, off + 10).unwrap_or(0);
            NameRecord {
                platform_id,
                encoding_id,
                language_id,
                name_id_raw,
                length,
                string_offset,
            }
        })
    }

    /// Decode a single record's string. Returns `None` if the
    /// platform / encoding pair is not one we decode (we cover
    /// Unicode-platform, Windows/Unicode-BMP, Windows/Unicode-UCS-4,
    /// and Macintosh / Roman) or if the on-disk bytes are not valid
    /// for that encoding.
    pub fn record_value(&self, rec: NameRecord) -> Option<String> {
        let start = self.string_offset as usize + rec.string_offset as usize;
        let end = start.checked_add(rec.length as usize)?;
        let raw = self.bytes.get(start..end)?;
        decode(rec.platform_id, rec.encoding_id, raw).map(|cow| cow.into_owned())
    }

    /// BCP 47 language tag for a name record's `languageID`, if the
    /// table is version 1 and the ID is in the spec range `[0x8000,
    /// 0x8000 + langTagCount)`. The tag is decoded from UTF-16BE per
    /// spec; non-Unicode storage is rejected (`None`).
    ///
    /// Returns `None` for any ID `< 0x8000` (which are platform-specific
    /// numeric IDs, not language tags) and for any ID outside the
    /// declared range (per spec: "the identity of the language is
    /// unknown; such name records should not be used").
    pub fn lang_tag(&self, language_id: u16) -> Option<String> {
        if self.version < 1 || language_id < 0x8000 {
            return None;
        }
        let idx = (language_id - 0x8000) as usize;
        if idx >= self.lang_tag_count as usize {
            return None;
        }
        let rec_off = self.lang_tag_array_offset + idx * LANG_TAG_RECORD_LEN;
        let length = read_u16(self.bytes, rec_off).ok()? as usize;
        let off = read_u16(self.bytes, rec_off + 2).ok()? as usize;
        let start = self.string_offset as usize + off;
        let end = start.checked_add(length)?;
        let raw = self.bytes.get(start..end)?;
        // Spec mandates UTF-16BE for language-tag strings.
        decode_utf16_be(raw).map(|s| match s {
            std::borrow::Cow::Borrowed(s) => s.to_string(),
            std::borrow::Cow::Owned(s) => s,
        })
    }

    /// Find the value of a name record by its standard `name_id`.
    /// Selects the best-ranked encoding (Windows / Unicode BMP English
    /// first, then Mac Roman English, then anything else).
    pub fn find(&self, name_id: u16) -> Option<&'a str> {
        let mut best: Option<(i32, std::borrow::Cow<'a, str>)> = None;

        for i in 0..self.count as usize {
            let off = NAME_HEADER_LEN + i * NAME_RECORD_LEN;
            let platform = read_u16(self.bytes, off).ok()?;
            let encoding = read_u16(self.bytes, off + 2).ok()?;
            let language = read_u16(self.bytes, off + 4).ok()?;
            let nid = read_u16(self.bytes, off + 6).ok()?;
            if nid != name_id {
                continue;
            }
            let length = read_u16(self.bytes, off + 8).ok()? as usize;
            let str_off = read_u16(self.bytes, off + 10).ok()? as usize;
            let start = self.string_offset as usize + str_off;
            let end = start.checked_add(length)?;
            let raw = self.bytes.get(start..end)?;
            let rank = rank_record(platform, encoding, language);
            let decoded = decode(platform, encoding, raw)?;
            match &best {
                Some((br, _)) if *br >= rank => {}
                _ => best = Some((rank, decoded)),
            }
        }
        let (_, c) = best?;
        Some(match c {
            std::borrow::Cow::Borrowed(s) => s,
            std::borrow::Cow::Owned(s) => Box::leak(s.into_boxed_str()),
        })
    }

    /// Convenience: same as [`NameTable::find`] but takes a typed
    /// [`NameId`].
    pub fn get(&self, name_id: NameId) -> Option<&'a str> {
        self.find(name_id.to_raw())
    }
}

fn rank_record(platform: u16, encoding: u16, language: u16) -> i32 {
    match (platform, encoding, language) {
        (3, 1, 0x0409) => 100,
        (3, 1, l) if l & 0xFF == 9 => 90,
        (3, 1, _) => 80,
        (3, 10, _) => 75,
        (1, 0, 0) => 70,
        (0, _, _) => 60,
        _ => 10,
    }
}

fn decode<'a>(platform: u16, encoding: u16, raw: &'a [u8]) -> Option<std::borrow::Cow<'a, str>> {
    match (platform, encoding) {
        (0, _) | (3, 1) | (3, 10) => decode_utf16_be(raw),
        (1, 0) => {
            if raw.iter().all(|&b| b < 0x80) {
                std::str::from_utf8(raw)
                    .ok()
                    .map(std::borrow::Cow::Borrowed)
            } else {
                Some(std::borrow::Cow::Owned(
                    raw.iter()
                        .map(|&b| if b < 0x80 { b as char } else { '?' })
                        .collect(),
                ))
            }
        }
        _ => None,
    }
}

fn decode_utf16_be(raw: &[u8]) -> Option<std::borrow::Cow<'_, str>> {
    if raw.len() % 2 != 0 {
        return None;
    }
    let mut s = String::with_capacity(raw.len() / 2);
    let mut i = 0;
    while i + 1 < raw.len() {
        let u = u16::from_be_bytes([raw[i], raw[i + 1]]);
        i += 2;
        if (0xD800..=0xDBFF).contains(&u) {
            if i + 1 >= raw.len() {
                return None;
            }
            let lo = u16::from_be_bytes([raw[i], raw[i + 1]]);
            if !(0xDC00..=0xDFFF).contains(&lo) {
                return None;
            }
            i += 2;
            let cp = 0x10000 + (((u - 0xD800) as u32) << 10) + (lo - 0xDC00) as u32;
            s.push(char::from_u32(cp)?);
        } else if (0xDC00..=0xDFFF).contains(&u) {
            // Unpaired low surrogate.
            return None;
        } else {
            s.push(char::from_u32(u as u32)?);
        }
    }
    Some(std::borrow::Cow::Owned(s))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal version-0 `name` table with a single Windows /
    /// Unicode BMP English record.
    fn build_minimal_v0() -> Vec<u8> {
        let utf16: Vec<u8> = "Hi".encode_utf16().flat_map(|u| u.to_be_bytes()).collect();
        let length = utf16.len() as u16;
        let header_size = NAME_HEADER_LEN + NAME_RECORD_LEN;
        let mut out = vec![0u8; header_size];
        out[0..2].copy_from_slice(&0u16.to_be_bytes()); // version 0
        out[2..4].copy_from_slice(&1u16.to_be_bytes()); // count
        out[4..6].copy_from_slice(&(header_size as u16).to_be_bytes()); // storageOffset
        out[6..8].copy_from_slice(&3u16.to_be_bytes()); // platform = Windows
        out[8..10].copy_from_slice(&1u16.to_be_bytes()); // encoding = Unicode BMP
        out[10..12].copy_from_slice(&0x0409u16.to_be_bytes()); // language = en-US
        out[12..14].copy_from_slice(&1u16.to_be_bytes()); // nameID = FontFamily
        out[14..16].copy_from_slice(&length.to_be_bytes());
        out[16..18].copy_from_slice(&0u16.to_be_bytes());
        out.extend_from_slice(&utf16);
        out
    }

    /// Build a version-1 `name` table with two name records (one
    /// platform-specific English, one referencing a language-tag
    /// record) and two language-tag records ("en", "zh-Hant-HK" —
    /// the spec's worked example from otspec-name.html §"naming
    /// table version 1").
    fn build_v1_with_lang_tags() -> Vec<u8> {
        // Name record strings (UTF-16BE).
        let s_en: Vec<u8> = "Family"
            .encode_utf16()
            .flat_map(|u| u.to_be_bytes())
            .collect();
        let s_zh: Vec<u8> = "字体"
            .encode_utf16()
            .flat_map(|u| u.to_be_bytes())
            .collect();
        // Language-tag strings (UTF-16BE).
        let lt_en: Vec<u8> = "en".encode_utf16().flat_map(|u| u.to_be_bytes()).collect();
        let lt_zh: Vec<u8> = "zh-Hant-HK"
            .encode_utf16()
            .flat_map(|u| u.to_be_bytes())
            .collect();

        let header = NAME_HEADER_LEN; // 6
        let records = 2 * NAME_RECORD_LEN; // 24
        let lt_count_field = 2; // uint16
        let lt_records = 2 * LANG_TAG_RECORD_LEN; // 8
        let storage_offset = header + records + lt_count_field + lt_records; // 40

        // Storage layout: s_en | s_zh | lt_en | lt_zh
        let off_s_en = 0u16;
        let off_s_zh = off_s_en + s_en.len() as u16;
        let off_lt_en = off_s_zh + s_zh.len() as u16;
        let off_lt_zh = off_lt_en + lt_en.len() as u16;
        let total = storage_offset + s_en.len() + s_zh.len() + lt_en.len() + lt_zh.len();

        let mut out = vec![0u8; total];
        out[0..2].copy_from_slice(&1u16.to_be_bytes()); // version 1
        out[2..4].copy_from_slice(&2u16.to_be_bytes()); // count
        out[4..6].copy_from_slice(&(storage_offset as u16).to_be_bytes());

        // Record 0: Windows / Unicode BMP / en-US / FontFamily
        let r0 = header;
        out[r0..r0 + 2].copy_from_slice(&3u16.to_be_bytes());
        out[r0 + 2..r0 + 4].copy_from_slice(&1u16.to_be_bytes());
        out[r0 + 4..r0 + 6].copy_from_slice(&0x0409u16.to_be_bytes());
        out[r0 + 6..r0 + 8].copy_from_slice(&1u16.to_be_bytes());
        out[r0 + 8..r0 + 10].copy_from_slice(&(s_en.len() as u16).to_be_bytes());
        out[r0 + 10..r0 + 12].copy_from_slice(&off_s_en.to_be_bytes());

        // Record 1: Windows / Unicode BMP / langTag idx 1 (0x8001) /
        // FontFamily.
        let r1 = r0 + NAME_RECORD_LEN;
        out[r1..r1 + 2].copy_from_slice(&3u16.to_be_bytes());
        out[r1 + 2..r1 + 4].copy_from_slice(&1u16.to_be_bytes());
        out[r1 + 4..r1 + 6].copy_from_slice(&0x8001u16.to_be_bytes());
        out[r1 + 6..r1 + 8].copy_from_slice(&1u16.to_be_bytes());
        out[r1 + 8..r1 + 10].copy_from_slice(&(s_zh.len() as u16).to_be_bytes());
        out[r1 + 10..r1 + 12].copy_from_slice(&off_s_zh.to_be_bytes());

        // langTagCount.
        let lc = header + records;
        out[lc..lc + 2].copy_from_slice(&2u16.to_be_bytes());

        // LangTagRecord[0] = "en" (becomes language ID 0x8000).
        let lt0 = lc + 2;
        out[lt0..lt0 + 2].copy_from_slice(&(lt_en.len() as u16).to_be_bytes());
        out[lt0 + 2..lt0 + 4].copy_from_slice(&off_lt_en.to_be_bytes());
        // LangTagRecord[1] = "zh-Hant-HK" (language ID 0x8001).
        let lt1 = lt0 + LANG_TAG_RECORD_LEN;
        out[lt1..lt1 + 2].copy_from_slice(&(lt_zh.len() as u16).to_be_bytes());
        out[lt1 + 2..lt1 + 4].copy_from_slice(&off_lt_zh.to_be_bytes());

        // Storage.
        let mut p = storage_offset;
        out[p..p + s_en.len()].copy_from_slice(&s_en);
        p += s_en.len();
        out[p..p + s_zh.len()].copy_from_slice(&s_zh);
        p += s_zh.len();
        out[p..p + lt_en.len()].copy_from_slice(&lt_en);
        p += lt_en.len();
        out[p..p + lt_zh.len()].copy_from_slice(&lt_zh);

        out
    }

    #[test]
    fn decodes_utf16_be_v0() {
        let bytes = build_minimal_v0();
        let n = NameTable::parse(&bytes).unwrap();
        assert_eq!(n.version(), 0);
        assert_eq!(n.find(1), Some("Hi"));
        assert_eq!(n.get(NameId::FontFamily), Some("Hi"));
        assert_eq!(n.record_count(), 1);
        assert_eq!(n.lang_tag_count(), 0);
    }

    #[test]
    fn rejects_version_above_one() {
        let mut bytes = build_minimal_v0();
        bytes[0..2].copy_from_slice(&2u16.to_be_bytes());
        assert_eq!(
            NameTable::parse(&bytes).unwrap_err(),
            Error::BadStructure("name.version > 1")
        );
    }

    #[test]
    fn name_id_round_trip_for_all_standard_ids() {
        for raw in 0u16..=25 {
            let nid = NameId::from_raw(raw).expect("standard ID");
            assert_eq!(nid.to_raw(), raw);
        }
        assert_eq!(NameId::from_raw(26), None);
        assert_eq!(NameId::from_raw(255), None);
        assert_eq!(NameId::from_raw(256), None);
        assert_eq!(NameId::from_raw(32767), None);
    }

    #[test]
    fn name_id_reserved_fifteen_is_distinct() {
        // The spec lists name ID 15 as reserved; we still surface it
        // as a distinct enum variant so callers can detect it.
        assert_eq!(NameId::from_raw(15), Some(NameId::Reserved15));
        assert_ne!(NameId::Reserved15, NameId::LicenseUrl);
        assert_ne!(NameId::Reserved15, NameId::TypographicFamily);
    }

    #[test]
    fn parses_v1_header_and_lang_tag_count() {
        let bytes = build_v1_with_lang_tags();
        let n = NameTable::parse(&bytes).unwrap();
        assert_eq!(n.version(), 1);
        assert_eq!(n.record_count(), 2);
        assert_eq!(n.lang_tag_count(), 2);
    }

    #[test]
    fn v1_lang_tag_resolves_per_spec_offsets() {
        let bytes = build_v1_with_lang_tags();
        let n = NameTable::parse(&bytes).unwrap();
        // Spec: "language ID 0x8000 + i indexes langTagRecord[i]."
        assert_eq!(n.lang_tag(0x8000).as_deref(), Some("en"));
        assert_eq!(n.lang_tag(0x8001).as_deref(), Some("zh-Hant-HK"));
        // Out-of-range IDs surface as None per spec's "should not be
        // used" recommendation.
        assert_eq!(n.lang_tag(0x8002), None);
        assert_eq!(n.lang_tag(0xFFFF), None);
        // Numeric (platform-specific) IDs < 0x8000 are not language
        // tags.
        assert_eq!(n.lang_tag(0x0409), None);
        assert_eq!(n.lang_tag(0x0000), None);
    }

    #[test]
    fn v0_lang_tag_always_returns_none() {
        let bytes = build_minimal_v0();
        let n = NameTable::parse(&bytes).unwrap();
        assert_eq!(n.version(), 0);
        assert_eq!(n.lang_tag(0x8000), None);
        assert_eq!(n.lang_tag(0x8001), None);
    }

    #[test]
    fn records_iter_in_directory_order() {
        let bytes = build_v1_with_lang_tags();
        let n = NameTable::parse(&bytes).unwrap();
        let recs: Vec<_> = n.records().collect();
        assert_eq!(recs.len(), 2);
        assert_eq!(recs[0].platform_id, 3);
        assert_eq!(recs[0].encoding_id, 1);
        assert_eq!(recs[0].language_id, 0x0409);
        assert_eq!(recs[0].name_id_raw, 1);
        assert_eq!(recs[0].name_id(), Some(NameId::FontFamily));
        assert_eq!(recs[1].language_id, 0x8001);
        // Decoded values.
        assert_eq!(n.record_value(recs[0]).as_deref(), Some("Family"));
        assert_eq!(n.record_value(recs[1]).as_deref(), Some("字体"));
    }

    #[test]
    fn v1_truncated_at_lang_tag_count_field() {
        // Build a valid v1 table then chop off the trailing storage
        // *and* the langTagRecord array bytes so the count field is
        // there but the array isn't.
        let bytes = build_v1_with_lang_tags();
        // Header + records + 2-byte count = where the array starts.
        let cut_at = NAME_HEADER_LEN + 2 * NAME_RECORD_LEN + 2;
        let short = &bytes[..cut_at];
        // The langTagCount = 2 → 8 bytes of records expected; cutting
        // here makes the array short.
        assert_eq!(NameTable::parse(short).unwrap_err(), Error::UnexpectedEof);
    }

    #[test]
    fn v1_missing_lang_tag_count_field_rejected() {
        // A v1 header + records but no langTagCount uint16 at all.
        let bytes = build_v1_with_lang_tags();
        let cut_at = NAME_HEADER_LEN + 2 * NAME_RECORD_LEN + 1; // one byte into the count field
        let short = &bytes[..cut_at];
        assert_eq!(NameTable::parse(short).unwrap_err(), Error::UnexpectedEof);
    }

    #[test]
    fn v1_storage_offset_into_lang_tag_array_rejected() {
        // Forge a v1 table where storageOffset lies inside the
        // langTagRecord array — invalid per the spec's table layout.
        let mut bytes = build_v1_with_lang_tags();
        // Move storageOffset back to right after the records (skipping
        // langTagCount + array), which causes the array to overlap
        // storage.
        let bad_offset = (NAME_HEADER_LEN + 2 * NAME_RECORD_LEN + 2) as u16;
        bytes[4..6].copy_from_slice(&bad_offset.to_be_bytes());
        assert_eq!(
            NameTable::parse(&bytes).unwrap_err(),
            Error::BadStructure("name.storageOffset overlaps langTagRecord array")
        );
    }

    #[test]
    fn rejects_string_offset_past_table_end() {
        let mut bytes = build_minimal_v0();
        // Set storageOffset to a value past the end of the table.
        let bad = bytes.len() as u16 + 1;
        bytes[4..6].copy_from_slice(&bad.to_be_bytes());
        assert_eq!(NameTable::parse(&bytes).unwrap_err(), Error::BadOffset);
    }

    #[test]
    fn rejects_truncated_record_array() {
        let mut bytes = build_minimal_v0();
        // Claim count=2 but only one record is actually present.
        bytes[2..4].copy_from_slice(&2u16.to_be_bytes());
        assert_eq!(NameTable::parse(&bytes).unwrap_err(), Error::UnexpectedEof);
    }

    #[test]
    fn decodes_utf16_be_surrogate_pair() {
        // Build a tiny v0 table whose single record decodes a
        // supplementary-plane codepoint (U+1F600 GRINNING FACE =
        // surrogate pair 0xD83D 0xDE00).
        let payload = b"\xD8\x3D\xDE\x00".to_vec();
        let header_size = NAME_HEADER_LEN + NAME_RECORD_LEN;
        let mut out = vec![0u8; header_size];
        out[0..2].copy_from_slice(&0u16.to_be_bytes());
        out[2..4].copy_from_slice(&1u16.to_be_bytes());
        out[4..6].copy_from_slice(&(header_size as u16).to_be_bytes());
        out[6..8].copy_from_slice(&3u16.to_be_bytes());
        out[8..10].copy_from_slice(&1u16.to_be_bytes());
        out[10..12].copy_from_slice(&0u16.to_be_bytes()); // language = 0
        out[12..14].copy_from_slice(&19u16.to_be_bytes()); // nameID = SampleText
        out[14..16].copy_from_slice(&(payload.len() as u16).to_be_bytes());
        out[16..18].copy_from_slice(&0u16.to_be_bytes());
        out.extend_from_slice(&payload);
        let n = NameTable::parse(&out).unwrap();
        let recs: Vec<_> = n.records().collect();
        assert_eq!(n.record_value(recs[0]).as_deref(), Some("\u{1F600}"));
    }

    #[test]
    fn rejects_unpaired_low_surrogate() {
        let payload = b"\xDE\x00\x00\x41".to_vec(); // low surrogate then 'A'
        let header_size = NAME_HEADER_LEN + NAME_RECORD_LEN;
        let mut out = vec![0u8; header_size];
        out[0..2].copy_from_slice(&0u16.to_be_bytes());
        out[2..4].copy_from_slice(&1u16.to_be_bytes());
        out[4..6].copy_from_slice(&(header_size as u16).to_be_bytes());
        out[6..8].copy_from_slice(&3u16.to_be_bytes());
        out[8..10].copy_from_slice(&1u16.to_be_bytes());
        out[10..12].copy_from_slice(&0u16.to_be_bytes());
        out[12..14].copy_from_slice(&19u16.to_be_bytes());
        out[14..16].copy_from_slice(&(payload.len() as u16).to_be_bytes());
        out[16..18].copy_from_slice(&0u16.to_be_bytes());
        out.extend_from_slice(&payload);
        let n = NameTable::parse(&out).unwrap();
        let recs: Vec<_> = n.records().collect();
        // Unpaired low surrogate => decode fails => None.
        assert_eq!(n.record_value(recs[0]), None);
    }

    #[test]
    fn mac_roman_ascii_subset_decodes_borrowed() {
        // Build a Mac Roman record with all-ASCII bytes.
        let payload = b"AB".to_vec();
        let header_size = NAME_HEADER_LEN + NAME_RECORD_LEN;
        let mut out = vec![0u8; header_size];
        out[0..2].copy_from_slice(&0u16.to_be_bytes());
        out[2..4].copy_from_slice(&1u16.to_be_bytes());
        out[4..6].copy_from_slice(&(header_size as u16).to_be_bytes());
        out[6..8].copy_from_slice(&1u16.to_be_bytes()); // Macintosh
        out[8..10].copy_from_slice(&0u16.to_be_bytes()); // Roman
        out[10..12].copy_from_slice(&0u16.to_be_bytes()); // English
        out[12..14].copy_from_slice(&1u16.to_be_bytes());
        out[14..16].copy_from_slice(&(payload.len() as u16).to_be_bytes());
        out[16..18].copy_from_slice(&0u16.to_be_bytes());
        out.extend_from_slice(&payload);
        let n = NameTable::parse(&out).unwrap();
        assert_eq!(n.find(1), Some("AB"));
    }

    #[test]
    fn windows_unicode_beats_mac_roman_in_find() {
        // Two records for the same name ID — one Windows en-US, one
        // Mac Roman English. The Windows record should win.
        let win_payload: Vec<u8> = "Win".encode_utf16().flat_map(|u| u.to_be_bytes()).collect();
        let mac_payload = b"Mac".to_vec();
        let header_size = NAME_HEADER_LEN + 2 * NAME_RECORD_LEN;
        let mut out = vec![0u8; header_size];
        out[0..2].copy_from_slice(&0u16.to_be_bytes());
        out[2..4].copy_from_slice(&2u16.to_be_bytes());
        out[4..6].copy_from_slice(&(header_size as u16).to_be_bytes());
        // Mac record (lower rank).
        out[6..8].copy_from_slice(&1u16.to_be_bytes());
        out[8..10].copy_from_slice(&0u16.to_be_bytes());
        out[10..12].copy_from_slice(&0u16.to_be_bytes());
        out[12..14].copy_from_slice(&1u16.to_be_bytes());
        out[14..16].copy_from_slice(&(mac_payload.len() as u16).to_be_bytes());
        out[16..18].copy_from_slice(&0u16.to_be_bytes());
        // Windows record (higher rank).
        out[18..20].copy_from_slice(&3u16.to_be_bytes());
        out[20..22].copy_from_slice(&1u16.to_be_bytes());
        out[22..24].copy_from_slice(&0x0409u16.to_be_bytes());
        out[24..26].copy_from_slice(&1u16.to_be_bytes());
        out[26..28].copy_from_slice(&(win_payload.len() as u16).to_be_bytes());
        out[28..30].copy_from_slice(&(mac_payload.len() as u16).to_be_bytes());
        out.extend_from_slice(&mac_payload);
        out.extend_from_slice(&win_payload);
        let n = NameTable::parse(&out).unwrap();
        assert_eq!(n.find(1), Some("Win"));
    }
}
