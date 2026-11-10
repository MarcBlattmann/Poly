//! Which package format a file is, from its content and not its name
//! (ADR 0132, decision 2): a download may be called anything.

use crate::ar;

/// A package format the installer knows by sight.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    /// A Debian package: an ar(5) archive whose first member is
    /// `debian-binary` (deb(5)).
    Deb,
    /// An RPM package: the lead's magic `ed ab ee db`.
    Rpm,
    /// An AppImage: an ELF program with `AI` and the type (1 or 2) at
    /// offset 8.
    AppImage,
    /// A Flatpak reference or repository file: a key file whose first
    /// group is `[Flatpak Ref]` or `[Flatpak Repo]`.
    Flatpak,
    /// A squashfs image (`hsqs`): what a snap is.
    Snap,
}

/// How many bytes from the start of a file [`detect`] wants.
pub const HEAD: usize = 256;

/// The format of the file that starts with `head`, if it is a package.
pub fn detect(head: &[u8]) -> Option<Format> {
    if head.starts_with(ar::MAGIC) {
        let first = ar::members(head).ok()?.into_iter().next()?;
        return (first.name == "debian-binary").then_some(Format::Deb);
    }
    if head.starts_with(&[0xed, 0xab, 0xee, 0xdb]) {
        return Some(Format::Rpm);
    }
    if head.starts_with(b"\x7fELF") && head.get(8..10) == Some(b"AI") && matches!(head.get(10), Some(1 | 2)) {
        return Some(Format::AppImage);
    }
    if head.starts_with(b"hsqs") {
        return Some(Format::Snap);
    }
    let text = core::str::from_utf8(head).unwrap_or("");
    let first_line = text.lines().map(str::trim).find(|l| !l.is_empty() && !l.starts_with('#'));
    if matches!(first_line, Some("[Flatpak Ref]" | "[Flatpak Repo]")) {
        return Some(Format::Flatpak);
    }
    None
}

impl Format {
    /// What the installer calls it.
    pub fn name(self) -> &'static str {
        match self {
            Format::Deb => "Debian package",
            Format::Rpm => "RPM package",
            Format::AppImage => "AppImage",
            Format::Flatpak => "Flatpak app",
            Format::Snap => "Snap package",
        }
    }

    /// Why the installer cannot install it, for the formats it only
    /// recognises; `None` for the ones it installs.
    pub fn not_yet(self) -> Option<&'static str> {
        match self {
            Format::Deb => None,
            Format::Rpm => Some("Poly cannot install RPM packages yet. Look for a Debian package (.deb) of this app."),
            Format::AppImage => {
                Some("Poly cannot install AppImages yet. Look for a Debian package (.deb) of this app.")
            }
            Format::Flatpak => {
                Some("Poly cannot install Flatpak apps yet. Look for a Debian package (.deb) of this app.")
            }
            // ADR 0132: Snap is not supported; say where to look instead.
            Format::Snap => Some(
                "Poly does not install Snap packages. Most apps that come as a snap also offer a Debian package (.deb).",
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_deb_is_an_archive_that_starts_with_debian_binary() {
        let deb = ar::build(&[("debian-binary", b"2.0\n"), ("control.tar", b"")]);
        assert_eq!(detect(&deb), Some(Format::Deb));
        // Its first 72 bytes are enough.
        assert_eq!(detect(&deb[..72]), Some(Format::Deb));
        // Any other archive (a static library) is not a package.
        assert_eq!(detect(&ar::build(&[("lib.o", b"1234")])), None);
    }

    #[test]
    fn the_other_formats_are_known_by_their_magic() {
        assert_eq!(detect(&[0xed, 0xab, 0xee, 0xdb, 3, 0]), Some(Format::Rpm));
        assert_eq!(detect(b"\x7fELF\x02\x01\x01\x00AI\x02\x00\x00"), Some(Format::AppImage));
        assert_eq!(detect(b"\x7fELF\x02\x01\x01\x00\x00\x00\x00\x00\x00"), None, "an ordinary program");
        assert_eq!(detect(b"hsqs\x07\x00\x00\x00"), Some(Format::Snap));
        assert_eq!(detect(b"# from flathub\n[Flatpak Ref]\nName=org.example.App\n"), Some(Format::Flatpak));
        assert_eq!(detect(b"[Flatpak Repo]\nUrl=https://example.org/\n"), Some(Format::Flatpak));
        assert_eq!(detect(b"[Desktop Entry]\n"), None);
        assert_eq!(detect(b""), None);
    }

    #[test]
    fn only_debian_packages_are_installed_today() {
        assert_eq!(Format::Deb.not_yet(), None);
        for f in [Format::Rpm, Format::AppImage, Format::Flatpak, Format::Snap] {
            assert!(f.not_yet().unwrap().contains(".deb"), "{}", f.name());
        }
    }
}
