//! Outline fonts for Poly (roadmap Phase 6, "Fonts"; ADR 0105).
//!
//! - [`Font`] parses a TrueType or OpenType font (or one font of a `.ttc`
//!   collection) from borrowed bytes: the table directory, `head`, `hhea`,
//!   `maxp`, `hmtx`, `cmap` (formats 0, 4, 6, 12), `name`, `OS/2`, `post`,
//!   quadratic outlines from `glyf`/`loca` (simple and composite glyphs),
//!   cubic outlines from `CFF `, and pair kerning from `GPOS` or `kern`.
//! - [`Rasterizer`] fills outlines with exact anti-aliased coverage by
//!   signed-area accumulation; [`render_glyph`] scales a glyph to a pixel
//!   size at a subpixel offset, in grey or a hinting-free LCD mode.
//! - [`GlyphCache`] keeps rendered glyphs keyed by font, size, glyph,
//!   subpixel offset and mode within a byte budget.
//! - [`layout`] places a string's glyphs: advances, kerning, line breaks
//!   at spaces; [`render_layout`] draws the result into an [`Image`].
//! - [`fc`] finds fonts the way fontconfig does: its pattern syntax
//!   (`DejaVu Sans-12:bold`), its matching order, `fonts.conf`.
//! - [`png`] writes images out, to look at.
//!
//! Everything is written from the public specifications (the OpenType
//! specification, Apple's TrueType Reference Manual, Adobe's CFF and
//! Type 2 charstring notes, the fontconfig user documentation); see
//! CLEAN_ROOM.md. `no_std` with `alloc`: the kernel or a userland
//! compositor can use it. Nothing panics on malformed font data.

#![no_std]

extern crate alloc;
#[cfg(test)]
extern crate std;

mod cache;
mod cff;
pub mod fc;
mod font;
mod glyf;
mod kern;
mod layout;
mod math;
mod path;
pub mod png;
mod raster;
mod reader;
mod render;
#[cfg(test)]
mod testfont;

pub use cache::{CacheStats, GlyphCache, GlyphKey, SUBPIXEL_STEPS, quantize};
pub use font::{Error, Font, GlyphId, HMetrics, OutlineFormat, StyleInfo, VMetrics, font_count, name_id};
pub use layout::{LayoutOptions, Line, PositionedGlyph, TextLayout, layout, render_layout};
pub use path::{Bounds, Path, Point, Segment};
pub use raster::Rasterizer;
pub use render::{Bitmap, Image, MAX_SIZE_PX, RenderMode, render_glyph};
