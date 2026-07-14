//! sfnt table parsers (the parts an OTF/CFF font shares with TTF).
//!
//! Each module here decodes one specific table from a `&[u8]` slice
//! borrowed from the parent font; nothing in this directory does its
//! own I/O. The four-byte ASCII table tags (`b"head"`, `b"cmap"`, …)
//! are documented per-module.

pub mod avar;
pub mod base;
pub mod cmap;
pub mod cmap_uvs;
pub mod colr;
pub mod context;
pub mod cpal;
pub mod device;
pub mod fvar;
pub mod gdef;
pub mod gpos;
pub mod gsub;
pub mod head;
pub mod hhea;
pub mod hmtx;
pub mod ivs;
pub mod kern;
pub mod layout;
pub mod maxp;
pub mod mvar;
pub mod name;
pub mod os2;
pub mod post;
pub mod stat;
pub mod vhea;
pub mod vmtx;
pub mod vorg;
pub mod xvar;
