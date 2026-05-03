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

        Ok(Self {
            bytes,
            name,
            strings,
            global_subrs,
            charstrings,
            charset,
            encoding,
            private,
        })
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
