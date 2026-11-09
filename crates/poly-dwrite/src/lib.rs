//! DirectWrite without COM (ADR 0170): the logic of Poly's `dwrite.dll`,
//! host-tested. The DLL (`personalities/nt/win32/dwrite`) is the COM glue.
//!
//! - [`sfnt`]: font files, the table directory, `Analyze`.
//! - [`names`]: the `name` table, locales and localized strings.
//! - [`face`]: weight, stretch, style, metrics, the character map, glyph
//!   metrics.
//! - [`model`]: families, simulated faces, matching.
//!
//! Written from Microsoft Learn, the public SDK headers (`dwrite*.h`), the
//! OpenType specification and the behaviour recorded in
//! `specs/windows/dwrite.dll` (CLEAN_ROOM.md).

#![no_std]

extern crate alloc;
#[cfg(test)]
extern crate std;

pub mod face;
pub mod model;
pub mod names;
pub mod render;
pub mod sfnt;
pub mod system;
