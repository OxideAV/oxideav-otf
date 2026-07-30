//! UAX #24 — Unicode **Script** and **Script_Extensions (scx)**
//! property support for script itemization.
//!
//! Source: `docs/text/unicode-script/uax24-script-extensions.md`
//! (Unicode Standard Annex #24, *Unicode Script Property*, Unicode
//! 17.0.0 revision 39; © Unicode, Inc.). This module implements the
//! machinery UAX #24 defines around the property data:
//!
//! - [`ScriptData::parse`] — the `Scripts.txt` UCD data-file format
//!   (§4.1): `codepoint-or-range ; Script-value`, default **Unknown**
//!   for unlisted code points.
//! - [`ScriptExtensions::parse`] — the `ScriptExtensions.txt` format
//!   (§4.2): `codepoint-or-range ; space-delimited short Script
//!   values`, default `{ Script(cp) }` for unlisted code points.
//! - [`loose_match`] — UAX #44 §5.9 loose property-value matching
//!   (case-insensitive; spaces, hyphens, and underscores ignored),
//!   the comparison UAX #24 §2.2 prescribes for script names.
//! - [`validate_scx_set`] — the §3.1 well-formedness rules for scx
//!   sets, including every ill-formed case of Table 8.
//! - [`resolve_sequence_script`] — the §5.2 combining-sequence
//!   resolution strategy: the Script of the first non-Inherited,
//!   non-Common value if one exists, otherwise Common.
//! - [`scx_compatible`] — the §5.3 run-continuation test: two scx
//!   sets are compatible when they intersect (implicit values act as
//!   wildcards), the check that keeps U+30FC out of a Latin run but
//!   inside a Hiragana/Katakana one.
//!
//! The per-code-point **data** lives in the UCD files. Verbatim
//! copies of the Unicode 17.0.0 `Scripts.txt` /
//! `ScriptExtensions.txt` / `PropertyValueAliases.txt` (staged under
//! `docs/text/opentype/ucd/`, © Unicode, Inc., distributed under the
//! UNICODE LICENSE V3 with the notice retained in each file's header)
//! are vendored under `data/ucd/` and exposed as lazily-parsed
//! statics ([`vendored_scripts`], [`vendored_script_extensions`],
//! and the [`script_of`] / [`scx_of`] per-character conveniences).
//! The parsers also accept caller-supplied text for newer UCD
//! releases (§3.2 warns scx values change more often than most
//! properties). The three special implicit values are [`COMMON`]
//! (`Zyyy`), [`INHERITED`] (`Zinh`), and [`UNKNOWN`] (`Zzzz`).

use crate::Error;
use std::sync::OnceLock;

/// The implicit `Common` Script value (short form `Zyyy`).
pub const COMMON: &str = "Common";
/// The implicit `Inherited` Script value (short form `Zinh`).
pub const INHERITED: &str = "Inherited";
/// The implicit `Unknown` Script value (short form `Zzzz`).
pub const UNKNOWN: &str = "Unknown";

/// UAX #44 §5.9 loose matching: compare property values
/// case-insensitively, ignoring spaces, hyphens, and underscores
/// (`"Script_Extensions"` ≡ `"scriptextensions"` ≡ `"Script Extensions"`).
pub fn loose_match(a: &str, b: &str) -> bool {
    let mut ia = a
        .chars()
        .filter(|c| !matches!(c, ' ' | '-' | '_'))
        .map(|c| c.to_ascii_lowercase());
    let mut ib = b
        .chars()
        .filter(|c| !matches!(c, ' ' | '-' | '_'))
        .map(|c| c.to_ascii_lowercase());
    loop {
        match (ia.next(), ib.next()) {
            (None, None) => return true,
            (Some(x), Some(y)) if x == y => continue,
            _ => return false,
        }
    }
}

/// Whether `script` is one of the three implicit Script values
/// (Common / Inherited / Unknown, long or short form, loose-matched).
/// The retired `Qaai` alias for Inherited is honored per §2.2.
pub fn is_implicit_script(script: &str) -> bool {
    [COMMON, "Zyyy", INHERITED, "Zinh", "Qaai", UNKNOWN, "Zzzz"]
        .iter()
        .any(|v| loose_match(script, v))
}

/// One `(range, value)` entry from a UCD script data file.
#[derive(Debug, Clone)]
struct RangeEntry {
    first: u32,
    last: u32,
    /// `Scripts.txt`: the single Script value. `ScriptExtensions.txt`:
    /// the space-delimited scx set, kept as parsed.
    values: Vec<Box<str>>,
}

/// Parse one data line (`first[..last] ; field2`) into a range and
/// its raw second field. Returns `None` for blank / comment lines.
fn parse_line(line: &str) -> Result<Option<(u32, u32, &str)>, Error> {
    let line = line.split('#').next().unwrap_or("").trim();
    if line.is_empty() {
        return Ok(None);
    }
    let (range, value) = line
        .split_once(';')
        .ok_or(Error::BadStructure("UCD script data: missing ';'"))?;
    let range = range.trim();
    let (first, last) = match range.split_once("..") {
        Some((a, b)) => (a.trim(), b.trim()),
        None => (range, range),
    };
    let first =
        u32::from_str_radix(first, 16).map_err(|_| Error::BadStructure("UCD: bad code point"))?;
    let last =
        u32::from_str_radix(last, 16).map_err(|_| Error::BadStructure("UCD: bad code point"))?;
    if first > last || last > 0x10FFFF {
        return Err(Error::BadStructure("UCD: bad code-point range"));
    }
    Ok(Some((first, last, value.trim())))
}

/// Binary-search a sorted range list for the entry covering `cp`.
fn find_range(ranges: &[RangeEntry], cp: u32) -> Option<&RangeEntry> {
    let i = match ranges.binary_search_by_key(&cp, |r| r.first) {
        Ok(i) => i,
        Err(0) => return None,
        Err(i) => i - 1,
    };
    let r = &ranges[i];
    (cp >= r.first && cp <= r.last).then_some(r)
}

/// Parsed `Scripts.txt` data: the Script (`sc`) property, a full
/// partition of the codespace with default **Unknown** (§4.1).
#[derive(Debug, Clone)]
pub struct ScriptData {
    ranges: Vec<RangeEntry>,
}

impl ScriptData {
    /// Parse `Scripts.txt`-format text. Lines are
    /// `cp-or-range ; Script-value`; `#` starts a comment; ranges may
    /// appear in any order (they are sorted for lookup).
    pub fn parse(text: &str) -> Result<Self, Error> {
        let mut ranges = Vec::new();
        for line in text.lines() {
            if let Some((first, last, value)) = parse_line(line)? {
                if value.is_empty() || value.contains(char::is_whitespace) {
                    return Err(Error::BadStructure(
                        "Scripts.txt: field 2 must be one value",
                    ));
                }
                ranges.push(RangeEntry {
                    first,
                    last,
                    values: vec![value.into()],
                });
            }
        }
        ranges.sort_by_key(|r| r.first);
        Ok(Self { ranges })
    }

    /// The Script property value of `cp`; **Unknown** when unlisted.
    pub fn script(&self, cp: u32) -> &str {
        find_range(&self.ranges, cp).map_or(UNKNOWN, |r| &r.values[0])
    }

    /// Number of parsed range entries.
    pub fn len(&self) -> usize {
        self.ranges.len()
    }

    /// Whether no entries were parsed.
    pub fn is_empty(&self) -> bool {
        self.ranges.is_empty()
    }
}

/// Parsed `ScriptExtensions.txt` data: the Script_Extensions (`scx`)
/// property (§4.2).
#[derive(Debug, Clone)]
pub struct ScriptExtensions {
    ranges: Vec<RangeEntry>,
}

impl ScriptExtensions {
    /// Parse `ScriptExtensions.txt`-format text: field 2 is a
    /// space-delimited list of short Script values.
    pub fn parse(text: &str) -> Result<Self, Error> {
        let mut ranges = Vec::new();
        for line in text.lines() {
            if let Some((first, last, value)) = parse_line(line)? {
                let values: Vec<Box<str>> = value.split_whitespace().map(|s| s.into()).collect();
                if values.is_empty() {
                    return Err(Error::BadStructure(
                        "ScriptExtensions.txt: empty scx set is disallowed",
                    ));
                }
                ranges.push(RangeEntry {
                    first,
                    last,
                    values,
                });
            }
        }
        ranges.sort_by_key(|r| r.first);
        Ok(Self { ranges })
    }

    /// The explicit scx set listed for `cp`, or `None` when the file
    /// has no entry (in which case the property defaults to the
    /// single-value set `{ Script(cp) }`).
    pub fn scx(&self, cp: u32) -> Option<&[Box<str>]> {
        find_range(&self.ranges, cp).map(|r| r.values.as_slice())
    }

    /// The scx set of `cp` with the §4.2 default applied: the listed
    /// set when present, otherwise `{ Script(cp) }` from `scripts`.
    pub fn scx_or_default<'a>(&'a self, cp: u32, scripts: &'a ScriptData) -> Vec<&'a str> {
        match self.scx(cp) {
            Some(set) => set.iter().map(|s| s.as_ref()).collect(),
            None => vec![scripts.script(cp)],
        }
    }

    /// Number of parsed range entries.
    pub fn len(&self) -> usize {
        self.ranges.len()
    }

    /// Whether no entries were parsed.
    pub fn is_empty(&self) -> bool {
        self.ranges.is_empty()
    }
}

/// Validate an scx set against the §3.1 well-formedness rules, given
/// the code point's Script value `sc`. Each Table 8 ill-formed case
/// yields an error:
///
/// - the empty set (rule A);
/// - duplicate values (rule B, `{Latn Latn}`);
/// - more than one implicit value, or an implicit value mixed with
///   explicit ones (rule C, `{Inherited Common}` / `{Latn Common}`);
/// - a sole implicit value different from `Script(cp)` (`{Common}`
///   with sc = Inherited);
/// - explicit values where `Script(cp)` is Unknown (`{Latn}` with
///   sc = Unknown);
/// - an explicit `Script(cp)` missing from the set (rule D,
///   `{Latn Grek}` with sc = Hani).
pub fn validate_scx_set(set: &[&str], sc: &str) -> Result<(), Error> {
    if set.is_empty() {
        return Err(Error::BadStructure("scx: the empty set is disallowed"));
    }
    for (i, a) in set.iter().enumerate() {
        for b in &set[i + 1..] {
            if loose_match(a, b) {
                return Err(Error::BadStructure("scx: duplicate value in set"));
            }
        }
    }
    let implicit_count = set.iter().filter(|v| is_implicit_script(v)).count();
    if implicit_count > 1 {
        return Err(Error::BadStructure(
            "scx: more than one implicit value in set",
        ));
    }
    if implicit_count == 1 {
        if set.len() > 1 {
            return Err(Error::BadStructure(
                "scx: implicit and explicit values mixed",
            ));
        }
        // A sole implicit value must match the Script property.
        let aliases: &[&[&str]] = &[
            &[COMMON, "Zyyy"],
            &[INHERITED, "Zinh", "Qaai"],
            &[UNKNOWN, "Zzzz"],
        ];
        let same_implicit = aliases.iter().any(|group| {
            group.iter().any(|v| loose_match(set[0], v)) && group.iter().any(|v| loose_match(sc, v))
        });
        if !same_implicit {
            return Err(Error::BadStructure(
                "scx: implicit value does not match Script(cp)",
            ));
        }
        return Ok(());
    }
    // All-explicit set.
    if loose_match(sc, UNKNOWN) || loose_match(sc, "Zzzz") {
        return Err(Error::BadStructure(
            "scx: explicit values with Script(cp) = Unknown",
        ));
    }
    if !is_implicit_script(sc) && !set.iter().any(|v| loose_match(v, sc)) {
        return Err(Error::BadStructure(
            "scx: explicit Script(cp) not in the set",
        ));
    }
    Ok(())
}

/// §5.2 combining-sequence resolution: the Script value of the first
/// non-Inherited, non-Common value in the sequence, if one exists;
/// otherwise **Common**. (Unknown counts as a real value — it is not
/// skipped — matching the "first non-Inherited, non-Common character"
/// wording.)
pub fn resolve_sequence_script<'a, I>(scripts: I) -> &'a str
where
    I: IntoIterator<Item = &'a str>,
{
    for s in scripts {
        let common_or_inherited = [COMMON, "Zyyy", INHERITED, "Zinh", "Qaai"]
            .iter()
            .any(|v| loose_match(s, v));
        if !common_or_inherited {
            return s;
        }
    }
    COMMON
}

/// §5.3 run-continuation test: whether two scx sets are compatible
/// (may belong to the same script run). A set containing an implicit
/// value (Common / Inherited / Unknown) is compatible with anything;
/// otherwise the sets must share at least one script (loose-matched).
pub fn scx_compatible(a: &[&str], b: &[&str]) -> bool {
    if a.iter().any(|v| is_implicit_script(v)) || b.iter().any(|v| is_implicit_script(v)) {
        return true;
    }
    a.iter().any(|x| b.iter().any(|y| loose_match(x, y)))
}

/// Parsed script-property rows of `PropertyValueAliases.txt` (UAX #44
/// §5.8.2): the `sc ; <short> ; <long> [; <other aliases>]` lines
/// that map every Script value between its short form (the ISO 15924
/// four-letter code, e.g. `Latn`) and its long form (`Latin`), plus
/// any retired aliases (e.g. `Qaai` for `Inherited`).
#[derive(Debug, Clone)]
pub struct ScriptAliases {
    /// `(short, long, extra-aliases)` per script, in file order.
    rows: Vec<AliasRow>,
}

/// One `sc` alias row: `(short, long, extra aliases)`.
type AliasRow = (Box<str>, Box<str>, Vec<Box<str>>);

impl ScriptAliases {
    /// Parse `PropertyValueAliases.txt`-format text, keeping the `sc`
    /// (Script) property rows. Lines are `;`-separated fields with
    /// surrounding whitespace trimmed and `#` comments stripped; a
    /// row needs at least the property alias, the short name, and the
    /// long name.
    pub fn parse(text: &str) -> Result<Self, Error> {
        let mut rows = Vec::new();
        for line in text.lines() {
            let line = line.split('#').next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            let mut fields = line.split(';').map(str::trim);
            if fields.next() != Some("sc") {
                continue;
            }
            let (Some(short), Some(long)) = (fields.next(), fields.next()) else {
                return Err(Error::BadStructure(
                    "PropertyValueAliases: sc row needs short and long names",
                ));
            };
            if short.is_empty() || long.is_empty() {
                return Err(Error::BadStructure(
                    "PropertyValueAliases: empty script alias",
                ));
            }
            rows.push((
                short.into(),
                long.into(),
                fields
                    .filter(|f| !f.is_empty())
                    .map(Into::into)
                    .collect::<Vec<Box<str>>>(),
            ));
        }
        Ok(Self { rows })
    }

    /// Find the row matching `name` against any of its aliases
    /// (short, long, or extra), using UAX #44 loose matching.
    fn find(&self, name: &str) -> Option<&AliasRow> {
        self.rows.iter().find(|(short, long, extra)| {
            loose_match(name, short)
                || loose_match(name, long)
                || extra.iter().any(|a| loose_match(name, a))
        })
    }

    /// The short (ISO 15924) form for a script named by any alias:
    /// `"Latin"` → `"Latn"`, `"Qaai"` → `"Zinh"`. Loose-matched.
    pub fn short_name(&self, name: &str) -> Option<&str> {
        self.find(name).map(|(short, _, _)| short.as_ref())
    }

    /// The long form for a script named by any alias: `"latn"` →
    /// `"Latin"`, `"Qaai"` → `"Inherited"`. Loose-matched.
    pub fn long_name(&self, name: &str) -> Option<&str> {
        self.find(name).map(|(_, long, _)| long.as_ref())
    }

    /// Whether two script names denote the same script under this
    /// alias table (loose-matched over all aliases); falls back to a
    /// direct loose comparison when either name is unlisted.
    pub fn same_script(&self, a: &str, b: &str) -> bool {
        match (self.find(a), self.find(b)) {
            (Some(ra), Some(rb)) => std::ptr::eq(ra, rb),
            _ => loose_match(a, b),
        }
    }

    /// Iterate `(short, long)` pairs in file order.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.rows.iter().map(|(s, l, _)| (s.as_ref(), l.as_ref()))
    }

    /// Number of `sc` rows parsed.
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    /// Whether no `sc` rows were parsed.
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }
}

// ---- vendored UCD data -----------------------------------------------------

/// The Unicode version of the UCD data files vendored under
/// `data/ucd/` (each file's first line self-identifies as its
/// versioned filename, e.g. `Scripts-17.0.0.txt`).
pub const VENDORED_UCD_VERSION: &str = "17.0.0";

/// Vendored `Scripts.txt` (Unicode 17.0.0), verbatim.
const SCRIPTS_TXT: &str = include_str!("../data/ucd/Scripts.txt");
/// Vendored `ScriptExtensions.txt` (Unicode 17.0.0), verbatim.
const SCRIPT_EXTENSIONS_TXT: &str = include_str!("../data/ucd/ScriptExtensions.txt");
/// Vendored `PropertyValueAliases.txt` (Unicode 17.0.0), verbatim.
const PROPERTY_VALUE_ALIASES_TXT: &str = include_str!("../data/ucd/PropertyValueAliases.txt");

/// The vendored `Scripts.txt` data, parsed on first use.
pub fn vendored_scripts() -> &'static ScriptData {
    static DATA: OnceLock<ScriptData> = OnceLock::new();
    DATA.get_or_init(|| {
        ScriptData::parse(SCRIPTS_TXT).expect("vendored Scripts.txt is well-formed")
    })
}

/// The vendored `ScriptExtensions.txt` data, parsed on first use.
pub fn vendored_script_extensions() -> &'static ScriptExtensions {
    static DATA: OnceLock<ScriptExtensions> = OnceLock::new();
    DATA.get_or_init(|| {
        ScriptExtensions::parse(SCRIPT_EXTENSIONS_TXT)
            .expect("vendored ScriptExtensions.txt is well-formed")
    })
}

/// The vendored `PropertyValueAliases.txt` script rows, parsed on
/// first use.
pub fn vendored_script_aliases() -> &'static ScriptAliases {
    static DATA: OnceLock<ScriptAliases> = OnceLock::new();
    DATA.get_or_init(|| {
        ScriptAliases::parse(PROPERTY_VALUE_ALIASES_TXT)
            .expect("vendored PropertyValueAliases.txt is well-formed")
    })
}

/// The Script property value of `c` (long form, e.g. `"Latin"`) from
/// the vendored data; [`UNKNOWN`] for unlisted code points (§4.1).
pub fn script_of(c: char) -> &'static str {
    vendored_scripts().script(c as u32)
}

/// The Script_Extensions set of `c` from the vendored data, with the
/// §4.2 default applied: the listed set of short script codes (e.g.
/// `["Hira", "Kana"]`), or the single-value set `{ Script(c) }` (long
/// form) when the file has no entry for `c`.
pub fn scx_of(c: char) -> Vec<&'static str> {
    vendored_script_extensions().scx_or_default(c as u32, vendored_scripts())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The §4.1 / §4.2 example lines, verbatim.
    const SCRIPTS: &str = "\
# Scripts.txt excerpt
0B01;       Oriya # Mn       ORIYA SIGN CANDRABINDU
0B02..0B03; Oriya # Mc   [2] ORIYA SIGN ANUSVARA..ORIYA SIGN VISARGA
0028;       Common
0300..036F; Inherited
0041..005A; Latin
30FC;       Common
";

    const SCX: &str = "\
# ScriptExtensions.txt excerpt
# @missing: 0000..10FFFF; <script>
0640          ; Adlm Arab Mand Mani Ougr Phlp Rohg Sogd Syrc # Lm       ARABIC TATWEEL
064B..0655    ; Arab Syrc # Mn  [11] ARABIC FATHATAN..ARABIC HAMZA BELOW
30FC          ; Hira Kana
";

    #[test]
    fn scripts_lookup_and_default() {
        let s = ScriptData::parse(SCRIPTS).unwrap();
        assert_eq!(s.script(0x0B01), "Oriya");
        assert_eq!(s.script(0x0B02), "Oriya");
        assert_eq!(s.script(0x0B03), "Oriya");
        assert_eq!(s.script(0x0041), "Latin");
        assert_eq!(s.script(0x0350), "Inherited");
        // Default for unlisted code points is Unknown (§4.1).
        assert_eq!(s.script(0x0B04), UNKNOWN);
        assert_eq!(s.script(0xE000), UNKNOWN);
    }

    #[test]
    fn scx_lookup_and_default() {
        let s = ScriptData::parse(SCRIPTS).unwrap();
        let x = ScriptExtensions::parse(SCX).unwrap();
        // U+0640 ARABIC TATWEEL: the 9-script set from §3.
        let tatweel = x.scx(0x0640).unwrap();
        assert_eq!(tatweel.len(), 9);
        assert_eq!(tatweel[0].as_ref(), "Adlm");
        assert!(tatweel.iter().any(|v| v.as_ref() == "Syrc"));
        // Range entry.
        assert_eq!(x.scx(0x064B).unwrap().len(), 2);
        assert_eq!(x.scx(0x0655).unwrap().len(), 2);
        assert!(x.scx(0x0656).is_none());
        // §3 example: U+30FC → {Hira Kana}.
        let mark = x.scx_or_default(0x30FC, &s);
        assert_eq!(mark, vec!["Hira", "Kana"]);
        // Default: single-value set { Script(cp) } (§4.2).
        assert_eq!(x.scx_or_default(0x0B01, &s), vec!["Oriya"]);
        assert_eq!(x.scx_or_default(0xE000, &s), vec![UNKNOWN]);
    }

    #[test]
    fn loose_matching_rules() {
        // UAX #44 §5.9: case-insensitive; spaces/hyphens/underscores
        // ignored.
        assert!(loose_match("Script_Extensions", "scriptextensions"));
        assert!(loose_match("Script Extensions", "SCRIPT-EXTENSIONS"));
        assert!(loose_match("Zinh", "zinh"));
        assert!(!loose_match("Latn", "Latin"));
        assert!(is_implicit_script("common"));
        assert!(is_implicit_script("Qaai")); // retired alias for Inherited
        assert!(is_implicit_script("ZZZZ"));
        assert!(!is_implicit_script("Grek"));
    }

    #[test]
    fn table8_ill_formed_sets() {
        // Every Table 8 row must be rejected.
        assert!(validate_scx_set(&["Latn"], UNKNOWN).is_err());
        assert!(validate_scx_set(&[COMMON], INHERITED).is_err());
        assert!(validate_scx_set(&["Latn", "Latn"], "Latn").is_err());
        assert!(validate_scx_set(&[INHERITED, COMMON], INHERITED).is_err());
        assert!(validate_scx_set(&["Latn", COMMON], "Latn").is_err());
        assert!(validate_scx_set(&["Latn", "Grek"], "Hani").is_err());
        // The empty set is disallowed (rule A).
        assert!(validate_scx_set(&[], "Latn").is_err());
    }

    #[test]
    fn well_formed_sets() {
        // Single implicit matching Script(cp).
        assert!(validate_scx_set(&[COMMON], COMMON).is_ok());
        assert!(validate_scx_set(&[UNKNOWN], UNKNOWN).is_ok());
        // Short-form implicit vs long-form sc, including the retired
        // Qaai alias.
        assert!(validate_scx_set(&["Zinh"], INHERITED).is_ok());
        assert!(validate_scx_set(&["Qaai"], INHERITED).is_ok());
        // Explicit sets containing Script(cp).
        assert!(validate_scx_set(&["Latn"], "Latn").is_ok());
        assert!(validate_scx_set(&["Latn", "Grek"], "Grek").is_ok());
        // Explicit set for a Common/Inherited character (the normal
        // scx case, e.g. U+30FC): Script(cp) need not be listed.
        assert!(validate_scx_set(&["Hira", "Kana"], COMMON).is_ok());
        assert!(validate_scx_set(&["Arab", "Syrc"], INHERITED).is_ok());
    }

    #[test]
    fn sequence_resolution() {
        // First non-Inherited, non-Common value wins.
        assert_eq!(
            resolve_sequence_script([COMMON, INHERITED, "Grek", "Latn"]),
            "Grek"
        );
        // All Common/Inherited → Common.
        assert_eq!(resolve_sequence_script([COMMON, INHERITED]), COMMON);
        assert_eq!(resolve_sequence_script([]), COMMON);
        // Unknown is not skipped.
        assert_eq!(resolve_sequence_script([UNKNOWN, "Latn"]), UNKNOWN);
    }

    #[test]
    fn run_compatibility() {
        // U+30FC {Hira Kana} continues a Katakana run but not Latin.
        assert!(scx_compatible(&["Hira", "Kana"], &["Kana"]));
        assert!(!scx_compatible(&["Hira", "Kana"], &["Latn"]));
        // Implicit values act as wildcards.
        assert!(scx_compatible(&[COMMON], &["Latn"]));
        assert!(scx_compatible(&["Grek"], &["Zinh"]));
        // Multi-script intersection.
        assert!(scx_compatible(&["Arab", "Syrc"], &["Syrc", "Thaa"]));
    }

    #[test]
    fn vendored_data_parses_and_answers_known_values() {
        // The vendored files self-identify their Unicode version on
        // line 1.
        assert!(SCRIPTS_TXT
            .lines()
            .next()
            .unwrap()
            .contains(&format!("Scripts-{VENDORED_UCD_VERSION}.txt")));
        assert!(SCRIPT_EXTENSIONS_TXT
            .lines()
            .next()
            .unwrap()
            .contains(&format!("ScriptExtensions-{VENDORED_UCD_VERSION}.txt")));

        let s = vendored_scripts();
        // Unicode 17.0.0 Scripts.txt carries thousands of ranges; the
        // scx file lists ~200 data lines.
        assert!(s.len() > 2000, "{}", s.len());
        assert!(vendored_script_extensions().len() > 150);

        // Data rows verified against the staged files.
        assert_eq!(script_of('A'), "Latin"); // 0041..005A ; Latin
        assert_eq!(script_of('\u{3041}'), "Hiragana"); // 3041..3096
        assert_eq!(script_of('\u{0640}'), "Common"); // ARABIC TATWEEL
        assert_eq!(script_of('\u{30FC}'), "Common"); // PROLONGED SOUND MARK
        assert_eq!(script_of('\u{0300}'), "Inherited");

        // scx rows: 30FC -> {Hira Kana}; 0640 -> the 9-script set.
        assert_eq!(scx_of('\u{30FC}'), vec!["Hira", "Kana"]);
        let tatweel = scx_of('\u{0640}');
        assert_eq!(tatweel.len(), 9);
        assert!(tatweel.contains(&"Arab"));
        assert!(tatweel.contains(&"Syrc"));
        // Default: { Script(cp) } in long form for unlisted entries.
        assert_eq!(scx_of('A'), vec!["Latin"]);
        // Unlisted code point: Unknown.
        assert_eq!(script_of('\u{E000}'), UNKNOWN); // private use
        assert_eq!(scx_of('\u{E000}'), vec![UNKNOWN]);
    }

    #[test]
    fn script_aliases_resolve_both_directions() {
        // The §5.8.2-format example rows, verbatim style.
        let a = ScriptAliases::parse(
            "\
# PropertyValueAliases.txt excerpt
gc ; L         ; Letter  # not a script row
sc ; Latn      ; Latin
sc ; Zinh      ; Inherited ; Qaai
sc ; Zyyy      ; Common
",
        )
        .unwrap();
        assert_eq!(a.len(), 3);
        assert_eq!(a.short_name("Latin"), Some("Latn"));
        assert_eq!(a.long_name("latn"), Some("Latin"));
        // Retired alias resolves through the extra-alias column.
        assert_eq!(a.short_name("Qaai"), Some("Zinh"));
        assert_eq!(a.long_name("Qaai"), Some("Inherited"));
        // Loose matching per UAX #44 §5.9.
        assert_eq!(a.long_name("ZINH"), Some("Inherited"));
        assert_eq!(a.short_name("in_herited"), Some("Zinh"));
        // same_script across alias forms; unlisted falls back to
        // loose comparison.
        assert!(a.same_script("Qaai", "Inherited"));
        assert!(a.same_script("Latn", "latin"));
        assert!(!a.same_script("Latn", "Common"));
        // Unlisted names ("Grek" is not in this excerpt) fall back to
        // direct loose comparison.
        assert!(!a.same_script("Grek", "greek"));
        assert!(a.same_script("Grek", "GREK"));
        // Unknown name.
        assert_eq!(a.short_name("NoSuchScript"), None);
    }

    #[test]
    fn vendored_aliases_cover_the_script_repertoire() {
        let a = vendored_script_aliases();
        // Unicode 17.0.0 defines 170+ script values.
        assert!(a.len() > 160, "{}", a.len());
        // Rows verified against the staged file.
        assert_eq!(a.short_name("Latin"), Some("Latn"));
        assert_eq!(a.short_name("Hiragana"), Some("Hira"));
        assert_eq!(a.long_name("Aghb"), Some("Caucasian_Albanian"));
        assert_eq!(a.long_name("Qaai"), Some("Inherited"));
        // Every long name resolved by the vendored Scripts.txt data
        // must have an alias row, and round-trip through its short
        // form (spot-check the values used elsewhere in this module).
        for name in [
            "Latin",
            "Greek",
            "Hiragana",
            "Common",
            "Inherited",
            "Unknown",
        ] {
            let short = a.short_name(name).expect(name);
            assert_eq!(a.long_name(short), Some(name));
        }
        // scx short codes resolve to Scripts.txt long values.
        assert_eq!(a.long_name("Hira"), Some("Hiragana"));
        assert_eq!(a.long_name("Kana"), Some("Katakana"));
    }

    #[test]
    fn parse_errors() {
        assert!(ScriptData::parse("0041 Latin").is_err()); // no ';'
        assert!(ScriptData::parse("ZZZZ; Latin").is_err()); // bad hex
        assert!(ScriptData::parse("0050..0041; Latin").is_err()); // reversed
        assert!(ScriptData::parse("110000; Latin").is_err()); // > 10FFFF
        assert!(ScriptData::parse("0041; Latin Greek").is_err()); // 2 values
        assert!(ScriptExtensions::parse("0041; ").is_err()); // empty set
    }
}
