//! CFF (Compact Font Format) parser — Adobe Technical Note #5176.
//!
//! The CFF table is a self-contained mini-container with its own
//! offset-style header, a small set of "INDEX" arrays (Name, Top
//! DICT, String, Global Subrs), per-font Top DICT entries, a
//! charset, an encoding, a CharStrings INDEX (one Type 2 charstring
//! per glyph), one or more Private DICTs, and Local Subr INDEXes.
//!
//! This module is split into one file per substructure to keep each
//! one short and individually testable. Top-level dispatch (parsing
//! the whole table into a `Cff` struct that the public `Font` API
//! holds) lives in [`Cff::parse`] below.
//!
//! Public surface from this module is re-exported through
//! `crate::lib::CubicOutline` etc. — callers don't usually need to
//! reach into `cff::*` directly.

pub mod charset;
pub mod charstring;
pub mod dict;
pub mod encoding;
pub mod header;
pub mod index;
pub mod private;
pub mod strings;
pub mod subrs;

use crate::outline::CubicOutline;
use crate::Error;

use self::charset::Charset;
use self::charstring::Interpreter;
use self::dict::{Dict, Operator};
use self::encoding::Encoding;
use self::header::CffHeader;
use self::index::Index;
use self::private::PrivateDict;
use self::strings::Strings;

/// CFF Top DICT metadata surfaced for callers.
///
/// All fields are pre-extracted from the Top DICT during `Cff::parse`
/// and cached here so accessors are O(1). String fields are SID
/// indices resolved through the CFF Strings table at lookup time.
#[derive(Debug, Clone, Default)]
pub struct TopMetadata {
    /// `FontBBox` (Top DICT op 5) — font-wide bounding box covering
    /// every glyph, in font-unit coordinates. Per TN5176 §9 the
    /// default is `[0 0 0 0]` (which is itself a sentinel meaning
    /// "consumer should compute the actual bbox from the glyph
    /// outlines").
    pub font_bbox: [f32; 4],
    /// `ItalicAngle` (Top DICT op 12 02), in degrees counterclockwise
    /// from vertical. `0.0` for upright fonts. Default: 0.
    pub italic_angle: f64,
    /// `UnderlinePosition` (Top DICT op 12 03), in font units. Default:
    /// -100 per TN5176 §9.
    pub underline_position: f64,
    /// `UnderlineThickness` (Top DICT op 12 04), in font units.
    /// Default: 50 per TN5176 §9.
    pub underline_thickness: f64,
    /// `isFixedPitch` (Top DICT op 12 01): true if the font is
    /// monospaced. Default: false.
    pub is_fixed_pitch: bool,

    // --- SID-valued string operators -----------------------------------
    /// `Notice` (Top DICT op 1) — copyright / trademark notice. SID.
    pub notice_sid: Option<u16>,
    /// `Copyright` (Top DICT op 12 00) — extended copyright field. SID.
    pub copyright_sid: Option<u16>,
    /// `Version` (Top DICT op 0) — version string. SID.
    pub version_sid: Option<u16>,
    /// `FullName` (Top DICT op 2). SID.
    pub full_name_sid: Option<u16>,
    /// `FamilyName` (Top DICT op 3). SID.
    pub family_name_sid: Option<u16>,
    /// `Weight` (Top DICT op 4) — e.g. "Regular", "Bold". SID.
    pub weight_sid: Option<u16>,
}

/// Parsed CFF table — round-1 supports a single-font CFF (the only
/// shape OpenType allows; the multi-font Name INDEX form is legacy
/// PostScript packaging).
#[derive(Debug, Clone)]
pub struct Cff<'a> {
    /// Original CFF table bytes — every offset in CFF is relative to
    /// the start of the table, so we keep the slice around for
    /// follow-up subroutine / charstring lookups.
    bytes: &'a [u8],

    /// PostScript font name from the Name INDEX (single-font: just
    /// the first entry).
    name: &'a [u8],

    /// All strings (standard + custom). SIDs index into this.
    strings: Strings<'a>,

    /// Global subroutines INDEX — shared by every font in a CFF set;
    /// for our single-font case it's just "the global subrs".
    global_subrs: Index<'a>,

    /// CharStrings INDEX — entry `gid` is the Type 2 charstring for
    /// glyph `gid`.
    charstrings: Index<'a>,

    /// Charset map: gid → SID. `gid == 0` is always `.notdef`; the
    /// charset table only stores `gid >= 1`.
    charset: Charset<'a>,

    /// Encoding map: codepoint → gid (only useful as a fallback —
    /// real OpenType fonts route through the sfnt `cmap` table).
    encoding: Encoding<'a>,

    /// Top-level Private DICT (with merged Local Subrs INDEX).
    private: PrivateDict<'a>,

    /// Pre-extracted Top DICT metadata.
    top: TopMetadata,
}

impl<'a> Cff<'a> {
    /// Parse the contents of a `CFF ` (TN5176, version 1) table.
    pub fn parse(bytes: &'a [u8]) -> Result<Self, Error> {
        let header = CffHeader::parse(bytes)?;
        // Per spec, Name INDEX immediately follows the header.
        let mut cursor = header.size as usize;

        let name_index = Index::parse(bytes, cursor)?;
        cursor = name_index.end;
        if name_index.count == 0 {
            return Err(Error::Cff("empty Name INDEX"));
        }
        // Single-font CFF: just take entry 0.
        let name = name_index.entry(0)?;

        let top_index = Index::parse(bytes, cursor)?;
        cursor = top_index.end;
        if top_index.count != name_index.count {
            return Err(Error::Cff("Top DICT INDEX count mismatch"));
        }
        let top_bytes = top_index.entry(0)?;
        let top_dict = Dict::parse(top_bytes)?;

        let string_index = Index::parse(bytes, cursor)?;
        cursor = string_index.end;
        let strings = Strings::new(string_index);

        let global_subrs = Index::parse(bytes, cursor)?;
        // We deliberately don't advance `cursor` past Global Subrs —
        // every subsequent table is referenced by absolute offset
        // from Top DICT.

        // CharStrings: required, offset operator 17.
        let cs_off = top_dict
            .get_int(Operator::CharStrings)
            .ok_or(Error::Cff("Top DICT missing CharStrings offset"))?;
        if cs_off < 0 {
            return Err(Error::Cff("negative CharStrings offset"));
        }
        let charstrings = Index::parse(bytes, cs_off as usize)?;

        // Charset: optional offset operator 15. 0 = ISOAdobe (predefined),
        // 1 = Expert (predefined), 2 = ExpertSubset (predefined), >=3 =
        // custom offset into the table.
        let charset_off = top_dict.get_int(Operator::Charset).unwrap_or(0);
        let charset = Charset::parse(bytes, charset_off, charstrings.count)?;

        // Encoding: optional offset operator 16. 0 = Standard, 1 =
        // Expert, >=2 = custom offset.
        let encoding_off = top_dict.get_int(Operator::Encoding).unwrap_or(0);
        let encoding = Encoding::parse(bytes, encoding_off)?;

        // Private DICT: required for non-CID fonts. Format: array of
        // two ints [size, offset] under operator 18.
        let private_arr = top_dict
            .get_array(Operator::Private)
            .ok_or(Error::Cff("Top DICT missing Private"))?;
        if private_arr.len() != 2 {
            return Err(Error::Cff("Private operand must be [size, offset]"));
        }
        let priv_size = private_arr[0].as_int().ok_or(Error::Cff("Private size"))?;
        let priv_off = private_arr[1]
            .as_int()
            .ok_or(Error::Cff("Private offset"))?;
        if priv_size < 0 || priv_off < 0 {
            return Err(Error::Cff("negative Private size/offset"));
        }
        let private = PrivateDict::parse(bytes, priv_off as usize, priv_size as usize)?;

        let top = extract_top_metadata(&top_dict);

        Ok(Self {
            bytes,
            name,
            strings,
            global_subrs,
            charstrings,
            charset,
            encoding,
            private,
            top,
        })
    }

    /// Borrow the extracted Top DICT metadata (FontBBox, italic
    /// angle, underline metrics, fixed-pitch flag, name SIDs).
    pub fn top_metadata(&self) -> &TopMetadata {
        &self.top
    }

    /// Resolve a Top DICT string operator's SID to its actual string,
    /// looked up through the CFF Strings table.
    pub(crate) fn resolve_sid(&self, sid: u16) -> Option<&str> {
        self.strings.get(sid)
    }

    /// Number of glyphs (== CharStrings INDEX count).
    pub fn glyph_count(&self) -> u16 {
        // Practical fonts cap at u16; CFF technically allows u32 but
        // OpenType bolts a u16 maxp.numGlyphs on top, so we mirror.
        self.charstrings.count.min(u16::MAX as u32) as u16
    }

    /// PostScript font name (typically ASCII, but spec allows any
    /// printable bytes other than `[(){}<>/%`).
    pub fn ps_name(&self) -> &'a [u8] {
        self.name
    }

    /// Look up a glyph id by codepoint via the CFF Encoding (legacy
    /// PostScript path — most callers should route through the sfnt
    /// `cmap` table instead).
    pub fn encoding_lookup(&self, codepoint: u8) -> Option<u16> {
        self.encoding
            .lookup(codepoint, &self.charset, &self.strings)
    }

    /// Decode the Type 2 charstring for `gid` into a cubic-Bezier
    /// outline.
    pub fn glyph_outline(&self, gid: u16) -> Result<CubicOutline, Error> {
        let gid_u = gid as u32;
        if gid_u >= self.charstrings.count {
            return Err(Error::GlyphOutOfRange(gid));
        }
        let cs = self.charstrings.entry(gid_u)?;
        let mut interp = Interpreter::new(
            &self.global_subrs,
            self.private.local_subrs.as_ref(),
            self.private.nominal_width_x,
            self.private.default_width_x,
        );
        interp.run(cs)?;
        let mut outline = interp.into_outline();
        outline.recompute_bounds();
        Ok(outline)
    }

    /// Borrowed CFF table bytes (mostly useful for diagnostics).
    pub fn bytes(&self) -> &'a [u8] {
        self.bytes
    }

    /// Borrow the strings table (used by the higher-level glyph-name
    /// accessors on `Font`).
    pub(crate) fn strings(&self) -> &Strings<'a> {
        &self.strings
    }

    /// Borrow the charset (used to resolve glyph names by gid).
    pub(crate) fn charset(&self) -> &Charset<'a> {
        &self.charset
    }
}

/// Pull the Top DICT metadata fields we surface on the public API out
/// of the parsed DICT. Operators not present in the font fall back to
/// the TN5176 §9 / Table 9 defaults (italicAngle = 0,
/// underlinePosition = -100, underlineThickness = 50, isFixedPitch =
/// false, FontBBox = [0, 0, 0, 0]).
fn extract_top_metadata(dict: &Dict) -> TopMetadata {
    let font_bbox = dict
        .get_array(Operator::FontBBox)
        .map(|operands| {
            // FontBBox is a 4-number array [xMin yMin xMax yMax].
            // Tolerate non-conforming fonts that emit the wrong count
            // by zero-filling missing slots.
            let mut out = [0.0f32; 4];
            for (i, o) in operands.iter().take(4).enumerate() {
                out[i] = o.as_f64() as f32;
            }
            out
        })
        .unwrap_or([0.0; 4]);

    let italic_angle = dict.get_number(Operator::ItalicAngle).unwrap_or(0.0);
    let underline_position = dict
        .get_number(Operator::UnderlinePosition)
        .unwrap_or(-100.0);
    let underline_thickness = dict
        .get_number(Operator::UnderlineThickness)
        .unwrap_or(50.0);
    let is_fixed_pitch = dict.get_int(Operator::IsFixedPitch).unwrap_or(0) != 0;

    // SID-valued operators — these are simple integer SIDs.
    let to_sid = |op| dict.get_int(op).and_then(|i| u16::try_from(i).ok());
    let notice_sid = to_sid(Operator::Notice);
    let copyright_sid = to_sid(Operator::Copyright);
    let version_sid = to_sid(Operator::Version);
    let full_name_sid = to_sid(Operator::FullName);
    let family_name_sid = to_sid(Operator::FamilyName);
    let weight_sid = to_sid(Operator::Weight);

    TopMetadata {
        font_bbox,
        italic_angle,
        underline_position,
        underline_thickness,
        is_fixed_pitch,
        notice_sid,
        copyright_sid,
        version_sid,
        full_name_sid,
        family_name_sid,
        weight_sid,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn top_metadata_defaults_when_dict_empty() {
        let dict = Dict::parse(&[]).unwrap();
        let m = extract_top_metadata(&dict);
        assert_eq!(m.font_bbox, [0.0, 0.0, 0.0, 0.0]);
        assert_eq!(m.italic_angle, 0.0);
        assert_eq!(m.underline_position, -100.0);
        assert_eq!(m.underline_thickness, 50.0);
        assert!(!m.is_fixed_pitch);
        assert_eq!(m.notice_sid, None);
        assert_eq!(m.copyright_sid, None);
        assert_eq!(m.version_sid, None);
        assert_eq!(m.family_name_sid, None);
    }

    #[test]
    fn top_metadata_picks_up_fontbbox_and_italic() {
        // FontBBox = [-100, -200, 1000, 800] under operator 5.
        // Operands encoded with TN5176 integer encoding.
        // -100 → 139 + (-100) is out of range for 32..246; use 251..254 form.
        //   -100 = -((b0-251)*256) - b1 - 108. Pick b0=251 → 0 - b1 - 108
        //   = -b1 - 108 = -100 → b1 = -8 (invalid). So pick b0=251, then
        //   -100 = -((251-251)*256) - b1 - 108 → b1 = -100 - 108 = -208
        //   (out of range). Use 28+i16 instead.
        // We'll just use opcode 28 (3-byte i16) for all four for clarity.
        let mut buf = Vec::new();
        for v in [-100i16, -200, 1000, 800] {
            buf.push(28);
            buf.extend_from_slice(&v.to_be_bytes());
        }
        buf.push(5); // FontBBox operator

        // italicAngle = -12 (op 12 02). -12 via opcode 32..246: 139 + (-12)
        // = 127, in range.
        buf.push(127);
        buf.push(12);
        buf.push(2);

        // isFixedPitch = 1 (op 12 01). 1 → 140.
        buf.push(140);
        buf.push(12);
        buf.push(1);

        let dict = Dict::parse(&buf).unwrap();
        let m = extract_top_metadata(&dict);
        assert_eq!(m.font_bbox, [-100.0, -200.0, 1000.0, 800.0]);
        assert_eq!(m.italic_angle, -12.0);
        assert!(m.is_fixed_pitch);
        // Underline defaults still applied because we didn't set them.
        assert_eq!(m.underline_position, -100.0);
        assert_eq!(m.underline_thickness, 50.0);
    }
}
