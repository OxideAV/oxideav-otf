//! `BASE` — baseline table (ISO/IEC 14496-22:2019 §6.3).
//!
//! The baseline table records, per layout axis (horizontal / vertical),
//! the baseline coordinate positions and min/max glyph extents for each
//! script, so glyphs of different scripts and sizes align on a common
//! line of text.
//!
//! Structure:
//!
//! ```text
//! BASE header (v1.0 / v1.1)
//!   uint16   majorVersion = 1
//!   uint16   minorVersion = 0 or 1
//!   Offset16 horizAxisOffset   (may be NULL)
//!   Offset16 vertAxisOffset    (may be NULL)
//!   Offset32 itemVarStoreOffset (v1.1 only; may be NULL)
//!
//! Axis table
//!   Offset16 baseTagListOffset    (may be NULL)
//!   Offset16 baseScriptListOffset
//!
//! BaseTagList:    uint16 baseTagCount; Tag baselineTags[]
//! BaseScriptList: uint16 baseScriptCount; BaseScriptRecord[]
//!   BaseScriptRecord: Tag baseScriptTag; Offset16 baseScriptOffset
//! BaseScript:
//!   Offset16 baseValuesOffset (may be NULL)
//!   Offset16 defaultMinMaxOffset (may be NULL)
//!   uint16 baseLangSysCount; BaseLangSysRecord[]
//! BaseValues:
//!   uint16 defaultBaselineIndex
//!   uint16 baseCoordCount
//!   Offset16 baseCoords[baseCoordCount]
//! BaseCoord: format 1 (value), 2 (value + contour point), 3 (value +
//! Device / VariationIndex).
//! ```
//!
//! This decoder surfaces the per-axis baseline tag list and, per script,
//! the default baseline index plus the resolved BaseCoord value for each
//! baseline tag. The MinMax / FeatMinMax extent sub-tables are reached
//! through raw offsets only (their detail is rarely consumed by a
//! shaper and the structure is recursive).

use crate::parser::{read_i16, read_tag, read_u16};
use crate::Error;

/// Which layout axis a query targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BaseAxis {
    /// Horizontal layout (`HorizAxis`): baseline values are Y coords.
    Horizontal,
    /// Vertical layout (`VertAxis`): baseline values are X coords.
    Vertical,
}

/// One layout axis of the BASE table.
#[derive(Debug, Clone)]
pub struct AxisTable {
    /// `baselineTags` — the baselines (e.g. `b"romn"`, `b"ideo"`,
    /// `b"hang"`), in alphabetical order, shared by every script in this
    /// axis. Empty when the `BaseTagList` offset was NULL.
    baseline_tags: Vec<[u8; 4]>,
    /// `(baseScriptTag, BaseScript)` records, alphabetical by tag.
    scripts: Vec<([u8; 4], BaseScript)>,
}

/// Per-script baseline data.
#[derive(Debug, Clone)]
pub struct BaseScript {
    /// `defaultBaselineIndex` — index into the axis's `baseline_tags`
    /// identifying this script's default baseline.
    pub default_baseline_index: u16,
    /// The resolved BaseCoord value for each baseline tag, in the same
    /// order as the axis's `baseline_tags`. `None` when the script has
    /// no `BaseValues` table.
    base_coords: Option<Vec<i16>>,
}

impl AxisTable {
    /// The axis's baseline tags (alphabetical).
    pub fn baseline_tags(&self) -> &[[u8; 4]] {
        &self.baseline_tags
    }

    /// Look up the per-script baseline data by script tag.
    pub fn script(&self, tag: &[u8; 4]) -> Option<&BaseScript> {
        self.scripts.iter().find(|(t, _)| t == tag).map(|(_, s)| s)
    }

    /// Every `(scriptTag, BaseScript)` pair.
    pub fn scripts(&self) -> impl Iterator<Item = (&[u8; 4], &BaseScript)> {
        self.scripts.iter().map(|(t, s)| (t, s))
    }
}

impl BaseScript {
    /// The resolved BaseCoord value (design units) for a baseline tag,
    /// given the axis's tag list. Returns `None` if the tag is unknown
    /// to the axis or the script has no `BaseValues` data.
    pub fn coord_for_tag(&self, axis: &AxisTable, baseline_tag: &[u8; 4]) -> Option<i16> {
        let idx = axis.baseline_tags.iter().position(|t| t == baseline_tag)?;
        self.base_coords.as_ref()?.get(idx).copied()
    }

    /// The resolved BaseCoord value (design units) for the script's
    /// default baseline.
    pub fn default_baseline_coord(&self) -> Option<i16> {
        self.base_coords
            .as_ref()?
            .get(self.default_baseline_index as usize)
            .copied()
    }
}

/// A parsed `BASE` table.
#[derive(Debug, Clone)]
pub struct BaseTable {
    major: u16,
    minor: u16,
    horiz: Option<AxisTable>,
    vert: Option<AxisTable>,
    /// Raw offset to the v1.1 ItemVariationStore (0 / absent ⇒ none).
    item_var_store_offset: u32,
}

impl BaseTable {
    pub fn parse(bytes: &[u8]) -> Result<Self, Error> {
        if bytes.len() < 6 {
            return Err(Error::UnexpectedEof);
        }
        let major = read_u16(bytes, 0)?;
        let minor = read_u16(bytes, 2)?;
        if major != 1 {
            return Err(Error::BadStructure("BASE: unsupported majorVersion"));
        }
        let horiz_off = read_u16(bytes, 4)? as usize;
        let vert_off = read_u16(bytes, 6)? as usize;
        let item_var_store_offset = if minor >= 1 {
            crate::parser::read_u32(bytes, 8).unwrap_or(0)
        } else {
            0
        };

        let horiz = if horiz_off != 0 {
            Some(parse_axis(bytes, horiz_off)?)
        } else {
            None
        };
        let vert = if vert_off != 0 {
            Some(parse_axis(bytes, vert_off)?)
        } else {
            None
        };

        Ok(Self {
            major,
            minor,
            horiz,
            vert,
            item_var_store_offset,
        })
    }

    /// `(major, minor)` version.
    pub fn version(&self) -> (u16, u16) {
        (self.major, self.minor)
    }

    /// The horizontal axis table (baseline Y values), if present.
    pub fn horizontal_axis(&self) -> Option<&AxisTable> {
        self.horiz.as_ref()
    }

    /// The vertical axis table (baseline X values), if present.
    pub fn vertical_axis(&self) -> Option<&AxisTable> {
        self.vert.as_ref()
    }

    /// Select an axis by direction.
    pub fn axis(&self, axis: BaseAxis) -> Option<&AxisTable> {
        match axis {
            BaseAxis::Horizontal => self.horiz.as_ref(),
            BaseAxis::Vertical => self.vert.as_ref(),
        }
    }

    /// Raw v1.1 `itemVarStoreOffset` (0 when absent). The delta-set
    /// ItemVariationStore it points at can be parsed with
    /// `tables::ivs::ItemVariationStore::parse_at` against the BASE
    /// table bytes.
    pub fn item_var_store_offset(&self) -> u32 {
        self.item_var_store_offset
    }

    /// Convenience: the baseline coordinate (design units) for a given
    /// `(script_tag, baseline_tag)` on an axis. Returns `None` when the
    /// axis, script, or baseline tag is absent.
    pub fn baseline_coord(
        &self,
        axis: BaseAxis,
        script_tag: &[u8; 4],
        baseline_tag: &[u8; 4],
    ) -> Option<i16> {
        let a = self.axis(axis)?;
        let s = a.script(script_tag)?;
        s.coord_for_tag(a, baseline_tag)
    }
}

// ---- baseline-tag registry + ideographic em-box / ICF ----------------------

/// The registered baseline tags (staged registry
/// `docs/text/opentype/registries/baseline-tags.html`):
/// `(tag, HorizAxis meaning, VertAxis meaning)`. A tag means one
/// thing per layout direction — e.g. `ideo` is the ideographic
/// em-box **bottom** edge in `HorizAxis` and its **left** edge in
/// `VertAxis`.
pub const REGISTERED_BASELINE_TAGS: &[([u8; 4], &str, &str)] = &[
    (
        *b"hang",
        "hanging baseline (syllables hang from it in Tibetan and similar scripts)",
        "hanging baseline for characters rotated 90 degrees clockwise",
    ),
    (
        *b"icfb",
        "ideographic character face bottom edge",
        "ideographic character face left edge",
    ),
    (
        *b"icft",
        "ideographic character face top edge",
        "ideographic character face right edge",
    ),
    (
        *b"ideo",
        "ideographic em-box bottom edge",
        "ideographic em-box left edge (must be 0 when present)",
    ),
    (
        *b"idtp",
        "ideographic em-box top edge",
        "ideographic em-box right edge (recommended: head.unitsPerEm)",
    ),
    (
        *b"math",
        "baseline about which mathematical characters are centered",
        "the same, for formulas rotated 90 degrees clockwise",
    ),
    (
        *b"romn",
        "baseline of alphabetic scripts (Latin, Cyrillic, Greek)",
        "alphabetic baseline for characters rotated 90 degrees clockwise",
    ),
];

/// Whether `tag` is a registered baseline tag.
pub fn is_registered_baseline_tag(tag: [u8; 4]) -> bool {
    REGISTERED_BASELINE_TAGS.iter().any(|(t, _, _)| *t == tag)
}

/// A font's **ideographic em-box** for one script: the rectangle
/// defining the standard escapement around full-width ideographic
/// glyphs, in design units (baseline-tags registry, "Ideographic
/// em-box"). Usually a square, but may be vertically condensed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IdeoEmBox {
    /// Left edge — always 0 by definition.
    pub left: i32,
    /// Bottom edge (`HorizAxis.ideo`, or `OS/2.sTypoDescender` for a
    /// CJK font without one).
    pub bottom: i32,
    /// Right edge (`VertAxis.idtp`, defaulting to `head.unitsPerEm`).
    pub right: i32,
    /// Top edge (`HorizAxis.idtp`, defaulting to
    /// `HorizAxis.ideo + head.unitsPerEm`, or `OS/2.sTypoAscender`
    /// for the CJK fallback).
    pub top: i32,
}

impl IdeoEmBox {
    /// The horizontal-axis center baseline: halfway between top and
    /// bottom, rounded to the design unit nearest 0 (the registry's
    /// division rule).
    pub fn horizontal_center(&self) -> i32 {
        (self.top + self.bottom) / 2
    }

    /// The vertical-axis center baseline: halfway between left and
    /// right, rounded to the design unit nearest 0.
    pub fn vertical_center(&self) -> i32 {
        (self.left + self.right) / 2
    }
}

/// A font's **ideographic character face** (ICF) box for one script:
/// the average/approximate bounding box of its ideographic glyphs,
/// in design units (baseline-tags registry, "Ideographic character
/// face"). The margin left over inside the ideographic em-box is the
/// font's default escapement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IcfBox {
    /// Left edge (`VertAxis.icfb`, defaulting to the bottom margin).
    pub left: i32,
    /// Bottom edge (`HorizAxis.icfb` — the minimum required datum).
    pub bottom: i32,
    /// Right edge (`VertAxis.icft`, defaulting to
    /// `ideoEmboxRight - icfLeft`).
    pub right: i32,
    /// Top edge (`HorizAxis.icft`, defaulting to
    /// `ideoEmboxTop - margin`).
    pub top: i32,
}

impl IcfBox {
    /// The horizontal-axis ICF center: halfway between top and
    /// bottom, rounded toward 0.
    pub fn horizontal_center(&self) -> i32 {
        (self.top + self.bottom) / 2
    }

    /// The vertical-axis ICF center: halfway between left and right,
    /// rounded toward 0.
    pub fn vertical_center(&self) -> i32 {
        (self.left + self.right) / 2
    }
}

impl BaseTable {
    /// Derive the ideographic em-box for `script_tag` per the
    /// baseline-tags registry algorithm.
    ///
    /// `cjk_fallback` carries `(OS/2.sTypoDescender,
    /// OS/2.sTypoAscender)` **when the font is CJK** (the registry
    /// suggests deciding via the `meta` table's `dlng` entry, the
    /// CJK `OS/2.ulUnicodeRange` bits, or `OS/2.ulCodePageRange`) —
    /// it is used only when `HorizAxis.ideo` is absent; pass `None`
    /// for a non-CJK font. Returns `None` when the em-box cannot be
    /// determined.
    ///
    /// A non-zero `VertAxis.ideo` is a bad value per the registry
    /// ("must be set to 0"); the left edge is 0 regardless.
    pub fn ideographic_em_box(
        &self,
        script_tag: &[u8; 4],
        units_per_em: u16,
        cjk_fallback: Option<(i16, i16)>,
    ) -> Option<IdeoEmBox> {
        let upem = units_per_em as i32;
        let horiz = |tag: &[u8; 4]| self.baseline_coord(BaseAxis::Horizontal, script_tag, tag);
        let vert = |tag: &[u8; 4]| self.baseline_coord(BaseAxis::Vertical, script_tag, tag);
        if let Some(ideo) = horiz(b"ideo") {
            let bottom = ideo as i32;
            let top = match horiz(b"idtp") {
                Some(idtp) => idtp as i32,
                None => bottom + upem,
            };
            let right = match vert(b"idtp") {
                Some(idtp) => idtp as i32,
                None => upem,
            };
            Some(IdeoEmBox {
                left: 0,
                bottom,
                right,
                top,
            })
        } else if let Some((descender, ascender)) = cjk_fallback {
            Some(IdeoEmBox {
                left: 0,
                bottom: descender as i32,
                right: upem,
                top: ascender as i32,
            })
        } else {
            None
        }
    }

    /// Derive the ideographic character face box for `script_tag`
    /// per the baseline-tags registry algorithm, given the already-
    /// derived ideographic em-box. `HorizAxis.icfb` is the minimum
    /// required datum; the other three edges default from the em-box
    /// and the bottom margin. Returns `None` when the font records
    /// no ICF information.
    pub fn ideographic_character_face(
        &self,
        script_tag: &[u8; 4],
        em_box: &IdeoEmBox,
    ) -> Option<IcfBox> {
        let horiz = |tag: &[u8; 4]| self.baseline_coord(BaseAxis::Horizontal, script_tag, tag);
        let vert = |tag: &[u8; 4]| self.baseline_coord(BaseAxis::Vertical, script_tag, tag);
        let icfb = horiz(b"icfb")?;
        let bottom = icfb as i32;
        let margin = bottom - em_box.bottom;
        let top = match horiz(b"icft") {
            Some(icft) => icft as i32,
            None => em_box.top - margin,
        };
        let left = match vert(b"icfb") {
            Some(icfb) => icfb as i32,
            None => margin,
        };
        let right = match vert(b"icft") {
            Some(icft) => icft as i32,
            None => em_box.right - left,
        };
        Some(IcfBox {
            left,
            bottom,
            right,
            top,
        })
    }
}

fn parse_axis(bytes: &[u8], off: usize) -> Result<AxisTable, Error> {
    if off + 4 > bytes.len() {
        return Err(Error::UnexpectedEof);
    }
    let base_tag_list_off = read_u16(bytes, off)? as usize;
    let base_script_list_off = read_u16(bytes, off + 2)? as usize;

    // BaseTagList (may be NULL).
    let mut baseline_tags = Vec::new();
    if base_tag_list_off != 0 {
        let tl = off + base_tag_list_off;
        let count = read_u16(bytes, tl)? as usize;
        for i in 0..count {
            baseline_tags.push(read_tag(bytes, tl + 2 + i * 4)?);
        }
    }

    // BaseScriptList.
    let mut scripts = Vec::new();
    if base_script_list_off != 0 {
        let sl = off + base_script_list_off;
        let count = read_u16(bytes, sl)? as usize;
        for i in 0..count {
            let rec = sl + 2 + i * 6;
            let tag = read_tag(bytes, rec)?;
            let bs_off = read_u16(bytes, rec + 4)? as usize;
            let bs = parse_base_script(bytes, sl + bs_off)?;
            scripts.push((tag, bs));
        }
    }

    Ok(AxisTable {
        baseline_tags,
        scripts,
    })
}

fn parse_base_script(bytes: &[u8], off: usize) -> Result<BaseScript, Error> {
    if off + 6 > bytes.len() {
        return Err(Error::UnexpectedEof);
    }
    let base_values_off = read_u16(bytes, off)? as usize;
    // defaultMinMaxOffset + baseLangSysCount are read structurally but
    // the MinMax detail is surfaced via raw offsets only.
    let mut default_baseline_index = 0u16;
    let base_coords = if base_values_off != 0 {
        let bv = off + base_values_off;
        default_baseline_index = read_u16(bytes, bv)?;
        let coord_count = read_u16(bytes, bv + 2)? as usize;
        let mut coords = Vec::with_capacity(coord_count);
        for i in 0..coord_count {
            let coord_off = read_u16(bytes, bv + 4 + i * 2)? as usize;
            if coord_off == 0 {
                coords.push(0);
                continue;
            }
            coords.push(parse_base_coord(bytes, bv + coord_off)?);
        }
        Some(coords)
    } else {
        None
    };

    Ok(BaseScript {
        default_baseline_index,
        base_coords,
    })
}

/// Parse a BaseCoord table — formats 1/2/3 all expose the design-unit
/// `coordinate` as the first int16 after the format word; the contour-
/// point (format 2) and Device/VariationIndex (format 3) refinements are
/// not applied (hinting is out of scope, and the default-instance value
/// is the format-3 coordinate).
fn parse_base_coord(bytes: &[u8], off: usize) -> Result<i16, Error> {
    let format = read_u16(bytes, off)?;
    match format {
        1..=3 => read_i16(bytes, off + 2),
        _ => Err(Error::BadStructure("BASE: unknown BaseCoord format")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a BASE v1.0 with a HorizAxis: two baselines ('ideo',
    /// 'romn'), one script 'latn' with BaseValues (default index 1,
    /// coords [-120 for ideo, 0 for romn]).
    fn build() -> Vec<u8> {
        // We assemble the axis blob first, then the header.
        // Within the axis (relative to axis start):
        //   0: baseTagListOffset, 2: baseScriptListOffset.
        // Lay out: header(4) tagList scriptList baseScript baseValues
        //          coord0 coord1.
        let axis_header = 4usize;
        let tag_list_off = axis_header; // at 4
        let tag_list_len = 2 + 2 * 4; // count + 2 tags = 10
        let script_list_off = tag_list_off + tag_list_len; // 14
        let script_list_len = 2 + 6; // count + 1 record = 8
        let base_script_off = script_list_off + script_list_len; // 22
        let base_script_len = 6; // baseValues + defMinMax + langSysCount
        let base_values_off = base_script_off + base_script_len; // 28
        let base_values_len = 4 + 2 * 2; // header + 2 coord offsets = 8
        let coord0_off = base_values_off + base_values_len; // 36
        let coord1_off = coord0_off + 4; // each BaseCoord fmt1 = 4 bytes

        let axis_len = coord1_off + 4;
        let mut axis = vec![0u8; axis_len];
        // axis header.
        axis[0..2].copy_from_slice(&(tag_list_off as u16).to_be_bytes());
        axis[2..4].copy_from_slice(&(script_list_off as u16).to_be_bytes());
        // BaseTagList.
        axis[tag_list_off..tag_list_off + 2].copy_from_slice(&2u16.to_be_bytes());
        axis[tag_list_off + 2..tag_list_off + 6].copy_from_slice(b"ideo");
        axis[tag_list_off + 6..tag_list_off + 10].copy_from_slice(b"romn");
        // BaseScriptList.
        axis[script_list_off..script_list_off + 2].copy_from_slice(&1u16.to_be_bytes());
        axis[script_list_off + 2..script_list_off + 6].copy_from_slice(b"latn");
        let rec_to_bs = (base_script_off - script_list_off) as u16;
        axis[script_list_off + 6..script_list_off + 8].copy_from_slice(&rec_to_bs.to_be_bytes());
        // BaseScript.
        let bs_to_bv = (base_values_off - base_script_off) as u16;
        axis[base_script_off..base_script_off + 2].copy_from_slice(&bs_to_bv.to_be_bytes());
        // defaultMinMaxOffset = 0, baseLangSysCount = 0 (already zero).
        // BaseValues.
        axis[base_values_off..base_values_off + 2].copy_from_slice(&1u16.to_be_bytes()); // default index 1
        axis[base_values_off + 2..base_values_off + 4].copy_from_slice(&2u16.to_be_bytes()); // count
        let bv_to_c0 = (coord0_off - base_values_off) as u16;
        let bv_to_c1 = (coord1_off - base_values_off) as u16;
        axis[base_values_off + 4..base_values_off + 6].copy_from_slice(&bv_to_c0.to_be_bytes());
        axis[base_values_off + 6..base_values_off + 8].copy_from_slice(&bv_to_c1.to_be_bytes());
        // BaseCoord 0 (ideo): format 1, value -120.
        axis[coord0_off..coord0_off + 2].copy_from_slice(&1u16.to_be_bytes());
        axis[coord0_off + 2..coord0_off + 4].copy_from_slice(&(-120i16).to_be_bytes());
        // BaseCoord 1 (romn): format 1, value 0.
        axis[coord1_off..coord1_off + 2].copy_from_slice(&1u16.to_be_bytes());
        axis[coord1_off + 2..coord1_off + 4].copy_from_slice(&0i16.to_be_bytes());

        // Header: v1.0, horizAxisOffset, vertAxisOffset = 0.
        let header_len = 8;
        let mut b = vec![0u8; header_len];
        b[0..2].copy_from_slice(&1u16.to_be_bytes()); // major
        b[2..4].copy_from_slice(&0u16.to_be_bytes()); // minor
        b[4..6].copy_from_slice(&(header_len as u16).to_be_bytes()); // horizAxisOffset
        b[6..8].copy_from_slice(&0u16.to_be_bytes()); // vertAxisOffset NULL
        b.extend_from_slice(&axis);
        b
    }

    #[test]
    fn parses_baseline_coords() {
        let base = BaseTable::parse(&build()).unwrap();
        assert_eq!(base.version(), (1, 0));
        let h = base.horizontal_axis().unwrap();
        assert_eq!(h.baseline_tags(), &[*b"ideo", *b"romn"]);
        let s = h.script(b"latn").unwrap();
        assert_eq!(s.default_baseline_index, 1);
        assert_eq!(s.coord_for_tag(h, b"ideo"), Some(-120));
        assert_eq!(s.coord_for_tag(h, b"romn"), Some(0));
        assert_eq!(s.default_baseline_coord(), Some(0));
        // convenience.
        assert_eq!(
            base.baseline_coord(BaseAxis::Horizontal, b"latn", b"ideo"),
            Some(-120)
        );
        // unknown script / axis.
        assert!(base
            .baseline_coord(BaseAxis::Horizontal, b"grek", b"romn")
            .is_none());
        assert!(base.vertical_axis().is_none());
    }

    #[test]
    fn rejects_bad_version() {
        let mut b = vec![0u8; 8];
        b[0..2].copy_from_slice(&2u16.to_be_bytes());
        assert!(matches!(BaseTable::parse(&b), Err(Error::BadStructure(_))));
    }

    /// Generic builder: one script ('hani') per axis with the given
    /// `(baseline tag, coord)` lists; either axis may be empty
    /// (NULL offset).
    fn build_axes(horiz: &[([u8; 4], i16)], vert: &[([u8; 4], i16)]) -> Vec<u8> {
        fn axis_blob(entries: &[([u8; 4], i16)]) -> Vec<u8> {
            let n = entries.len();
            let tag_list_off = 4usize;
            let tag_list_len = 2 + 4 * n;
            let script_list_off = tag_list_off + tag_list_len;
            let script_list_len = 2 + 6;
            let base_script_off = script_list_off + script_list_len;
            let base_script_len = 6;
            let base_values_off = base_script_off + base_script_len;
            let base_values_len = 4 + 2 * n;
            let coords_off = base_values_off + base_values_len;
            let mut a = vec![0u8; coords_off + 4 * n];
            a[0..2].copy_from_slice(&(tag_list_off as u16).to_be_bytes());
            a[2..4].copy_from_slice(&(script_list_off as u16).to_be_bytes());
            a[tag_list_off..tag_list_off + 2].copy_from_slice(&(n as u16).to_be_bytes());
            for (i, (tag, _)) in entries.iter().enumerate() {
                let at = tag_list_off + 2 + i * 4;
                a[at..at + 4].copy_from_slice(tag);
            }
            a[script_list_off..script_list_off + 2].copy_from_slice(&1u16.to_be_bytes());
            a[script_list_off + 2..script_list_off + 6].copy_from_slice(b"hani");
            let rec = (base_script_off - script_list_off) as u16;
            a[script_list_off + 6..script_list_off + 8].copy_from_slice(&rec.to_be_bytes());
            let bv = (base_values_off - base_script_off) as u16;
            a[base_script_off..base_script_off + 2].copy_from_slice(&bv.to_be_bytes());
            a[base_values_off..base_values_off + 2].copy_from_slice(&0u16.to_be_bytes());
            a[base_values_off + 2..base_values_off + 4].copy_from_slice(&(n as u16).to_be_bytes());
            for (i, (_, coord)) in entries.iter().enumerate() {
                let off_at = base_values_off + 4 + i * 2;
                let c_off = (coords_off + i * 4 - base_values_off) as u16;
                a[off_at..off_at + 2].copy_from_slice(&c_off.to_be_bytes());
                let c_at = coords_off + i * 4;
                a[c_at..c_at + 2].copy_from_slice(&1u16.to_be_bytes());
                a[c_at + 2..c_at + 4].copy_from_slice(&coord.to_be_bytes());
            }
            a
        }
        let mut b = vec![0u8; 8];
        b[0..2].copy_from_slice(&1u16.to_be_bytes());
        let mut at = 8usize;
        if !horiz.is_empty() {
            b[4..6].copy_from_slice(&(at as u16).to_be_bytes());
            let blob = axis_blob(horiz);
            at += blob.len();
            b.extend_from_slice(&blob);
        }
        if !vert.is_empty() {
            b[6..8].copy_from_slice(&(at as u16).to_be_bytes());
            b.extend_from_slice(&axis_blob(vert));
        }
        b
    }

    #[test]
    fn baseline_tag_registry() {
        assert_eq!(REGISTERED_BASELINE_TAGS.len(), 7);
        for tag in [
            b"hang", b"icfb", b"icft", b"ideo", b"idtp", b"math", b"romn",
        ] {
            assert!(is_registered_baseline_tag(*tag));
        }
        assert!(!is_registered_baseline_tag(*b"latn"));
    }

    /// The registry's Kozuka Mincho example: 1000-unit em,
    /// HorizAxis.ideo = -120 recorded alone describes the square
    /// em-box (0, -120)..(1000, 880); adding HorizAxis.idtp = 880 is
    /// equivalent.
    #[test]
    fn ideo_em_box_registry_example() {
        for entries in [
            &[(*b"ideo", -120i16)][..],
            &[(*b"ideo", -120), (*b"idtp", 880)][..],
        ] {
            let bytes = build_axes(entries, &[]);
            let base = BaseTable::parse(&bytes).unwrap();
            let em = base.ideographic_em_box(b"hani", 1000, None).unwrap();
            assert_eq!(
                em,
                IdeoEmBox {
                    left: 0,
                    bottom: -120,
                    right: 1000,
                    top: 880
                }
            );
            assert_eq!(em.horizontal_center(), 380);
            assert_eq!(em.vertical_center(), 500);
        }
        // VertAxis.idtp overrides the right edge.
        let bytes = build_axes(&[(*b"ideo", -120)], &[(*b"idtp", 950)]);
        let base = BaseTable::parse(&bytes).unwrap();
        let em = base.ideographic_em_box(b"hani", 1000, None).unwrap();
        assert_eq!(em.right, 950);
        // Center division rounds toward 0 (registry rule): top 881,
        // bottom -120 -> 761 / 2 = 380 (not 380.5 rounded up).
        let bytes = build_axes(&[(*b"ideo", -120), (*b"idtp", 881)], &[]);
        let base = BaseTable::parse(&bytes).unwrap();
        assert_eq!(
            base.ideographic_em_box(b"hani", 1000, None)
                .unwrap()
                .horizontal_center(),
            380
        );
    }

    #[test]
    fn ideo_em_box_cjk_fallback() {
        // No HorizAxis.ideo: a CJK font falls back to the OS/2 typo
        // metrics (the registry example's -120 / 880 pair).
        let bytes = build_axes(&[(*b"romn", 0)], &[]);
        let base = BaseTable::parse(&bytes).unwrap();
        let em = base
            .ideographic_em_box(b"hani", 1000, Some((-120, 880)))
            .unwrap();
        assert_eq!(
            em,
            IdeoEmBox {
                left: 0,
                bottom: -120,
                right: 1000,
                top: 880
            }
        );
        // Non-CJK without HorizAxis.ideo: undeterminable.
        assert!(base.ideographic_em_box(b"hani", 1000, None).is_none());
        // Unknown script: HorizAxis.ideo lookup fails; fallback still
        // applies for a CJK font.
        assert!(base
            .ideographic_em_box(b"grek", 1000, Some((-120, 880)))
            .is_some());
    }

    /// The registry's Kozuka Mincho Extra Light ICF example: with the
    /// em-box (0, -120)..(1000, 880), recording only
    /// HorizAxis.icfb = -79 derives margin 41 and the full box
    /// VertAxis.icfb = 41, HorizAxis.icft = 839, VertAxis.icft = 959.
    #[test]
    fn icf_registry_example() {
        let em = IdeoEmBox {
            left: 0,
            bottom: -120,
            right: 1000,
            top: 880,
        };
        // Minimal recording.
        let bytes = build_axes(&[(*b"ideo", -120), (*b"icfb", -79)], &[]);
        let base = BaseTable::parse(&bytes).unwrap();
        let icf = base.ideographic_character_face(b"hani", &em).unwrap();
        assert_eq!(
            icf,
            IcfBox {
                left: 41,
                bottom: -79,
                right: 959,
                top: 839
            }
        );
        // Full recording (the Heavy example): identical derivation.
        let bytes = build_axes(
            &[(*b"ideo", -120), (*b"icfb", -94), (*b"icft", 854)],
            &[(*b"icfb", 26), (*b"icft", 974)],
        );
        let base = BaseTable::parse(&bytes).unwrap();
        let icf = base.ideographic_character_face(b"hani", &em).unwrap();
        assert_eq!(
            icf,
            IcfBox {
                left: 26,
                bottom: -94,
                right: 974,
                top: 854
            }
        );
        assert_eq!(icf.horizontal_center(), 380);
        assert_eq!(icf.vertical_center(), 500);
        // No HorizAxis.icfb: no ICF information.
        let bytes = build_axes(&[(*b"ideo", -120)], &[]);
        let base = BaseTable::parse(&bytes).unwrap();
        assert!(base.ideographic_character_face(b"hani", &em).is_none());
    }
}
