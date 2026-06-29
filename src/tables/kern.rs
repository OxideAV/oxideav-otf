//! `kern` — kerning (ISO/IEC 14496-22:2019 §5.7.5).
//!
//! The legacy kerning table predates GPOS pair adjustment but is still
//! shipped by many fonts for compatibility. The OFF/Windows version of
//! the table has a 4-byte header (`version: uint16 = 0`, `nTables`),
//! followed by `nTables` subtables. Each subtable has its own header
//! (`version`, `length`, `coverage`) and one of two body formats:
//!
//! - **Format 0** — a sorted list of `(left, right, value)` pairs,
//!   binary-searchable on `(left << 16) | right`. The only format
//!   Windows supports.
//! - **Format 2** — a two-dimensional class-kerning array: each glyph
//!   maps (separately for the left and right sides) to a class, and the
//!   `(leftClass, rightClass)` cell holds the kerning value. The class
//!   tables store values **pre-multiplied** (left class value = byte
//!   offset of the row; right class value = byte offset of the cell
//!   within a row) so a lookup is `array + leftValue + rightValue`.
//!
//! The `coverage` byte-field's low bits flag horizontal vs vertical,
//! kerning vs minimum, cross-stream, and override semantics; the high
//! byte is the subtable format. We decode the standard horizontal,
//! non-minimum, non-cross-stream case (the overwhelming majority) and
//! expose the coverage flags so a shaper can decide whether to apply a
//! given subtable.

use crate::parser::{read_i16, read_u16};
use crate::Error;

/// `coverage` bit 0: 1 = horizontal data, 0 = vertical.
pub const KERN_COVERAGE_HORIZONTAL: u16 = 0x0001;
/// `coverage` bit 1: 1 = minimum values, 0 = kerning values.
pub const KERN_COVERAGE_MINIMUM: u16 = 0x0002;
/// `coverage` bit 2: 1 = cross-stream kerning.
pub const KERN_COVERAGE_CROSS_STREAM: u16 = 0x0004;
/// `coverage` bit 3: 1 = override (replace accumulated value).
pub const KERN_COVERAGE_OVERRIDE: u16 = 0x0008;

/// A parsed `kern` table — a list of decoded subtables.
#[derive(Debug, Clone)]
pub struct KernTable<'a> {
    subtables: Vec<KernSubtable<'a>>,
}

/// One decoded `kern` subtable, retaining its coverage flags.
#[derive(Debug, Clone)]
pub struct KernSubtable<'a> {
    coverage: u16,
    body: KernBody<'a>,
}

#[derive(Debug, Clone)]
enum KernBody<'a> {
    /// Format 0: sorted pair list. We retain the slice of `nPairs * 6`
    /// pair bytes and binary-search it on demand.
    Format0 { pairs: &'a [u8], n_pairs: u16 },
    /// Format 2: class-kerning array.
    Format2(KernFormat2<'a>),
}

impl<'a> KernTable<'a> {
    /// Parse the whole `kern` table.
    pub fn parse(bytes: &'a [u8]) -> Result<Self, Error> {
        if bytes.len() < 4 {
            return Err(Error::UnexpectedEof);
        }
        let version = read_u16(bytes, 0)?;
        if version != 0 {
            // The Apple-flavoured `kern` (version 1.0 = 0x00010000 with a
            // 32-bit header) is a different on-disk layout we don't
            // decode here; the OFF spec defines only version 0.
            return Err(Error::BadStructure("kern: unsupported table version"));
        }
        let n_tables = read_u16(bytes, 2)?;
        let mut subtables = Vec::new();
        let mut off = 4usize;
        for _ in 0..n_tables {
            // Subtable header: version(2) length(2) coverage(2).
            if off + 6 > bytes.len() {
                return Err(Error::UnexpectedEof);
            }
            let length = read_u16(bytes, off + 2)? as usize;
            let coverage = read_u16(bytes, off + 4)?;
            if length < 6 || off + length > bytes.len() {
                return Err(Error::BadStructure("kern: subtable length out of range"));
            }
            let format = (coverage >> 8) & 0xff;
            // Body begins after the 6-byte subtable header.
            let body_bytes = &bytes[off + 6..off + length];
            // The subtable slice as a whole (offsets in format 2 are
            // relative to the start of the subtable, i.e. its header).
            let sub_slice = &bytes[off..off + length];
            match format {
                0 => {
                    let body = parse_format0(body_bytes)?;
                    subtables.push(KernSubtable { coverage, body });
                }
                2 => {
                    let f2 = KernFormat2::parse(sub_slice)?;
                    subtables.push(KernSubtable {
                        coverage,
                        body: KernBody::Format2(f2),
                    });
                }
                _ => {
                    // Reserved formats (1, 3..255) are skipped, not
                    // rejected: an unknown subtable shouldn't sink the
                    // whole table.
                }
            }
            off += length;
        }
        Ok(Self { subtables })
    }

    /// Number of decoded subtables.
    pub fn subtable_count(&self) -> usize {
        self.subtables.len()
    }

    /// Borrow a decoded subtable.
    pub fn subtable(&self, i: usize) -> Option<&KernSubtable<'a>> {
        self.subtables.get(i)
    }

    /// Iterate the decoded subtables.
    pub fn subtables(&self) -> impl Iterator<Item = &KernSubtable<'a>> {
        self.subtables.iter()
    }

    /// Accumulated horizontal kerning adjustment for an ordered glyph
    /// pair, in font units. Sums the `value` from every horizontal,
    /// non-minimum, non-override kerning subtable (per the spec: kerning
    /// adjustments are additive and subtable order doesn't matter for
    /// value subtables). Override subtables, if present, replace the
    /// accumulator. Minimum / cross-stream subtables are ignored by this
    /// convenience accessor (a shaper that needs them walks `subtables`).
    pub fn kerning(&self, left: u16, right: u16) -> i16 {
        let mut acc: i32 = 0;
        for st in &self.subtables {
            if st.coverage & KERN_COVERAGE_HORIZONTAL == 0 {
                continue; // vertical
            }
            if st.coverage & KERN_COVERAGE_MINIMUM != 0 {
                continue; // minimum-value subtable, not a kerning delta
            }
            if st.coverage & KERN_COVERAGE_CROSS_STREAM != 0 {
                continue;
            }
            if let Some(v) = st.value(left, right) {
                if st.coverage & KERN_COVERAGE_OVERRIDE != 0 {
                    acc = v as i32;
                } else {
                    acc += v as i32;
                }
            }
        }
        acc.clamp(i16::MIN as i32, i16::MAX as i32) as i16
    }
}

impl<'a> KernSubtable<'a> {
    /// Raw `coverage` field.
    pub fn coverage(&self) -> u16 {
        self.coverage
    }

    /// Subtable format (high byte of `coverage`): 0 or 2.
    pub fn format(&self) -> u16 {
        (self.coverage >> 8) & 0xff
    }

    /// `true` if this subtable carries horizontal data.
    pub fn is_horizontal(&self) -> bool {
        self.coverage & KERN_COVERAGE_HORIZONTAL != 0
    }

    /// `true` if this subtable carries minimum values (not kerning deltas).
    pub fn is_minimum(&self) -> bool {
        self.coverage & KERN_COVERAGE_MINIMUM != 0
    }

    /// `true` if this subtable carries cross-stream kerning.
    pub fn is_cross_stream(&self) -> bool {
        self.coverage & KERN_COVERAGE_CROSS_STREAM != 0
    }

    /// `true` if this subtable overrides the accumulated value.
    pub fn is_override(&self) -> bool {
        self.coverage & KERN_COVERAGE_OVERRIDE != 0
    }

    /// The kerning value this subtable assigns to the ordered glyph
    /// pair, or `None` if the pair is uncovered (format 0) or maps to a
    /// zero-class cell (format 2 returns the cell value, which is 0 for
    /// class 0).
    pub fn value(&self, left: u16, right: u16) -> Option<i16> {
        match &self.body {
            KernBody::Format0 { pairs, n_pairs } => format0_lookup(pairs, *n_pairs, left, right),
            KernBody::Format2(f2) => Some(f2.value(left, right)),
        }
    }
}

fn parse_format0(body: &[u8]) -> Result<KernBody<'_>, Error> {
    // Format-0 body: nPairs(2) searchRange(2) entrySelector(2)
    // rangeShift(2), then nPairs * (left(2) right(2) value(2)).
    if body.len() < 8 {
        return Err(Error::UnexpectedEof);
    }
    let n_pairs = read_u16(body, 0)?;
    let pairs_off = 8usize;
    let need = pairs_off + n_pairs as usize * 6;
    if body.len() < need {
        return Err(Error::BadStructure("kern: format 0 pair list truncated"));
    }
    Ok(KernBody::Format0 {
        pairs: &body[pairs_off..need],
        n_pairs,
    })
}

/// Binary-search the sorted format-0 pair list on the 32-bit key
/// `(left << 16) | right`.
fn format0_lookup(pairs: &[u8], n_pairs: u16, left: u16, right: u16) -> Option<i16> {
    let key = ((left as u32) << 16) | right as u32;
    let mut lo = 0i64;
    let mut hi = n_pairs as i64 - 1;
    while lo <= hi {
        let mid = (lo + hi) / 2;
        let off = mid as usize * 6;
        let pleft = read_u16(pairs, off).ok()?;
        let pright = read_u16(pairs, off + 2).ok()?;
        let pkey = ((pleft as u32) << 16) | pright as u32;
        match pkey.cmp(&key) {
            std::cmp::Ordering::Equal => return read_i16(pairs, off + 4).ok(),
            std::cmp::Ordering::Less => lo = mid + 1,
            std::cmp::Ordering::Greater => hi = mid - 1,
        }
    }
    None
}

/// Format-2 class-kerning subtable.
#[derive(Debug, Clone)]
struct KernFormat2<'a> {
    /// The full subtable slice (offsets are relative to it).
    sub: &'a [u8],
    left_class_off: usize,
    right_class_off: usize,
    array_off: usize,
}

impl<'a> KernFormat2<'a> {
    fn parse(sub: &'a [u8]) -> Result<Self, Error> {
        // Subtable header (6) + format-2 header:
        //   rowWidth(2) leftClassTable(2) rightClassTable(2) array(2).
        if sub.len() < 6 + 8 {
            return Err(Error::UnexpectedEof);
        }
        // rowWidth at offset 6 is informational here: the class tables
        // store pre-multiplied offsets, so a lookup is
        // `array + leftValue + rightValue` without needing rowWidth.
        let left_class_off = read_u16(sub, 8)? as usize;
        let right_class_off = read_u16(sub, 10)? as usize;
        let array_off = read_u16(sub, 12)? as usize;
        Ok(Self {
            sub,
            left_class_off,
            right_class_off,
            array_off,
        })
    }

    /// The kerning value for the ordered pair. A class table maps each
    /// glyph to a *pre-multiplied* offset (left → byte offset of its row
    /// relative to `array`; right → byte offset of the cell within a
    /// row). Glyphs outside a class range, or those mapped to class 0,
    /// land on the all-zero row/column and yield 0.
    fn value(&self, left: u16, right: u16) -> i16 {
        let left_val = self.class_value(self.left_class_off, left);
        let right_val = self.class_value(self.right_class_off, right);
        // The array cell lives at array_off + left_val + right_val,
        // measured from the start of the subtable.
        let cell = self.array_off + left_val + right_val;
        read_i16(self.sub, cell).unwrap_or(0)
    }

    /// Look up a glyph's pre-multiplied class value from a class table.
    /// `default_val` is returned for glyphs outside the table's range
    /// (left side defaults to row 0 = `0`; the caller passes the right
    /// default of 0 too).
    fn class_value(&self, table_off: usize, glyph: u16) -> usize {
        // Class table: firstGlyph(2) nGlyphs(2) then nGlyphs * uint16.
        let first = match read_u16(self.sub, table_off) {
            Ok(v) => v,
            Err(_) => return 0,
        };
        let n = match read_u16(self.sub, table_off + 2) {
            Ok(v) => v as u32,
            Err(_) => return 0,
        };
        let g = glyph as u32;
        if g < first as u32 || g >= first as u32 + n {
            return 0;
        }
        let idx = (g - first as u32) as usize;
        read_u16(self.sub, table_off + 4 + idx * 2).unwrap_or(0) as usize
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a one-subtable format-0 kern table from `(left, right,
    /// value)` triples (must be pre-sorted by the 32-bit key).
    fn build_format0(pairs: &[(u16, u16, i16)]) -> Vec<u8> {
        let n = pairs.len() as u16;
        let mut sub = Vec::new();
        // subtable header: version length coverage
        sub.extend_from_slice(&0u16.to_be_bytes()); // version
        let length = 6 + 8 + pairs.len() * 6;
        sub.extend_from_slice(&(length as u16).to_be_bytes());
        sub.extend_from_slice(&KERN_COVERAGE_HORIZONTAL.to_be_bytes()); // format 0, horizontal
                                                                        // format-0 body header
        sub.extend_from_slice(&n.to_be_bytes());
        sub.extend_from_slice(&0u16.to_be_bytes()); // searchRange
        sub.extend_from_slice(&0u16.to_be_bytes()); // entrySelector
        sub.extend_from_slice(&0u16.to_be_bytes()); // rangeShift
        for &(l, r, v) in pairs {
            sub.extend_from_slice(&l.to_be_bytes());
            sub.extend_from_slice(&r.to_be_bytes());
            sub.extend_from_slice(&v.to_be_bytes());
        }
        let mut t = Vec::new();
        t.extend_from_slice(&0u16.to_be_bytes()); // table version
        t.extend_from_slice(&1u16.to_be_bytes()); // nTables
        t.extend_from_slice(&sub);
        t
    }

    #[test]
    fn format0_binary_search() {
        let pairs = [(1u16, 2u16, -40i16), (1, 5, 10), (3, 4, -100), (3, 9, 25)];
        let t = build_format0(&pairs);
        let k = KernTable::parse(&t).unwrap();
        assert_eq!(k.subtable_count(), 1);
        assert_eq!(k.subtable(0).unwrap().format(), 0);
        assert_eq!(k.kerning(1, 2), -40);
        assert_eq!(k.kerning(1, 5), 10);
        assert_eq!(k.kerning(3, 4), -100);
        assert_eq!(k.kerning(3, 9), 25);
        // uncovered pair
        assert_eq!(k.kerning(2, 2), 0);
        assert_eq!(k.subtable(0).unwrap().value(7, 7), None);
    }

    #[test]
    fn additive_subtables() {
        // Two horizontal kerning subtables sum.
        let s0 = {
            let mut s = Vec::new();
            s.extend_from_slice(&0u16.to_be_bytes());
            s.extend_from_slice(&(6 + 8 + 6u16).to_be_bytes());
            s.extend_from_slice(&KERN_COVERAGE_HORIZONTAL.to_be_bytes());
            s.extend_from_slice(&1u16.to_be_bytes()); // nPairs
            s.extend_from_slice(&[0u8; 6]); // search header
            s.extend_from_slice(&1u16.to_be_bytes());
            s.extend_from_slice(&2u16.to_be_bytes());
            s.extend_from_slice(&(-10i16).to_be_bytes());
            s
        };
        let s1 = {
            let mut s = Vec::new();
            s.extend_from_slice(&0u16.to_be_bytes());
            s.extend_from_slice(&(6 + 8 + 6u16).to_be_bytes());
            s.extend_from_slice(&KERN_COVERAGE_HORIZONTAL.to_be_bytes());
            s.extend_from_slice(&1u16.to_be_bytes());
            s.extend_from_slice(&[0u8; 6]);
            s.extend_from_slice(&1u16.to_be_bytes());
            s.extend_from_slice(&2u16.to_be_bytes());
            s.extend_from_slice(&(-5i16).to_be_bytes());
            s
        };
        let mut t = Vec::new();
        t.extend_from_slice(&0u16.to_be_bytes());
        t.extend_from_slice(&2u16.to_be_bytes());
        t.extend_from_slice(&s0);
        t.extend_from_slice(&s1);
        let k = KernTable::parse(&t).unwrap();
        assert_eq!(k.subtable_count(), 2);
        assert_eq!(k.kerning(1, 2), -15);
    }

    #[test]
    fn vertical_and_minimum_skipped_by_convenience() {
        // A vertical subtable shouldn't contribute to the horizontal
        // `kerning()` accessor.
        let mut sub = Vec::new();
        sub.extend_from_slice(&0u16.to_be_bytes());
        sub.extend_from_slice(&(6 + 8 + 6u16).to_be_bytes());
        sub.extend_from_slice(&0u16.to_be_bytes()); // coverage = vertical, format 0
        sub.extend_from_slice(&1u16.to_be_bytes());
        sub.extend_from_slice(&[0u8; 6]);
        sub.extend_from_slice(&1u16.to_be_bytes());
        sub.extend_from_slice(&2u16.to_be_bytes());
        sub.extend_from_slice(&(99i16).to_be_bytes());
        let mut t = Vec::new();
        t.extend_from_slice(&0u16.to_be_bytes());
        t.extend_from_slice(&1u16.to_be_bytes());
        t.extend_from_slice(&sub);
        let k = KernTable::parse(&t).unwrap();
        assert!(!k.subtable(0).unwrap().is_horizontal());
        assert_eq!(k.kerning(1, 2), 0);
    }

    #[test]
    fn format2_class_array() {
        // Build a format-2 subtable: 3 left classes (0,1,2), 3 right
        // classes (0,1,2). rowWidth = 3 classes * 2 bytes = 6.
        // left class values are pre-multiplied by rowWidth, right by 2.
        let row_width = 6u16;
        // Layout we will assemble (offsets from subtable start):
        //  0..6   : subtable header (version, length, coverage)
        //  6..14  : format-2 header (rowWidth, left, right, array)
        //  then left class table, right class table, array.
        // Compute offsets.
        let header = 6 + 8; // 14
        let left_tab_off = header; // firstGlyph,nGlyphs + 2 entries
        let left_tab_len = 4 + 2 * 2; // glyphs 10,11
        let right_tab_off = left_tab_off + left_tab_len;
        let right_tab_len = 4 + 2 * 2; // glyphs 20,21
        let array_off = right_tab_off + right_tab_len;
        let array_len = 3 * 3 * 2; // 3x3 i16 cells
        let length = array_off + array_len;

        let mut s = vec![0u8; length];
        // subtable header
        s[0..2].copy_from_slice(&0u16.to_be_bytes()); // version
        s[2..4].copy_from_slice(&(length as u16).to_be_bytes());
        s[4..6].copy_from_slice(&0x0201u16.to_be_bytes()); // format 2, horizontal
                                                           // format-2 header
        s[6..8].copy_from_slice(&row_width.to_be_bytes());
        s[8..10].copy_from_slice(&(left_tab_off as u16).to_be_bytes());
        s[10..12].copy_from_slice(&(right_tab_off as u16).to_be_bytes());
        s[12..14].copy_from_slice(&(array_off as u16).to_be_bytes());
        // left class table: firstGlyph=10 nGlyphs=2; glyph10→class1
        // (premultiplied row offset = 1*rowWidth = 6), glyph11→class2 (12)
        s[left_tab_off..left_tab_off + 2].copy_from_slice(&10u16.to_be_bytes());
        s[left_tab_off + 2..left_tab_off + 4].copy_from_slice(&2u16.to_be_bytes());
        s[left_tab_off + 4..left_tab_off + 6].copy_from_slice(&6u16.to_be_bytes()); // row 1
        s[left_tab_off + 6..left_tab_off + 8].copy_from_slice(&12u16.to_be_bytes()); // row 2
                                                                                     // right class table: firstGlyph=20 nGlyphs=2; glyph20→class1
                                                                                     // (premultiplied col offset = 1*2 = 2), glyph21→class2 (4)
        s[right_tab_off..right_tab_off + 2].copy_from_slice(&20u16.to_be_bytes());
        s[right_tab_off + 2..right_tab_off + 4].copy_from_slice(&2u16.to_be_bytes());
        s[right_tab_off + 4..right_tab_off + 6].copy_from_slice(&2u16.to_be_bytes()); // col 1
        s[right_tab_off + 6..right_tab_off + 8].copy_from_slice(&4u16.to_be_bytes()); // col 2
                                                                                      // array: 3 rows * 3 cols, set cell [row1][col1] = -55,
                                                                                      // [row2][col2] = 30.
        let put = |s: &mut [u8], r: usize, c: usize, v: i16| {
            let off = array_off + r * row_width as usize + c * 2;
            s[off..off + 2].copy_from_slice(&v.to_be_bytes());
        };
        put(&mut s, 1, 1, -55);
        put(&mut s, 2, 2, 30);

        let mut t = Vec::new();
        t.extend_from_slice(&0u16.to_be_bytes()); // table version
        t.extend_from_slice(&1u16.to_be_bytes()); // nTables
        t.extend_from_slice(&s);

        let k = KernTable::parse(&t).unwrap();
        assert_eq!(k.subtable(0).unwrap().format(), 2);
        // glyph10 (left class1) + glyph20 (right class1) → cell[1][1]
        assert_eq!(k.kerning(10, 20), -55);
        // glyph11 (left class2) + glyph21 (right class2) → cell[2][2]
        assert_eq!(k.kerning(11, 21), 30);
        // glyph not in a class table → class 0 → all-zero row/col.
        assert_eq!(k.kerning(99, 20), 0);
        assert_eq!(k.kerning(10, 99), 0);
    }

    #[test]
    fn rejects_apple_version() {
        let mut t = vec![0u8; 8];
        t[0..2].copy_from_slice(&1u16.to_be_bytes()); // version 1
        assert!(matches!(KernTable::parse(&t), Err(Error::BadStructure(_))));
    }

    #[test]
    fn rejects_short() {
        assert!(matches!(
            KernTable::parse(&[0u8; 2]),
            Err(Error::UnexpectedEof)
        ));
    }
}
