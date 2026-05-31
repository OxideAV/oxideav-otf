//! `OS/2` — OS/2 and Windows Metrics Table.
//!
//! Spec: Microsoft / ISO/IEC 14496-22 OpenType `OS/2` table
//! (`docs/text/opentype/otspec-os2.html`). Carries metrics and
//! style classification originally introduced for the OS/2 platform
//! and required by Windows for installation; modern apps consult it
//! for weight / width / line-spacing / Unicode-range coverage / font
//! embedding licensing.
//!
//! Version landscape (per the spec's "OS/2 Table Formats" preamble,
//! six versions defined, all supported):
//!
//! * **Version 0** (TrueType 1.5) — 68 bytes (the legacy "short"
//!   layout truncated after `usLastCharIndex`, the Apple TrueType
//!   Reference Manual stops there) **or** 78 bytes (Microsoft's
//!   final v0 spec, which added `sTypoAscender` …
//!   `usWinDescent`). Spec note: "applications should check the
//!   table length for a version 0 OS/2 table before reading these
//!   fields." We honour both.
//! * **Version 1** (TrueType 1.66) — 86 bytes; adds
//!   `ulCodePageRange1` / `ulCodePageRange2`.
//! * **Version 2** (OpenType 1.1) — 96 bytes; adds `sxHeight`,
//!   `sCapHeight`, `usDefaultChar`, `usBreakChar`, `usMaxContext`.
//! * **Version 3** (OpenType 1.4) — identical layout to version 2;
//!   semantics of certain fields revised for Unicode 3.2.
//! * **Version 4** (OpenType 1.5) — identical layout to version 2/3;
//!   adds fsSelection bits 7 (USE_TYPO_METRICS), 8 (WWS), 9
//!   (OBLIQUE) and re-tightens fsType bit-0-to-3 mutual exclusion.
//! * **Version 5** (OpenType 1.7) — 100 bytes; adds
//!   `usLowerOpticalPointSize` / `usUpperOpticalPointSize` (TWIPs,
//!   i.e. 1/20th of a point).
//!
//! Layout (cumulative — see spec for the per-version table; offsets
//! are byte offsets from the start of the table):
//!
//! ```text
//!   0  / 2 / version              uint16
//!   2  / 2 / xAvgCharWidth        FWORD (i16)
//!   4  / 2 / usWeightClass        uint16 (1..1000; FW_THIN..FW_BLACK common)
//!   6  / 2 / usWidthClass         uint16 (1..9; FWIDTH_ULTRA_CONDENSED..FWIDTH_ULTRA_EXPANDED)
//!   8  / 2 / fsType               uint16 (embedding licensing bitfield)
//!  10  / 2 / ySubscriptXSize      FWORD
//!  12  / 2 / ySubscriptYSize      FWORD
//!  14  / 2 / ySubscriptXOffset    FWORD
//!  16  / 2 / ySubscriptYOffset    FWORD
//!  18  / 2 / ySuperscriptXSize    FWORD
//!  20  / 2 / ySuperscriptYSize    FWORD
//!  22  / 2 / ySuperscriptXOffset  FWORD
//!  24  / 2 / ySuperscriptYOffset  FWORD
//!  26  / 2 / yStrikeoutSize       FWORD
//!  28  / 2 / yStrikeoutPosition   FWORD
//!  30  / 2 / sFamilyClass         int16   (high byte = class, low byte = subclass)
//!  32  / 10 / panose              uint8[10]
//!  42  / 4 / ulUnicodeRange1      uint32  (bits 0..31)
//!  46  / 4 / ulUnicodeRange2      uint32  (bits 32..63)
//!  50  / 4 / ulUnicodeRange3      uint32  (bits 64..95)
//!  54  / 4 / ulUnicodeRange4      uint32  (bits 96..127)
//!  58  / 4 / achVendID            Tag (uint8[4])
//!  62  / 2 / fsSelection          uint16  (style bitfield)
//!  64  / 2 / usFirstCharIndex     uint16
//!  66  / 2 / usLastCharIndex      uint16
//!  /-- end of v0 short layout (68 bytes) ---------------------/
//!  68  / 2 / sTypoAscender        FWORD
//!  70  / 2 / sTypoDescender       FWORD
//!  72  / 2 / sTypoLineGap         FWORD
//!  74  / 2 / usWinAscent          UFWORD (u16)
//!  76  / 2 / usWinDescent         UFWORD
//!  /-- end of v0 full layout (78 bytes) ----------------------/
//!  78  / 4 / ulCodePageRange1     uint32  (bits 0..31; v1+)
//!  82  / 4 / ulCodePageRange2     uint32  (bits 32..63; v1+)
//!  /-- end of v1 layout (86 bytes) ---------------------------/
//!  86  / 2 / sxHeight             FWORD   (v2+)
//!  88  / 2 / sCapHeight           FWORD   (v2+)
//!  90  / 2 / usDefaultChar        uint16  (v2+)
//!  92  / 2 / usBreakChar          uint16  (v2+)
//!  94  / 2 / usMaxContext         uint16  (v2+)
//!  /-- end of v2/v3/v4 layout (96 bytes) ---------------------/
//!  96  / 2 / usLowerOpticalPointSize  uint16 (TWIPs; v5+)
//!  98  / 2 / usUpperOpticalPointSize  uint16 (TWIPs; v5+)
//!  /-- end of v5 layout (100 bytes) --------------------------/
//! ```
//!
//! This implementation:
//!
//! * Parses every version, including the v0-short variant (Apple's
//!   short layout, 68 bytes).
//! * Decodes every field listed above, plus per-flag bit helpers for
//!   `fsType` (embedding licensing) and `fsSelection` (style bits 0
//!   to 9), and the (class, subclass) split of `sFamilyClass`.
//! * Exposes the four-byte `achVendID` as both `&[u8; 4]` and a
//!   best-effort `&str` (the spec says it's a registered ASCII tag).
//! * Surfaces `usWidthClass` as both the raw enum value and the
//!   spec-defined "% of normal" scale (50 / 62.5 / 75 / 87.5 / 100 /
//!   112.5 / 125 / 150 / 200), so callers can drive variable-font
//!   `wdth`-axis math without a separate lookup.

use crate::parser::{read_i16, read_u16, read_u32};
use crate::Error;

/// Parsed `OS/2` table.
#[derive(Debug, Clone, Copy)]
pub struct Os2Table {
    version: u16,
    table_len: usize,

    // --- Always present (every version ≥ 0, "short" or "full") ----
    x_avg_char_width: i16,
    us_weight_class: u16,
    us_width_class: u16,
    fs_type: u16,
    y_subscript_x_size: i16,
    y_subscript_y_size: i16,
    y_subscript_x_offset: i16,
    y_subscript_y_offset: i16,
    y_superscript_x_size: i16,
    y_superscript_y_size: i16,
    y_superscript_x_offset: i16,
    y_superscript_y_offset: i16,
    y_strikeout_size: i16,
    y_strikeout_position: i16,
    s_family_class: i16,
    panose: [u8; 10],
    ul_unicode_range1: u32,
    ul_unicode_range2: u32,
    ul_unicode_range3: u32,
    ul_unicode_range4: u32,
    ach_vend_id: [u8; 4],
    fs_selection: u16,
    us_first_char_index: u16,
    us_last_char_index: u16,

    // --- v0-full or higher (table_len ≥ 78) -----------------------
    has_typo_metrics: bool,
    s_typo_ascender: i16,
    s_typo_descender: i16,
    s_typo_line_gap: i16,
    us_win_ascent: u16,
    us_win_descent: u16,

    // --- v1 or higher (table_len ≥ 86) ----------------------------
    has_code_page_range: bool,
    ul_code_page_range1: u32,
    ul_code_page_range2: u32,

    // --- v2..v4 (table_len ≥ 96) ---------------------------------
    has_v2_extension: bool,
    sx_height: i16,
    s_cap_height: i16,
    us_default_char: u16,
    us_break_char: u16,
    us_max_context: u16,

    // --- v5 (table_len ≥ 100) ------------------------------------
    has_optical_size: bool,
    us_lower_optical_point_size: u16,
    us_upper_optical_point_size: u16,
}

// --- fsType embedding-licensing bit constants ------------------------------
// Spec: §"fsType" in `docs/text/opentype/otspec-os2.html`.

/// Sub-field mask covering bits 0..3 (usage permissions).
pub const FS_TYPE_USAGE_MASK: u16 = 0x000F;

/// Bit 1 (mask `0x0002`): "Restricted License embedding".
pub const FS_TYPE_RESTRICTED_LICENSE: u16 = 0x0002;

/// Bit 2 (mask `0x0004`): "Preview & Print embedding".
pub const FS_TYPE_PREVIEW_AND_PRINT: u16 = 0x0004;

/// Bit 3 (mask `0x0008`): "Editable embedding".
pub const FS_TYPE_EDITABLE: u16 = 0x0008;

/// Bit 8 (mask `0x0100`): "No subsetting".
pub const FS_TYPE_NO_SUBSETTING: u16 = 0x0100;

/// Bit 9 (mask `0x0200`): "Bitmap embedding only".
pub const FS_TYPE_BITMAP_EMBEDDING_ONLY: u16 = 0x0200;

// --- fsSelection style-bit constants ---------------------------------------
// Spec: §"fsSelection" in `docs/text/opentype/otspec-os2.html`.

/// fsSelection bit 0 — ITALIC.
pub const FS_SELECTION_ITALIC: u16 = 0x0001;
/// fsSelection bit 1 — UNDERSCORE.
pub const FS_SELECTION_UNDERSCORE: u16 = 0x0002;
/// fsSelection bit 2 — NEGATIVE.
pub const FS_SELECTION_NEGATIVE: u16 = 0x0004;
/// fsSelection bit 3 — OUTLINED.
pub const FS_SELECTION_OUTLINED: u16 = 0x0008;
/// fsSelection bit 4 — STRIKEOUT.
pub const FS_SELECTION_STRIKEOUT: u16 = 0x0010;
/// fsSelection bit 5 — BOLD.
pub const FS_SELECTION_BOLD: u16 = 0x0020;
/// fsSelection bit 6 — REGULAR.
pub const FS_SELECTION_REGULAR: u16 = 0x0040;
/// fsSelection bit 7 — USE_TYPO_METRICS (v4+).
pub const FS_SELECTION_USE_TYPO_METRICS: u16 = 0x0080;
/// fsSelection bit 8 — WWS (v4+).
pub const FS_SELECTION_WWS: u16 = 0x0100;
/// fsSelection bit 9 — OBLIQUE (v4+).
pub const FS_SELECTION_OBLIQUE: u16 = 0x0200;

/// fsType embedding-licensing usage permission. Bits 0..3 of `fsType`
/// are mutually exclusive in v3+ (least-restrictive wins in v0..v2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmbeddingPermission {
    /// `0`: Installable embedding.
    Installable,
    /// `2`: Restricted License embedding.
    RestrictedLicense,
    /// `4`: Preview & Print embedding.
    PreviewAndPrint,
    /// `8`: Editable embedding.
    Editable,
    /// Any other value in the 0..15 range (e.g. multiple bits set
    /// simultaneously, a v0..v2 font using bit 0 — which is reserved
    /// in the final spec — or an undefined combination). The raw
    /// 4-bit value is preserved for inspection; callers should treat
    /// it as "vendor-specific" / inspect the font themselves.
    Other(u8),
}

impl Os2Table {
    /// Parse the table from a raw `OS/2` byte slice.
    pub fn parse(bytes: &[u8]) -> Result<Self, Error> {
        // Smallest legal layout (v0 short, Apple's TrueType Reference
        // Manual variant) is 68 bytes; reject anything shorter.
        if bytes.len() < 68 {
            return Err(Error::UnexpectedEof);
        }
        let version = read_u16(bytes, 0)?;
        // The spec defines v0..v5; reject anything else as malformed.
        // We don't pre-validate further here — the table_len-driven
        // tail decoding below decides which optional fields exist
        // (some legacy v0 tables ship without the typo-metrics tail).
        if version > 5 {
            return Err(Error::BadStructure("OS/2 version above 5"));
        }

        let x_avg_char_width = read_i16(bytes, 2)?;
        let us_weight_class = read_u16(bytes, 4)?;
        let us_width_class = read_u16(bytes, 6)?;
        let fs_type = read_u16(bytes, 8)?;
        let y_subscript_x_size = read_i16(bytes, 10)?;
        let y_subscript_y_size = read_i16(bytes, 12)?;
        let y_subscript_x_offset = read_i16(bytes, 14)?;
        let y_subscript_y_offset = read_i16(bytes, 16)?;
        let y_superscript_x_size = read_i16(bytes, 18)?;
        let y_superscript_y_size = read_i16(bytes, 20)?;
        let y_superscript_x_offset = read_i16(bytes, 22)?;
        let y_superscript_y_offset = read_i16(bytes, 24)?;
        let y_strikeout_size = read_i16(bytes, 26)?;
        let y_strikeout_position = read_i16(bytes, 28)?;
        let s_family_class = read_i16(bytes, 30)?;

        let mut panose = [0u8; 10];
        panose.copy_from_slice(&bytes[32..42]);

        let ul_unicode_range1 = read_u32(bytes, 42)?;
        let ul_unicode_range2 = read_u32(bytes, 46)?;
        let ul_unicode_range3 = read_u32(bytes, 50)?;
        let ul_unicode_range4 = read_u32(bytes, 54)?;

        let mut ach_vend_id = [0u8; 4];
        ach_vend_id.copy_from_slice(&bytes[58..62]);

        let fs_selection = read_u16(bytes, 62)?;
        let us_first_char_index = read_u16(bytes, 64)?;
        let us_last_char_index = read_u16(bytes, 66)?;

        // v0-full and up: typo-metrics tail. Some legacy v0 tables
        // truncate the table at this point; we accept the short
        // layout and leave the typo fields zero. v1+ requires the
        // full layout — if a v1+ table is too short, that's a hard
        // BadStructure.
        let has_typo_metrics;
        let s_typo_ascender;
        let s_typo_descender;
        let s_typo_line_gap;
        let us_win_ascent;
        let us_win_descent;
        if bytes.len() >= 78 {
            has_typo_metrics = true;
            s_typo_ascender = read_i16(bytes, 68)?;
            s_typo_descender = read_i16(bytes, 70)?;
            s_typo_line_gap = read_i16(bytes, 72)?;
            us_win_ascent = read_u16(bytes, 74)?;
            us_win_descent = read_u16(bytes, 76)?;
        } else if version == 0 {
            has_typo_metrics = false;
            s_typo_ascender = 0;
            s_typo_descender = 0;
            s_typo_line_gap = 0;
            us_win_ascent = 0;
            us_win_descent = 0;
        } else {
            return Err(Error::BadStructure(
                "OS/2 v1+ truncated before typo-metrics tail",
            ));
        }

        // v1+ code-page range. Optional only on v0; v1+ tables must
        // carry both fields.
        let has_code_page_range;
        let ul_code_page_range1;
        let ul_code_page_range2;
        if bytes.len() >= 86 {
            has_code_page_range = true;
            ul_code_page_range1 = read_u32(bytes, 78)?;
            ul_code_page_range2 = read_u32(bytes, 82)?;
        } else if version <= 1 {
            // v1 truncated → malformed; v0 → simply not present.
            if version == 1 {
                return Err(Error::BadStructure(
                    "OS/2 v1 truncated before ulCodePageRange",
                ));
            }
            has_code_page_range = false;
            ul_code_page_range1 = 0;
            ul_code_page_range2 = 0;
        } else {
            return Err(Error::BadStructure(
                "OS/2 v2+ truncated before ulCodePageRange",
            ));
        }

        // v2+ extension: sxHeight, sCapHeight, usDefaultChar,
        // usBreakChar, usMaxContext.
        let has_v2_extension;
        let sx_height;
        let s_cap_height;
        let us_default_char;
        let us_break_char;
        let us_max_context;
        if bytes.len() >= 96 {
            has_v2_extension = true;
            sx_height = read_i16(bytes, 86)?;
            s_cap_height = read_i16(bytes, 88)?;
            us_default_char = read_u16(bytes, 90)?;
            us_break_char = read_u16(bytes, 92)?;
            us_max_context = read_u16(bytes, 94)?;
        } else if version <= 1 {
            has_v2_extension = false;
            sx_height = 0;
            s_cap_height = 0;
            us_default_char = 0;
            us_break_char = 0;
            us_max_context = 0;
        } else {
            return Err(Error::BadStructure(
                "OS/2 v2+ truncated before sxHeight/sCapHeight tail",
            ));
        }

        // v5: optical-size point-size range, in TWIPs (1/20th point).
        let has_optical_size;
        let us_lower_optical_point_size;
        let us_upper_optical_point_size;
        if bytes.len() >= 100 {
            has_optical_size = true;
            us_lower_optical_point_size = read_u16(bytes, 96)?;
            us_upper_optical_point_size = read_u16(bytes, 98)?;
        } else if version <= 4 {
            has_optical_size = false;
            us_lower_optical_point_size = 0;
            us_upper_optical_point_size = 0;
        } else {
            return Err(Error::BadStructure(
                "OS/2 v5 truncated before usLowerOpticalPointSize",
            ));
        }

        Ok(Self {
            version,
            table_len: bytes.len(),
            x_avg_char_width,
            us_weight_class,
            us_width_class,
            fs_type,
            y_subscript_x_size,
            y_subscript_y_size,
            y_subscript_x_offset,
            y_subscript_y_offset,
            y_superscript_x_size,
            y_superscript_y_size,
            y_superscript_x_offset,
            y_superscript_y_offset,
            y_strikeout_size,
            y_strikeout_position,
            s_family_class,
            panose,
            ul_unicode_range1,
            ul_unicode_range2,
            ul_unicode_range3,
            ul_unicode_range4,
            ach_vend_id,
            fs_selection,
            us_first_char_index,
            us_last_char_index,
            has_typo_metrics,
            s_typo_ascender,
            s_typo_descender,
            s_typo_line_gap,
            us_win_ascent,
            us_win_descent,
            has_code_page_range,
            ul_code_page_range1,
            ul_code_page_range2,
            has_v2_extension,
            sx_height,
            s_cap_height,
            us_default_char,
            us_break_char,
            us_max_context,
            has_optical_size,
            us_lower_optical_point_size,
            us_upper_optical_point_size,
        })
    }

    // --- header --------------------------------------------------------

    /// On-disk `version` field (0..=5).
    pub fn version(&self) -> u16 {
        self.version
    }

    /// On-disk table length in bytes (covers the short-v0 / full-v0 /
    /// v1 / v2 / v5 cases below).
    pub fn table_len(&self) -> usize {
        self.table_len
    }

    // --- v0+ fields (always present) -----------------------------------

    /// `xAvgCharWidth` — average weighted escapement. Spec recommends
    /// **against** using this for layout. Versions 0..2 used a 26-letter
    /// weighted formula; v3+ define it as the arithmetic average of
    /// non-zero glyph widths.
    pub fn x_avg_char_width(&self) -> i16 {
        self.x_avg_char_width
    }

    /// `usWeightClass` — 1..=1000, with 400 = Regular, 700 = Bold per
    /// the spec's common-values table.
    pub fn weight_class(&self) -> u16 {
        self.us_weight_class
    }

    /// `usWidthClass` — 1..=9, with 5 = Medium (normal) per the spec.
    pub fn width_class(&self) -> u16 {
        self.us_width_class
    }

    /// Spec-defined "% of normal" for the current `usWidthClass`. Maps
    /// 1..=9 to (50, 62.5, 75, 87.5, 100, 112.5, 125, 150, 200) per
    /// the spec table; values outside that range round to the
    /// nearest endpoint.
    pub fn width_class_percent(&self) -> f32 {
        match self.us_width_class {
            0 | 1 => 50.0,
            2 => 62.5,
            3 => 75.0,
            4 => 87.5,
            5 => 100.0,
            6 => 112.5,
            7 => 125.0,
            8 => 150.0,
            _ => 200.0,
        }
    }

    /// Raw `fsType` bitfield. See the `FS_TYPE_*` constants and
    /// [`Self::embedding_permission`] for decoded helpers.
    pub fn fs_type(&self) -> u16 {
        self.fs_type
    }

    /// `fsType` bits 0..3 decoded into the named permission. For v3+
    /// fonts these bits are required to be mutually exclusive; for
    /// v0..v2 fonts where multiple may be set simultaneously, this
    /// returns the highest single-bit value matching the table, or
    /// [`EmbeddingPermission::Other`] for unexpected combinations.
    /// Spec note: bit 0 is permanently reserved; any v0..v2 font with
    /// bit 0 set lands in `Other`.
    pub fn embedding_permission(&self) -> EmbeddingPermission {
        let usage = (self.fs_type & FS_TYPE_USAGE_MASK) as u8;
        match usage {
            0 => EmbeddingPermission::Installable,
            2 => EmbeddingPermission::RestrictedLicense,
            4 => EmbeddingPermission::PreviewAndPrint,
            8 => EmbeddingPermission::Editable,
            // 0..15 with multiple or reserved bits set.
            other => EmbeddingPermission::Other(other),
        }
    }

    /// `fsType` bit 8 — "No subsetting" embedding restriction.
    pub fn fs_type_no_subsetting(&self) -> bool {
        self.fs_type & FS_TYPE_NO_SUBSETTING != 0
    }

    /// `fsType` bit 9 — "Bitmap embedding only".
    pub fn fs_type_bitmap_embedding_only(&self) -> bool {
        self.fs_type & FS_TYPE_BITMAP_EMBEDDING_ONLY != 0
    }

    // --- subscript / superscript / strikeout ---------------------------

    /// `ySubscriptXSize`.
    pub fn y_subscript_x_size(&self) -> i16 {
        self.y_subscript_x_size
    }
    /// `ySubscriptYSize`.
    pub fn y_subscript_y_size(&self) -> i16 {
        self.y_subscript_y_size
    }
    /// `ySubscriptXOffset`.
    pub fn y_subscript_x_offset(&self) -> i16 {
        self.y_subscript_x_offset
    }
    /// `ySubscriptYOffset`.
    pub fn y_subscript_y_offset(&self) -> i16 {
        self.y_subscript_y_offset
    }
    /// `ySuperscriptXSize`.
    pub fn y_superscript_x_size(&self) -> i16 {
        self.y_superscript_x_size
    }
    /// `ySuperscriptYSize`.
    pub fn y_superscript_y_size(&self) -> i16 {
        self.y_superscript_y_size
    }
    /// `ySuperscriptXOffset`.
    pub fn y_superscript_x_offset(&self) -> i16 {
        self.y_superscript_x_offset
    }
    /// `ySuperscriptYOffset`.
    pub fn y_superscript_y_offset(&self) -> i16 {
        self.y_superscript_y_offset
    }
    /// `yStrikeoutSize` — recommended strikeout stroke thickness.
    pub fn y_strikeout_size(&self) -> i16 {
        self.y_strikeout_size
    }
    /// `yStrikeoutPosition` — y-coordinate of the strikeout line.
    pub fn y_strikeout_position(&self) -> i16 {
        self.y_strikeout_position
    }

    // --- family classification -----------------------------------------

    /// Raw `sFamilyClass` — high byte is the class id (0..14 defined),
    /// low byte the subclass.
    pub fn s_family_class(&self) -> i16 {
        self.s_family_class
    }

    /// `(class, subclass)` split of `sFamilyClass`. Per the spec the
    /// fields are 8-bit each in network order inside a single int16.
    pub fn family_class_split(&self) -> (u8, u8) {
        let raw = self.s_family_class as u16;
        ((raw >> 8) as u8, (raw & 0xFF) as u8)
    }

    // --- PANOSE / vendor ----------------------------------------------

    /// Raw 10-byte PANOSE classification. Per the spec the entries
    /// are: bFamilyType, bSerifStyle, bWeight, bProportion, bContrast,
    /// bStrokeVariation, bArmStyle, bLetterform, bMidline, bXHeight.
    pub fn panose(&self) -> &[u8; 10] {
        &self.panose
    }

    /// Raw `achVendID` four-byte tag (registered with Microsoft).
    pub fn ach_vend_id(&self) -> &[u8; 4] {
        &self.ach_vend_id
    }

    /// Best-effort UTF-8 / ASCII view of `achVendID`. Returns `None`
    /// when the four bytes aren't valid UTF-8; the field is
    /// spec-required to be ASCII so this is effectively an
    /// "ascii_or_none" helper.
    pub fn ach_vend_id_str(&self) -> Option<&str> {
        std::str::from_utf8(&self.ach_vend_id).ok()
    }

    // --- Unicode-range bitfield ---------------------------------------

    /// `ulUnicodeRange1` (bits 0..31). Each bit indicates that the
    /// font is designed to support the named Unicode block; see the
    /// spec's Unicode-range table for the bit assignments.
    pub fn unicode_range1(&self) -> u32 {
        self.ul_unicode_range1
    }
    /// `ulUnicodeRange2` (bits 32..63).
    pub fn unicode_range2(&self) -> u32 {
        self.ul_unicode_range2
    }
    /// `ulUnicodeRange3` (bits 64..95).
    pub fn unicode_range3(&self) -> u32 {
        self.ul_unicode_range3
    }
    /// `ulUnicodeRange4` (bits 96..127).
    pub fn unicode_range4(&self) -> u32 {
        self.ul_unicode_range4
    }

    /// Test whether `bit` (0..=127) is set in `ulUnicodeRange*`.
    /// Returns `false` for any bit ≥ 128.
    pub fn has_unicode_range_bit(&self, bit: u8) -> bool {
        let word = match bit / 32 {
            0 => self.ul_unicode_range1,
            1 => self.ul_unicode_range2,
            2 => self.ul_unicode_range3,
            3 => self.ul_unicode_range4,
            _ => return false,
        };
        (word >> (bit % 32)) & 1 != 0
    }

    // --- style / character-coverage ------------------------------------

    /// Raw `fsSelection` bitfield. See the `FS_SELECTION_*` constants
    /// and the per-bit helpers below.
    pub fn fs_selection(&self) -> u16 {
        self.fs_selection
    }

    /// fsSelection bit 0 (ITALIC) — italic / oblique glyphs present.
    pub fn is_italic(&self) -> bool {
        self.fs_selection & FS_SELECTION_ITALIC != 0
    }
    /// fsSelection bit 1 (UNDERSCORE) — glyphs underscored.
    pub fn is_underscore(&self) -> bool {
        self.fs_selection & FS_SELECTION_UNDERSCORE != 0
    }
    /// fsSelection bit 2 (NEGATIVE) — foreground/background reversed.
    pub fn is_negative(&self) -> bool {
        self.fs_selection & FS_SELECTION_NEGATIVE != 0
    }
    /// fsSelection bit 3 (OUTLINED) — hollow glyphs.
    pub fn is_outlined(&self) -> bool {
        self.fs_selection & FS_SELECTION_OUTLINED != 0
    }
    /// fsSelection bit 4 (STRIKEOUT) — glyphs overstruck.
    pub fn is_strikeout(&self) -> bool {
        self.fs_selection & FS_SELECTION_STRIKEOUT != 0
    }
    /// fsSelection bit 5 (BOLD) — emboldened glyphs.
    pub fn is_bold(&self) -> bool {
        self.fs_selection & FS_SELECTION_BOLD != 0
    }
    /// fsSelection bit 6 (REGULAR) — standard weight/style. Per spec,
    /// if set, bits 0 and 5 must be clear.
    pub fn is_regular(&self) -> bool {
        self.fs_selection & FS_SELECTION_REGULAR != 0
    }
    /// fsSelection bit 7 (USE_TYPO_METRICS, v4+). Strongly
    /// recommended for new fonts; instructs apps to use
    /// `sTypoAscender − sTypoDescender + sTypoLineGap` for default
    /// line spacing.
    pub fn use_typo_metrics(&self) -> bool {
        self.fs_selection & FS_SELECTION_USE_TYPO_METRICS != 0
    }
    /// fsSelection bit 8 (WWS, v4+) — `name` table strings are
    /// consistent with a weight/width/slope family without requiring
    /// name IDs 21 / 22.
    pub fn is_wws(&self) -> bool {
        self.fs_selection & FS_SELECTION_WWS != 0
    }
    /// fsSelection bit 9 (OBLIQUE, v4+) — font is to be treated as
    /// oblique by processes distinguishing oblique from italic
    /// (e.g. CSS font-matching).
    pub fn is_oblique(&self) -> bool {
        self.fs_selection & FS_SELECTION_OBLIQUE != 0
    }

    /// `usFirstCharIndex` — minimum Unicode codepoint in the font's
    /// `cmap` platform-3 encoding-0/1 subtable. Capped at `0xFFFF`;
    /// supplementary-plane fonts report `0xFFFF` per spec.
    pub fn first_char_index(&self) -> u16 {
        self.us_first_char_index
    }
    /// `usLastCharIndex` — maximum Unicode codepoint, capped at
    /// `0xFFFF` for supplementary-plane fonts per spec.
    pub fn last_char_index(&self) -> u16 {
        self.us_last_char_index
    }

    // --- v0-full / v1+ typo metrics ------------------------------------

    /// `true` when the table is the v0-full layout or any version ≥ 1,
    /// i.e. the `sTypoAscender` / `sTypoDescender` / `sTypoLineGap` /
    /// `usWinAscent` / `usWinDescent` block is present. `false` only
    /// for legacy 68-byte v0-short tables (Apple's TrueType Reference
    /// Manual variant).
    pub fn has_typo_metrics(&self) -> bool {
        self.has_typo_metrics
    }
    /// `sTypoAscender` (FWORD) — typographic ascender. Combine with
    /// `sTypoDescender` + `sTypoLineGap` for default line spacing.
    /// Returns `None` for the v0-short layout.
    pub fn typo_ascender(&self) -> Option<i16> {
        self.has_typo_metrics.then_some(self.s_typo_ascender)
    }
    /// `sTypoDescender` — typically negative.
    pub fn typo_descender(&self) -> Option<i16> {
        self.has_typo_metrics.then_some(self.s_typo_descender)
    }
    /// `sTypoLineGap`.
    pub fn typo_line_gap(&self) -> Option<i16> {
        self.has_typo_metrics.then_some(self.s_typo_line_gap)
    }
    /// `usWinAscent` (UFWORD = u16) — Windows clipping ascender.
    pub fn win_ascent(&self) -> Option<u16> {
        self.has_typo_metrics.then_some(self.us_win_ascent)
    }
    /// `usWinDescent` (UFWORD) — Windows clipping descender,
    /// reported as a positive value per spec.
    pub fn win_descent(&self) -> Option<u16> {
        self.has_typo_metrics.then_some(self.us_win_descent)
    }

    // --- v1+ code-page range ------------------------------------------

    /// `true` when the table carries `ulCodePageRange1` /
    /// `ulCodePageRange2`, i.e. version ≥ 1 with a non-truncated tail.
    pub fn has_code_page_range(&self) -> bool {
        self.has_code_page_range
    }
    /// `ulCodePageRange1` (bits 0..31) — supported codepages.
    /// Returns `None` for v0 fonts.
    pub fn code_page_range1(&self) -> Option<u32> {
        self.has_code_page_range.then_some(self.ul_code_page_range1)
    }
    /// `ulCodePageRange2` (bits 32..63).
    pub fn code_page_range2(&self) -> Option<u32> {
        self.has_code_page_range.then_some(self.ul_code_page_range2)
    }
    /// Test whether codepage bit `bit` (0..=63) is set.
    /// Returns `false` for v0 tables, missing tails, or `bit ≥ 64`.
    pub fn has_code_page_bit(&self, bit: u8) -> bool {
        if !self.has_code_page_range {
            return false;
        }
        let word = match bit / 32 {
            0 => self.ul_code_page_range1,
            1 => self.ul_code_page_range2,
            _ => return false,
        };
        (word >> (bit % 32)) & 1 != 0
    }

    // --- v2..v4 extension ----------------------------------------------

    /// `true` when the table carries `sxHeight` / `sCapHeight` /
    /// `usDefaultChar` / `usBreakChar` / `usMaxContext`.
    pub fn has_v2_extension(&self) -> bool {
        self.has_v2_extension
    }
    /// `sxHeight` (v2+) — height of lowercase x in font units.
    pub fn x_height(&self) -> Option<i16> {
        self.has_v2_extension.then_some(self.sx_height)
    }
    /// `sCapHeight` (v2+) — height of uppercase letters.
    pub fn cap_height(&self) -> Option<i16> {
        self.has_v2_extension.then_some(self.s_cap_height)
    }
    /// `usDefaultChar` (v2+) — Unicode index of the default
    /// (substitute) glyph; spec recommends 0 when there is no
    /// distinguished default glyph.
    pub fn default_char(&self) -> Option<u16> {
        self.has_v2_extension.then_some(self.us_default_char)
    }
    /// `usBreakChar` (v2+) — Unicode index of the word-break
    /// character; conventionally `0x0020` (space).
    pub fn break_char(&self) -> Option<u16> {
        self.has_v2_extension.then_some(self.us_break_char)
    }
    /// `usMaxContext` (v2+) — maximum length of a target glyph
    /// context for any GSUB / GPOS lookup. `1` means single-glyph
    /// only.
    pub fn max_context(&self) -> Option<u16> {
        self.has_v2_extension.then_some(self.us_max_context)
    }

    // --- v5 optical-size range -----------------------------------------

    /// `true` when the table carries
    /// `usLowerOpticalPointSize` / `usUpperOpticalPointSize` (v5+).
    pub fn has_optical_size(&self) -> bool {
        self.has_optical_size
    }
    /// `usLowerOpticalPointSize` (v5+, TWIPs = 1/20th of a point).
    pub fn lower_optical_point_size_twips(&self) -> Option<u16> {
        self.has_optical_size
            .then_some(self.us_lower_optical_point_size)
    }
    /// `usUpperOpticalPointSize` (v5+, TWIPs = 1/20th of a point).
    pub fn upper_optical_point_size_twips(&self) -> Option<u16> {
        self.has_optical_size
            .then_some(self.us_upper_optical_point_size)
    }
    /// Convenience: `usLowerOpticalPointSize` converted from TWIPs
    /// back to points (TWIPs / 20.0).
    pub fn lower_optical_point_size_pt(&self) -> Option<f32> {
        self.lower_optical_point_size_twips()
            .map(|t| t as f32 / 20.0)
    }
    /// Convenience: `usUpperOpticalPointSize` converted from TWIPs
    /// back to points.
    pub fn upper_optical_point_size_pt(&self) -> Option<f32> {
        self.upper_optical_point_size_twips()
            .map(|t| t as f32 / 20.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- builders -----------------------------------------------------

    /// Build a 100-byte (v5) OS/2 table with every field set to a
    /// distinct trace value so each accessor's offset can be verified
    /// independently. Caller may truncate the returned vector to test
    /// shorter layouts.
    fn build_v5_trace() -> Vec<u8> {
        let mut t = vec![0u8; 100];
        // version = 5
        t[0..2].copy_from_slice(&5u16.to_be_bytes());
        // xAvgCharWidth = 519 (signed)
        t[2..4].copy_from_slice(&519i16.to_be_bytes());
        // usWeightClass = 400 (regular)
        t[4..6].copy_from_slice(&400u16.to_be_bytes());
        // usWidthClass = 5 (medium)
        t[6..8].copy_from_slice(&5u16.to_be_bytes());
        // fsType = 0x0108 (Editable, no... actually set 0x0108 → bit 8 + bit 3)
        t[8..10].copy_from_slice(&0x0108u16.to_be_bytes());
        // subscript: x=650,y=600,xo=0,yo=75
        t[10..12].copy_from_slice(&650i16.to_be_bytes());
        t[12..14].copy_from_slice(&600i16.to_be_bytes());
        t[14..16].copy_from_slice(&0i16.to_be_bytes());
        t[16..18].copy_from_slice(&75i16.to_be_bytes());
        // superscript: x=650,y=600,xo=0,yo=350
        t[18..20].copy_from_slice(&650i16.to_be_bytes());
        t[20..22].copy_from_slice(&600i16.to_be_bytes());
        t[22..24].copy_from_slice(&0i16.to_be_bytes());
        t[24..26].copy_from_slice(&350i16.to_be_bytes());
        // yStrikeoutSize = 50, yStrikeoutPosition = 291
        t[26..28].copy_from_slice(&50i16.to_be_bytes());
        t[28..30].copy_from_slice(&291i16.to_be_bytes());
        // sFamilyClass = (class=8, subclass=2) = 0x0802
        t[30..32].copy_from_slice(&0x0802i16.to_be_bytes());
        // panose
        t[32..42].copy_from_slice(&[2, 11, 5, 3, 3, 4, 3, 2, 2, 4]);
        // ulUnicodeRange1/2/3/4
        t[42..46].copy_from_slice(&0xE000_02FFu32.to_be_bytes());
        t[46..50].copy_from_slice(&0x0000_2003u32.to_be_bytes());
        t[50..54].copy_from_slice(&0x0000_0000u32.to_be_bytes());
        t[54..58].copy_from_slice(&0x0000_0000u32.to_be_bytes());
        // achVendID
        t[58..62].copy_from_slice(b"ADBO");
        // fsSelection = REGULAR (bit 6) + USE_TYPO_METRICS (bit 7) + WWS (bit 8)
        t[62..64].copy_from_slice(&0x01C0u16.to_be_bytes());
        // usFirstCharIndex / usLastCharIndex
        t[64..66].copy_from_slice(&0x0020u16.to_be_bytes());
        t[66..68].copy_from_slice(&0xFFFFu16.to_be_bytes());
        // sTypoAscender=1000, sTypoDescender=-326, sTypoLineGap=0,
        // usWinAscent=1000, usWinDescent=326
        t[68..70].copy_from_slice(&1000i16.to_be_bytes());
        t[70..72].copy_from_slice(&(-326i16).to_be_bytes());
        t[72..74].copy_from_slice(&0i16.to_be_bytes());
        t[74..76].copy_from_slice(&1000u16.to_be_bytes());
        t[76..78].copy_from_slice(&326u16.to_be_bytes());
        // ulCodePageRange1/2
        t[78..82].copy_from_slice(&0x2000_019Fu32.to_be_bytes());
        t[82..86].copy_from_slice(&0u32.to_be_bytes());
        // sxHeight=486, sCapHeight=660, usDefaultChar=0,
        // usBreakChar=0x20, usMaxContext=5
        t[86..88].copy_from_slice(&486i16.to_be_bytes());
        t[88..90].copy_from_slice(&660i16.to_be_bytes());
        t[90..92].copy_from_slice(&0u16.to_be_bytes());
        t[92..94].copy_from_slice(&0x0020u16.to_be_bytes());
        t[94..96].copy_from_slice(&5u16.to_be_bytes());
        // usLowerOpticalPointSize=120 TWIPs (=6pt),
        // usUpperOpticalPointSize=240 TWIPs (=12pt)
        t[96..98].copy_from_slice(&120u16.to_be_bytes());
        t[98..100].copy_from_slice(&240u16.to_be_bytes());
        t
    }

    // ---- parse: every version -----------------------------------------

    #[test]
    fn parses_v5_full_layout() {
        let bytes = build_v5_trace();
        let os2 = Os2Table::parse(&bytes).expect("parse");
        assert_eq!(os2.version(), 5);
        assert_eq!(os2.table_len(), 100);

        assert_eq!(os2.x_avg_char_width(), 519);
        assert_eq!(os2.weight_class(), 400);
        assert_eq!(os2.width_class(), 5);
        assert!((os2.width_class_percent() - 100.0).abs() < 1e-6);

        // fsType = 0x0108 → bit 3 (Editable, usage = 8) + bit 8 (NoSubsetting).
        assert_eq!(os2.fs_type(), 0x0108);
        assert_eq!(os2.embedding_permission(), EmbeddingPermission::Editable);
        assert!(os2.fs_type_no_subsetting());
        assert!(!os2.fs_type_bitmap_embedding_only());

        // sub/super/strikeout (matches Source Sans 3 trace).
        assert_eq!(os2.y_subscript_x_size(), 650);
        assert_eq!(os2.y_subscript_y_size(), 600);
        assert_eq!(os2.y_subscript_y_offset(), 75);
        assert_eq!(os2.y_superscript_y_offset(), 350);
        assert_eq!(os2.y_strikeout_size(), 50);
        assert_eq!(os2.y_strikeout_position(), 291);

        // sFamilyClass split.
        assert_eq!(os2.family_class_split(), (8, 2));

        assert_eq!(os2.panose(), &[2, 11, 5, 3, 3, 4, 3, 2, 2, 4]);

        // Unicode-range bitfield + bit query.
        assert_eq!(os2.unicode_range1(), 0xE000_02FF);
        assert!(os2.has_unicode_range_bit(0)); // Basic Latin
        assert!(os2.has_unicode_range_bit(31)); // bit 31 of UR1 set (0xE000_02FF >> 31 = 1)
        assert!(os2.has_unicode_range_bit(45)); // bit 13 of UR2 → bit 45 (0x0000_2003 >> 13 = 1)
        assert!(!os2.has_unicode_range_bit(127));
        assert!(!os2.has_unicode_range_bit(200));

        assert_eq!(os2.ach_vend_id(), b"ADBO");
        assert_eq!(os2.ach_vend_id_str(), Some("ADBO"));

        // fsSelection: REGULAR | USE_TYPO_METRICS | WWS
        assert_eq!(os2.fs_selection(), 0x01C0);
        assert!(!os2.is_italic());
        assert!(!os2.is_bold());
        assert!(os2.is_regular());
        assert!(os2.use_typo_metrics());
        assert!(os2.is_wws());
        assert!(!os2.is_oblique());

        assert_eq!(os2.first_char_index(), 0x0020);
        assert_eq!(os2.last_char_index(), 0xFFFF);

        assert!(os2.has_typo_metrics());
        assert_eq!(os2.typo_ascender(), Some(1000));
        assert_eq!(os2.typo_descender(), Some(-326));
        assert_eq!(os2.typo_line_gap(), Some(0));
        assert_eq!(os2.win_ascent(), Some(1000));
        assert_eq!(os2.win_descent(), Some(326));

        assert!(os2.has_code_page_range());
        assert_eq!(os2.code_page_range1(), Some(0x2000_019F));
        assert_eq!(os2.code_page_range2(), Some(0));
        // Codepage bit 0 = 1252 Latin 1 (spec table). 0x019F has bit 0 set.
        assert!(os2.has_code_page_bit(0));
        assert!(!os2.has_code_page_bit(63));
        assert!(!os2.has_code_page_bit(64));

        assert!(os2.has_v2_extension());
        assert_eq!(os2.x_height(), Some(486));
        assert_eq!(os2.cap_height(), Some(660));
        assert_eq!(os2.default_char(), Some(0));
        assert_eq!(os2.break_char(), Some(0x0020));
        assert_eq!(os2.max_context(), Some(5));

        assert!(os2.has_optical_size());
        assert_eq!(os2.lower_optical_point_size_twips(), Some(120));
        assert_eq!(os2.upper_optical_point_size_twips(), Some(240));
        assert!((os2.lower_optical_point_size_pt().unwrap() - 6.0).abs() < 1e-6);
        assert!((os2.upper_optical_point_size_pt().unwrap() - 12.0).abs() < 1e-6);
    }

    #[test]
    fn parses_v4_layout_drops_optical_size() {
        // v4 layout = v5 minus the trailing 4 bytes of optical-size
        // fields.
        let mut bytes = build_v5_trace();
        bytes[0..2].copy_from_slice(&4u16.to_be_bytes());
        bytes.truncate(96);

        let os2 = Os2Table::parse(&bytes).expect("parse");
        assert_eq!(os2.version(), 4);
        assert_eq!(os2.table_len(), 96);
        assert!(os2.has_v2_extension());
        assert!(!os2.has_optical_size());
        assert_eq!(os2.lower_optical_point_size_twips(), None);
        assert_eq!(os2.upper_optical_point_size_twips(), None);
        // Earlier tail still present.
        assert_eq!(os2.x_height(), Some(486));
        assert_eq!(os2.max_context(), Some(5));
    }

    #[test]
    fn parses_v1_layout_drops_v2_extension() {
        let mut bytes = build_v5_trace();
        bytes[0..2].copy_from_slice(&1u16.to_be_bytes());
        bytes.truncate(86);

        let os2 = Os2Table::parse(&bytes).expect("parse");
        assert_eq!(os2.version(), 1);
        assert_eq!(os2.table_len(), 86);
        assert!(os2.has_typo_metrics());
        assert!(os2.has_code_page_range());
        assert!(!os2.has_v2_extension());
        assert!(!os2.has_optical_size());
        assert_eq!(os2.x_height(), None);
        assert_eq!(os2.cap_height(), None);
        assert_eq!(os2.code_page_range1(), Some(0x2000_019F));
    }

    #[test]
    fn parses_v0_full_layout_drops_code_page_range() {
        let mut bytes = build_v5_trace();
        bytes[0..2].copy_from_slice(&0u16.to_be_bytes());
        bytes.truncate(78);

        let os2 = Os2Table::parse(&bytes).expect("parse");
        assert_eq!(os2.version(), 0);
        assert_eq!(os2.table_len(), 78);
        assert!(os2.has_typo_metrics());
        assert!(!os2.has_code_page_range());
        assert_eq!(os2.code_page_range1(), None);
        assert_eq!(os2.typo_ascender(), Some(1000));
    }

    #[test]
    fn parses_v0_short_layout_drops_typo_metrics() {
        // Apple's TrueType Reference Manual variant: 68 bytes, stops
        // at usLastCharIndex.
        let mut bytes = build_v5_trace();
        bytes[0..2].copy_from_slice(&0u16.to_be_bytes());
        bytes.truncate(68);

        let os2 = Os2Table::parse(&bytes).expect("parse");
        assert_eq!(os2.version(), 0);
        assert_eq!(os2.table_len(), 68);
        assert!(!os2.has_typo_metrics());
        assert!(!os2.has_code_page_range());
        assert!(!os2.has_v2_extension());
        assert_eq!(os2.typo_ascender(), None);
        assert_eq!(os2.win_ascent(), None);
        // Header fields still decoded.
        assert_eq!(os2.weight_class(), 400);
        assert_eq!(os2.first_char_index(), 0x0020);
    }

    // ---- error paths --------------------------------------------------

    #[test]
    fn rejects_truncated_under_68_bytes() {
        let bytes = vec![0u8; 67];
        assert!(matches!(Os2Table::parse(&bytes), Err(Error::UnexpectedEof)));
    }

    #[test]
    fn rejects_version_above_5() {
        let mut bytes = build_v5_trace();
        bytes[0..2].copy_from_slice(&6u16.to_be_bytes());
        assert!(matches!(
            Os2Table::parse(&bytes),
            Err(Error::BadStructure("OS/2 version above 5"))
        ));
    }

    #[test]
    fn rejects_v1_table_truncated_before_typo_block() {
        // v1 declared but only 68 bytes long → v1 spec mandates the
        // typo-metrics block.
        let mut bytes = build_v5_trace();
        bytes[0..2].copy_from_slice(&1u16.to_be_bytes());
        bytes.truncate(68);
        assert!(matches!(
            Os2Table::parse(&bytes),
            Err(Error::BadStructure(_))
        ));
    }

    #[test]
    fn rejects_v1_table_truncated_before_code_page_range() {
        let mut bytes = build_v5_trace();
        bytes[0..2].copy_from_slice(&1u16.to_be_bytes());
        bytes.truncate(78);
        assert!(matches!(
            Os2Table::parse(&bytes),
            Err(Error::BadStructure(_))
        ));
    }

    #[test]
    fn rejects_v2_truncated_before_extension() {
        let mut bytes = build_v5_trace();
        bytes[0..2].copy_from_slice(&2u16.to_be_bytes());
        bytes.truncate(86);
        assert!(matches!(
            Os2Table::parse(&bytes),
            Err(Error::BadStructure(_))
        ));
    }

    #[test]
    fn rejects_v5_truncated_before_optical_size() {
        let mut bytes = build_v5_trace();
        bytes.truncate(96);
        assert!(matches!(
            Os2Table::parse(&bytes),
            Err(Error::BadStructure(_))
        ));
    }

    // ---- decoded helpers ----------------------------------------------

    #[test]
    fn width_class_percent_spec_table() {
        let mut bytes = build_v5_trace();
        for (wd, pct) in &[
            (1u16, 50.0_f32),
            (2, 62.5),
            (3, 75.0),
            (4, 87.5),
            (5, 100.0),
            (6, 112.5),
            (7, 125.0),
            (8, 150.0),
            (9, 200.0),
        ] {
            bytes[6..8].copy_from_slice(&wd.to_be_bytes());
            let os2 = Os2Table::parse(&bytes).expect("parse");
            assert!(
                (os2.width_class_percent() - pct).abs() < 1e-6,
                "wd={wd} → {} expected {pct}",
                os2.width_class_percent(),
            );
        }
    }

    #[test]
    fn embedding_permission_decodes_each_named_value() {
        let mut bytes = build_v5_trace();
        for (raw, expect) in &[
            (0u16, EmbeddingPermission::Installable),
            (2, EmbeddingPermission::RestrictedLicense),
            (4, EmbeddingPermission::PreviewAndPrint),
            (8, EmbeddingPermission::Editable),
        ] {
            bytes[8..10].copy_from_slice(&raw.to_be_bytes());
            let os2 = Os2Table::parse(&bytes).expect("parse");
            assert_eq!(os2.embedding_permission(), *expect, "raw={raw}");
        }
        // Multiple bits set in usage sub-field → Other(raw_low_nibble).
        bytes[8..10].copy_from_slice(&0x000Cu16.to_be_bytes());
        let os2 = Os2Table::parse(&bytes).expect("parse");
        assert_eq!(os2.embedding_permission(), EmbeddingPermission::Other(12));
        // Bit 0 only (spec-reserved; legacy v0..v2 mistake) →
        // Other(1).
        bytes[8..10].copy_from_slice(&0x0001u16.to_be_bytes());
        let os2 = Os2Table::parse(&bytes).expect("parse");
        assert_eq!(os2.embedding_permission(), EmbeddingPermission::Other(1));
    }

    #[test]
    fn fs_selection_bit_helpers_match_constants() {
        let mut bytes = build_v5_trace();
        bytes[62..64].copy_from_slice(&0x03FFu16.to_be_bytes()); // bits 0..9 all set
        let os2 = Os2Table::parse(&bytes).expect("parse");
        assert!(os2.is_italic());
        assert!(os2.is_underscore());
        assert!(os2.is_negative());
        assert!(os2.is_outlined());
        assert!(os2.is_strikeout());
        assert!(os2.is_bold());
        assert!(os2.is_regular());
        assert!(os2.use_typo_metrics());
        assert!(os2.is_wws());
        assert!(os2.is_oblique());

        bytes[62..64].copy_from_slice(&0u16.to_be_bytes());
        let os2 = Os2Table::parse(&bytes).expect("parse");
        assert!(!os2.is_italic());
        assert!(!os2.use_typo_metrics());
    }

    #[test]
    fn ach_vend_id_str_handles_non_ascii() {
        let mut bytes = build_v5_trace();
        bytes[58..62].copy_from_slice(&[0xFFu8, 0xFE, 0xFD, 0xFC]);
        let os2 = Os2Table::parse(&bytes).expect("parse");
        assert_eq!(os2.ach_vend_id(), &[0xFF, 0xFE, 0xFD, 0xFC]);
        assert!(os2.ach_vend_id_str().is_none());
    }

    #[test]
    fn has_unicode_range_bit_walks_every_word() {
        let mut bytes = build_v5_trace();
        // Set bit 96 (word 3, lsb).
        bytes[54..58].copy_from_slice(&0x0000_0001u32.to_be_bytes());
        let os2 = Os2Table::parse(&bytes).expect("parse");
        assert!(os2.has_unicode_range_bit(96));
        assert!(!os2.has_unicode_range_bit(97));
    }

    #[test]
    fn family_class_split_round_trip() {
        let mut bytes = build_v5_trace();
        // 0x0703 → class=7, subclass=3 (Sans Serif / Neo-grotesque).
        bytes[30..32].copy_from_slice(&0x0703i16.to_be_bytes());
        let os2 = Os2Table::parse(&bytes).expect("parse");
        assert_eq!(os2.family_class_split(), (7, 3));
    }

    #[test]
    fn optical_size_pt_conversion_inverts_twips() {
        let mut bytes = build_v5_trace();
        // 9pt → 180 TWIPs, 14.4pt → 288 TWIPs (matches a 9..14.4
        // optical-size range used in some commercial fonts).
        bytes[96..98].copy_from_slice(&180u16.to_be_bytes());
        bytes[98..100].copy_from_slice(&288u16.to_be_bytes());
        let os2 = Os2Table::parse(&bytes).expect("parse");
        assert!((os2.lower_optical_point_size_pt().unwrap() - 9.0).abs() < 1e-6);
        assert!((os2.upper_optical_point_size_pt().unwrap() - 14.4).abs() < 1e-6);
    }
}
