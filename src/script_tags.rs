//! OpenType Layout **script tags** — the registered script-tag
//! vocabulary and its mapping from Unicode Script values.
//!
//! Source: the staged script-tag registry
//! (`docs/text/opentype/registries/script-tags.html`, OpenType 1.9.1).
//! Script tags "generally correspond to a Unicode script", but the
//! association is not one-to-one: tags predate ISO 15924 / the
//! Unicode Script property, and one Unicode script may have more than
//! one registered tag (e.g. `deva` and `dev2` for an older and a
//! newer shaping-engine implementation). Registered tags are four
//! lowercase ASCII letters by convention, space-padded where shorter
//! (the registry's `Yi` row spells out the byte sequence
//! `0x79 0x69 0x20 0x20`); `DFLT` is the special default tag.
//!
//! [`ot_script_tags`] maps a Unicode Script value (any UAX #44 alias
//! form, resolved through the vendored `PropertyValueAliases.txt`
//! table) to its registered candidate tags in preference order, for
//! use with the shaping pipeline's script selection:
//!
//! - the registry pairs a v.2 tag with a classic tag for ten scripts
//!   (`bng2`/`beng`, `dev2`/`deva`, `gjr2`/`gujr`, `gur2`/`guru`,
//!   `knd2`/`knda`, `mlm2`/`mlym`, `mym2`/`mymr`, `ory2`/`orya`,
//!   `tml2`/`taml`, `tel2`/`telu`); the newer tag is listed first
//!   (for Myanmar the registry itself recommends `mym2` over `mymr`)
//!   and a caller picks the first tag the font actually provides;
//! - Hiragana, Katakana, and the `Hrkt` (Katakana_Or_Hiragana) value
//!   all map to `kana` (the registry assigns `kana` to both kana
//!   scripts);
//! - Hangul lists `hang` first and the not-recommended `jamo` second;
//! - `lao `, `nko `, `vai `, and `yi  ` carry their space padding;
//! - the implicit Common / Inherited / Unknown values map to `DFLT`;
//! - every other script follows the registry's lowercased-ISO-code
//!   pattern, verified against the registered-tag table.

use crate::unicode_script::{is_implicit_script, vendored_script_aliases};

/// Every registered script tag: `(registry script name, tag)`, in the
/// registry's order. `kana` appears twice (Hiragana and Katakana).
pub const REGISTERED_SCRIPT_TAGS: &[(&str, [u8; 4])] = &[
    ("Adlam", *b"adlm"),
    ("Ahom", *b"ahom"),
    ("Anatolian Hieroglyphs", *b"hluw"),
    ("Arabic", *b"arab"),
    ("Armenian", *b"armn"),
    ("Avestan", *b"avst"),
    ("Balinese", *b"bali"),
    ("Bamum", *b"bamu"),
    ("Bassa Vah", *b"bass"),
    ("Batak", *b"batk"),
    ("Bangla", *b"beng"),
    ("Bangla v.2", *b"bng2"),
    ("Beria Erfe", *b"berf"),
    ("Bhaiksuki", *b"bhks"),
    ("Bopomofo", *b"bopo"),
    ("Brahmi", *b"brah"),
    ("Braille", *b"brai"),
    ("Buginese", *b"bugi"),
    ("Buhid", *b"buhd"),
    ("Byzantine Music", *b"byzm"),
    ("Canadian Syllabics", *b"cans"),
    ("Carian", *b"cari"),
    ("Caucasian Albanian", *b"aghb"),
    ("Chakma", *b"cakm"),
    ("Cham", *b"cham"),
    ("Cherokee", *b"cher"),
    ("Chorasmian", *b"chrs"),
    ("CJK Ideographic", *b"hani"),
    ("Coptic", *b"copt"),
    ("Cypriot Syllabary", *b"cprt"),
    ("Cypro-Minoan", *b"cpmn"),
    ("Cyrillic", *b"cyrl"),
    ("Default", *b"DFLT"),
    ("Deseret", *b"dsrt"),
    ("Devanagari", *b"deva"),
    ("Devanagari v.2", *b"dev2"),
    ("Dives Akuru", *b"diak"),
    ("Dogra", *b"dogr"),
    ("Duployan", *b"dupl"),
    ("Egyptian Hieroglyphs", *b"egyp"),
    ("Elbasan", *b"elba"),
    ("Elymaic", *b"elym"),
    ("Ethiopic", *b"ethi"),
    ("Garay", *b"gara"),
    ("Georgian", *b"geor"),
    ("Glagolitic", *b"glag"),
    ("Gothic", *b"goth"),
    ("Grantha", *b"gran"),
    ("Greek", *b"grek"),
    ("Gujarati", *b"gujr"),
    ("Gujarati v.2", *b"gjr2"),
    ("Gunjala Gondi", *b"gong"),
    ("Gurmukhi", *b"guru"),
    ("Gurmukhi v.2", *b"gur2"),
    ("Gurung Khema", *b"gukh"),
    ("Hangul", *b"hang"),
    ("Hangul Jamo", *b"jamo"),
    ("Hanifi Rohingya", *b"rohg"),
    ("Hanunoo", *b"hano"),
    ("Hatran", *b"hatr"),
    ("Hebrew", *b"hebr"),
    ("Hiragana", *b"kana"),
    ("Imperial Aramaic", *b"armi"),
    ("Inscriptional Pahlavi", *b"phli"),
    ("Inscriptional Parthian", *b"prti"),
    ("Javanese", *b"java"),
    ("Kaithi", *b"kthi"),
    ("Kannada", *b"knda"),
    ("Kannada v.2", *b"knd2"),
    ("Katakana", *b"kana"),
    ("Kawi", *b"kawi"),
    ("Kayah Li", *b"kali"),
    ("Kharosthi", *b"khar"),
    ("Khitan Small Script", *b"kits"),
    ("Khmer", *b"khmr"),
    ("Khojki", *b"khoj"),
    ("Khudawadi", *b"sind"),
    ("Kirat Rai", *b"krai"),
    ("Lao", *b"lao "),
    ("Latin", *b"latn"),
    ("Lepcha", *b"lepc"),
    ("Limbu", *b"limb"),
    ("Linear A", *b"lina"),
    ("Linear B", *b"linb"),
    ("Lisu (Fraser)", *b"lisu"),
    ("Lycian", *b"lyci"),
    ("Lydian", *b"lydi"),
    ("Mahajani", *b"mahj"),
    ("Makasar", *b"maka"),
    ("Malayalam", *b"mlym"),
    ("Malayalam v.2", *b"mlm2"),
    ("Mandaic, Mandaean", *b"mand"),
    ("Manichaean", *b"mani"),
    ("Marchen", *b"marc"),
    ("Masaram Gondi", *b"gonm"),
    ("Mathematical text layout", *b"math"),
    ("Medefaidrin (Oberi Okaime, Oberi Ɔkaimɛ)", *b"medf"),
    ("Meitei Mayek (Meithei, Meetei)", *b"mtei"),
    ("Mende Kikakui", *b"mend"),
    ("Meroitic Cursive", *b"merc"),
    ("Meroitic Hieroglyphs", *b"mero"),
    ("Miao", *b"plrd"),
    ("Modi", *b"modi"),
    ("Mongolian", *b"mong"),
    ("Mro", *b"mroo"),
    ("Multani", *b"mult"),
    ("Musical Symbols", *b"musc"),
    ("Myanmar", *b"mymr"),
    ("Myanmar v.2", *b"mym2"),
    ("Nabataean", *b"nbat"),
    ("Nag Mundari", *b"nagm"),
    ("Nandinagari", *b"nand"),
    ("Newa", *b"newa"),
    ("New Tai Lue", *b"talu"),
    ("N'Ko", *b"nko "),
    ("Nüshu", *b"nshu"),
    ("Nyiakeng Puachue Hmong", *b"hmnp"),
    ("Odia", *b"orya"),
    ("Odia v.2", *b"ory2"),
    ("Ogham", *b"ogam"),
    ("Ol Chiki", *b"olck"),
    ("Ol Onal", *b"onao"),
    ("Old Italic", *b"ital"),
    ("Old Hungarian", *b"hung"),
    ("Old North Arabian", *b"narb"),
    ("Old Permic", *b"perm"),
    ("Old Persian Cuneiform", *b"xpeo"),
    ("Old Sogdian", *b"sogo"),
    ("Old South Arabian", *b"sarb"),
    ("Old Turkic, Orkhon Runic", *b"orkh"),
    ("Old Uyghur", *b"ougr"),
    ("Osage", *b"osge"),
    ("Osmanya", *b"osma"),
    ("Pahawh Hmong", *b"hmng"),
    ("Palmyrene", *b"palm"),
    ("Pau Cin Hau", *b"pauc"),
    ("Phags-pa", *b"phag"),
    ("Phoenician", *b"phnx"),
    ("Psalter Pahlavi", *b"phlp"),
    ("Rejang", *b"rjng"),
    ("Runic", *b"runr"),
    ("Samaritan", *b"samr"),
    ("Saurashtra", *b"saur"),
    ("Sharada", *b"shrd"),
    ("Shavian", *b"shaw"),
    ("Siddham", *b"sidd"),
    ("Sidetic", *b"sidt"),
    ("Sign Writing", *b"sgnw"),
    ("Sinhala", *b"sinh"),
    ("Sogdian", *b"sogd"),
    ("Sora Sompeng", *b"sora"),
    ("Soyombo", *b"soyo"),
    ("Sumero-Akkadian Cuneiform", *b"xsux"),
    ("Sundanese", *b"sund"),
    ("Sunuwar", *b"sunu"),
    ("Syloti Nagri", *b"sylo"),
    ("Syriac", *b"syrc"),
    ("Tagalog", *b"tglg"),
    ("Tagbanwa", *b"tagb"),
    ("Tai Le", *b"tale"),
    ("Tai Tham (Lanna)", *b"lana"),
    ("Tai Viet", *b"tavt"),
    ("Tai Yo", *b"tayo"),
    ("Takri", *b"takr"),
    ("Tamil", *b"taml"),
    ("Tamil v.2", *b"tml2"),
    ("Tangsa", *b"tnsa"),
    ("Tangut", *b"tang"),
    ("Telugu", *b"telu"),
    ("Telugu v.2", *b"tel2"),
    ("Thaana", *b"thaa"),
    ("Thai", *b"thai"),
    ("Tibetan", *b"tibt"),
    ("Tifinagh", *b"tfng"),
    ("Tirhuta", *b"tirh"),
    ("Todhri", *b"todr"),
    ("Tolong Siki", *b"tols"),
    ("Toto", *b"toto"),
    ("Tulu-Tigalari", *b"tutg"),
    ("Ugaritic Cuneiform", *b"ugar"),
    ("Vai", *b"vai "),
    ("Vithkuqi", *b"vith"),
    ("Wancho", *b"wcho"),
    ("Warang Citi", *b"wara"),
    ("Yezidi", *b"yezi"),
    ("Yi", *b"yi  "),
    ("Zanabazar Square (Zanabazarin Dörböljin Useg, Xewtee Dörböljin Bicig, Horizontal Square Script)", *b"zanb"),
];

/// The special default script tag.
pub const DFLT: [u8; 4] = *b"DFLT";

/// Whether `tag` is a registered script tag (including `DFLT`).
pub fn is_registered_script_tag(tag: [u8; 4]) -> bool {
    REGISTERED_SCRIPT_TAGS.iter().any(|(_, t)| *t == tag)
}

/// The registry's script name(s) for a tag, in registry order (two
/// names for `kana`).
pub fn script_tag_names(tag: [u8; 4]) -> Vec<&'static str> {
    REGISTERED_SCRIPT_TAGS
        .iter()
        .filter(|(_, t)| *t == tag)
        .map(|(n, _)| *n)
        .collect()
}

/// Candidate v.2 / classic tag pairs, newest first, keyed by the ISO
/// 15924 short code of the script.
const V2_PAIRS: &[(&str, [[u8; 4]; 2])] = &[
    ("Beng", [*b"bng2", *b"beng"]),
    ("Deva", [*b"dev2", *b"deva"]),
    ("Gujr", [*b"gjr2", *b"gujr"]),
    ("Guru", [*b"gur2", *b"guru"]),
    ("Knda", [*b"knd2", *b"knda"]),
    ("Mlym", [*b"mlm2", *b"mlym"]),
    ("Mymr", [*b"mym2", *b"mymr"]),
    ("Orya", [*b"ory2", *b"orya"]),
    ("Taml", [*b"tml2", *b"taml"]),
    ("Telu", [*b"tel2", *b"telu"]),
];

/// Registered OpenType script tags for a Unicode Script value, in
/// preference order (see the module docs for the rules). The name is
/// resolved through the vendored alias table, so any UAX #44 alias
/// form works ("Latin", "latn", or the registry's "Odia" via Oriya's
/// code `Orya`). Empty when the script has no registered tag -- the
/// caller falls back to [`DFLT`].
pub fn ot_script_tags(script: &str) -> Vec<[u8; 4]> {
    // Common / Inherited / Unknown text carries no script identity of
    // its own: the default tag.
    if is_implicit_script(script) {
        return vec![DFLT];
    }
    // Resolve any alias form to the ISO short code; an unlisted name
    // is used as-is (it may already be a short code from a newer UCD).
    let aliases = vendored_script_aliases();
    let short = aliases.short_name(script).unwrap_or(script);

    for (code, pair) in V2_PAIRS {
        if short.eq_ignore_ascii_case(code) {
            return pair.to_vec();
        }
    }
    // Both kana scripts (and the mixed Hrkt value) share 'kana'.
    if ["Hira", "Kana", "Hrkt"]
        .iter()
        .any(|c| short.eq_ignore_ascii_case(c))
    {
        return vec![*b"kana"];
    }
    // Hangul: 'hang', with the not-recommended 'jamo' as a fallback
    // candidate.
    if short.eq_ignore_ascii_case("Hang") {
        return vec![*b"hang", *b"jamo"];
    }
    // Scripts whose registered tag is shorter than the ISO code and
    // space-padded ('lao ', 'nko ', 'vai ', 'yi  ').
    const PADDED: &[(&str, [u8; 4])] = &[
        ("Laoo", *b"lao "),
        ("Nkoo", *b"nko "),
        ("Vaii", *b"vai "),
        ("Yiii", *b"yi  "),
    ];
    for (code, tag) in PADDED {
        if short.eq_ignore_ascii_case(code) {
            return vec![*tag];
        }
    }
    // Registry pattern: the lowercased ISO code, space-padded to four
    // bytes; only returned when actually registered.
    let lower = short.to_ascii_lowercase();
    let b = lower.as_bytes();
    if b.len() < 2 || b.len() > 4 || !b.iter().all(|c| c.is_ascii_lowercase()) {
        return Vec::new();
    }
    let mut tag = *b"    ";
    tag[..b.len()].copy_from_slice(b);
    if is_registered_script_tag(tag) {
        vec![tag]
    } else {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::unicode_script::vendored_script_aliases;

    #[test]
    fn registry_table_shape() {
        // 187 rows transcribed from the registry; the only tag
        // registered for two scripts is 'kana'.
        assert_eq!(REGISTERED_SCRIPT_TAGS.len(), 187);
        assert!(is_registered_script_tag(DFLT));
        assert!(is_registered_script_tag(*b"latn"));
        assert!(!is_registered_script_tag(*b"zzzz"));
        assert_eq!(script_tag_names(*b"kana"), vec!["Hiragana", "Katakana"]);
        assert_eq!(script_tag_names(*b"zanb").len(), 1);
        // Space-padded tags carry real 0x20 bytes (the Yi remark's
        // explicit byte sequence).
        for tag in [*b"lao ", *b"nko ", *b"vai ", *b"yi  "] {
            assert!(is_registered_script_tag(tag));
        }
        // Tags are four ASCII bytes, lowercase letters, digits, or
        // space padding (DFLT excepted).
        for (name, tag) in REGISTERED_SCRIPT_TAGS {
            assert!(!name.is_empty());
            if tag == &DFLT {
                continue;
            }
            assert!(
                tag.iter()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b' '),
                "{name}"
            );
        }
    }

    #[test]
    fn unicode_script_mapping_spot_checks() {
        assert_eq!(ot_script_tags("Latin"), vec![*b"latn"]);
        assert_eq!(ot_script_tags("latn"), vec![*b"latn"]);
        assert_eq!(ot_script_tags("Greek"), vec![*b"grek"]);
        // Han maps to the registry's "CJK Ideographic" tag.
        assert_eq!(ot_script_tags("Han"), vec![*b"hani"]);
        // Both kana scripts and Hrkt share 'kana'.
        assert_eq!(ot_script_tags("Hiragana"), vec![*b"kana"]);
        assert_eq!(ot_script_tags("Katakana"), vec![*b"kana"]);
        assert_eq!(ot_script_tags("Katakana_Or_Hiragana"), vec![*b"kana"]);
        // Space-padded registry tags.
        assert_eq!(ot_script_tags("Lao"), vec![*b"lao "]);
        assert_eq!(ot_script_tags("Nko"), vec![*b"nko "]);
        assert_eq!(ot_script_tags("Vai"), vec![*b"vai "]);
        assert_eq!(ot_script_tags("Yi"), vec![*b"yi  "]);
        // v.2 pairs, newest first; "Odia" is the registry's name for
        // Oriya.
        assert_eq!(ot_script_tags("Devanagari"), vec![*b"dev2", *b"deva"]);
        assert_eq!(ot_script_tags("Oriya"), vec![*b"ory2", *b"orya"]);
        assert_eq!(ot_script_tags("Myanmar"), vec![*b"mym2", *b"mymr"]);
        assert_eq!(ot_script_tags("Bengali"), vec![*b"bng2", *b"beng"]);
        // Hangul: hang preferred, jamo deprecated fallback.
        assert_eq!(ot_script_tags("Hangul"), vec![*b"hang", *b"jamo"]);
        // Implicit values -> DFLT.
        assert_eq!(ot_script_tags("Common"), vec![DFLT]);
        assert_eq!(ot_script_tags("Inherited"), vec![DFLT]);
        assert_eq!(ot_script_tags("Unknown"), vec![DFLT]);
        // Registry-name aliases resolve through the UCD alias table
        // only; a made-up name yields no tags.
        assert!(ot_script_tags("NoSuchScript").is_empty());
    }

    #[test]
    fn every_unicode_script_value_maps_to_registered_tags() {
        // Every `sc` row of the vendored PropertyValueAliases.txt
        // resolves to at least one candidate tag, and every candidate
        // is registered.
        for (short, long) in vendored_script_aliases().iter() {
            let tags = ot_script_tags(long);
            assert!(!tags.is_empty(), "{short} ({long}) has no tags");
            for t in &tags {
                assert!(is_registered_script_tag(*t), "{short} -> {t:?}");
            }
            // Short-code and long-form lookups agree.
            assert_eq!(tags, ot_script_tags(short), "{short}");
        }
        // The V2 pair table's members are themselves registered.
        for (code, pair) in V2_PAIRS {
            assert!(vendored_script_aliases().long_name(code).is_some());
            for t in pair {
                assert!(is_registered_script_tag(*t));
            }
        }
    }
}
