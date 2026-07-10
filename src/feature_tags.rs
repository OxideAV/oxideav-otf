//! OpenType Layout **feature-tag registry** — the controlled
//! vocabulary of registered `GSUB` / `GPOS` feature tags.
//!
//! Source: `docs/text/opentype/registries/feature-tags.md` (OpenType
//! Layout Tag Registry, *Registered features*, OpenType 1.9.1;
//! © Microsoft Corporation, OpenType specification, CC-BY-4.0). The
//! registry holds 126 entries; two of them are ranges — `cv01`–`cv99`
//! (Character Variants) and `ss01`–`ss20` (Stylistic Sets) — that
//! expand to 99 + 20 individual tags, for **243 registered feature
//! tags** in total. Registered tags are four lowercase ASCII letters
//! or digits; the all-uppercase tag space is reserved for private
//! vendor features.
//!
//! A shaping engine uses this registry to map the features a font's
//! `FeatureList` references to their typographic function — e.g. to
//! decide which features are on by default (`ccmp`, `liga`, `kern`,
//! …), which are user preferences (`smcp`, `onum`, `ss07`), and which
//! must never be disabled (`rlig`, `rclt`, `rvrn`).
//!
//! [`feature_tag`] answers a lookup for any of the 243 tags —
//! range tags resolve to [`FeatureTag::CharacterVariant`] /
//! [`FeatureTag::StylisticSet`] carrying their 1-based index, fixed
//! tags to the registry record with friendly name + one-line function
//! summary. [`registered_feature_tags`] iterates all 243 tags in
//! registry order.

/// One fixed registry entry: tag, friendly name, and a one-line
/// summary of the feature's function.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FeatureTagRecord {
    /// The four-byte feature tag.
    pub tag: [u8; 4],
    /// The registry's friendly name (e.g. "Standard Ligatures").
    pub friendly_name: &'static str,
    /// One-line function summary from the registry.
    pub function: &'static str,
}

/// A resolved registered feature tag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeatureTag {
    /// One of the 124 individually registered tags.
    Registered(&'static FeatureTagRecord),
    /// `cv01`–`cv99` — Character Variant *n* (1-based).
    CharacterVariant(u8),
    /// `ss01`–`ss20` — Stylistic Set *n* (1-based).
    StylisticSet(u8),
}

impl FeatureTag {
    /// The friendly name: the record's name for fixed tags,
    /// `"Character Variant"` / `"Stylistic Set"` for range tags (the
    /// index is carried in the variant).
    pub fn friendly_name(&self) -> &'static str {
        match self {
            FeatureTag::Registered(r) => r.friendly_name,
            FeatureTag::CharacterVariant(_) => "Character Variant",
            FeatureTag::StylisticSet(_) => "Stylistic Set",
        }
    }

    /// The one-line function summary.
    pub fn function(&self) -> &'static str {
        match self {
            FeatureTag::Registered(r) => r.function,
            FeatureTag::CharacterVariant(_) => {
                "Provides per-character control over glyph variants for individual characters."
            }
            FeatureTag::StylisticSet(_) => {
                "Selects typographic alternatives for a coordinated set of glyphs."
            }
        }
    }
}

/// Number of individually registered feature tags (243: 124 fixed +
/// 99 character variants + 20 stylistic sets).
pub const REGISTERED_FEATURE_TAG_COUNT: usize = 243;

/// The 124 fixed registry entries, in registry (alphabetical) order.
/// The `cv01`–`cv99` and `ss01`–`ss20` ranges are resolved
/// programmatically by [`feature_tag`].
pub static FEATURE_TAG_REGISTRY: [FeatureTagRecord; 124] = [
    r(b"aalt", "Access All Alternates", "Makes all alternate forms of a selected character accessible for the user to choose from."),
    r(b"abvf", "Above-base Forms", "Substitutes the above-base form of a vowel (e.g., split vowels in Khmer/Brahmi-derived scripts)."),
    r(b"abvm", "Above-base Mark Positioning", "Positions mark glyphs above base glyphs."),
    r(b"abvs", "Above-base Substitutions", "Substitutes a ligature for a base glyph and an above-base mark."),
    r(b"afrc", "Alternative Fractions", "Replaces slash-separated figures with an alternative (stacked/\"nut\") fraction form."),
    r(b"akhn", "Akhand", "Preferentially substitutes a ligature for a character sequence regardless of surrounding context (unbreakable conjuncts)."),
    r(b"apkn", "Kerning for Alternate Proportional Widths", "Applies kerning to glyphs made proportional-width by the `palt` feature."),
    r(b"blwf", "Below-base Forms", "Substitutes the below-base form of a consonant in conjuncts."),
    r(b"blwm", "Below-base Mark Positioning", "Positions mark glyphs below base glyphs."),
    r(b"blws", "Below-base Substitutions", "Produces ligatures comprising a base glyph and below-base forms."),
    r(b"calt", "Contextual Alternates", "Replaces default glyphs with alternates giving better joining/spacing in specified contexts."),
    r(b"case", "Case-sensitive Forms", "Shifts punctuation up and changes oldstyle to lining figures to fit all-cap/lining text."),
    r(b"ccmp", "Glyph Composition / Decomposition", "Composes or decomposes glyphs for better glyph processing (applied first)."),
    r(b"cfar", "Conjunct Form After Ro", "Substitutes alternate below-/post-base forms in Khmer after a conjoined Ro."),
    r(b"chws", "Contextual Half-width Spacing", "Contextually re-spaces full-width glyphs onto half-em widths for CJK layout."),
    r(b"cjct", "Conjunct Forms", "Produces conjunct forms of consonants in Indic scripts."),
    r(b"clig", "Contextual Ligatures", "Replaces a glyph sequence with a ligature in a specified context."),
    r(b"cpct", "Centered CJK Punctuation", "Centers specific CJK punctuation marks."),
    r(b"cpsp", "Capital Spacing", "Globally adjusts inter-glyph spacing for all-capital text."),
    r(b"cswh", "Contextual Swash", "Replaces default glyphs with swash forms in a specified context."),
    r(b"curs", "Cursive Positioning", "Positions adjacent glyphs for cursive connections via entry/exit points."),
    r(b"c2pc", "Petite Capitals From Capitals", "Turns capital characters into petite capitals."),
    r(b"c2sc", "Small Capitals From Capitals", "Turns capital characters into small capitals."),
    r(b"dist", "Distances", "Provides required control of distance between glyphs (not user-overridable kerning)."),
    r(b"dlig", "Discretionary Ligatures", "Replaces a glyph sequence with a ligature used for special effect at the user's preference."),
    r(b"dnom", "Denominators", "Replaces figures following a slash with denominator figures."),
    r(b"dtls", "Dotless Forms", "Provides dotless forms of Math Alphanumeric characters for placing accents over them."),
    r(b"expt", "Expert Forms", "Replaces standard Japanese forms with expert forms preferred by typographers."),
    r(b"falt", "Final Glyph on Line Alternates", "Replaces line-final glyphs with alternate forms to aid justification."),
    r(b"fin2", "Terminal Forms #2", "Replaces the Syriac Alaph with its final form when preceded by a non-joining, non-Dalath/Rish base."),
    r(b"fin3", "Terminal Forms #3", "Replaces the Syriac Alaph with its final form when preceded by Dalath, Rish, or dotless Dalath-Rish."),
    r(b"fina", "Terminal Forms", "Replaces glyphs with final forms in a final joining context."),
    r(b"flac", "Flattened Accent Forms", "Provides flattened accent forms for use over high-rise bases in math formulas."),
    r(b"frac", "Fractions", "Replaces slash-separated figures with common (diagonal) fractions."),
    r(b"fwid", "Full Widths", "Replaces glyphs with full (em) width forms."),
    r(b"half", "Half Forms", "Produces half forms of consonants in Indic scripts."),
    r(b"haln", "Halant Forms", "Produces halant forms (consonant with overt halant) in Indic scripts."),
    r(b"halt", "Alternate Half Widths", "Re-spaces full-width glyphs onto half-em widths (metrics only, non-contextual)."),
    r(b"hist", "Historical Forms", "Replaces default single-character forms with historical (archaic) alternates."),
    r(b"hkna", "Horizontal Kana Alternates", "Replaces standard kana with forms designed for horizontal writing."),
    r(b"hlig", "Historical Ligatures", "Replaces default forms with historical ligature alternates."),
    r(b"hngl", "Hangul (deprecated)", "Replaces hanja (Chinese-style) Korean characters with corresponding hangul characters."),
    r(b"hojo", "Hojo Kanji Forms (JIS X 0212-1990)", "Accesses JIS X 0212-1990 (\"Hojo\") kanji glyphs."),
    r(b"hwid", "Half Widths", "Replaces glyphs with half-em (en) width forms."),
    r(b"init", "Initial Forms", "Replaces glyphs with initial forms in an initial joining context."),
    r(b"isol", "Isolated Forms", "Replaces glyphs with isolated (non-joining) forms."),
    r(b"ital", "Italics", "Replaces Roman glyphs with corresponding Italic glyphs."),
    r(b"jalt", "Justification Alternates", "Replaces glyphs with alternate forms to improve justification."),
    r(b"jp78", "JIS78 Forms", "Replaces default (JIS90) glyphs with JIS C 6226-1978 forms."),
    r(b"jp83", "JIS83 Forms", "Replaces default (JIS90) glyphs with JIS X 0208-1983 forms."),
    r(b"jp90", "JIS90 Forms", "Replaces JIS78/JIS83 glyphs with JIS X 0208-1990 forms."),
    r(b"jp04", "JIS2004 Forms", "Accesses JIS X 0213:2004 prototypical glyphs (a subset of `nlck`)."),
    r(b"kern", "Kerning", "Adjusts spacing between specific glyph pairs in horizontal layout."),
    r(b"lfbd", "Left Bounds", "Aligns glyphs by apparent left extents at the left ends of horizontal lines."),
    r(b"liga", "Standard Ligatures", "Replaces a glyph sequence with a preferred ligature used under normal conditions."),
    r(b"ljmo", "Leading Jamo Forms", "Substitutes the form for a Hangul leading consonant jamo in a cluster."),
    r(b"lnum", "Lining Figures", "Changes non-lining figures to lining figures."),
    r(b"locl", "Localized Forms", "Substitutes localized variant forms of glyphs based on the language of the text."),
    r(b"ltra", "Left-to-right Alternates", "Applies glyph variants (other than mirrored forms) appropriate for left-to-right text."),
    r(b"ltrm", "Left-to-right Mirrored Forms", "Applies mirrored forms appropriate for left-to-right text."),
    r(b"mark", "Mark Positioning", "Positions mark glyphs with respect to base glyphs."),
    r(b"med2", "Medial Forms #2", "Replaces the Syriac Alaph with a medial form when the preceding base can be joined to."),
    r(b"medi", "Medial Forms", "Replaces glyphs with medial forms in a medial (dual-joining) context."),
    r(b"mgrk", "Mathematical Greek", "Replaces standard Greek glyphs with forms used in mathematical notation."),
    r(b"mkmk", "Mark to Mark Positioning", "Positions marks with respect to other marks."),
    r(b"mset", "Mark Positioning via Substitution", "Positions Arabic combining marks via glyph substitution (legacy; not for new fonts)."),
    r(b"nalt", "Alternate Annotation Forms", "Replaces default glyphs with notational forms (circled, boxed, parenthesized, etc.)."),
    r(b"nlck", "NLC Kanji Forms", "Accesses the NLC (2000) kanji glyph shapes."),
    r(b"nukt", "Nukta Forms", "Produces nukta forms in Indic scripts."),
    r(b"numr", "Numerators", "Replaces figures preceding a slash with numerator figures and the slash with a fraction slash."),
    r(b"onum", "Oldstyle Figures", "Changes figures from the default/lining style to oldstyle form."),
    r(b"opbd", "Optical Bounds (deprecated)", "Aligns glyphs by their apparent optical extents (visual justification)."),
    r(b"ordn", "Ordinals", "Replaces alphabetic glyphs with ordinal forms for use after figures."),
    r(b"ornm", "Ornaments", "Provides access to ornament glyphs (fleurons, dingbats, border elements)."),
    r(b"palt", "Proportional Alternate Widths", "Re-spaces full-em glyphs onto proportional widths (metrics only, not substitution)."),
    r(b"pcap", "Petite Capitals", "Turns lowercase characters into petite capitals."),
    r(b"pkna", "Proportional Kana", "Replaces uniform-width kana with proportional kana glyphs."),
    r(b"pnum", "Proportional Figures", "Replaces tabular figures with proportional-width figures."),
    r(b"pref", "Pre-base Forms", "Substitutes the pre-base form of a consonant."),
    r(b"pres", "Pre-base Substitutions", "Produces pre-base forms of conjuncts (and pre-base vowel-sign variants) in Indic scripts."),
    r(b"pstf", "Post-base Forms", "Substitutes the post-base form of a consonant."),
    r(b"psts", "Post-base Substitutions", "Substitutes a base + post-base sequence with its ligature form."),
    r(b"pwid", "Proportional Widths", "Replaces uniform-width glyphs with proportionally spaced glyphs."),
    r(b"qwid", "Quarter Widths", "Replaces glyphs with quarter-em width forms."),
    r(b"rand", "Randomize", "Uses multiple alternate forms to emulate the irregularity of handwritten text."),
    r(b"rclt", "Required Contextual Alternates", "Contextual alternates required for correct layout (cannot be turned off)."),
    r(b"rkrf", "Rakar Forms", "Produces conjoined rakar (Ra) forms in Devanagari and Gujarati."),
    r(b"rlig", "Required Ligatures", "Ligatures required for correct display of a script (e.g., Arabic lam-alef)."),
    r(b"rphf", "Reph Form", "Substitutes the reph form for a consonant + halant sequence."),
    r(b"rtbd", "Right Bounds", "Aligns glyphs by apparent right extents at the right ends of horizontal lines."),
    r(b"rtla", "Right-to-left Alternates", "Applies glyph variants (other than mirrored forms) appropriate for right-to-left text."),
    r(b"rtlm", "Right-to-left Mirrored Forms", "Applies mirrored forms for right-to-left text beyond character-level mirroring."),
    r(b"ruby", "Ruby Notation Forms", "Substitutes smaller kana glyphs designed for ruby annotation."),
    r(b"rvrn", "Required Variation Alternates", "Selects alternate glyphs for particular variation instances in variable fonts."),
    r(b"salt", "Stylistic Alternates", "Replaces default forms with purely esthetic stylistic alternates."),
    r(b"sinf", "Scientific Inferiors", "Replaces figures (and some letters) with inferior forms for scientific/mathematical notation."),
    r(b"size", "Optical size (superseded by STAT)", "Stores design size and recommended size-range information for optical sizing."),
    r(b"smcp", "Small Capitals", "Turns lowercase characters into small capitals."),
    r(b"smpl", "Simplified Forms", "Replaces traditional Chinese/Japanese forms with simplified forms."),
    r(b"ssty", "Math Script-style Alternates", "Provides glyph variants suited for math subscripts and superscripts."),
    r(b"stch", "Stretching Glyph Decomposition", "Decomposes an enclosing glyph into parts that stretch to fit the enclosed text."),
    r(b"subs", "Subscript", "Presents subscript forms."),
    r(b"sups", "Superscript", "Replaces figures/letters with superior (superscript) forms."),
    r(b"swsh", "Swash", "Replaces default glyphs with swash glyphs."),
    r(b"titl", "Titling", "Replaces default glyphs with titling forms designed for viewing at large sizes."),
    r(b"tjmo", "Trailing Jamo Forms", "Substitutes the form for a Hangul trailing consonant jamo in a cluster."),
    r(b"tnam", "Traditional Name Forms", "Replaces simplified kanji with traditional forms proper for use in personal names."),
    r(b"tnum", "Tabular Figures", "Replaces proportional figures with uniform (tabular) width figures."),
    r(b"trad", "Traditional Forms", "Replaces simplified Chinese/Japanese forms with traditional forms."),
    r(b"twid", "Third Widths", "Replaces glyphs with one-third-em width forms."),
    r(b"unic", "Unicase", "Maps upper- and lowercase letters to a single mixed (unicase) alphabet."),
    r(b"valt", "Alternate Vertical Metrics", "Repositions glyphs to visually center them within full-height metrics for vertical setting."),
    r(b"vapk", "Kerning for Alternate Proportional Vertical Metrics", "Applies vertical kerning to glyphs made proportional-height by `vpal`."),
    r(b"vatu", "Vattu Variants", "Substitutes a ligature for a base (or half) consonant and a following vattu form."),
    r(b"vchw", "Vertical Contextual Half-width Spacing", "Contextually re-spaces full-height glyphs onto half-height for vertical CJK layout."),
    r(b"vert", "Vertical Alternates", "Transforms default glyphs into forms appropriate for upright presentation in vertical writing."),
    r(b"vhal", "Alternate Vertical Half Metrics", "Re-spaces full-em-height glyphs onto half-em heights."),
    r(b"vjmo", "Vowel Jamo Forms", "Substitutes the form for a Hangul vowel jamo in a cluster."),
    r(b"vkna", "Vertical Kana Alternates", "Replaces standard kana with forms designed for vertical writing."),
    r(b"vkrn", "Vertical Kerning", "Adjusts spacing between specific glyph pairs in vertical layout."),
    r(b"vpal", "Proportional Alternate Vertical Metrics", "Re-spaces full-em-height glyphs onto proportional vertical heights."),
    r(b"vrt2", "Vertical Alternates and Rotation", "Replaces glyphs with 90\u{b0}-rotated forms suitable for vertical writing (a superset of `vert`)."),
    r(b"vrtr", "Vertical Alternates for Rotation", "Transforms glyphs into forms appropriate for sideways presentation in vertical writing."),
    r(b"zero", "Slashed Zero", "Replaces the default lining zero with a slashed form."),
];

/// Terse record constructor (keeps the table readable).
const fn r(tag: &[u8; 4], friendly_name: &'static str, function: &'static str) -> FeatureTagRecord {
    FeatureTagRecord {
        tag: *tag,
        friendly_name,
        function,
    }
}

/// Decode a `cvXX` / `ssXX` numeric suffix; `Some(n)` when both bytes
/// are ASCII digits and `lo <= n <= hi`.
fn range_index(tag: [u8; 4], lo: u8, hi: u8) -> Option<u8> {
    if !tag[2].is_ascii_digit() || !tag[3].is_ascii_digit() {
        return None;
    }
    let n = (tag[2] - b'0') * 10 + (tag[3] - b'0');
    (lo..=hi).contains(&n).then_some(n)
}

/// Look up a feature tag in the registry. Resolves all 243 registered
/// tags: the 124 fixed entries, `cv01`–`cv99`, and `ss01`–`ss20`.
/// Returns `None` for unregistered (including private all-uppercase)
/// tags.
pub fn feature_tag(tag: [u8; 4]) -> Option<FeatureTag> {
    if tag[0] == b'c' && tag[1] == b'v' {
        if let Some(n) = range_index(tag, 1, 99) {
            return Some(FeatureTag::CharacterVariant(n));
        }
    }
    if tag[0] == b's' && tag[1] == b's' {
        if let Some(n) = range_index(tag, 1, 20) {
            return Some(FeatureTag::StylisticSet(n));
        }
    }
    FEATURE_TAG_REGISTRY
        .iter()
        .find(|rec| rec.tag == tag)
        .map(FeatureTag::Registered)
}

/// Whether `tag` is one of the 243 registered feature tags.
pub fn is_registered_feature_tag(tag: [u8; 4]) -> bool {
    feature_tag(tag).is_some()
}

/// Iterate all 243 registered feature tags in registry order (fixed
/// entries alphabetically with the `cv01`–`cv99` range in its
/// alphabetical slot after `curs`, and `ss01`–`ss20` after `smpl`).
pub fn registered_feature_tags() -> impl Iterator<Item = [u8; 4]> {
    // Positions of the range blocks within the fixed table: `cv01…`
    // sorts between `curs` and `c2pc` in the registry's order, and
    // `ss01…` between `smpl` and `ssty`.
    let cv_after = FEATURE_TAG_REGISTRY
        .iter()
        .position(|rec| rec.tag == *b"curs")
        .unwrap_or(0);
    let ss_after = FEATURE_TAG_REGISTRY
        .iter()
        .position(|rec| rec.tag == *b"smpl")
        .unwrap_or(0);
    FEATURE_TAG_REGISTRY
        .iter()
        .enumerate()
        .flat_map(move |(i, rec)| {
            let mut out = vec![rec.tag];
            if i == cv_after {
                out.extend((1..=99u8).map(|n| [b'c', b'v', b'0' + n / 10, b'0' + n % 10]));
            }
            if i == ss_after {
                out.extend((1..=20u8).map(|n| [b's', b's', b'0' + n / 10, b'0' + n % 10]));
            }
            out
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_tag_lookup() {
        let liga = feature_tag(*b"liga").unwrap();
        assert_eq!(liga.friendly_name(), "Standard Ligatures");
        let FeatureTag::Registered(rec) = liga else {
            panic!("liga must be a fixed entry");
        };
        assert_eq!(rec.tag, *b"liga");
        assert!(rec.function.contains("ligature"));

        // First and last entries of the table.
        assert_eq!(
            feature_tag(*b"aalt").unwrap().friendly_name(),
            "Access All Alternates"
        );
        assert_eq!(
            feature_tag(*b"zero").unwrap().friendly_name(),
            "Slashed Zero"
        );
    }

    #[test]
    fn range_tag_lookup() {
        assert_eq!(feature_tag(*b"cv01"), Some(FeatureTag::CharacterVariant(1)));
        assert_eq!(
            feature_tag(*b"cv99"),
            Some(FeatureTag::CharacterVariant(99))
        );
        assert_eq!(feature_tag(*b"ss01"), Some(FeatureTag::StylisticSet(1)));
        assert_eq!(feature_tag(*b"ss20"), Some(FeatureTag::StylisticSet(20)));
        // Out-of-range numbers are not registered.
        assert_eq!(feature_tag(*b"cv00"), None);
        assert_eq!(feature_tag(*b"ss00"), None);
        assert_eq!(feature_tag(*b"ss21"), None);
        // Non-digit suffixes are not range tags.
        assert_eq!(feature_tag(*b"cvxx"), None);
        assert_eq!(
            feature_tag(*b"ss1a"),
            None,
            "both suffix bytes must be digits"
        );
        assert_eq!(
            feature_tag(*b"cv07").unwrap().friendly_name(),
            "Character Variant"
        );
    }

    #[test]
    fn unregistered_tags() {
        assert!(!is_registered_feature_tag(*b"zzzz"));
        // Private (all-uppercase) tag space is never registered.
        assert!(!is_registered_feature_tag(*b"TEST"));
        assert!(is_registered_feature_tag(*b"kern"));
        assert!(is_registered_feature_tag(*b"vkrn"));
    }

    #[test]
    fn registry_is_complete_and_unique() {
        let all: Vec<[u8; 4]> = registered_feature_tags().collect();
        assert_eq!(all.len(), REGISTERED_FEATURE_TAG_COUNT);
        // Every enumerated tag resolves, and no tag repeats.
        let mut seen = std::collections::HashSet::new();
        for tag in &all {
            assert!(is_registered_feature_tag(*tag), "{tag:?}");
            assert!(seen.insert(*tag), "duplicate {tag:?}");
        }
        // Every tag is four lowercase ASCII letters/digits (registered
        // space).
        for tag in &all {
            assert!(
                tag.iter()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit()),
                "{tag:?}"
            );
        }
        // The ranges landed in their alphabetical slots.
        let pos = |t: &[u8; 4]| all.iter().position(|x| x == t).unwrap();
        assert!(pos(b"curs") < pos(b"cv01"));
        assert!(pos(b"cv99") < pos(b"c2pc"));
        assert!(pos(b"smpl") < pos(b"ss01"));
        assert!(pos(b"ss20") < pos(b"ssty"));
    }

    #[test]
    fn shaping_default_features_are_registered() {
        // The shaping pipeline's default-enabled feature sets must all
        // be registry entries.
        for tag in [
            b"ccmp", b"locl", b"liga", b"clig", b"calt", b"rlig", b"rvrn", b"kern", b"mark",
            b"mkmk", b"curs", b"dist",
        ] {
            assert!(is_registered_feature_tag(*tag), "{tag:?}");
        }
    }
}
