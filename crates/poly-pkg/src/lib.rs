//! Linux packages, as far as Poly's installer needs to know them (ADR 0132:
//! one installer for every package format; ADR 0173: `.deb`).
//!
//! Poly does not unpack or configure packages itself: the distribution's own
//! `dpkg` and `apt` do, running unchanged on the Linux personality in the
//! Debian base root. This crate is what is Poly's around them:
//!
//! - [`format`]: which package format a file is, from its content;
//! - [`ar`], [`tar`], [`control`], [`deb`]: reading a `.deb` (deb(5): an
//!   ar(5) archive of `debian-binary`, `control.tar` and `data.tar`) for
//!   what the installer shows before anything is installed: name, version,
//!   what it is, size, what it depends on;
//! - [`appimage`]: an AppImage (type 2: an ELF runtime with a squashfs image)
//!   read for its desktop file and icon, and where an installed one lives;
//! - [`dpkgdb`]: the database `dpkg` keeps under `/var/lib/dpkg`, which the
//!   tool that builds the base root writes for the packages it unpacked
//!   (their maintainer scripts never ran there), and which the installer
//!   reads to say what is installed and which apps a package brought;
//! - [`apt`]: the commands the installer runs and the progress lines `apt`
//!   writes for a front end;
//! - [`remove`]: which apps can be uninstalled and what removing one takes
//!   away (ADR 0180);
//! - [`installer`]: the installer's model: what its window says and which
//!   command comes next.
//!
//! Everything is written from the formats' public descriptions (the Debian
//! manual pages deb(5), deb-control(5), deb-triggers(5), dpkg(1),
//! apt-get(8), apt.conf(5), the Debian Policy Manual, ar(5), POSIX pax for
//! tar) and from what the tools print when run.

#![no_std]

extern crate alloc;
#[cfg(test)]
extern crate std;

pub mod appimage;
pub mod apt;
pub mod ar;
pub mod control;
pub mod deb;
pub mod dpkgdb;
pub mod format;
pub mod installer;
pub mod remove;
pub mod tar;

/// Why a package file could not be read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// Not the format it was taken for (a bad magic number or header).
    NotThisFormat,
    /// The file ends inside a header or a member.
    Truncated,
    /// A compressed member does not decode.
    Corrupt,
    /// A member is compressed in a way this reader does not know.
    Compression,
    /// The package has no `control` file, or it has no `Package` field.
    NoControl,
    /// A member is larger than a package's control data may be.
    TooLarge,
    /// An AppImage of type 1 (an ISO image), which no one makes any more.
    OldAppImage,
    /// An AppImage without a desktop file that names the app.
    NoDesktopEntry,
}

impl Error {
    /// A sentence for the installer's window.
    pub fn describe(self) -> &'static str {
        match self {
            Error::NotThisFormat => "This file is not a package Poly can read.",
            Error::Truncated => "The file is incomplete: the download may have been cut off.",
            Error::Corrupt => "The package is damaged: its contents cannot be read.",
            Error::Compression => "The package is compressed in a way Poly does not know.",
            Error::NoControl => "The package does not say what it is (no control data).",
            Error::TooLarge => "The package's description is larger than a package's may be.",
            Error::OldAppImage => "This is an AppImage of the old type 1. Look for a newer version of the app.",
            Error::NoDesktopEntry => "This AppImage does not say what app it is (no desktop file).",
        }
    }
}
