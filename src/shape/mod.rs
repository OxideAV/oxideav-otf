//! Text shaping: a `shape(text, features) → positioned glyphs`
//! pipeline over the crate's cmap / GSUB / GPOS / GDEF / hmtx tables.
//!
//! Spec: `docs/text/opentype/otspec-chapter2-common-layout-tables.html`
//! §"Features and lookups" defines the processing model implemented
//! here:
//!
//! 1. The client picks a script and language system, reads the
//!    LangSys table's feature indices, and selects the features it
//!    wants to implement (plus the LangSys `requiredFeatureIndex`,
//!    which is always applied).
//! 2. The lookup indices of every selected feature are merged and
//!    "arranged numerically into their LookupList order" — lookups
//!    from different features interleave by LookupList index.
//! 3. Each lookup is applied over the whole glyph run before the next
//!    lookup starts; within a lookup, subtables are tried in order and
//!    the first subtable that matches at the current glyph is used.
//!
//! The same model runs twice: once against GSUB (substitution — the
//! glyph buffer mutates) and once against GPOS (positioning — a
//! parallel array of placements/advances mutates). GSUB lookup
//! application lives in [`gsub`]; GPOS application in [`gpos`];
//! buffer + LookupFlag skip filtering in [`buffer`].
//!
//! Scope notes (documented limitations, not spec deviations):
//! * Horizontal, left-to-right layout only. Script-specific
//!   preprocessing (bidi reordering, Arabic joining analysis, Indic
//!   syllable reordering, Unicode normalization) is out of scope —
//!   the spec itself places such processing outside OpenType Layout
//!   ("Details on such script-specific processing is outside the
//!   scope of this specification").
//! * The staged copy of the feature-tags registry is a stub, so the
//!   *default-enabled* feature sets below are a crate policy (the
//!   spec: "a client chooses the features to be applied"): GSUB
//!   defaults `ccmp`, `locl`, `liga`, `clig`, `calt`, `rlig`; GPOS
//!   defaults `kern`, `mark`, `mkmk`, `curs`, `dist`. Callers can
//!   enable/disable any feature via [`ShapeOptions::features`].

pub(crate) mod buffer;
pub(crate) mod gpos;
pub(crate) mod gsub;

use crate::tables::layout::{FeatureList, LangSys, Script, ScriptList};
use crate::Error;
use crate::Font;
use buffer::GlyphInfo;

/// Maximum nesting depth for contextual lookups that reference nested
/// lookups (GSUB types 5/6, GPOS types 7/8). The spec does not bound
/// the nesting, but a malformed font can make it cyclic; 8 levels is
/// far beyond any legitimate layout.
pub(crate) const MAX_NESTING_DEPTH: usize = 8;

/// One shaped glyph: the output of [`Font::shape`].
///
/// All values are in font design units (`head.unitsPerEm` scale).
/// `x_offset` / `y_offset` displace the glyph's ink without moving the
/// pen; `x_advance` / `y_advance` move the pen to the next glyph
/// (GPOS chapter, §"Basic glyph positioning").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShapedGlyph {
    /// Final glyph ID after all substitutions.
    pub glyph: u16,
    /// Index (in Unicode scalar values, zero-based) of the character
    /// in the input text this glyph derives from. Ligatures take the
    /// smallest cluster of their components.
    pub cluster: u32,
    /// Horizontal pen advance, design units.
    pub x_advance: i32,
    /// Vertical pen advance, design units (0 in horizontal layout
    /// unless a GPOS `yAdvance` adjusted it).
    pub y_advance: i32,
    /// Horizontal displacement of the glyph ink, design units.
    pub x_offset: i32,
    /// Vertical displacement of the glyph ink, design units.
    pub y_offset: i32,
}

/// A `(feature tag, value)` request.
///
/// * `value == 0` disables the feature (removes it from the default
///   set, if present there).
/// * `value == 1` enables the feature.
/// * `value >= 2` enables the feature and, for GSUB alternate
///   substitution (lookup type 3), selects the `value`'th alternate
///   (1-based) from the AlternateSet — the spec leaves the choice of
///   alternate to the client, and a numeric selector is this crate's
///   client policy. Lookup types other than alternate substitution
///   are applied only for `value == 1`, so a selector aimed at an
///   AlternateSet does not accidentally trigger a feature's plain
///   single substitutions (relevant to `aalt`-style features that mix
///   both).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FeatureSetting {
    /// Registered feature tag, e.g. `*b"liga"`.
    pub tag: [u8; 4],
    /// 0 = off; 1 = on; ≥2 = on with alternate selection.
    pub value: u32,
}

impl FeatureSetting {
    /// Convenience constructor.
    pub fn new(tag: [u8; 4], value: u32) -> Self {
        FeatureSetting { tag, value }
    }
}

/// Options for [`Font::shape`].
#[derive(Debug, Clone, Default)]
pub struct ShapeOptions {
    /// OpenType script tag to shape with (e.g. `*b"latn"`). `None`
    /// selects `DFLT`, falling back to `latn` when the font has no
    /// `DFLT` script.
    pub script: Option<[u8; 4]>,
    /// OpenType language-system tag (e.g. `*b"TRK "`). `None` — or a
    /// tag the script does not define — selects the script's default
    /// language system.
    pub language: Option<[u8; 4]>,
    /// User feature overrides, applied on top of the default-enabled
    /// sets (see the module docs).
    pub features: Vec<FeatureSetting>,
    /// Variable-font user-scale axis coordinates (same order as
    /// `fvar` axes). Empty = the default instance. When non-empty,
    /// HVAR advance deltas and GPOS VariationIndex deltas are applied.
    pub coords: Vec<f32>,
}

/// The default-enabled GSUB features (crate policy; see module docs).
const GSUB_DEFAULT_FEATURES: [[u8; 4]; 6] =
    [*b"ccmp", *b"locl", *b"liga", *b"clig", *b"calt", *b"rlig"];

/// The default-enabled GPOS features (crate policy; see module docs).
const GPOS_DEFAULT_FEATURES: [[u8; 4]; 5] = [*b"kern", *b"mark", *b"mkmk", *b"curs", *b"dist"];

/// One planned lookup application: which lookup, and the value of the
/// feature that requested it (for alternate selection).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PlannedLookup {
    pub lookup_index: u16,
    pub feature_value: u32,
}

/// Resolve `script` / `language` to a LangSys per the chapter-2 model.
fn resolve_lang_sys<'a>(
    scripts: &ScriptList<'a>,
    script: Option<[u8; 4]>,
    language: Option<[u8; 4]>,
) -> Option<LangSys<'a>> {
    let script_table: Script<'a> = match script {
        Some(tag) => scripts.find(&tag)?.ok()?,
        None => match scripts.find(b"DFLT") {
            Some(s) => s.ok()?,
            None => scripts.find(b"latn")?.ok()?,
        },
    };
    if let Some(lang) = language {
        if let Some(Ok(ls)) = script_table.find_lang_sys(&lang) {
            return Some(ls);
        }
    }
    script_table.default_lang_sys()?.ok()
}

/// Build the ordered lookup plan for one stage (GSUB or GPOS).
///
/// Implements chapter 2 §"Features and lookups": collect the lookup
/// indices of the required feature plus every enabled feature, then
/// arrange them "numerically into their LookupList order". When two
/// features reference the same lookup, the first (lowest feature
/// index) wins the value slot.
pub(crate) fn build_plan(
    scripts: &ScriptList<'_>,
    features: &FeatureList<'_>,
    defaults: &[[u8; 4]],
    options: &ShapeOptions,
) -> Vec<PlannedLookup> {
    let Some(lang_sys) = resolve_lang_sys(scripts, options.script, options.language) else {
        return Vec::new();
    };

    // Effective feature set: defaults, overridden by user settings.
    let value_of = |tag: [u8; 4]| -> u32 {
        for f in &options.features {
            if f.tag == tag {
                return f.value;
            }
        }
        if defaults.contains(&tag) {
            1
        } else {
            0
        }
    };

    let mut planned: Vec<PlannedLookup> = Vec::new();
    let mut add_feature = |feature_index: u16, value: u32| {
        let Some(Ok(feature)) = features.feature(feature_index) else {
            return;
        };
        for li in feature.lookup_indices() {
            planned.push(PlannedLookup {
                lookup_index: li,
                feature_value: value,
            });
        }
    };

    // The required feature is always applied (value 1).
    if let Some(required) = lang_sys.required_feature_index() {
        add_feature(required, 1);
    }
    for fi in lang_sys.feature_indices() {
        let Some(tag) = features.tag(fi) else {
            continue;
        };
        let value = value_of(tag);
        if value != 0 {
            add_feature(fi, value);
        }
    }

    // LookupList order; first feature wins on a shared lookup.
    planned.sort_by_key(|p| p.lookup_index);
    planned.dedup_by_key(|p| p.lookup_index);
    planned
}

/// `true` when the resolved language system exposes `tag` and the
/// effective feature set (defaults + user overrides) enables it.
pub(crate) fn stage_has_enabled_feature(
    scripts: &ScriptList<'_>,
    features: &FeatureList<'_>,
    tag: [u8; 4],
    defaults: &[[u8; 4]],
    options: &ShapeOptions,
) -> bool {
    let Some(lang_sys) = resolve_lang_sys(scripts, options.script, options.language) else {
        return false;
    };
    let mut value = if defaults.contains(&tag) { 1 } else { 0 };
    for f in &options.features {
        if f.tag == tag {
            value = f.value;
        }
    }
    if value == 0 {
        return false;
    }
    let found = lang_sys
        .feature_indices()
        .any(|fi| features.tag(fi) == Some(tag));
    found
}

impl<'a> Font<'a> {
    /// Shape `text` into positioned glyphs.
    ///
    /// Pipeline: cmap character→glyph mapping (with cluster tracking),
    /// GSUB substitution (script/langsys feature resolution → lookups
    /// applied in LookupList order), then GPOS positioning over hmtx
    /// advances. Fonts without GSUB/GPOS shape to their plain cmap +
    /// hmtx form; if GPOS provides no `kern` feature for the resolved
    /// script, the legacy `kern` table (when present and `kern` is
    /// enabled) supplies pair kerning instead.
    ///
    /// See [`ShapeOptions`] for script/language/feature/variation
    /// selection and the module docs for the default feature sets and
    /// scope limitations (horizontal LTR; no Unicode preprocessing).
    pub fn shape(&self, text: &str, options: &ShapeOptions) -> Result<Vec<ShapedGlyph>, Error> {
        // 1. Characters → glyph buffer. Unmapped characters map to
        //    glyph 0 (.notdef) per the cmap convention.
        let mut glyphs: Vec<GlyphInfo> = text
            .chars()
            .enumerate()
            .map(|(i, ch)| GlyphInfo::new(self.glyph_index(ch).unwrap_or(0), i as u32))
            .collect();

        // 2. GSUB substitution pass.
        if let Some(gsub_table) = self.gsub() {
            let plan = match (gsub_table.script_list(), gsub_table.feature_list()) {
                (Ok(scripts), Ok(features)) => {
                    build_plan(&scripts, &features, &GSUB_DEFAULT_FEATURES, options)
                }
                _ => Vec::new(),
            };
            for p in &plan {
                gsub::apply_lookup(gsub_table, self.gdef(), p, &mut glyphs);
            }
        }

        // 3. GPOS positioning pass over hmtx advances.
        let normalized = if options.coords.is_empty() {
            Vec::new()
        } else {
            self.normalize_coords(&options.coords)
        };
        let mut positions = gpos::init_positions(self, &glyphs, &normalized);
        let mut gpos_kerned = false;
        if let Some(gpos_table) = self.gpos() {
            let plan = match (gpos_table.script_list(), gpos_table.feature_list()) {
                (Ok(scripts), Ok(features)) => {
                    gpos_kerned = stage_has_enabled_feature(
                        &scripts,
                        &features,
                        *b"kern",
                        &GPOS_DEFAULT_FEATURES,
                        options,
                    );
                    build_plan(&scripts, &features, &GPOS_DEFAULT_FEATURES, options)
                }
                _ => Vec::new(),
            };
            for p in &plan {
                gpos::apply_lookup(
                    gpos_table,
                    self.gdef(),
                    p,
                    &glyphs,
                    &mut positions,
                    &normalized,
                );
            }
        }

        // 4. Legacy `kern` fallback (GPOS pair adjustment supersedes
        //    it when the font provides a GPOS `kern` feature for the
        //    resolved script/language system).
        if !gpos_kerned {
            gpos::apply_legacy_kern(self, &glyphs, &mut positions, options);
        }

        Ok(glyphs
            .iter()
            .zip(positions.iter())
            .map(|(g, p)| ShapedGlyph {
                glyph: g.glyph,
                cluster: g.cluster,
                x_advance: p.x_advance,
                y_advance: p.y_advance,
                x_offset: p.x_offset,
                y_offset: p.y_offset,
            })
            .collect())
    }
}
