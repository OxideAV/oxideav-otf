//! Font collections (`ttcf`) whose members carry CFF / CFF2 outlines.
//!
//! No collection file is committed: every collection used here is
//! assembled in memory from the bundled Source Sans 3 Regular
//! (SIL OFL 1.1, `fixtures/SOURCE-SANS-LICENSE`) plus hand-built
//! tables, following "Font Collections" / "TTC Header" of
//! `docs/text/opentype/otspec-otff.html`:
//!
//! * the header is `'ttcf'`, a version, `numFonts` and one
//!   table-directory offset per member;
//! * each member has a complete table directory of its own;
//! * every table offset is measured from the beginning of the file, so
//!   members may point at the same table bytes.
//!
//! The standard fixture has four members:
//!
//! | # | contents                                                        |
//! |---|-----------------------------------------------------------------|
//! | 0 | every Source Sans table, shared                                 |
//! | 1 | the same shared tables but its own `cmap` and `name`            |
//! | 2 | the shared tables minus `CFF ` (a member without CFF outlines)  |
//! | 3 | a synthetic one-glyph `CFF2` font                               |

use oxideav_otf::collection::{self, CollectionHeader};
use oxideav_otf::{CubicSegment, Error, Font, is_collection};

const SOURCE_SANS: &[u8] = include_bytes!("fixtures/SourceSans3-Regular.otf");

const OTTO: u32 = 0x4F54_544F;
const TRUETYPE: u32 = 0x0001_0000;

// ---------------------------------------------------------------------
// Collection builder
// ---------------------------------------------------------------------

/// One member: its sfnt version and `(tag, blob index)` per table.
struct Member {
    sfnt_version: u32,
    tables: Vec<([u8; 4], usize)>,
}

/// Where the pieces of a built collection ended up.
struct Layout {
    bytes: Vec<u8>,
    /// File offset of each member's table directory.
    directories: Vec<usize>,
    /// `(offset, length)` of each blob.
    blobs: Vec<(usize, usize)>,
}

fn table_checksum(data: &[u8]) -> u32 {
    data.chunks(4).fold(0u32, |sum, c| {
        let mut word = [0u8; 4];
        word[..c.len()].copy_from_slice(c);
        sum.wrapping_add(u32::from_be_bytes(word))
    })
}

/// Lay out `header | directories | blobs`, each blob stored once and
/// padded to a four-byte boundary.
fn build_collection(major_version: u16, blobs: &[Vec<u8>], members: &[Member]) -> Layout {
    let mut header_len = 12 + 4 * members.len();
    if major_version >= 2 {
        header_len += 12; // dsigTag / dsigLength / dsigOffset, all null
    }
    let mut cursor = header_len;
    let mut directories = Vec::new();
    for m in members {
        directories.push(cursor);
        cursor += 12 + 16 * m.tables.len();
    }
    let mut placed = Vec::new();
    for b in blobs {
        placed.push((cursor, b.len()));
        cursor += b.len().next_multiple_of(4);
    }

    let mut out = Vec::with_capacity(cursor);
    out.extend_from_slice(b"ttcf");
    out.extend_from_slice(&major_version.to_be_bytes());
    out.extend_from_slice(&0u16.to_be_bytes());
    out.extend_from_slice(&(members.len() as u32).to_be_bytes());
    for d in &directories {
        out.extend_from_slice(&(*d as u32).to_be_bytes());
    }
    out.resize(header_len, 0);
    for m in members {
        let n = m.tables.len() as u16;
        let entry_selector = 15 - n.leading_zeros() as u16;
        let search_range = 16u16 << entry_selector;
        out.extend_from_slice(&m.sfnt_version.to_be_bytes());
        out.extend_from_slice(&n.to_be_bytes());
        out.extend_from_slice(&search_range.to_be_bytes());
        out.extend_from_slice(&entry_selector.to_be_bytes());
        out.extend_from_slice(&(n * 16 - search_range).to_be_bytes());
        let mut records = m.tables.clone();
        records.sort_by_key(|(tag, _)| *tag);
        for (tag, blob) in records {
            let (offset, length) = placed[blob];
            out.extend_from_slice(&tag);
            out.extend_from_slice(&table_checksum(&blobs[blob]).to_be_bytes());
            out.extend_from_slice(&(offset as u32).to_be_bytes());
            out.extend_from_slice(&(length as u32).to_be_bytes());
        }
    }
    for (b, (offset, _)) in blobs.iter().zip(&placed) {
        assert_eq!(out.len(), *offset);
        out.extend_from_slice(b);
        out.resize(out.len().next_multiple_of(4), 0);
    }
    Layout {
        bytes: out,
        directories,
        blobs: placed,
    }
}

/// Split a standalone sfnt file into `(tag, table bytes)`.
fn split_sfnt(font: &[u8]) -> Vec<([u8; 4], Vec<u8>)> {
    let be32 = |o: usize| u32::from_be_bytes(font[o..o + 4].try_into().unwrap()) as usize;
    let count = u16::from_be_bytes([font[4], font[5]]) as usize;
    (0..count)
        .map(|i| {
            let rec = 12 + 16 * i;
            let (offset, length) = (be32(rec + 8), be32(rec + 12));
            (
                font[rec..rec + 4].try_into().unwrap(),
                font[offset..offset + length].to_vec(),
            )
        })
        .collect()
}

/// A `cmap` with one (3, 10) format-12 subtable holding `pairs`
/// (sorted by character), one group per character.
fn build_cmap(pairs: &[(char, u16)]) -> Vec<u8> {
    let mut t = Vec::new();
    t.extend_from_slice(&0u16.to_be_bytes()); // version
    t.extend_from_slice(&1u16.to_be_bytes()); // numTables
    t.extend_from_slice(&3u16.to_be_bytes()); // platformID: Windows
    t.extend_from_slice(&10u16.to_be_bytes()); // encodingID: full Unicode
    t.extend_from_slice(&12u32.to_be_bytes()); // subtable offset
    t.extend_from_slice(&12u16.to_be_bytes()); // format
    t.extend_from_slice(&0u16.to_be_bytes()); // reserved
    t.extend_from_slice(&(16 + 12 * pairs.len() as u32).to_be_bytes()); // length
    t.extend_from_slice(&0u32.to_be_bytes()); // language
    t.extend_from_slice(&(pairs.len() as u32).to_be_bytes()); // numGroups
    for (c, gid) in pairs {
        t.extend_from_slice(&(*c as u32).to_be_bytes());
        t.extend_from_slice(&(*c as u32).to_be_bytes());
        t.extend_from_slice(&(*gid as u32).to_be_bytes());
    }
    t
}

/// A version-0 `name` table of Windows / Unicode BMP / en-US records.
fn build_name(records: &[(u16, &str)]) -> Vec<u8> {
    let mut storage = Vec::new();
    let mut t = Vec::new();
    t.extend_from_slice(&0u16.to_be_bytes());
    t.extend_from_slice(&(records.len() as u16).to_be_bytes());
    t.extend_from_slice(&(6 + 12 * records.len() as u16).to_be_bytes());
    for (name_id, text) in records {
        let encoded: Vec<u8> = text.encode_utf16().flat_map(u16::to_be_bytes).collect();
        t.extend_from_slice(&3u16.to_be_bytes());
        t.extend_from_slice(&1u16.to_be_bytes());
        t.extend_from_slice(&0x0409u16.to_be_bytes());
        t.extend_from_slice(&name_id.to_be_bytes());
        t.extend_from_slice(&(encoded.len() as u16).to_be_bytes());
        t.extend_from_slice(&(storage.len() as u16).to_be_bytes());
        storage.extend_from_slice(&encoded);
    }
    t.extend_from_slice(&storage);
    t
}

/// Tables of a one-glyph `CFF2` font whose glyph 0 is a triangle:
/// `100 100 rmoveto 100 0 0 100 rlineto` (CFF2 charstrings end at the
/// end of their data; the open path is closed implicitly).
fn cff2_tables() -> Vec<([u8; 4], Vec<u8>)> {
    let small = |v: i32| (v + 139) as u8; // one-byte operand, -107..=107
    let charstring = [
        small(100),
        small(100),
        21, // rmoveto
        small(100),
        small(0),
        small(0),
        small(100),
        5, // rlineto
    ];
    let charstrings_at = 14usize;
    let font_dicts_at = charstrings_at + 4 + 1 + 2 + charstring.len();
    let mut cff2 = vec![2, 0, 5, 0, 5]; // major, minor, headerSize, topDictLength
    cff2.extend_from_slice(&[small(charstrings_at as i32), 17]); // CharStrings
    cff2.extend_from_slice(&[small(font_dicts_at as i32), 12, 36]); // FDArray
    cff2.extend_from_slice(&[0, 0, 0, 0]); // empty Global Subr INDEX
    assert_eq!(cff2.len(), charstrings_at);
    cff2.extend_from_slice(&[0, 0, 0, 1, 1, 1, 1 + charstring.len() as u8]);
    cff2.extend_from_slice(&charstring);
    assert_eq!(cff2.len(), font_dicts_at);
    cff2.extend_from_slice(&[0, 0, 0, 1, 1, 1, 1]); // one empty Font DICT

    let mut head = vec![0u8; 54];
    head[0..4].copy_from_slice(&0x0001_0000u32.to_be_bytes());
    head[12..16].copy_from_slice(&0x5F0F_3CF5u32.to_be_bytes());
    head[18..20].copy_from_slice(&2048u16.to_be_bytes()); // unitsPerEm
    let mut hhea = vec![0u8; 36];
    hhea[0..4].copy_from_slice(&0x0001_0000u32.to_be_bytes());
    hhea[4..6].copy_from_slice(&1600i16.to_be_bytes());
    hhea[6..8].copy_from_slice(&(-400i16).to_be_bytes());
    hhea[34..36].copy_from_slice(&1u16.to_be_bytes()); // numberOfHMetrics
    let mut maxp = vec![0u8; 6];
    maxp[0..4].copy_from_slice(&0x0000_5000u32.to_be_bytes());
    maxp[4..6].copy_from_slice(&1u16.to_be_bytes());
    let mut hmtx = vec![0u8; 4];
    hmtx[0..2].copy_from_slice(&777u16.to_be_bytes());

    vec![
        (*b"CFF2", cff2),
        (*b"cmap", build_cmap(&[('A', 0)])),
        (*b"head", head),
        (*b"hhea", hhea),
        (*b"hmtx", hmtx),
        (*b"maxp", maxp),
        (
            *b"name",
            build_name(&[(1, "Collection Variable"), (4, "Collection Variable")]),
        ),
    ]
}

/// Serialise `tables` as a standalone sfnt file.
fn build_standalone(sfnt_version: u32, tables: &[([u8; 4], Vec<u8>)]) -> Vec<u8> {
    let blobs: Vec<Vec<u8>> = tables.iter().map(|(_, b)| b.clone()).collect();
    let member = Member {
        sfnt_version,
        tables: tables
            .iter()
            .enumerate()
            .map(|(i, (t, _))| (*t, i))
            .collect(),
    };
    // Reuse the collection layout with zero header bytes: build a
    // one-member collection, then drop its 16-byte header and rebase.
    let layout = build_collection(1, &blobs, &[member]);
    let shift = layout.directories[0] as u32;
    let mut out = layout.bytes[shift as usize..].to_vec();
    for i in 0..tables.len() {
        let at = 12 + 16 * i + 8;
        let offset = u32::from_be_bytes(out[at..at + 4].try_into().unwrap()) - shift;
        out[at..at + 4].copy_from_slice(&offset.to_be_bytes());
    }
    out
}

const MEMBER_TWO_FAMILY: &str = "Collection Member Two";

/// The standard four-member fixture described in the module docs.
fn standard_collection(major_version: u16) -> Layout {
    let standalone = Font::from_bytes(SOURCE_SANS).unwrap();
    let source = split_sfnt(SOURCE_SANS);
    let mut blobs: Vec<Vec<u8>> = source.iter().map(|(_, b)| b.clone()).collect();
    let shared: Vec<([u8; 4], usize)> = source
        .iter()
        .enumerate()
        .map(|(i, (t, _))| (*t, i))
        .collect();

    // Member 1: upper and lower case swapped, and a name of its own.
    let mut pairs = Vec::new();
    for (from, to) in [('A'..='Z', 'a'), ('a'..='z', 'A')] {
        for (i, c) in from.enumerate() {
            let target = char::from_u32(to as u32 + i as u32).unwrap();
            pairs.push((c, standalone.glyph_index(target).unwrap()));
        }
    }
    let own_cmap = blobs.len();
    blobs.push(build_cmap(&pairs));
    let own_name = blobs.len();
    blobs.push(build_name(&[
        (1, MEMBER_TWO_FAMILY),
        (2, "Regular"),
        (4, MEMBER_TWO_FAMILY),
        (6, "CollectionMemberTwo-Regular"),
    ]));
    let second: Vec<([u8; 4], usize)> = shared
        .iter()
        .map(|&(tag, blob)| match &tag {
            b"cmap" => (tag, own_cmap),
            b"name" => (tag, own_name),
            _ => (tag, blob),
        })
        .collect();

    let without_cff: Vec<([u8; 4], usize)> = shared
        .iter()
        .copied()
        .filter(|(tag, _)| tag != b"CFF ")
        .collect();

    let mut variable = Vec::new();
    for (tag, blob) in cff2_tables() {
        variable.push((tag, blobs.len()));
        blobs.push(blob);
    }

    build_collection(
        major_version,
        &blobs,
        &[
            Member {
                sfnt_version: OTTO,
                tables: shared,
            },
            Member {
                sfnt_version: OTTO,
                tables: second,
            },
            Member {
                sfnt_version: TRUETYPE,
                tables: without_cff,
            },
            Member {
                sfnt_version: OTTO,
                tables: variable,
            },
        ],
    )
}

// ---------------------------------------------------------------------
// Well-formed collections
// ---------------------------------------------------------------------

#[test]
fn fixture_is_small_and_shares_tables() {
    let c = standard_collection(1);
    // Four members cost little more than one font: the tables are
    // stored once.
    assert!(c.bytes.len() < SOURCE_SANS.len() + 8 * 1024);
    assert!(c.bytes.len() < 1024 * 1024);
}

#[test]
fn probes_and_member_count() {
    for major in [1u16, 2] {
        let c = standard_collection(major);
        assert!(is_collection(&c.bytes));
        assert_eq!(collection::font_count(&c.bytes), Ok(4));
        let header = CollectionHeader::parse(&c.bytes).unwrap();
        assert_eq!(header.version(), (major, 0));
        assert_eq!(header.num_fonts(), 4);
        let expected: Vec<u32> = c.directories.iter().map(|d| *d as u32).collect();
        assert_eq!(header.font_offsets(), expected.as_slice());
        // Both header versions open identically.
        let f = Font::from_collection_bytes(&c.bytes, 0).unwrap();
        assert!(f.glyph_count() > 1500);
        assert!(std::ptr::eq(f.bytes(), c.bytes.as_slice()));
    }

    // A single font is not a collection, and the two constructors do
    // not accept each other's input.
    assert!(!is_collection(SOURCE_SANS));
    assert_eq!(collection::font_count(SOURCE_SANS), Err(Error::BadMagic));
    assert_eq!(
        Font::from_collection_bytes(SOURCE_SANS, 0).err(),
        Some(Error::BadMagic)
    );
    let c = standard_collection(1);
    assert_eq!(Font::from_bytes(&c.bytes).err(), Some(Error::BadMagic));
}

#[test]
fn shared_member_matches_standalone_font() {
    let c = standard_collection(1);
    let member = Font::from_collection_bytes(&c.bytes, 0).unwrap();
    let alone = Font::from_bytes(SOURCE_SANS).unwrap();

    assert_eq!(member.family_name(), alone.family_name());
    assert_eq!(member.full_name(), alone.full_name());
    assert_eq!(member.ps_name(), alone.ps_name());
    assert_eq!(member.units_per_em(), alone.units_per_em());
    assert_eq!(member.ascent(), alone.ascent());
    assert_eq!(member.descent(), alone.descent());
    assert_eq!(member.line_gap(), alone.line_gap());
    assert_eq!(member.font_bbox(), alone.font_bbox());
    assert_eq!(member.is_cff2(), alone.is_cff2());
    assert_eq!(member.is_cid(), alone.is_cid());
    assert_eq!(member.glyph_count(), alone.glyph_count());
    assert!(member.glyph_count() > 1500);

    // cmap: the whole BMP plus a slice of the supplementary planes.
    let mut mapped = 0usize;
    for cp in (0u32..0x1_0000).chain(0x1_F000..0x1_F800) {
        let Some(ch) = char::from_u32(cp) else {
            continue;
        };
        let gid = member.glyph_index(ch);
        assert_eq!(gid, alone.glyph_index(ch), "U+{cp:04X}");
        mapped += usize::from(gid.is_some());
    }
    assert!(mapped > 1000, "only {mapped} characters mapped");

    // Outlines and metrics, glyph for glyph.
    let mut with_contours = 0usize;
    for gid in 0..alone.glyph_count() {
        let a = alone.glyph_outline(gid).unwrap();
        let m = member.glyph_outline(gid).unwrap();
        assert_eq!(m, a, "outline of glyph {gid}");
        with_contours += usize::from(!m.is_empty());
        assert_eq!(member.glyph_advance(gid), alone.glyph_advance(gid));
        assert_eq!(member.glyph_lsb(gid), alone.glyph_lsb(gid));
        assert_eq!(member.glyph_name(gid), alone.glyph_name(gid));
    }
    assert!(with_contours > 1500, "only {with_contours} drawn glyphs");

    // Layout tables come through the same directory.
    let text = "Waffle office To AV";
    let opts = oxideav_otf::ShapeOptions::default();
    assert_eq!(member.shape(text, &opts), alone.shape(text, &opts));
}

#[test]
fn member_with_its_own_cmap_and_name() {
    let c = standard_collection(1);
    let member = Font::from_collection_bytes(&c.bytes, 1).unwrap();
    let alone = Font::from_bytes(SOURCE_SANS).unwrap();

    assert_eq!(member.family_name(), Some(MEMBER_TWO_FAMILY));
    assert_ne!(member.family_name(), alone.family_name());

    // Its cmap swaps the two cases and maps nothing else.
    for (upper, lower) in ('A'..='Z').zip('a'..='z') {
        assert_eq!(member.glyph_index(upper), alone.glyph_index(lower));
        assert_eq!(member.glyph_index(lower), alone.glyph_index(upper));
    }
    assert_eq!(member.glyph_index('0'), None);
    assert!(alone.glyph_index('0').is_some());

    // The glyph data is the shared `CFF ` / `hmtx`: same glyph id,
    // same outline and metrics.
    assert_eq!(member.glyph_count(), alone.glyph_count());
    for gid in 0..alone.glyph_count() {
        assert_eq!(
            member.glyph_outline(gid).unwrap(),
            alone.glyph_outline(gid).unwrap(),
            "outline of glyph {gid}"
        );
        assert_eq!(member.glyph_advance(gid), alone.glyph_advance(gid));
    }
    let via_member = member.glyph_outline(member.glyph_index('A').unwrap());
    let via_alone = alone.glyph_outline(alone.glyph_index('a').unwrap());
    assert_eq!(via_member, via_alone);
    assert!(!via_member.unwrap().is_empty());
}

#[test]
fn member_without_cff_outlines_is_a_typed_error() {
    let c = standard_collection(1);
    assert_eq!(
        Font::from_collection_bytes(&c.bytes, 2).err(),
        Some(Error::MissingCff)
    );
}

#[test]
fn member_index_out_of_range_is_a_typed_error() {
    let c = standard_collection(1);
    for index in [4u32, 5, 1000, u32::MAX] {
        assert_eq!(
            Font::from_collection_bytes(&c.bytes, index).err(),
            Some(Error::SubfontOutOfRange(index))
        );
    }
}

#[test]
fn cff2_member_matches_standalone_font() {
    let c = standard_collection(1);
    let member = Font::from_collection_bytes(&c.bytes, 3).unwrap();
    let standalone_bytes = build_standalone(OTTO, &cff2_tables());
    let alone = Font::from_bytes(&standalone_bytes).unwrap();

    assert!(member.is_cff2() && alone.is_cff2());
    assert_eq!(member.family_name(), Some("Collection Variable"));
    assert_eq!(member.units_per_em(), 2048);
    assert_eq!(member.ascent(), 1600);
    assert_eq!(member.descent(), -400);
    assert_eq!(member.glyph_count(), 1);
    assert_eq!(member.glyph_index('A'), Some(0));
    assert_eq!(member.glyph_advance(0), 777);

    let outline = member.glyph_outline(0).unwrap();
    assert_eq!(outline, alone.glyph_outline(0).unwrap());
    assert_eq!(outline.contours.len(), 1);
    let lines = outline.contours[0]
        .segments
        .iter()
        .filter(|s| matches!(s, CubicSegment::LineTo(_)))
        .count();
    assert!(lines >= 2, "triangle has {lines} line segments");
    let b = outline.bounds;
    assert_eq!(
        (b.x_min, b.y_min, b.x_max, b.y_max),
        (100.0, 100.0, 200.0, 200.0)
    );
    assert_eq!(member.glyph_outline_var(0, &[]).unwrap(), outline);
}

// ---------------------------------------------------------------------
// Damaged collections
// ---------------------------------------------------------------------

/// Open every member (and one past the end) and touch the accessors a
/// renderer would. Nothing here may panic, whatever the bytes.
fn exercise(bytes: &[u8]) {
    let _ = is_collection(bytes);
    let count = collection::font_count(bytes).unwrap_or(0).min(8);
    for index in 0..=count {
        let Ok(f) = Font::from_collection_bytes(bytes, index) else {
            continue;
        };
        let _ = (f.family_name(), f.full_name(), f.ps_name(), f.is_cid());
        let _ = (f.units_per_em(), f.ascent(), f.descent(), f.line_gap());
        for ch in ['A', 'g', '0', '\u{20AC}', '\u{4E2D}', '\u{1F600}'] {
            if let Some(gid) = f.glyph_index(ch) {
                let _ = f.glyph_outline(gid);
                let _ = f.glyph_advance(gid);
            }
        }
        let last = f.glyph_count().wrapping_sub(1);
        for gid in [0u16, 1, 2, 37, 300, last, last.wrapping_add(1), u16::MAX] {
            let _ = f.glyph_outline(gid);
            let _ = f.glyph_outline_var(gid, &[0.5, -1.0]);
            let _ = (f.glyph_advance(gid), f.glyph_lsb(gid), f.glyph_name(gid));
            let _ = f.glyph_private_hints(gid);
        }
        let _ = f.kern_pair(1, 2);
        let _ = f.shape("Aa fi", &oxideav_otf::ShapeOptions::default());
    }
}

fn put32(bytes: &mut [u8], at: usize, value: u32) {
    bytes[at..at + 4].copy_from_slice(&value.to_be_bytes());
}

#[test]
fn truncated_header_and_directories() {
    let c = standard_collection(2);
    let structure_end = c.blobs[0].0;
    // Every prefix that ends inside the header or a directory.
    for len in 0..structure_end {
        let cut = &c.bytes[..len];
        exercise(cut);
        for index in 0..4 {
            assert!(
                Font::from_collection_bytes(cut, index).is_err(),
                "prefix {len} opened member {index}"
            );
        }
    }
    assert_eq!(
        Font::from_collection_bytes(&c.bytes[..3], 0).err(),
        Some(Error::UnexpectedEof)
    );
    assert_eq!(
        Font::from_collection_bytes(&c.bytes[..11], 0).err(),
        Some(Error::UnexpectedEof)
    );
    assert_eq!(
        Font::from_collection_bytes(&c.bytes[..20], 0).err(),
        Some(Error::UnexpectedEof)
    );
    // Header intact, member 0's directory cut after its sfnt header.
    assert_eq!(
        Font::from_collection_bytes(&c.bytes[..c.directories[0] + 12], 0).err(),
        Some(Error::BadOffset),
        "the other members' directories now start past the end"
    );
    // Prefixes ending inside the table data: sparse, the tail tables
    // go missing one by one.
    let step = (c.bytes.len() - structure_end) / 97;
    for len in (structure_end..c.bytes.len()).step_by(step) {
        exercise(&c.bytes[..len]);
    }
}

#[test]
fn offsets_outside_the_file() {
    let c = standard_collection(1);
    let len = c.bytes.len() as u32;

    // A member's directory offset at / past / far past the end.
    for bad in [len - 11, len, len + 1, u32::MAX] {
        let mut m = c.bytes.clone();
        put32(&mut m, 12 + 4, bad);
        // The header as a whole is rejected: no member opens.
        assert_eq!(collection::font_count(&m), Err(Error::BadOffset));
        assert_eq!(
            Font::from_collection_bytes(&m, 0).err(),
            Some(Error::BadOffset)
        );
        exercise(&m);
    }

    // A directory offset that is in range but lands in table data.
    let mut m = c.bytes.clone();
    put32(&mut m, 12 + 4, c.blobs[0].0 as u32 + 2);
    assert!(Font::from_collection_bytes(&m, 1).is_err());
    assert!(Font::from_collection_bytes(&m, 0).is_ok());
    exercise(&m);

    // A directory that claims more records than the file can hold.
    let mut m = c.bytes.clone();
    m.truncate(c.directories[3] + 12 + 16);
    put32(&mut m, 12, c.directories[3] as u32);
    assert_eq!(
        Font::from_collection_bytes(&m, 0).err(),
        Some(Error::UnexpectedEof)
    );

    // Each table record of member 1 in turn: offset, then length,
    // pushed past the end of the file. Shared tables stay valid for
    // member 0, whose own directory is untouched.
    let dir = c.directories[1];
    let records = u16::from_be_bytes([c.bytes[dir + 4], c.bytes[dir + 5]]) as usize;
    for i in 0..records {
        let rec = dir + 12 + 16 * i;
        for (field, value) in [(8, len), (8, u32::MAX), (12, len), (12, u32::MAX)] {
            let mut m = c.bytes.clone();
            put32(&mut m, rec + field, value);
            assert_eq!(
                Font::from_collection_bytes(&m, 1).err(),
                Some(Error::BadOffset),
                "record {i} field {field} value {value:#x}"
            );
            assert!(Font::from_collection_bytes(&m, 0).is_ok());
        }
        // Offset == file length with zero length is in range but empty.
        let mut m = c.bytes.clone();
        put32(&mut m, rec + 8, len);
        put32(&mut m, rec + 12, 0);
        exercise(&m);
    }

    // A record pointing back into the header / directories.
    for i in 0..records {
        let mut m = c.bytes.clone();
        put32(&mut m, dir + 12 + 16 * i + 8, 0);
        exercise(&m);
    }
}

#[test]
fn header_field_corruption() {
    let c = standard_collection(1);
    let mut m = c.bytes.clone();
    m[4..6].copy_from_slice(&3u16.to_be_bytes());
    assert_eq!(
        Font::from_collection_bytes(&m, 0).err(),
        Some(Error::BadHeader)
    );
    for count in [0u32, 5, 4097, u32::MAX] {
        let mut m = c.bytes.clone();
        put32(&mut m, 8, count);
        exercise(&m);
        if count == 5 {
            // The fifth "offset" is member 0's sfnt version field
            // ('OTTO' as a number), far outside the file.
            assert_eq!(collection::font_count(&m), Err(Error::BadOffset));
        } else {
            assert_eq!(collection::font_count(&m), Err(Error::BadHeader));
        }
    }
    // A member whose directory starts with an unknown sfnt version.
    let mut m = c.bytes.clone();
    m[c.directories[0]..c.directories[0] + 4].copy_from_slice(b"wOFF");
    assert_eq!(
        Font::from_collection_bytes(&m, 0).err(),
        Some(Error::BadMagic)
    );
    assert!(Font::from_collection_bytes(&m, 1).is_ok());
    // A member with zero tables.
    let mut m = c.bytes.clone();
    m[c.directories[1] + 4..c.directories[1] + 6].copy_from_slice(&[0, 0]);
    assert_eq!(
        Font::from_collection_bytes(&m, 1).err(),
        Some(Error::BadHeader)
    );
}

/// xorshift64: a fixed, reproducible corruption stream.
fn next(state: &mut u64) -> u64 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    *state
}

/// Random corruption of a collection, in three flavours per round:
/// header + directory bytes, one table body, and directory offsets
/// re-aimed at arbitrary places inside the file.
///
/// The default budget keeps a debug run short. Setting
/// `OXIDEAV_OTF_HOSTILE_SECONDS=<n>` keeps going (same stream, further
/// along) until `n` seconds have passed.
#[test]
fn no_panic_on_corrupted_collections() {
    let c = standard_collection(1);
    let structure_end = c.blobs[0].0;
    let deadline = std::env::var("OXIDEAV_OTF_HOSTILE_SECONDS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .map(|s| std::time::Instant::now() + std::time::Duration::from_secs(s));
    let mut state = 0x6f74_6663_6f6c_6c31u64;
    let mut rounds = 0u64;
    loop {
        match deadline {
            Some(d) if std::time::Instant::now() >= d => break,
            None if rounds >= 150 => break,
            _ => {}
        }
        rounds += 1;

        // 1. Header and directories.
        let mut m = c.bytes.clone();
        for _ in 0..1 + next(&mut state) % 4 {
            let at = next(&mut state) as usize % structure_end;
            m[at] = next(&mut state) as u8;
        }
        exercise(&m);

        // 2. One table body (never empty in this fixture).
        let mut m = c.bytes.clone();
        let (offset, length) = c.blobs[next(&mut state) as usize % c.blobs.len()];
        for _ in 0..1 + next(&mut state) % 8 {
            m[offset + next(&mut state) as usize % length] = next(&mut state) as u8;
        }
        exercise(&m);

        // 3. A table record re-aimed somewhere else in the file.
        let mut m = c.bytes.clone();
        let dir = c.directories[next(&mut state) as usize % c.directories.len()];
        let records = u16::from_be_bytes([m[dir + 4], m[dir + 5]]) as u64;
        let rec = dir + 12 + 16 * (next(&mut state) % records) as usize;
        let offset = next(&mut state) % m.len() as u64;
        let length = next(&mut state) % (m.len() as u64 - offset + 1);
        put32(&mut m, rec + 8, offset as u32);
        if next(&mut state).is_multiple_of(2) {
            put32(&mut m, rec + 12, length as u32);
        }
        exercise(&m);
    }
    eprintln!("[collection] {rounds} corruption rounds without a panic");
}

// ---------------------------------------------------------------------
// System collections (present on macOS only; skipped elsewhere)
// ---------------------------------------------------------------------

#[test]
fn system_cff_collections_open() {
    let files = [
        "/System/Library/Fonts/AppleSDGothicNeo.ttc",
        "/System/Library/Fonts/Hiragino Sans GB.ttc",
        "/System/Library/Fonts/Kohinoor.ttc",
    ];
    // Latin, Hangul, Han, Devanagari.
    let samples = [
        'A', 'e', '\u{D55C}', '\u{AE00}', '\u{4E2D}', '\u{6587}', '\u{0915}',
    ];
    for path in files {
        let Ok(bytes) = std::fs::read(path) else {
            continue;
        };
        assert!(is_collection(&bytes), "{path}");
        let count = collection::font_count(&bytes).unwrap();
        assert!(count >= 2, "{path}: {count} members");
        for index in 0..count {
            let f = Font::from_collection_bytes(&bytes, index)
                .unwrap_or_else(|e| panic!("{path} member {index}: {e}"));
            assert!(f.family_name().is_some(), "{path} member {index}");
            assert!(f.glyph_count() > 100, "{path} member {index}");
            let mut drawn = 0;
            for ch in samples {
                let Some(gid) = f.glyph_index(ch) else {
                    continue;
                };
                let outline = f
                    .glyph_outline(gid)
                    .unwrap_or_else(|e| panic!("{path} member {index} {ch:?}: {e}"));
                assert!(!outline.is_empty(), "{path} member {index} {ch:?}");
                assert!(f.glyph_advance(gid) > 0, "{path} member {index} {ch:?}");
                drawn += 1;
            }
            assert!(drawn >= 2, "{path} member {index}: {drawn} samples drawn");
        }
        assert_eq!(
            Font::from_collection_bytes(&bytes, count).err(),
            Some(Error::SubfontOutOfRange(count))
        );
    }
}
