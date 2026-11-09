//! The browsing core of Poly's **Files** app and, later (roadmap Phase 8),
//! of **the one file picker every app world uses**.
//!
//! It has no UI and makes no system calls: everything goes through the
//! [`fs::Fs`] trait, which the Files app implements with Poly's native
//! system calls, the picker with whatever world asked, and the tests with a
//! temp directory or an in-memory fake. The Files window (a Wayland client
//! in the Alpine Shell look) draws what this crate decides.
//!
//! - [`fs`]: the file system trait, metadata and errors.
//! - [`path`]: normalizing, joining, extensions, breadcrumbs, percent
//!   encoding.
//! - [`places`]: the sidebar (home, XDG user folders, the root, the trash,
//!   mounted disks from `/proc/mounts`).
//! - [`mime`]: MIME types by name and by content, their descriptions and
//!   icons.
//! - [`apps`]: which app opens a file (desktop entries' `MimeType=`,
//!   `mimeapps.list`), `Exec` field codes, and what a double click does.
//! - [`model`]: the [`model::Browser`]: listing, sorting, filtering, hidden
//!   files, selection by mouse and keys, type-ahead, history, the picker's
//!   filter.
//! - [`ops`]: new folder, rename, copy/cut and paste, permanent delete.
//! - [`trash`]: the freedesktop.org trash (home trash).
//!
//! `no_std` with `alloc`, no `unsafe`, and no panics on any input.

#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;
#[cfg(test)]
extern crate std;

pub mod apps;
pub mod fs;
pub mod mime;
pub mod model;
pub mod ops;
pub mod path;
pub mod places;
pub mod trash;

#[cfg(test)]
mod testfs;

pub use fs::{Fs, FsError, Kind, Meta, RawEntry};
pub use mime::FileIcon;
pub use model::{Browser, Entry};
