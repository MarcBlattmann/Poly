//! The sidebar's places.
//!
//! In order: **Home** (always), the XDG user folders that exist (Desktop,
//! Documents, Downloads, Music, Pictures, Videos), the **Computer** root
//! `/`, the **Trash**, then the **mounted disks**.
//!
//! The user folders come from `$XDG_CONFIG_HOME/user-dirs.dirs` (the XDG
//! user directories format: shell-style lines
//! `XDG_DESKTOP_DIR="$HOME/Desktop"`, the value either `$HOME/…` or an
//! absolute path; `"$HOME/"` alone means the folder is disabled). A folder
//! the file does not name defaults to `$HOME/<English name>`.
//!
//! The disks come from `/proc/mounts` (`source mountpoint type options
//! dump pass`, with spaces and other special bytes written as octal
//! escapes such as `\040`): a mount is shown when its source is a block
//! device (`/dev/…`, not a loop device), its type is not a pseudo or image
//! file system, and its mount point is not `/` or a system location
//! (`/boot`, `/efi`, `/usr`, `/var`, `/home`, `/tmp`, `/snap`, …). Each is
//! labelled by its mount point's last component and shown once, at the
//! first mount point listed for it. Mounts under `/media`, `/run/media` and
//! `/mnt` get the removable-drive icon.
//!
//! The trash is not a folder the browser lists directly (its items are
//! listed with their original names by [`crate::trash::list`]), so its
//! place has the path [`TRASH_URI`] and the UI shows the trash view for it.

use alloc::string::{String, ToString};
use alloc::vec::Vec;

use crate::path::{HOME_LABEL, ROOT_LABEL, file_name, normalize};

/// The path of the Trash place.
pub const TRASH_URI: &str = "trash:///";

/// What a place is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PlaceKind {
    Home,
    Desktop,
    Documents,
    Downloads,
    Music,
    Pictures,
    Videos,
    Root,
    /// Every installed app (`poly_shell::applications`, ADR 0175).
    Applications,
    Trash,
    Mount,
}

/// The place's icon; the UI maps it to a Lucide glyph.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PlaceIcon {
    Home,
    Desktop,
    Documents,
    Downloads,
    Music,
    Pictures,
    Videos,
    Computer,
    Applications,
    Trash,
    /// A fixed disk.
    Drive,
    /// A USB stick, card or other disk mounted under `/media`, `/run/media`
    /// or `/mnt`.
    Removable,
}

/// One entry of the sidebar.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Place {
    pub label: String,
    pub path: String,
    pub icon: PlaceIcon,
    pub kind: PlaceKind,
}

/// The user folders: key in `user-dirs.dirs`, default name, label, kind,
/// icon.
const USER_DIRS: [(&str, &str, PlaceKind, PlaceIcon); 6] = [
    ("XDG_DESKTOP_DIR", "Desktop", PlaceKind::Desktop, PlaceIcon::Desktop),
    ("XDG_DOCUMENTS_DIR", "Documents", PlaceKind::Documents, PlaceIcon::Documents),
    ("XDG_DOWNLOAD_DIR", "Downloads", PlaceKind::Downloads, PlaceIcon::Downloads),
    ("XDG_MUSIC_DIR", "Music", PlaceKind::Music, PlaceIcon::Music),
    ("XDG_PICTURES_DIR", "Pictures", PlaceKind::Pictures, PlaceIcon::Pictures),
    ("XDG_VIDEOS_DIR", "Videos", PlaceKind::Videos, PlaceIcon::Videos),
];

/// The sidebar for user `home`. `user_dirs` is the contents of
/// `user-dirs.dirs` if there is one, `mounts` the contents of
/// `/proc/mounts`, `exists` whether a folder exists.
pub fn places(home: &str, user_dirs: Option<&str>, mounts: &str, exists: &dyn Fn(&str) -> bool) -> Vec<Place> {
    let home = normalize(home);
    let mut out = alloc::vec![Place {
        label: HOME_LABEL.into(),
        path: home.clone(),
        icon: PlaceIcon::Home,
        kind: PlaceKind::Home,
    }];
    for (key, default, kind, icon) in USER_DIRS {
        let path = match user_dirs.and_then(|t| user_dir(t, key, &home)) {
            Some(Some(p)) => p,
            Some(None) => continue, // disabled
            None => crate::path::join(&home, default),
        };
        if path != home && exists(&path) {
            let label = if path == crate::path::join(&home, default) {
                default.to_string()
            } else {
                file_name(&path).to_string()
            };
            out.push(Place { label, path, icon, kind });
        }
    }
    out.push(Place { label: ROOT_LABEL.into(), path: "/".into(), icon: PlaceIcon::Computer, kind: PlaceKind::Root });
    out.push(Place { label: "Trash".into(), path: TRASH_URI.into(), icon: PlaceIcon::Trash, kind: PlaceKind::Trash });
    out.extend(disks(mounts));
    out
}

/// `places` with the Applications folder (ADR 0175) before Computer: for
/// Files, not for a file picker (an app is not a file to pick).
pub fn with_applications(mut places: Vec<Place>) -> Vec<Place> {
    let at = places.iter().position(|p| p.kind == PlaceKind::Root).unwrap_or(places.len());
    places.insert(
        at,
        Place {
            label: poly_shell::applications::LABEL.into(),
            path: poly_shell::applications::URI.into(),
            icon: PlaceIcon::Applications,
            kind: PlaceKind::Applications,
        },
    );
    places
}

/// The value of `key` in a `user-dirs.dirs` text: `None` if not named,
/// `Some(None)` if disabled (`"$HOME/"`) or unusable (relative), else the
/// path.
pub fn user_dir(text: &str, key: &str, home: &str) -> Option<Option<String>> {
    let mut found = None;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('#') {
            continue;
        }
        let Some((k, v)) = line.split_once('=') else { continue };
        if k.trim() != key {
            continue;
        }
        let v = unquote(v.trim());
        // The last assignment wins, as in a shell.
        found = Some(if let Some(rest) = v.strip_prefix("$HOME") {
            let rest = rest.trim_start_matches('/');
            if rest.is_empty() { None } else { Some(crate::path::join(home, rest)) }
        } else if v.starts_with('/') {
            Some(normalize(&v))
        } else {
            None
        });
    }
    found
}

/// A shell double-quoted string's contents (`\"`, `\\`, `\$`, `` \` ``).
fn unquote(v: &str) -> String {
    let inner = v.strip_prefix('"').map(|s| s.strip_suffix('"').unwrap_or(s)).unwrap_or(v);
    let mut out = String::with_capacity(inner.len());
    let mut it = inner.chars();
    while let Some(c) = it.next() {
        if c == '\\' {
            if let Some(n) = it.next() {
                out.push(n);
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// File system types that are never a user's disk.
const PSEUDO: [&str; 27] = [
    "proc",
    "sysfs",
    "tmpfs",
    "devtmpfs",
    "devpts",
    "cgroup",
    "cgroup2",
    "securityfs",
    "debugfs",
    "tracefs",
    "pstore",
    "bpf",
    "mqueue",
    "hugetlbfs",
    "configfs",
    "fusectl",
    "autofs",
    "binfmt_misc",
    "efivarfs",
    "ramfs",
    "rpc_pipefs",
    "nsfs",
    "overlay",
    "squashfs",
    "swap",
    "selinuxfs",
    "fuse.portal",
];

/// Mount points (and everything below them) that belong to the system.
const SYSTEM: [&str; 11] = ["/boot", "/efi", "/usr", "/var", "/home", "/tmp", "/snap", "/opt", "/srv", "/proc", "/sys"];

/// The user-visible disks in a `/proc/mounts` text.
pub fn disks(mounts: &str) -> Vec<Place> {
    let mut out: Vec<Place> = Vec::new();
    let mut sources: Vec<String> = Vec::new();
    for line in mounts.lines() {
        let mut f = line.split_ascii_whitespace();
        let (Some(src), Some(dir), Some(fstype)) = (f.next(), f.next(), f.next()) else { continue };
        let (src, dir) = (unescape_octal(src), normalize(&unescape_octal(dir)));
        if !src.starts_with("/dev/") || src.starts_with("/dev/loop") || !dir.starts_with('/') {
            continue;
        }
        if PSEUDO.contains(&fstype) || dir == "/" || SYSTEM.iter().any(|s| crate::path::is_inside(&dir, s)) {
            continue;
        }
        if sources.contains(&src) || out.iter().any(|p| p.path == dir) {
            continue;
        }
        let removable = ["/media", "/run/media", "/mnt"].iter().any(|m| crate::path::is_inside(&dir, m) && dir != *m);
        out.push(Place {
            label: file_name(&dir).to_string(),
            path: dir,
            icon: if removable { PlaceIcon::Removable } else { PlaceIcon::Drive },
            kind: PlaceKind::Mount,
        });
        sources.push(src);
    }
    out
}

/// `/proc/mounts`' escapes: a backslash and three octal digits stand for
/// one byte (`\040` space, `\011` tab, `\012` newline, `\134` backslash).
pub fn unescape_octal(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'\\' {
            let digits = b.get(i + 1..i + 4).filter(|d| d.iter().all(|c| (b'0'..=b'7').contains(c)));
            if let Some(d) = digits {
                let v = d.iter().fold(0u32, |acc, c| acc * 8 + u32::from(c - b'0'));
                if let Ok(byte) = u8::try_from(v) {
                    out.push(byte);
                    i += 4;
                    continue;
                }
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    const MOUNTS: &str = "\
/dev/vda1 / ext4 rw,relatime 0 0
proc /proc proc rw,nosuid 0 0
sysfs /sys sysfs rw 0 0
tmpfs /run tmpfs rw 0 0
devtmpfs /dev devtmpfs rw 0 0
/dev/vda2 /boot/efi vfat rw 0 0
/dev/sdb1 /run/media/u/USB\\040Stick vfat rw 0 0
/dev/sdb1 /mnt/again vfat rw 0 0
/dev/nvme0n1p3 /data ext4 rw 0 0
/dev/loop0 /snap/core/1 squashfs ro 0 0
/dev/sdc1 /home ext4 rw 0 0
overlay /var/lib/docker/x overlay rw 0 0
garbage
";

    #[test]
    fn mounted_disks() {
        let d = disks(MOUNTS);
        let got: Vec<(&str, &str, PlaceIcon)> = d.iter().map(|p| (p.label.as_str(), p.path.as_str(), p.icon)).collect();
        assert_eq!(
            got,
            [("USB Stick", "/run/media/u/USB Stick", PlaceIcon::Removable), ("data", "/data", PlaceIcon::Drive)]
        );
        assert!(d.iter().all(|p| p.kind == PlaceKind::Mount));
        assert_eq!(unescape_octal(r"a\040b\134c\9x\04"), r"a b\c\9x\04");
    }

    #[test]
    fn user_dirs_file() {
        let text = "# written by xdg-user-dirs-update\nXDG_DESKTOP_DIR=\"$HOME/Schreibtisch\"\n\
                    XDG_DOWNLOAD_DIR=\"/srv/dl\"\nXDG_DOCUMENTS_DIR=\"$HOME/\"\nXDG_MUSIC_DIR=\"$HOME/My \\\"Music\\\"\"\n";
        assert_eq!(user_dir(text, "XDG_DESKTOP_DIR", "/home/u"), Some(Some("/home/u/Schreibtisch".into())));
        assert_eq!(user_dir(text, "XDG_DOWNLOAD_DIR", "/home/u"), Some(Some("/srv/dl".into())));
        assert_eq!(user_dir(text, "XDG_DOCUMENTS_DIR", "/home/u"), Some(None));
        assert_eq!(user_dir(text, "XDG_MUSIC_DIR", "/home/u"), Some(Some("/home/u/My \"Music\"".into())));
        assert_eq!(user_dir(text, "XDG_VIDEOS_DIR", "/home/u"), None);

        let existing = ["/home/u/Schreibtisch", "/srv/dl", "/home/u/Documents", "/home/u/Pictures"];
        let exists = |p: &str| existing.contains(&p);
        let all = places("/home/u", Some(text), MOUNTS, &exists);
        let got: Vec<(&str, PlaceKind)> = all.iter().map(|p| (p.label.as_str(), p.kind)).collect();
        assert_eq!(
            got,
            [
                ("Home", PlaceKind::Home),
                ("Schreibtisch", PlaceKind::Desktop),
                ("dl", PlaceKind::Downloads),
                ("Pictures", PlaceKind::Pictures),
                ("Computer", PlaceKind::Root),
                ("Trash", PlaceKind::Trash),
                ("USB Stick", PlaceKind::Mount),
                ("data", PlaceKind::Mount),
            ]
        );
        assert_eq!(all[5].path, TRASH_URI);
    }

    #[test]
    fn defaults_without_a_user_dirs_file() {
        let exists = |p: &str| p == "/home/u/Desktop" || p == "/home/u/Downloads";
        let all = places("/home/u/", None, "", &exists);
        let labels: Vec<&str> = all.iter().map(|p| p.label.as_str()).collect();
        assert_eq!(labels, ["Home", "Desktop", "Downloads", "Computer", "Trash"]);
        assert_eq!(all[0].path, "/home/u");
        assert_eq!(all[1].path, "/home/u/Desktop");
    }

    #[test]
    fn applications_come_before_computer() {
        let exists = |p: &str| p == "/home/u/Desktop";
        let all = with_applications(places("/home/u", None, MOUNTS, &exists));
        let labels: Vec<&str> = all.iter().map(|p| p.label.as_str()).collect();
        assert_eq!(labels, ["Home", "Desktop", "Applications", "Computer", "Trash", "USB Stick", "data"]);
        assert_eq!(all[2].path, "applications:///");
        assert_eq!((all[2].kind, all[2].icon), (PlaceKind::Applications, PlaceIcon::Applications));
        // Without a Computer row it still has a place.
        assert_eq!(with_applications(Vec::new()).len(), 1);
    }
}
