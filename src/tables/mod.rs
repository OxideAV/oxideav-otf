//! sfnt table parsers (the parts an OTF/CFF font shares with TTF).
//!
//! Each module here decodes one specific table from a `&[u8]` slice
//! borrowed from the parent font; nothing in this directory does its
//! own I/O. The four-byte ASCII table tags (`b"head"`, `b"cmap"`, …)
//! are documented per-module.

pub mod cmap;
pub mod head;
pub mod hhea;
pub mod hmtx;
pub mod maxp;
pub mod name;
pub mod os2;
pub mod post;
