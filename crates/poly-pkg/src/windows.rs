//! Windows programs and installers, as far as Poly's installer needs to know
//! them (ADR 0230, batch B5 of `docs/windows/chrome-installer-plan.md`).
//!
//! Poly does not unpack a Windows installer itself: the installer program
//! runs unchanged on the NT personality, with its own windows, and does what
//! it does on Windows. This module is what is Poly's around it:
//!
//! - [`program`]: what the installer's window says about a `.exe` before
//!   anything runs (its name, version, maker, size and icon from the PE
//!   version and icon resources), and whether it is an installer or a plain
//!   program; whether a compound file is a Windows Installer package;
//! - [`apps`]: what the launcher gets from the shortcuts (`.lnk`) an
//!   installer leaves in the Start menu and on the desktop: a desktop file
//!   per shortcut, its `Exec` the shortcut's target and arguments as a path
//!   of Poly's file system. The shortcuts are read by `poly-lnk`, the one
//!   reader Poly has, shared with `shell32`'s `ShellLink` (ADR 0227). Also
//!   the app's `Uninstall` key in the registry, which the Applications
//!   folder offers as **Uninstall**.
//!
//! Sources: the PE format specification (learn.microsoft.com/windows/win32/
//! debug/pe-format: the resource and certificate directories), Learn's
//! "VS_VERSIONINFO", "StringFileInfo", "String", "VS_FIXEDFILEINFO",
//! "GRPICONDIR"/"GRPICONDIRENTRY" ("Icons" in the resource file formats),
//! "Parsing C++ command-line arguments", "Uninstall Registry Key", "Known
//! folders"; [MS-SHLLINK] and [MS-CFB]. Nothing of any other program's
//! source was read.

pub mod apps;
pub mod cab;
pub mod cfb;
pub mod lzx;
pub mod msi;
pub mod program;

#[cfg(test)]
mod msi_tests;
#[cfg(test)]
pub(crate) mod testfiles;
#[cfg(test)]
mod tests;
