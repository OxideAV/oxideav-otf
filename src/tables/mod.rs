//! sfnt table parsers (the parts an OTF/CFF font shares with TTF).
//!
//! Each module here decodes one specific table from a `&[u8]` slice
//! borrowed from the parent font; nothing in this directory does its
//! own I/O. The four-byte ASCII table tags (`b"head"`, `b"cmap"`, …)
//! are documented per-module.

// Most submodules below are `#[doc(hidden)]`: they are internal table
// parsers exposed for tests/fuzz, and every stable type they define
// (the typed views the README documents) is re-exported at the crate
// root, which is the supported path. `gdef` / `name` / `os2` stay
// visible because their table structs appear in public `Font` method
// signatures without a crate-root re-export.

// internal — exposed for tests/fuzz; not part of the stable API
#[doc(hidden)]
pub mod avar;
// internal — exposed for tests/fuzz; not part of the stable API
#[doc(hidden)]
pub mod base;
// internal — exposed for tests/fuzz; not part of the stable API
#[doc(hidden)]
pub mod cmap;
// internal — exposed for tests/fuzz; not part of the stable API
#[doc(hidden)]
pub mod cmap_uvs;
// internal — exposed for tests/fuzz; not part of the stable API
#[doc(hidden)]
pub mod colr;
// internal — exposed for tests/fuzz; not part of the stable API
#[doc(hidden)]
pub mod context;
// internal — exposed for tests/fuzz; not part of the stable API
#[doc(hidden)]
pub mod cpal;
// internal — exposed for tests/fuzz; not part of the stable API
#[doc(hidden)]
pub mod device;
// internal — exposed for tests/fuzz; not part of the stable API
#[doc(hidden)]
pub mod ebdt;
// internal — exposed for tests/fuzz; not part of the stable API
#[doc(hidden)]
pub mod eblc;
// internal — exposed for tests/fuzz; not part of the stable API
#[doc(hidden)]
pub mod ebsc;
// internal — exposed for tests/fuzz; not part of the stable API
#[doc(hidden)]
pub mod fvar;
pub mod gdef;
// internal — exposed for tests/fuzz; not part of the stable API
#[doc(hidden)]
pub mod gpos;
// internal — exposed for tests/fuzz; not part of the stable API
#[doc(hidden)]
pub mod gsub;
// internal — exposed for tests/fuzz; not part of the stable API
#[doc(hidden)]
pub mod head;
// internal — exposed for tests/fuzz; not part of the stable API
#[doc(hidden)]
pub mod hhea;
// internal — exposed for tests/fuzz; not part of the stable API
#[doc(hidden)]
pub mod hmtx;
// internal — exposed for tests/fuzz; not part of the stable API
#[doc(hidden)]
pub mod ivs;
// internal — exposed for tests/fuzz; not part of the stable API
#[doc(hidden)]
pub mod kern;
// internal — exposed for tests/fuzz; not part of the stable API
#[doc(hidden)]
pub mod layout;
// internal — exposed for tests/fuzz; not part of the stable API
#[doc(hidden)]
pub mod maxp;
// internal — exposed for tests/fuzz; not part of the stable API
#[doc(hidden)]
pub mod mvar;
pub mod name;
pub mod os2;
// internal — exposed for tests/fuzz; not part of the stable API
#[doc(hidden)]
pub mod post;
// internal — exposed for tests/fuzz; not part of the stable API
#[doc(hidden)]
pub mod sbix;
// internal — exposed for tests/fuzz; not part of the stable API
#[doc(hidden)]
pub mod stat;
// internal — exposed for tests/fuzz; not part of the stable API
#[doc(hidden)]
pub mod svg;
// internal — exposed for tests/fuzz; not part of the stable API
#[doc(hidden)]
pub mod vhea;
// internal — exposed for tests/fuzz; not part of the stable API
#[doc(hidden)]
pub mod vmtx;
// internal — exposed for tests/fuzz; not part of the stable API
#[doc(hidden)]
pub mod vorg;
// internal — exposed for tests/fuzz; not part of the stable API
#[doc(hidden)]
pub mod xvar;
