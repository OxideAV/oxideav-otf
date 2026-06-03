//! Adobe Glyph List (AGL) — the canonical PostScript glyph-name to
//! Unicode-scalar-value mapping shipped by Adobe.
//!
//! Source: `data/agl-glyphlist.txt`, a verbatim copy of the AGL 2.0
//! table (September 20, 2002) staged under
//! `docs/text/opentype/spec/agl-glyphlist.txt`. The table format is
//! described in `docs/text/opentype/spec/agl-aglfn-README.md` and is a
//! plain-text two-column listing:
//!
//! ```text
//! # comment lines start with '#'
//! A;0041
//! AE;00C6
//! dalethatafpatah;05D3 05B2
//! …
//! ```
//!
//! - Column 1: PostScript glyph name (ASCII letters + digits only).
//! - Column 2: one or more space-separated 4-uppercase-hex-digit
//!   Unicode scalar values. AGL 2.0 ships **4200** single-codepoint
//!   entries plus **81** multi-codepoint sequences (mostly Hebrew
//!   base+point combinations) — **4281 total**.
//!
//! Multiple glyph names can map to the same codepoint sequence (most
//! common case: Hebrew vowel-pointing variants);
//! [`codepoint_to_name`] therefore picks the *first* name in
//! ASCII-sorted order (which is the file's on-disk order — the AGL
//! README documents it as sorted by glyph name in increasing ASCII
//! order).
//!
//! ## Scope
//!
//! This module is **only** the static AGL table lookup. The full AGL
//! Specification §6 ("Mapping glyph names to character sequences")
//! defines an algorithm for decomposing component glyph names like
//! `f_f_i` (→ `ffi`) and the `uniXXXX` / `uXXXXX` hex-encoded forms.
//! Those decomposition rules are not implemented here because the AGL
//! Specification document itself is not staged in `docs/text/opentype/`
//! — only the raw AGL table and its `aglfn-README.md` companion are.
//! Once the AGL Specification is staged, [`name_to_codepoints`] can
//! be extended with the §6 algorithm without an API change (the
//! existing exact-match path stays correct as the spec's step 2
//! "look up the name in AGL").
//!
//! ## Build-time vs. runtime
//!
//! The 78 KB table is parsed at first use via [`std::sync::OnceLock`];
//! no parsing happens at crate-load time. The lookup tables cost
//! ~250 KB resident once initialised. Both lookups are O(1) average.

use std::collections::HashMap;
use std::sync::OnceLock;

/// Raw AGL 2.0 table, included verbatim from `data/agl-glyphlist.txt`.
///
/// Format: `name;XXXX[ XXXX...]\n` per line, comment lines start with
/// `#`, blank lines are ignored. See module docs for the source
/// description.
const AGL_TEXT: &str = include_str!("../data/agl-glyphlist.txt");

/// AGL entry kind: a single Unicode codepoint or a sequence of
/// codepoints (the latter for ~81 Hebrew base + vowel-pointing
/// combinations in AGL 2.0).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Codepoints<'a> {
    /// The glyph name maps to exactly one Unicode scalar value.
    Single(char),
    /// The glyph name maps to a sequence of two or more Unicode
    /// scalar values. The slice is borrowed from the static AGL
    /// table.
    Sequence(&'a [char]),
}

impl<'a> Codepoints<'a> {
    /// Returns the first codepoint of the sequence (or the single
    /// codepoint for a [`Codepoints::Single`]). The AGL has no
    /// empty sequences, so this is always well-defined.
    pub fn first(&self) -> char {
        match *self {
            Codepoints::Single(c) => c,
            // Multi-codepoint sequences in AGL 2.0 are all
            // non-empty; the parser only emits a Sequence when at
            // least two codepoints are present.
            Codepoints::Sequence(slice) => slice[0],
        }
    }

    /// Returns the codepoint slice. For [`Codepoints::Single`] this
    /// is a one-element slice borrowed from the lazily-built parse
    /// table.
    pub fn as_slice(&self) -> &'a [char] {
        match self {
            Codepoints::Single(_) => {
                // We can't return a borrow to the local `Single`
                // copy. Callers that want a slice should construct
                // one from the single codepoint themselves; this
                // accessor is here for the sequence case.
                unreachable!("call as_slice() only on Sequence variants; use first() or match")
            }
            Codepoints::Sequence(s) => s,
        }
    }

    /// Length of the codepoint sequence (always 1 for
    /// [`Codepoints::Single`]).
    pub fn len(&self) -> usize {
        match self {
            Codepoints::Single(_) => 1,
            Codepoints::Sequence(s) => s.len(),
        }
    }

    /// `true` when the sequence is empty (never the case for AGL
    /// entries; included for API completeness).
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Parsed entry stored in the static table: one or more codepoints.
#[derive(Debug)]
enum StaticEntry {
    Single(char),
    Sequence(Vec<char>),
}

/// Parsed `name → entry` table (lazily initialised on first use).
fn name_table() -> &'static HashMap<&'static str, StaticEntry> {
    static MAP: OnceLock<HashMap<&'static str, StaticEntry>> = OnceLock::new();
    MAP.get_or_init(|| {
        let mut m = HashMap::with_capacity(4400);
        for (name, codepoints) in raw_entries() {
            let entry = if codepoints.len() == 1 {
                StaticEntry::Single(codepoints[0])
            } else {
                StaticEntry::Sequence(codepoints)
            };
            // The AGL is name-unique. First-wins on the off-chance
            // a future revision introduces a collision (keeps
            // behaviour deterministic).
            m.entry(name).or_insert(entry);
        }
        m
    })
}

/// Parsed `single-codepoint → first-name` table (lazily initialised).
/// Only single-codepoint entries participate in reverse lookup —
/// multi-codepoint sequences would require the caller to also know
/// the rest of the sequence and aren't surfaced through the simple
/// `char → name` accessor.
fn codepoint_table() -> &'static HashMap<u32, &'static str> {
    static MAP: OnceLock<HashMap<u32, &'static str>> = OnceLock::new();
    MAP.get_or_init(|| {
        let mut m = HashMap::with_capacity(4300);
        for (name, codepoints) in raw_entries() {
            if codepoints.len() != 1 {
                continue;
            }
            // First-wins: on-disk order is ASCII-sorted by name, so
            // this picks the alphabetically-first glyph name for any
            // codepoint that has multiple AGL aliases.
            m.entry(codepoints[0] as u32).or_insert(name);
        }
        m
    })
}

/// Parse the AGL into `(name, Vec<char>)` pairs. The Vec allocations
/// are paid once at table-build time; both `name_table` and
/// `codepoint_table` re-walk this iterator at init.
fn raw_entries() -> impl Iterator<Item = (&'static str, Vec<char>)> {
    AGL_TEXT.lines().filter_map(|line| {
        let line = line.trim_end_matches('\r');
        if line.is_empty() || line.starts_with('#') {
            return None;
        }
        let (name, hex_field) = line.split_once(';')?;
        // The hex field is one or more 4-digit hex codepoints
        // separated by ASCII space (per the AGL spec format).
        let mut codepoints = Vec::with_capacity(2);
        for hex in hex_field.split(' ') {
            if hex.len() != 4 {
                return None;
            }
            let cp = u32::from_str_radix(hex, 16).ok()?;
            let c = char::from_u32(cp)?;
            codepoints.push(c);
        }
        if codepoints.is_empty() {
            return None;
        }
        Some((name, codepoints))
    })
}

/// Resolve an Adobe Glyph List glyph name to its Unicode scalar value
/// sequence.
///
/// Returns `None` for names absent from the AGL 2.0 table. The match
/// is exact: the AGL Specification's §6 component-name decomposition
/// algorithm (`f_f_i` → `ffi`, `uniXXXX` → `U+XXXX`, etc.) is not
/// implemented because the AGL Specification document itself is not
/// staged under `docs/text/opentype/`.
///
/// Most entries surface as [`Codepoints::Single`]; the ~81 Hebrew
/// base + vowel-pointing combinations surface as
/// [`Codepoints::Sequence`].
///
/// ```
/// use oxideav_otf::agl::{name_to_codepoints, Codepoints};
///
/// assert_eq!(name_to_codepoints("A"),          Some(Codepoints::Single('A')));
/// assert_eq!(name_to_codepoints("AE"),         Some(Codepoints::Single('\u{00C6}')));
/// assert_eq!(name_to_codepoints("zero"),       Some(Codepoints::Single('0')));
/// assert!(matches!(
///     name_to_codepoints("dalethatafpatah"),
///     Some(Codepoints::Sequence(s)) if s == ['\u{05D3}', '\u{05B2}']
/// ));
/// assert_eq!(name_to_codepoints("not_a_glyph"), None);
/// ```
pub fn name_to_codepoints(name: &str) -> Option<Codepoints<'static>> {
    match name_table().get(name)? {
        StaticEntry::Single(c) => Some(Codepoints::Single(*c)),
        StaticEntry::Sequence(v) => Some(Codepoints::Sequence(v.as_slice())),
    }
}

/// Resolve an Adobe Glyph List glyph name to its *single* Unicode
/// scalar value. Returns `None` for names absent from AGL **and** for
/// names that map to a multi-codepoint sequence (callers wanting the
/// full sequence should use [`name_to_codepoints`]).
///
/// This is the common-case helper: most font / PDF consumers only
/// care about names that round-trip to one Unicode scalar.
pub fn name_to_codepoint(name: &str) -> Option<char> {
    match name_table().get(name)? {
        StaticEntry::Single(c) => Some(*c),
        StaticEntry::Sequence(_) => None,
    }
}

/// Resolve a Unicode codepoint to its canonical Adobe Glyph List name.
///
/// Returns `None` if no AGL entry maps to this codepoint. When
/// multiple glyph names share a codepoint (most common case: Hebrew
/// vowel-pointing combining marks; ~17 names share U+05B8 in AGL
/// 2.0), this returns the *first* such name in the AGL's on-disk
/// (ASCII-sorted) order. For callers who need every alias, iterate
/// [`entries`]. Multi-codepoint sequence entries (e.g.
/// `dalethatafpatah → [U+05D3, U+05B2]`) do **not** participate in
/// reverse lookup — a single `char` argument can't disambiguate them.
///
/// ```
/// use oxideav_otf::agl::codepoint_to_name;
///
/// assert_eq!(codepoint_to_name('A'),         Some("A"));
/// assert_eq!(codepoint_to_name('\u{00C6}'),  Some("AE"));
/// assert_eq!(codepoint_to_name('0'),         Some("zero"));
/// // U+FFFE is not encoded by Unicode and has no AGL entry.
/// assert_eq!(codepoint_to_name('\u{FFFE}'),  None);
/// ```
pub fn codepoint_to_name(codepoint: char) -> Option<&'static str> {
    codepoint_table().get(&(codepoint as u32)).copied()
}

/// Total number of `(name, codepoints)` entries in the AGL table.
/// Constant per AGL version (4281 in AGL 2.0).
pub fn entry_count() -> usize {
    name_table().len()
}

/// Number of *distinct* codepoints reachable through
/// [`codepoint_to_name`]. AGL 2.0 has 4200 single-codepoint entries
/// but only **3680** distinct codepoints among them — many AGL names
/// alias the same codepoint (Hebrew vowel-pointing combinations, and
/// the Mac / Windows / PUA legacy duplicates).
pub fn distinct_codepoint_count() -> usize {
    codepoint_table().len()
}

/// Iterate every `(name, codepoints)` pair in the AGL in on-disk
/// (ASCII-sorted-by-name) order.
///
/// Both single-codepoint and multi-codepoint entries are yielded.
pub fn entries() -> impl Iterator<Item = (&'static str, Codepoints<'static>)> {
    // Walk the table (which the OnceLock has already parsed by the
    // time anyone is iterating).
    let table = name_table();
    // Re-walk the on-disk order (`raw_entries` is ASCII-sorted),
    // looking each name up in the table to borrow the static slice.
    raw_entries().filter_map(move |(name, _)| {
        let entry = table.get(name)?;
        let cp = match entry {
            StaticEntry::Single(c) => Codepoints::Single(*c),
            StaticEntry::Sequence(v) => Codepoints::Sequence(v.as_slice()),
        };
        Some((name, cp))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entry_count_matches_agl_2_0() {
        // AGL 2.0 ships 4281 (name, codepoints) entries — see the
        // `agl-glyphlist.txt` source (sum of non-comment, non-blank
        // lines). Any change here means the source table was edited;
        // the test is the canary.
        assert_eq!(entry_count(), 4281);
    }

    #[test]
    fn distinct_codepoint_count_is_3680() {
        // AGL 2.0 has 4200 single-codepoint entries but only **3680**
        // *distinct* codepoints — many AGL names alias the same
        // codepoint (Hebrew vowel-pointing variants, Mac / Windows
        // legacy duplicates, and `Acutesmall` / `acutesmall`-style
        // case pairs that share a PUA slot). The remaining 81 of
        // 4281 are multi-codepoint sequences.
        assert_eq!(distinct_codepoint_count(), 3680);
    }

    #[test]
    fn sequence_entry_count_is_81() {
        // Count entries that surface as `Codepoints::Sequence`.
        let n = entries()
            .filter(|(_, c)| !matches!(c, Codepoints::Single(_)))
            .count();
        assert_eq!(n, 81);
    }

    #[test]
    fn ascii_uppercase_letters_round_trip() {
        for c in 'A'..='Z' {
            let name = c.to_string();
            assert_eq!(name_to_codepoint(&name), Some(c), "AGL miss: {name}");
            let reverse = codepoint_to_name(c).expect("ASCII letter has AGL name");
            assert_eq!(reverse, name.as_str(), "reverse lookup for {c}");
        }
    }

    #[test]
    fn ascii_digits_round_trip() {
        let pairs: &[(char, &str)] = &[
            ('0', "zero"),
            ('1', "one"),
            ('2', "two"),
            ('3', "three"),
            ('4', "four"),
            ('5', "five"),
            ('6', "six"),
            ('7', "seven"),
            ('8', "eight"),
            ('9', "nine"),
        ];
        for &(c, name) in pairs {
            assert_eq!(name_to_codepoint(name), Some(c));
            assert_eq!(codepoint_to_name(c), Some(name));
        }
    }

    #[test]
    fn common_punctuation_pst_names() {
        // Worked PostScript-name landmarks from the AGL.
        let pairs: &[(&str, char)] = &[
            ("space", ' '),
            ("exclam", '!'),
            ("quotedbl", '"'),
            ("numbersign", '#'),
            ("dollar", '$'),
            ("percent", '%'),
            ("ampersand", '&'),
            ("parenleft", '('),
            ("parenright", ')'),
            ("comma", ','),
            ("hyphen", '-'),
            ("period", '.'),
            ("slash", '/'),
        ];
        for &(name, c) in pairs {
            assert_eq!(name_to_codepoint(name), Some(c), "AGL miss: {name}");
        }
    }

    #[test]
    fn pua_small_caps_landmarks() {
        // Small-cap forms live in the Adobe Corporate Use Subarea
        // (`F6xx`..`F7xx`). Direct spec landmarks read from the
        // shipped AGL table.
        assert_eq!(name_to_codepoint("Acutesmall"), Some('\u{F7B4}'));
        assert_eq!(name_to_codepoint("Asmall"), Some('\u{F761}'));
        assert_eq!(name_to_codepoint("AEsmall"), Some('\u{F7E6}'));
    }

    #[test]
    fn ligatures_with_bmp_codepoints() {
        // AGL ligatures that have real BMP codepoints (not PUA).
        assert_eq!(name_to_codepoint("AE"), Some('\u{00C6}'));
        assert_eq!(name_to_codepoint("ae"), Some('\u{00E6}'));
        assert_eq!(name_to_codepoint("OE"), Some('\u{0152}'));
        assert_eq!(name_to_codepoint("oe"), Some('\u{0153}'));
        // The ASCII `ffi` ligature maps to the FB03 presentation
        // form per AGL (AGLFN omits it because §6 of the AGL spec
        // decomposes it to f+f+i — but we don't have the spec
        // staged, so the raw AGL entry is what we surface).
        assert_eq!(name_to_codepoint("ffi"), Some('\u{FB03}'));
    }

    #[test]
    fn cjk_landmarks() {
        // AGL covers Japanese kana with the same `hiragana` /
        // `katakana` suffix convention. Spec-listed entries from the
        // file's tail.
        assert_eq!(name_to_codepoint("ahiragana"), Some('\u{3042}'));
        assert_eq!(name_to_codepoint("akatakana"), Some('\u{30A2}'));
        // Last-listed entry in the AGL (per file tail).
        assert_eq!(name_to_codepoint("zukatakana"), Some('\u{30BA}'));
    }

    #[test]
    fn unknown_name_returns_none() {
        assert_eq!(name_to_codepoint(""), None);
        assert_eq!(name_to_codepoint("not_a_real_glyph_name"), None);
        // Names with embedded whitespace are not in AGL (the spec
        // restricts glyph-name characters to letters + digits).
        assert_eq!(name_to_codepoint("A B"), None);
        // Case matters — `a` and `A` are distinct glyph names.
        assert!(name_to_codepoint("A").is_some());
        assert!(name_to_codepoint("a").is_some());
        assert_ne!(name_to_codepoint("A"), name_to_codepoint("a"));
    }

    #[test]
    fn multi_codepoint_sequence_entry() {
        // `dalethatafpatah` is the canonical AGL multi-codepoint
        // example: a Hebrew DALET (U+05D3) plus the HATAF PATAH
        // combining vowel (U+05B2).
        let cp = name_to_codepoints("dalethatafpatah").expect("entry exists");
        match cp {
            Codepoints::Sequence(s) => {
                assert_eq!(s, ['\u{05D3}', '\u{05B2}']);
                assert_eq!(s.len(), 2);
            }
            Codepoints::Single(_) => panic!("expected a Sequence"),
        }
        // The single-codepoint shortcut returns None for sequence
        // entries.
        assert_eq!(name_to_codepoint("dalethatafpatah"), None);
        // Reverse lookup doesn't surface sequence entries either
        // (a single `char` argument can't disambiguate).
        assert_ne!(codepoint_to_name('\u{05D3}'), Some("dalethatafpatah"));
    }

    #[test]
    fn codepoints_first_and_len() {
        let single = Codepoints::Single('A');
        assert_eq!(single.first(), 'A');
        assert_eq!(single.len(), 1);
        assert!(!single.is_empty());

        let slice: &[char] = &['\u{05D3}', '\u{05B2}'];
        let seq = Codepoints::Sequence(slice);
        assert_eq!(seq.first(), '\u{05D3}');
        assert_eq!(seq.len(), 2);
        assert!(!seq.is_empty());
        assert_eq!(seq.as_slice(), slice);
    }

    #[test]
    fn codepoint_to_name_first_in_sort_order() {
        // U+05B8 (HEBREW POINT QAMATS) is shared by ~17 AGL aliases.
        // The codepoint→name table returns the alphabetically-first
        // one (in ASCII sort order). All such names must round-trip
        // through `name_to_codepoint`.
        let chosen = codepoint_to_name('\u{05B8}').expect("U+05B8 has AGL aliases");
        assert_eq!(name_to_codepoint(chosen), Some('\u{05B8}'));
        // Sanity: the chosen alias is among the ASCII-sorted names
        // mapping to U+05B8.
        let all_aliases: Vec<&str> = entries()
            .filter_map(|(n, c)| match c {
                Codepoints::Single('\u{05B8}') => Some(n),
                _ => None,
            })
            .collect();
        assert!(all_aliases.len() >= 2, "expected multiple aliases");
        assert_eq!(*all_aliases.first().unwrap(), chosen);
    }

    #[test]
    fn entries_yields_all_pairs() {
        let collected: Vec<(&'static str, Codepoints<'static>)> = entries().collect();
        assert_eq!(collected.len(), 4281);
        // First entry is `A;0041` per the file head.
        assert_eq!(collected[0].0, "A");
        assert_eq!(collected[0].1.first(), 'A');
        // Last entry is `zukatakana;30BA` per the file tail.
        assert_eq!(collected[4280].0, "zukatakana");
        assert_eq!(collected[4280].1.first(), '\u{30BA}');
    }

    #[test]
    fn reverse_lookup_for_unencoded_codepoint() {
        // U+FFFE is not encoded by Unicode and is not in AGL.
        assert_eq!(codepoint_to_name('\u{FFFE}'), None);
        // Astral planes are not covered by AGL 2.0 at all (the
        // shipped table is BMP-only).
        assert_eq!(codepoint_to_name('\u{1F600}'), None);
    }

    #[test]
    fn glyph_names_are_ascii() {
        // AGL format spec: "Glyph name—upper/lowercase letters and
        // digits." Every parsed name must be pure ASCII alphanumeric
        // (the AGL file itself is ASCII-only). This is a defence
        // against future source-file corruption.
        for (name, _) in entries() {
            assert!(
                name.bytes().all(|b| b.is_ascii_alphanumeric()),
                "non-alphanumeric glyph name: {name}"
            );
            assert!(!name.is_empty());
        }
    }
}
