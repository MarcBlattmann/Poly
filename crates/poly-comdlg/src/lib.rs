//! The rules of `comdlg32.dll`'s common dialogs (Learn, "Common Dialog Box
//! Library"; `commdlg.h`, `cderr.h`, `dlgs.h`), apart from the windows:
//!
//! - [`check`]: what is wrong with a structure before a dialog is shown, and
//!   the `CDERR_*` code that says so (the order and the codes are those the
//!   oracle's tests record, `specs/windows/comdlg32.dll`);
//! - [`filter`]: the `lpstrFilter` list and wildcards;
//! - [`path`]: file names: offsets, default extensions, joining and parents,
//!   and the result buffer of single and multiple selections;
//! - [`color`] and [`font`]: the arithmetic of the two dialogs;
//! - [`template`]: in-memory `DLGTEMPLATE`s the dialogs are made from.
//!
//! The DLL (`personalities/nt/win32/comdlg32`) is the windows and the hooks.

#![no_std]

extern crate alloc;

pub mod check;
pub mod color;
pub mod filter;
pub mod font;
pub mod path;
pub mod template;

/// `CDERR_*`, `CFERR_*`, `FNERR_*` (`cderr.h`).
pub mod cderr {
    pub const STRUCTSIZE: u32 = 0x0001;
    pub const INITIALIZATION: u32 = 0x0002;
    pub const NOTEMPLATE: u32 = 0x0003;
    pub const NOHINSTANCE: u32 = 0x0004;
    pub const LOADSTRFAILURE: u32 = 0x0005;
    pub const FINDRESFAILURE: u32 = 0x0006;
    pub const LOADRESFAILURE: u32 = 0x0007;
    pub const LOCKRESFAILURE: u32 = 0x0008;
    pub const MEMALLOCFAILURE: u32 = 0x0009;
    pub const MEMLOCKFAILURE: u32 = 0x000A;
    pub const NOHOOK: u32 = 0x000B;
    pub const REGISTERMSGFAIL: u32 = 0x000C;
    pub const DIALOGFAILURE: u32 = 0xFFFF;
    pub const CFERR_NOFONTS: u32 = 0x2001;
    pub const CFERR_MAXLESSTHANMIN: u32 = 0x2002;
    pub const FNERR_SUBCLASSFAILURE: u32 = 0x3001;
    pub const FNERR_INVALIDFILENAME: u32 = 0x3002;
    pub const FNERR_BUFFERTOOSMALL: u32 = 0x3003;
}
