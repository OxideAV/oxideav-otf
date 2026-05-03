//! CFF Strings — SIDs and the predefined standard strings table
//! (Adobe TN5176 Appendix A).
//!
//! All glyph names, the FontInfo strings, and a few other fields are
//! identified by an SID (String Identifier). SIDs 0..390 are
//! predefined and resolve to the entries in the standard-strings
//! table reproduced below; SIDs >= 391 index into the per-font String
//! INDEX, with SID `391 + i` resolving to entry `i`.
//!
//! Round 1 includes the full 391-entry table verbatim from TN5176
//! Appendix A — these are short ASCII labels with no licensing
//! implication. The table is used for glyph-name lookup in
//! `Cff::glyph_name` and as a fallback in the legacy Encoding
//! resolver.

use crate::cff::index::Index;

/// Number of predefined standard strings (SIDs 0..=NUM_STANDARD-1).
pub(crate) const NUM_STANDARD: u16 = 391;

/// Standard strings table — TN5176 Appendix A, in SID order.
/// Transcribed verbatim from the Adobe Compact Font Format
/// Specification (4 Dec 03), pages 31-35. Total: 391 entries
/// (SID 0..=390).
pub(crate) const STANDARD_STRINGS: &[&str] = &[
    // 0..20
    ".notdef", "space", "exclam", "quotedbl", "numbersign", "dollar",
    "percent", "ampersand", "quoteright", "parenleft", "parenright",
    "asterisk", "plus", "comma", "hyphen", "period", "slash", "zero",
    "one", "two", "three",
    // 21..41
    "four", "five", "six", "seven", "eight", "nine", "colon", "semicolon",
    "less", "equal", "greater", "question", "at", "A", "B", "C", "D", "E",
    "F", "G", "H",
    // 42..62
    "I", "J", "K", "L", "M", "N", "O", "P", "Q", "R", "S", "T", "U", "V",
    "W", "X", "Y", "Z", "bracketleft", "backslash", "bracketright",
    // 63..83
    "asciicircum", "underscore", "quoteleft", "a", "b", "c", "d", "e",
    "f", "g", "h", "i", "j", "k", "l", "m", "n", "o", "p", "q", "r",
    // 84..104
    "s", "t", "u", "v", "w", "x", "y", "z", "braceleft", "bar",
    "braceright", "asciitilde", "exclamdown", "cent", "sterling",
    "fraction", "yen", "florin", "section", "currency", "quotesingle",
    // 105..125
    "quotedblleft", "guillemotleft", "guilsinglleft", "guilsinglright",
    "fi", "fl", "endash", "dagger", "daggerdbl", "periodcentered",
    "paragraph", "bullet", "quotesinglbase", "quotedblbase",
    "quotedblright", "guillemotright", "ellipsis", "perthousand",
    "questiondown", "grave", "acute",
    // 126..146
    "circumflex", "tilde", "macron", "breve", "dotaccent", "dieresis",
    "ring", "cedilla", "hungarumlaut", "ogonek", "caron", "emdash", "AE",
    "ordfeminine", "Lslash", "Oslash", "OE", "ordmasculine", "ae",
    "dotlessi", "lslash",
    // 147..167
    "oslash", "oe", "germandbls", "onesuperior", "logicalnot", "mu",
    "trademark", "Eth", "onehalf", "plusminus", "Thorn", "onequarter",
    "divide", "brokenbar", "degree", "thorn", "threequarters",
    "twosuperior", "registered", "minus", "eth",
    // 168..188
    "multiply", "threesuperior", "copyright", "Aacute", "Acircumflex",
    "Adieresis", "Agrave", "Aring", "Atilde", "Ccedilla", "Eacute",
    "Ecircumflex", "Edieresis", "Egrave", "Iacute", "Icircumflex",
    "Idieresis", "Igrave", "Ntilde", "Oacute", "Ocircumflex",
    // 189..209
    "Odieresis", "Ograve", "Otilde", "Scaron", "Uacute", "Ucircumflex",
    "Udieresis", "Ugrave", "Yacute", "Ydieresis", "Zcaron", "aacute",
    "acircumflex", "adieresis", "agrave", "aring", "atilde", "ccedilla",
    "eacute", "ecircumflex", "edieresis",
    // 210..230
    "egrave", "iacute", "icircumflex", "idieresis", "igrave", "ntilde",
    "oacute", "ocircumflex", "odieresis", "ograve", "otilde", "scaron",
    "uacute", "ucircumflex", "udieresis", "ugrave", "yacute", "ydieresis",
    "zcaron", "exclamsmall", "Hungarumlautsmall",
    // 231..251
    "dollaroldstyle", "dollarsuperior", "ampersandsmall", "Acutesmall",
    "parenleftsuperior", "parenrightsuperior", "twodotenleader",
    "onedotenleader", "zerooldstyle", "oneoldstyle", "twooldstyle",
    "threeoldstyle", "fouroldstyle", "fiveoldstyle", "sixoldstyle",
    "sevenoldstyle", "eightoldstyle", "nineoldstyle", "commasuperior",
    "threequartersemdash", "periodsuperior",
    // 252..272
    "questionsmall", "asuperior", "bsuperior", "centsuperior", "dsuperior",
    "esuperior", "isuperior", "lsuperior", "msuperior", "nsuperior",
    "osuperior", "rsuperior", "ssuperior", "tsuperior", "ff", "ffi", "ffl",
    "parenleftinferior", "parenrightinferior", "Circumflexsmall",
    "hyphensuperior",
    // 273..293
    "Gravesmall", "Asmall", "Bsmall", "Csmall", "Dsmall", "Esmall",
    "Fsmall", "Gsmall", "Hsmall", "Ismall", "Jsmall", "Ksmall", "Lsmall",
    "Msmall", "Nsmall", "Osmall", "Psmall", "Qsmall", "Rsmall", "Ssmall",
    "Tsmall",
    // 294..314
    "Usmall", "Vsmall", "Wsmall", "Xsmall", "Ysmall", "Zsmall",
    "colonmonetary", "onefitted", "rupiah", "Tildesmall", "exclamdownsmall",
    "centoldstyle", "Lslashsmall", "Scaronsmall", "Zcaronsmall",
    "Dieresissmall", "Brevesmall", "Caronsmall", "Dotaccentsmall",
    "Macronsmall", "figuredash",
    // 315..335
    "hypheninferior", "Ogoneksmall", "Ringsmall", "Cedillasmall",
    "questiondownsmall", "oneeighth", "threeeighths", "fiveeighths",
    "seveneighths", "onethird", "twothirds", "zerosuperior", "foursuperior",
    "fivesuperior", "sixsuperior", "sevensuperior", "eightsuperior",
    "ninesuperior", "zeroinferior", "oneinferior", "twoinferior",
    // 336..356
    "threeinferior", "fourinferior", "fiveinferior", "sixinferior",
    "seveninferior", "eightinferior", "nineinferior", "centinferior",
    "dollarinferior", "periodinferior", "commainferior", "Agravesmall",
    "Aacutesmall", "Acircumflexsmall", "Atildesmall", "Adieresissmall",
    "Aringsmall", "AEsmall", "Ccedillasmall", "Egravesmall", "Eacutesmall",
    // 357..377
    "Ecircumflexsmall", "Edieresissmall", "Igravesmall", "Iacutesmall",
    "Icircumflexsmall", "Idieresissmall", "Ethsmall", "Ntildesmall",
    "Ogravesmall", "Oacutesmall", "Ocircumflexsmall", "Otildesmall",
    "Odieresissmall", "OEsmall", "Oslashsmall", "Ugravesmall", "Uacutesmall",
    "Ucircumflexsmall", "Udieresissmall", "Yacutesmall", "Thornsmall",
    // 378..390
    "Ydieresissmall", "001.000", "001.001", "001.002", "001.003", "Black",
    "Bold", "Book", "Light", "Medium", "Regular", "Roman", "Semibold",
];

/// CFF strings table — wraps the per-font String INDEX and answers
/// SID lookups against either the predefined standard strings or the
/// per-font INDEX.
#[derive(Debug, Clone)]
pub(crate) struct Strings<'a> {
    custom: Index<'a>,
}

impl<'a> Strings<'a> {
    pub(crate) fn new(custom: Index<'a>) -> Self {
        // note: the spec says NUM_STANDARD == 391, and the constant
        // STANDARD_STRINGS array above has exactly that many entries
        // — checked once with debug_assert so a future edit doesn't
        // silently desync the two.
        debug_assert_eq!(STANDARD_STRINGS.len(), NUM_STANDARD as usize);
        Self { custom }
    }

    /// Resolve `sid` → string slice. Returns `None` for an out-of-range
    /// custom SID.
    pub(crate) fn get(&self, sid: u16) -> Option<&'a str> {
        if (sid as usize) < STANDARD_STRINGS.len() {
            // Standard strings live in &'static str data — extending
            // the lifetime to 'a is sound because they outlive 'a.
            return Some(STANDARD_STRINGS[sid as usize]);
        }
        let custom_idx = (sid as u32) - NUM_STANDARD as u32;
        let bytes = self.custom.entry(custom_idx).ok()?;
        std::str::from_utf8(bytes).ok()
    }

    /// Look up the SID for a glyph name in the standard strings table
    /// (used by the standard / expert encoding lookup). Returns `None`
    /// if the name isn't one of the 391 predefined ones.
    #[allow(dead_code)]
    pub(crate) fn standard_sid(name: &str) -> Option<u16> {
        STANDARD_STRINGS
            .iter()
            .position(|s| *s == name)
            .map(|i| i as u16)
    }
}

/// Map a glyph-name string to the unicode codepoint it represents,
/// using the well-known Adobe Glyph List subset that covers the
/// first ~250 entries of the standard strings table.
///
/// We carry only the entries needed for legacy Encoding lookup; the
/// full AGL is too large to ship here without crossing into "external
/// lookup table" territory and serves no purpose for round 1 (the
/// real `cmap` table in the sfnt directory always wins). Returns
/// `None` for unrecognised names.
pub(crate) fn glyph_name_to_codepoint(_name: &str) -> Option<u32> {
    // round-2 / blocked: this mapping is in the Adobe Glyph List
    // (`AGL`) document. We don't need it for round 1 because every
    // OpenType-CFF font carries a sfnt `cmap` table that supplies
    // the correct codepoint→GID mapping, and that's what the public
    // `Font::glyph_index` uses. The stub is here so `encoding.rs`
    // compiles cleanly and round-2 can wire the table without an API
    // break.
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standard_strings_count_matches() {
        assert_eq!(STANDARD_STRINGS.len(), NUM_STANDARD as usize);
    }

    #[test]
    fn standard_sid_zero_is_notdef() {
        // Empty custom INDEX (count = 0).
        let bytes = vec![0u8, 0];
        let idx = Index::parse(&bytes, 0).unwrap();
        let s = Strings::new(idx);
        assert_eq!(s.get(0), Some(".notdef"));
        assert_eq!(s.get(1), Some("space"));
    }

    #[test]
    fn custom_sid_falls_through_to_index() {
        // Custom INDEX with one entry "myname".
        // count=1, offSize=1, offsets=[1, 7], data="myname".
        let bytes: Vec<u8> = vec![0x00, 0x01, 0x01, 0x01, 0x07, b'm', b'y', b'n', b'a', b'm', b'e'];
        let idx = Index::parse(&bytes, 0).unwrap();
        let s = Strings::new(idx);
        assert_eq!(s.get(NUM_STANDARD), Some("myname"));
    }
}
