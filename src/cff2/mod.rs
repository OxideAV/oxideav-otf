//! CFF2 — Compact Font Format Version 2 (OpenType 1.9.1 `CFF2` table).
//!
//! CFF2 is the variation-aware successor to CFF1 (Adobe TN5176). It
//! differs from CFF1 in five spec-visible ways relevant to this
//! parser:
//!
//! 1. The on-disk **header** is `(major, minor, headerSize,
//!    topDICTSize: uint16)`, not the CFF1 `(major, minor, hdrSize,
//!    offSize: Card8)` form.
//! 2. There is **no Name INDEX, no String INDEX, no Encoding, and no
//!    Charset** — CFF2 reuses the sfnt-level `name`, `cmap`, and
//!    `post` tables instead of replicating them.
//! 3. The **TopDICT is a single DICT**, not a CFF1-style "INDEX of one
//!    DICT" — it lives directly after the header for `topDICTSize`
//!    bytes.
//! 4. The CFF2 **INDEX format** uses a `uint32` count (CFF1: `Card16`).
//!    An empty CFF2 INDEX is 4 bytes (CFF1: 2 bytes).
//! 5. The **TopDICT operator set is restricted** to five operators
//!    (§7) and the **CharString operator set** adds `blend` (16) and
//!    `vsindex` (15) for variable-font outlines (§9).
//!
//! This module implements points (1)–(4): header, INDEX, and Top DICT
//! parsing, plus the GlobalSubrINDEX + CharStringINDEX + FontDICTINDEX
//! walks they reference, and the ItemVariationStore (§12) for variable
//! fonts (`VariationRegionList` + `ItemVariationData` subtables; see
//! [`varstore`]). The Type 2 charstring decoder (§9) and the per-glyph
//! `blend`/`vsindex` resolution against the variation store are still
//! deferred — calling `Font::glyph_outline` on a CFF2 font surfaces
//! `Error::Cff2NotImplemented` — but the variation-region geometry the
//! blend math will need is now parsed and exposed.
//!
//! Spec: `docs/text/opentype/otspec-cff2.html`.

pub mod header;
pub mod index;
pub mod top_dict;
pub mod varstore;

pub use self::header::Cff2Header;
pub use self::index::Cff2Index;
pub use self::top_dict::{Cff2Op, Cff2TopDict, DEFAULT_FONT_MATRIX};
pub use self::varstore::{
    ItemVariationData, ItemVariationStore, RegionAxisCoordinates, VariationRegion,
};

use crate::Error;

/// Parsed CFF2 table — currently exposes the header, Top DICT, and
/// the structural INDEXes (GlobalSubrINDEX, CharStringINDEX,
/// FontDICTINDEX, plus each Font DICT's raw bytes). The Type 2
/// charstring decoder is not extended to CFF2 in this round;
/// `Cff::glyph_outline` is the path that still rejects with
/// [`Error::Cff2NotImplemented`].
#[derive(Debug, Clone)]
pub struct Cff2<'a> {
    /// Original CFF2 table bytes; every offset is relative to byte 0
    /// of this slice.
    bytes: &'a [u8],
    /// Parsed 5-byte header.
    header: Cff2Header,
    /// Parsed Top DICT.
    top: Cff2TopDict,
    /// GlobalSubrINDEX — sits at `headerSize + topDICTSize` per spec
    /// §6.
    global_subrs: Cff2Index<'a>,
    /// CharStringINDEX — located at `top.charstring_index_offset` from
    /// the start of the table. Its `count` equals the font's glyph
    /// count (spec §8) and must match the sfnt `maxp.numGlyphs` (we
    /// don't enforce that constraint here; the higher-level `Font`
    /// parser does).
    charstrings: Cff2Index<'a>,
    /// FontDICTINDEX — located at `top.font_dict_index_offset`.
    /// At least one Font DICT; multiple are allowed (spec §7.2
    /// "FontDICTINDEX").
    font_dicts: Cff2Index<'a>,
    /// Parsed ItemVariationStore (§12), present iff the Top DICT
    /// carried a `VariationStoreOffset` operator. `None` for
    /// non-variable CFF2 fonts (where `blend`/`vsindex` must not
    /// appear, per §12).
    variation_store: Option<ItemVariationStore>,
}

impl<'a> Cff2<'a> {
    /// Parse a `CFF2` table from raw bytes.
    pub fn parse(bytes: &'a [u8]) -> Result<Self, Error> {
        let header = Cff2Header::parse(bytes)?;

        // Top DICT lives in [header_size .. header_size + top_dict_size).
        let td_start = header.top_dict_offset();
        let td_end = td_start
            .checked_add(header.top_dict_size as usize)
            .ok_or(Error::Cff("CFF2 Top DICT extent overflow"))?;
        if td_end > bytes.len() {
            return Err(Error::UnexpectedEof);
        }
        let top = Cff2TopDict::parse(&bytes[td_start..td_end])?;

        // GlobalSubrINDEX immediately follows the Top DICT.
        let global_subrs = Cff2Index::parse(bytes, header.global_subr_index_offset())?;

        // CharStringINDEX at top.charstring_index_offset.
        let cs_off = top.charstring_index_offset as usize;
        let charstrings = Cff2Index::parse(bytes, cs_off)?;

        // FontDICTINDEX at top.font_dict_index_offset. Required to be
        // non-empty per spec §7.2 ("must contain at least one
        // FontDICT").
        let fd_off = top.font_dict_index_offset as usize;
        let font_dicts = Cff2Index::parse(bytes, fd_off)?;
        if font_dicts.count == 0 {
            return Err(Error::Cff(
                "CFF2 FontDICTINDEX must contain at least one FontDICT",
            ));
        }

        // ItemVariationStore — present iff VariationStoreOffset is set
        // (§12). Offsets are relative to byte 0 of the CFF2 table.
        let variation_store = match top.variation_store_offset {
            Some(off) => Some(ItemVariationStore::parse_variation_store(
                bytes,
                off as usize,
            )?),
            None => None,
        };

        Ok(Self {
            bytes,
            header,
            top,
            global_subrs,
            charstrings,
            font_dicts,
            variation_store,
        })
    }

    /// Borrow the parsed CFF2 header (`major`, `minor`, `headerSize`,
    /// `topDICTSize`).
    pub fn header(&self) -> &Cff2Header {
        &self.header
    }

    /// Borrow the parsed Top DICT (offsets + FontMatrix).
    pub fn top_dict(&self) -> &Cff2TopDict {
        &self.top
    }

    /// Number of glyphs in the CharStringINDEX (per spec §8, this is
    /// the font's glyph count and must match `maxp.numGlyphs`).
    pub fn glyph_count(&self) -> u32 {
        self.charstrings.count
    }

    /// Number of FontDICTs in the FontDICTINDEX. Always >= 1 per spec
    /// §7.2.
    pub fn font_dict_count(&self) -> u32 {
        self.font_dicts.count
    }

    /// Number of subroutines in the GlobalSubrINDEX. An empty
    /// GlobalSubrINDEX is spec-allowed (§6 "An empty INDEX is
    /// represented by a count field with a 0 value").
    pub fn global_subr_count(&self) -> u32 {
        self.global_subrs.count
    }

    /// `true` if the font carries a `VariationStoreOffset` operator in
    /// its Top DICT — i.e. it is a variable font with an embedded
    /// ItemVariationStore.
    pub fn is_variable(&self) -> bool {
        self.top.is_variable()
    }

    /// The parsed ItemVariationStore (§12), or `None` for a
    /// non-variable CFF2 font. Exposes the `VariationRegionList` and
    /// `ItemVariationData` subtables a future `blend`/`vsindex`
    /// charstring pass will consume.
    pub fn variation_store(&self) -> Option<&ItemVariationStore> {
        self.variation_store.as_ref()
    }

    /// Raw bytes for the CharString at glyph index `gid`. Returned
    /// even though the Type 2 + blend interpreter for CFF2 is not
    /// implemented this round, so callers can inspect / count
    /// operators or store them for a later decoder pass.
    pub fn charstring(&self, gid: u32) -> Result<&'a [u8], Error> {
        self.charstrings.entry(gid)
    }

    /// Raw bytes for FontDICT `fd_index`. Useful for callers that
    /// want to introspect the font's per-FD Private DICT pointers
    /// without waiting for the full CFF2 metadata decode.
    pub fn font_dict(&self, fd_index: u32) -> Result<&'a [u8], Error> {
        self.font_dicts.entry(fd_index)
    }

    /// Raw bytes for global subroutine `i`.
    pub fn global_subr(&self, i: u32) -> Result<&'a [u8], Error> {
        self.global_subrs.entry(i)
    }

    /// Borrowed CFF2 table bytes — exposed for diagnostics; offsets in
    /// the Top DICT are relative to byte 0 of this slice.
    pub fn bytes(&self) -> &'a [u8] {
        self.bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal CFF2 table with: 5-byte header, 7-byte Top DICT
    /// (CharStringINDEXOffset + FontDICTINDEXOffset), empty
    /// GlobalSubrINDEX (4 bytes), 1-entry CharStringINDEX, 1-entry
    /// FontDICTINDEX (each entry is 1 byte).
    #[allow(clippy::vec_init_then_push)]
    fn build_minimal_cff2() -> Vec<u8> {
        // Layout we want:
        //   bytes 0..5    header (major=2, minor=0, headerSize=5, topDICTSize=7)
        //   bytes 5..12   Top DICT (7 bytes)
        //   bytes 12..16  GlobalSubrINDEX (count=0, 4 bytes)
        //   bytes 16..?   CharStringINDEX (1 entry, single byte "C")
        //                   uint32 count=1, uint8 offSize=1, offsets=[1,2], data="C" → 8 bytes
        //   bytes 24..?   FontDICTINDEX (1 entry, single byte "F") → 8 bytes
        //
        // So CharStringINDEXOffset = 16, FontDICTINDEXOffset = 24.

        // Top DICT encoding:
        //   CharStringINDEXOffset = 16:
        //     16 + 139 = 155 → b0 = 155 (1-byte operand), then op = 17 (1-byte).
        //   FontDICTINDEXOffset = 24:
        //     24 + 139 = 163 → b0 = 163 (1-byte operand), then op = 12, 36 (2-byte).
        // Total Top DICT bytes: 155, 17, 163, 12, 36 = 5 bytes. But we
        // declared topDICTSize=7, so pad with two zero-operands. Actually
        // we want to test the realistic size — recompute:
        //   Top DICT = [155, 17, 163, 12, 36]  // 5 bytes
        // Set topDICTSize = 5, GlobalSubr at offset 10, CharString at
        // offset 14, FontDICT at offset 22.
        let cs_off = 14u32;
        let fd_off = 22u32;
        let mut v = Vec::new();
        // Header.
        v.push(2); // major
        v.push(0); // minor
        v.push(5); // headerSize
        v.push(0); // topDICTSize hi
        v.push(5); // topDICTSize lo

        // Top DICT.
        // (cs_off - 139) as u8 = 14 + (256-139) = ... DICT integer
        // 32..246 → b0 - 139, so for value `v`: b0 = v + 139.
        v.push((cs_off + 139) as u8); // operand 14
        v.push(17); // CharStringINDEXOffset
        v.push((fd_off + 139) as u8); // operand 22
        v.extend_from_slice(&[12, 36]); // FontDICTINDEXOffset

        // GlobalSubrINDEX (empty, 4 bytes).
        v.extend_from_slice(&[0, 0, 0, 0]);

        assert_eq!(v.len(), cs_off as usize);

        // CharStringINDEX: count=1 (uint32), offSize=1, offsets=[1,2],
        // data="C". 8 bytes total.
        v.extend_from_slice(&[0, 0, 0, 1]); // count
        v.push(1); // offSize
        v.extend_from_slice(&[1, 2]); // offsets
        v.push(b'C'); // data

        assert_eq!(v.len(), fd_off as usize);

        // FontDICTINDEX: same shape, data "F".
        v.extend_from_slice(&[0, 0, 0, 1]);
        v.push(1);
        v.extend_from_slice(&[1, 2]);
        v.push(b'F');

        v
    }

    #[test]
    fn parses_minimal_cff2() {
        let v = build_minimal_cff2();
        let c = Cff2::parse(&v).expect("parse");
        assert_eq!(c.header.major, 2);
        assert_eq!(c.header.header_size, 5);
        assert_eq!(c.header.top_dict_size, 5);
        assert_eq!(c.top.charstring_index_offset, 14);
        assert_eq!(c.top.font_dict_index_offset, 22);
        assert_eq!(c.glyph_count(), 1);
        assert_eq!(c.font_dict_count(), 1);
        assert_eq!(c.global_subr_count(), 0);
        assert!(!c.is_variable());
        assert_eq!(c.charstring(0).unwrap(), b"C");
        assert_eq!(c.font_dict(0).unwrap(), b"F");
    }

    #[test]
    #[allow(clippy::vec_init_then_push)]
    fn rejects_empty_font_dict_index() {
        // Same as build_minimal_cff2 but FontDICTINDEX is empty (count=0).
        let cs_off = 14u32;
        let fd_off = 22u32;
        let mut v = Vec::new();
        v.push(2);
        v.push(0);
        v.push(5);
        v.push(0);
        v.push(5);
        v.push((cs_off + 139) as u8);
        v.push(17);
        v.push((fd_off + 139) as u8);
        v.extend_from_slice(&[12, 36]);
        v.extend_from_slice(&[0, 0, 0, 0]);
        v.extend_from_slice(&[0, 0, 0, 1, 1, 1, 2, b'C']);
        v.extend_from_slice(&[0, 0, 0, 0]); // empty FontDICTINDEX
        let err = Cff2::parse(&v).unwrap_err();
        match err {
            Error::Cff(s) => assert!(s.contains("FontDICTINDEX must contain at least one")),
            _ => panic!("unexpected: {err:?}"),
        }
    }

    #[test]
    #[allow(clippy::vec_init_then_push)]
    fn detects_variable_font() {
        // Build a Top DICT with a VariationStoreOffset operator and
        // re-use the rest of the minimal layout.
        let cs_off = 16u32;
        let fd_off = 24u32;
        let mut v = Vec::new();
        v.push(2);
        v.push(0);
        v.push(5);
        v.push(0);
        v.push(7); // topDICTSize = 7 (3 short-int operands + 4 operator bytes)
                   // Top DICT:
        v.push((cs_off + 139) as u8); // operand cs_off
        v.push(17); // CharStringINDEXOffset
        v.push((fd_off + 139) as u8); // operand fd_off
        v.extend_from_slice(&[12, 36]); // FontDICTINDEXOffset
        v.push(50 + 139); // operand 50
        v.push(24); // VariationStoreOffset
                    // GlobalSubrINDEX (4 bytes empty).
        v.extend_from_slice(&[0, 0, 0, 0]);

        assert_eq!(v.len(), cs_off as usize);

        v.extend_from_slice(&[0, 0, 0, 1, 1, 1, 2, b'C']);
        assert_eq!(v.len(), fd_off as usize);
        v.extend_from_slice(&[0, 0, 0, 1, 1, 1, 2, b'F']);
        // FontDICTINDEX ends at byte 32. Pad to byte 50, where the Top
        // DICT's VariationStoreOffset points, then place the CFF2
        // spec's worked-example VariationStore (2-byte length wrapper +
        // ItemVariationStore) so the IVS parse succeeds.
        v.resize(50, 0);
        v.extend_from_slice(&[0x00, 0x26]); // VariationStore length = 38
        v.extend_from_slice(&[0x00, 0x01]); // format = 1
        v.extend_from_slice(&[0x00, 0x00, 0x00, 0x0C]); // regionListOffset = 12
        v.extend_from_slice(&[0x00, 0x01]); // itemVariationDataCount = 1
        v.extend_from_slice(&[0x00, 0x00, 0x00, 0x1C]); // ivdOffsets[0] = 28
        v.extend_from_slice(&[0x00, 0x01]); // axisCount = 1
        v.extend_from_slice(&[0x00, 0x02]); // regionCount = 2
        v.extend_from_slice(&[0xC0, 0x00, 0xE0, 0x00, 0x00, 0x00]); // region0
        v.extend_from_slice(&[0xC0, 0x00, 0xC0, 0x00, 0xE0, 0x00]); // region1
        v.extend_from_slice(&[0x00, 0x00]); // itemCount = 0
        v.extend_from_slice(&[0x00, 0x00]); // shortDeltaCount = 0
        v.extend_from_slice(&[0x00, 0x02]); // regionIndexCount = 2
        v.extend_from_slice(&[0x00, 0x00, 0x00, 0x01]); // regionIndexes = {0,1}

        let c = Cff2::parse(&v).expect("parse");
        assert!(c.is_variable());
        assert_eq!(c.top.variation_store_offset, Some(50));
        let ivs = c.variation_store().expect("variation store parsed");
        assert_eq!(ivs.axis_count, 1);
        assert_eq!(ivs.regions.len(), 2);
        assert_eq!(ivs.item_variation_data_count(), 1);
        assert_eq!(
            ivs.item_variation_data_at(0).unwrap().region_indexes,
            vec![0, 1]
        );
    }

    #[test]
    fn non_variable_font_has_no_variation_store() {
        let v = build_minimal_cff2();
        let c = Cff2::parse(&v).expect("parse");
        assert!(!c.is_variable());
        assert!(c.variation_store().is_none());
    }

    #[test]
    fn rejects_top_dict_extent_past_eof() {
        // Claim topDICTSize=100 but only 5 bytes remain.
        let v = vec![2, 0, 5, 0, 100, 0, 0, 0, 0, 0];
        let err = Cff2::parse(&v).unwrap_err();
        assert!(matches!(err, Error::UnexpectedEof));
    }

    #[test]
    fn rejects_charstring_offset_past_eof() {
        // Set CharStringINDEXOffset = 200 in a 100-byte buffer.
        let cs_off = 200u32;
        let fd_off = 50u32;
        let mut v = vec![2, 0, 5, 0, 8];
        v.push(29); // 5-byte i32 operand
        v.extend_from_slice(&cs_off.to_be_bytes()); // cs_off
        v.push(17);
        v.push((fd_off + 139) as u8);
        v.extend_from_slice(&[12, 36]);
        // Pad to 100 bytes total.
        v.resize(100, 0);
        let err = Cff2::parse(&v).unwrap_err();
        assert!(matches!(err, Error::UnexpectedEof));
    }
}
