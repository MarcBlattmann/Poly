//! The registry views of WOW64 (ADR 0186, stage 2; Learn: "Registry Redirector", "Registry Keys
//! Affected by WOW64", "Accessing an Alternate Registry View").
//!
//! The registry has two logical views of some of its keys: the 64-bit (native) one and the 32-bit
//! one. A 32-bit process sees the 32-bit view and a 64-bit process the native one, unless
//! `KEY_WOW64_64KEY` (0x0100) or `KEY_WOW64_32KEY` (0x0200) is in the desired access of the call
//! (Learn: "Access a 32-bit key from either a 32-bit or 64-bit application"); the flags have no
//! effect on shared keys. The physical location of the 32-bit view of a redirected key is under
//! `Wow6432Node` ("`HKEY_LOCAL_MACHINE\Software` is redirected to
//! `HKEY_LOCAL_MACHINE\Software\Wow6432Node`").
//!
//! [`wow64_view`] is the whole rule, over NT paths, with no state: the services give it the full
//! path (the root handle's path and the relative name, or the absolute name) and open what it
//! returns. A path already under `Wow6432Node` is physical, so a key opened below a redirected
//! handle is not redirected twice; below a redirected handle a shared key is mapped back to its
//! one physical key (the key `Software\Wow6432Node\Microsoft\Windows\CurrentVersion\Policies` of
//! the 32-bit view is `Software\Microsoft\Windows\CurrentVersion\Policies`), so that opening it
//! in one step or relative to a handle gives the same key.

use alloc::vec::Vec;
use poly_registry::consts::{KEY_WOW64_32KEY, KEY_WOW64_64KEY};

pub use poly_registry::consts::WOW6432_NODE;

/// Keys below `HKLM\SOFTWARE` that are **shared** by the two views on Windows 7 and newer,
/// relative to `SOFTWARE` (Learn: "Registry Keys Affected by WOW64", the column "Windows Server
/// 2008 R2, Windows 7, and Newer"; every subkey inherits the behaviour unless it is itself listed
/// in [`CLASSES_REDIRECTED`]). The page lists `Microsoft\Shared Tools\MSInfo` with a doubled
/// `SOFTWARE\Microsoft\` prefix, which is a typo of the page: the key meant is this one.
pub const SHARED: &[&str] = &[
    "Classes",
    "Classes\\Appid",
    "Classes\\HCP",
    "Clients",
    "Microsoft\\COM3",
    "Microsoft\\Cryptography\\Calais\\Current",
    "Microsoft\\Cryptography\\Calais\\Readers",
    "Microsoft\\Cryptography\\Services",
    "Microsoft\\CTF\\SystemShared",
    "Microsoft\\CTF\\TIP",
    "Microsoft\\DFS",
    "Microsoft\\Driver Signing",
    "Microsoft\\EnterpriseCertificates",
    "Microsoft\\EventSystem",
    "Microsoft\\MSMQ",
    "Microsoft\\Non-Driver Signing",
    "Microsoft\\Notepad\\DefaultFonts",
    "Microsoft\\OLE",
    "Microsoft\\RAS",
    "Microsoft\\RPC",
    "Microsoft\\Shared Tools\\MSInfo",
    "Microsoft\\SystemCertificates",
    "Microsoft\\TermServLicensing",
    "Microsoft\\TransactionServer",
    "Microsoft\\Windows\\CurrentVersion\\App Paths",
    "Microsoft\\Windows\\CurrentVersion\\Control Panel\\Cursors\\Schemes",
    "Microsoft\\Windows\\CurrentVersion\\Explorer\\AutoplayHandlers",
    "Microsoft\\Windows\\CurrentVersion\\Explorer\\DriveIcons",
    "Microsoft\\Windows\\CurrentVersion\\Explorer\\KindMap",
    "Microsoft\\Windows\\CurrentVersion\\Group Policy",
    "Microsoft\\Windows\\CurrentVersion\\Policies",
    "Microsoft\\Windows\\CurrentVersion\\PreviewHandlers",
    "Microsoft\\Windows\\CurrentVersion\\Setup",
    "Microsoft\\Windows\\CurrentVersion\\Telephony\\Locations",
    "Microsoft\\Windows NT\\CurrentVersion\\Console",
    "Microsoft\\Windows NT\\CurrentVersion\\FontDpi",
    "Microsoft\\Windows NT\\CurrentVersion\\FontLink",
    "Microsoft\\Windows NT\\CurrentVersion\\FontMapper",
    "Microsoft\\Windows NT\\CurrentVersion\\Fonts",
    "Microsoft\\Windows NT\\CurrentVersion\\FontSubstitutes",
    "Microsoft\\Windows NT\\CurrentVersion\\Gre_Initialize",
    "Microsoft\\Windows NT\\CurrentVersion\\Image File Execution Options",
    "Microsoft\\Windows NT\\CurrentVersion\\Language Pack",
    "Microsoft\\Windows NT\\CurrentVersion\\NetworkCards",
    "Microsoft\\Windows NT\\CurrentVersion\\Perflib",
    "Microsoft\\Windows NT\\CurrentVersion\\Ports",
    "Microsoft\\Windows NT\\CurrentVersion\\Print",
    "Microsoft\\Windows NT\\CurrentVersion\\ProfileList",
    "Microsoft\\Windows NT\\CurrentVersion\\Time Zones",
    "Policies",
    "RegisteredApplications",
];

/// The subkeys of `Classes` (below `HKLM\SOFTWARE` and `HKCU\SOFTWARE`) that are **redirected**
/// although `Classes` is shared (Learn, same table, Windows 7 and newer): their 32-bit view is
/// `Classes\Wow6432Node\<name>`.
pub const CLASSES_REDIRECTED: &[&str] = &["CLSID", "DirectShow", "Interface", "Media Type", "MediaFoundation"];

/// Which of the two logical views a call sees.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum View {
    /// The 64-bit view: the physical keys.
    Native,
    /// The 32-bit view: redirected keys live under `Wow6432Node`.
    X86,
}

/// The view a call sees: the one `access`'s `KEY_WOW64_64KEY` / `KEY_WOW64_32KEY` names, else the
/// process's own (`wow64`: a 32-bit process). Both flags together are an error of the Win32
/// layer (Learn: "ERROR_INVALID_PARAMETER") and mean nothing here.
pub fn view_of(wow64: bool, access: u64) -> View {
    let (n64, n32) = (access as u32 & KEY_WOW64_64KEY != 0, access as u32 & KEY_WOW64_32KEY != 0);
    match (n64, n32) {
        (true, false) => View::Native,
        (false, true) => View::X86,
        _ if wow64 => View::X86,
        _ => View::Native,
    }
}

fn eq(a: &[u16], b: &str) -> bool {
    let mut it = b.encode_utf16();
    a.iter().all(|&c| it.next().is_some_and(|d| fold(c) == fold(d))) && it.next().is_none()
}

fn fold(c: u16) -> u16 {
    if (b'A' as u16..=b'Z' as u16).contains(&c) { c + 32 } else { c }
}

fn ends_with_ignore_case(a: &[u16], suffix: &str) -> bool {
    let s: Vec<u16> = suffix.encode_utf16().collect();
    a.len() >= s.len() && a[a.len() - s.len()..].iter().zip(&s).all(|(&x, &y)| fold(x) == fold(y))
}

/// Whether `rest` (components below `SOFTWARE`) is `shared` or below it.
fn is_below(rest: &[&[u16]], shared: &str) -> bool {
    let parts: Vec<&str> = shared.split('\\').collect();
    rest.len() >= parts.len() && rest.iter().zip(&parts).all(|(c, p)| eq(c, p))
}

/// Whether the key `rest` below `HKLM\SOFTWARE` is shared by both views.
fn is_shared(rest: &[&[u16]]) -> bool {
    if rest.len() >= 2 && eq(rest[0], "Classes") && CLASSES_REDIRECTED.iter().any(|n| eq(rest[1], n)) {
        return false;
    }
    SHARED.iter().any(|s| is_below(rest, s))
}

fn join(parts: &[&[u16]]) -> Vec<u16> {
    let mut out = Vec::new();
    for p in parts {
        out.push(u16::from(b'\\'));
        out.extend_from_slice(p);
    }
    out
}

fn node() -> Vec<u16> {
    WOW6432_NODE.encode_utf16().collect()
}

/// The physical key of the 32-bit view of `rest` (components below `HKLM\SOFTWARE`), or `None`
/// if it is the key itself, unchanged.
fn machine_software(rest: &[&[u16]]) -> Option<Vec<Vec<u16>>> {
    let own = |parts: &[&[u16]]| parts.iter().map(|p| p.to_vec()).collect::<Vec<_>>();
    if rest.first().is_some_and(|c| eq(c, WOW6432_NODE)) {
        // Already physical; a shared key below it is the shared one.
        let below = &rest[1..];
        // (`Wow6432Node\Classes` is Learn's compatibility link to `Classes\Wow6432Node`, not made.)
        if !below.is_empty() && !eq(below[0], "Classes") && is_shared(below) {
            return Some(own(below));
        }
        return None;
    }
    if rest.is_empty() {
        return Some(alloc::vec![node()]);
    }
    if is_shared(rest) {
        // `Classes\<redirected>`: the node goes under Classes.
        return None;
    }
    if rest.len() >= 2 && eq(rest[0], "Classes") {
        // Only a redirected Classes subkey gets here.
        let mut out = own(&rest[..1]);
        out.push(node());
        out.extend(own(&rest[1..]));
        return Some(out);
    }
    let mut out = alloc::vec![node()];
    out.extend(own(rest));
    Some(out)
}

/// The key of the 32-bit view of `rest` below `Software\Classes` of a user: `CLSID` and the other
/// redirected subkeys live under `Classes\Wow6432Node`.
fn user_classes(rest: &[&[u16]]) -> Option<Vec<Vec<u16>>> {
    let first = rest.first()?;
    if CLASSES_REDIRECTED.iter().any(|n| eq(first, n)) {
        let mut out = alloc::vec![node()];
        out.extend(rest.iter().map(|p| p.to_vec()));
        return Some(out);
    }
    None
}

/// The path `path` (an NT path, `\REGISTRY\MACHINE\SOFTWARE\...`, case-insensitive) names in the
/// 32-bit view, for a call whose view is [`view_of`]`(wow64, access)`; `None` when the call
/// opens it as it is (the native view, or a key the views share, or a path outside the redirected
/// parts). Learn: "Registry Keys Affected by WOW64".
///
/// - `HKLM\SOFTWARE` and everything below it moves under `HKLM\SOFTWARE\Wow6432Node`, except the
///   keys of [`SHARED`] (and, below `Classes`, [`CLASSES_REDIRECTED`], which move under
///   `Classes\Wow6432Node`).
/// - `HKCU\SOFTWARE` is shared; its `Classes` subkeys of [`CLASSES_REDIRECTED`] move under
///   `Classes\Wow6432Node` (the user's classes are `\REGISTRY\USER\<sid>\Software\Classes`, or the
///   `\REGISTRY\USER\<sid>_Classes` hive that Windows links there).
/// - Symbolic links Learn lists for programs with hard-coded `Wow6432Node` paths are not made.
pub fn wow64_view(path: &[u16], wow64: bool, access: u64) -> Option<Vec<u16>> {
    if view_of(wow64, access) != View::X86 {
        return None;
    }
    let parts: Vec<&[u16]> = path.split(|&c| c == u16::from(b'\\')).filter(|c| !c.is_empty()).collect();
    if parts.len() < 2 || !eq(parts[0], "REGISTRY") {
        return None;
    }
    let physical: Vec<Vec<u16>> = if eq(parts[1], "MACHINE") && parts.get(2).is_some_and(|c| eq(c, "SOFTWARE")) {
        let below = machine_software(&parts[3..])?;
        let mut out: Vec<Vec<u16>> = parts[..3].iter().map(|p| p.to_vec()).collect();
        out.extend(below);
        out
    } else if eq(parts[1], "USER") && parts.len() >= 3 {
        // `<sid>\Software\Classes\...` or `<sid>_Classes\...`.
        let (at, rest) = if ends_with_ignore_case(parts[2], "_Classes") {
            (3, &parts[3..])
        } else if parts.len() >= 5 && eq(parts[3], "Software") && eq(parts[4], "Classes") {
            (5, &parts[5..])
        } else {
            return None;
        };
        let below = user_classes(rest)?;
        let mut out: Vec<Vec<u16>> = parts[..at].iter().map(|p| p.to_vec()).collect();
        out.extend(below);
        out
    } else {
        return None;
    };
    let refs: Vec<&[u16]> = physical.iter().map(Vec::as_slice).collect();
    Some(join(&refs))
}

#[cfg(test)]
mod tests {
    use super::*;
    extern crate std;
    use std::string::String;

    fn w(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }

    fn view(path: &str, wow64: bool, access: u32) -> Option<String> {
        wow64_view(&w(path), wow64, u64::from(access)).map(|v| String::from_utf16(&v).unwrap())
    }

    const SW: &str = "\\REGISTRY\\MACHINE\\SOFTWARE";

    #[test]
    fn a_32_bit_process_sees_software_under_wow6432node() {
        assert_eq!(view(SW, true, 0).as_deref(), Some("\\REGISTRY\\MACHINE\\SOFTWARE\\Wow6432Node"));
        assert_eq!(
            view("\\REGISTRY\\MACHINE\\SOFTWARE\\Vendor\\App", true, 0).as_deref(),
            Some("\\REGISTRY\\MACHINE\\SOFTWARE\\Wow6432Node\\Vendor\\App")
        );
        // Case does not matter for the match; the caller's spelling is kept below it.
        assert_eq!(
            view("\\registry\\machine\\software\\Vendor", true, 0).as_deref(),
            Some("\\registry\\machine\\software\\Wow6432Node\\Vendor")
        );
    }

    #[test]
    fn a_64_bit_process_sees_the_native_keys() {
        assert_eq!(view(SW, false, 0), None);
        assert_eq!(view("\\REGISTRY\\MACHINE\\SOFTWARE\\Vendor", false, 0), None);
    }

    #[test]
    fn the_flags_choose_the_view_in_either_kind_of_process() {
        // A 32-bit process with KEY_WOW64_64KEY sees the physical key; a 64-bit one with
        // KEY_WOW64_32KEY the 32-bit view.
        assert_eq!(view("\\REGISTRY\\MACHINE\\SOFTWARE\\Vendor", true, KEY_WOW64_64KEY), None);
        assert_eq!(
            view("\\REGISTRY\\MACHINE\\SOFTWARE\\Vendor", false, KEY_WOW64_32KEY).as_deref(),
            Some("\\REGISTRY\\MACHINE\\SOFTWARE\\Wow6432Node\\Vendor")
        );
        // The same flag as the process's own changes nothing; the rights next to it are ignored.
        assert_eq!(view("\\REGISTRY\\MACHINE\\SOFTWARE\\Vendor", false, KEY_WOW64_64KEY | 0xF003F), None);
        assert!(view("\\REGISTRY\\MACHINE\\SOFTWARE\\Vendor", true, KEY_WOW64_32KEY | 0x1).is_some());
        // Both flags: the process's own view.
        assert_eq!(view_of(true, u64::from(KEY_WOW64_32KEY | KEY_WOW64_64KEY)), View::X86);
        assert_eq!(view_of(false, u64::from(KEY_WOW64_32KEY | KEY_WOW64_64KEY)), View::Native);
    }

    #[test]
    fn the_keys_learn_lists_as_shared_are_not_redirected() {
        for key in SHARED {
            let path = std::format!("{SW}\\{key}");
            assert_eq!(view(&path, true, 0), None, "{key}");
            // And their subkeys inherit it.
            let child = std::format!("{path}\\Child");
            assert_eq!(view(&child, true, 0), None, "{key}\\Child");
        }
        assert_eq!(view("\\REGISTRY\\MACHINE\\SOFTWARE\\Classes\\.txt", true, 0), None);
        assert_eq!(view("\\REGISTRY\\MACHINE\\SOFTWARE\\Classes\\Appid\\{x}", true, 0), None);
        assert_eq!(
            view("\\REGISTRY\\MACHINE\\SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Policies\\X", true, 0),
            None
        );
        // A sibling that merely starts like a shared key is redirected.
        assert!(view("\\REGISTRY\\MACHINE\\SOFTWARE\\Microsoft\\OLE2", true, 0).is_some());
        assert!(view("\\REGISTRY\\MACHINE\\SOFTWARE\\Microsoft\\Windows\\CurrentVersion", true, 0).is_some());
        assert!(view("\\REGISTRY\\MACHINE\\SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion", true, 0).is_some());
    }

    #[test]
    fn five_classes_subkeys_are_redirected_below_classes() {
        for name in CLASSES_REDIRECTED {
            let path = std::format!("{SW}\\Classes\\{name}\\{{1234}}");
            let want = std::format!("{SW}\\Classes\\Wow6432Node\\{name}\\{{1234}}");
            assert_eq!(view(&path, true, 0).as_deref(), Some(want.as_str()), "{name}");
            assert_eq!(view(&path, false, 0), None, "{name}");
            assert_eq!(view(&path, false, KEY_WOW64_32KEY).as_deref(), Some(want.as_str()), "{name}");
            assert_eq!(view(&path, true, KEY_WOW64_64KEY), None, "{name}");
        }
        // `Classes` itself and its other subkeys are shared.
        assert_eq!(view("\\REGISTRY\\MACHINE\\SOFTWARE\\Classes", true, 0), None);
        assert_eq!(
            view("\\REGISTRY\\MACHINE\\SOFTWARE\\Classes\\clsid", true, 0).as_deref(),
            Some("\\REGISTRY\\MACHINE\\SOFTWARE\\Classes\\Wow6432Node\\clsid")
        );
    }

    #[test]
    fn a_path_already_in_the_32_bit_view_is_not_moved_again() {
        let physical = "\\REGISTRY\\MACHINE\\SOFTWARE\\Wow6432Node\\Vendor";
        assert_eq!(view(physical, true, 0), None);
        assert_eq!(view(physical, false, KEY_WOW64_32KEY), None);
        assert_eq!(view("\\REGISTRY\\MACHINE\\SOFTWARE\\Wow6432Node", true, 0), None);
        // Below a handle of the 32-bit view a shared key is the shared one (a relative open of
        // `Microsoft\Windows\CurrentVersion\Policies` from `HKLM\Software`).
        assert_eq!(
            view(
                "\\REGISTRY\\MACHINE\\SOFTWARE\\Wow6432Node\\Microsoft\\Windows\\CurrentVersion\\Policies\\X",
                true,
                0
            )
            .as_deref(),
            Some("\\REGISTRY\\MACHINE\\SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Policies\\X")
        );
        // Redirected Classes subkeys stay where they are below the node.
        assert_eq!(view("\\REGISTRY\\MACHINE\\SOFTWARE\\Classes\\Wow6432Node\\CLSID", true, 0), None);
    }

    #[test]
    fn other_hives_and_other_paths_are_untouched() {
        for p in [
            "\\REGISTRY\\MACHINE\\SYSTEM\\CurrentControlSet",
            "\\REGISTRY\\MACHINE",
            "\\REGISTRY",
            "\\REGISTRY\\USER\\S-1-5-21-1\\Software\\Vendor",
            "\\REGISTRY\\USER\\S-1-5-21-1\\Software\\Classes\\.txt",
            "\\REGISTRY\\USER\\S-1-5-21-1\\Software\\Classes",
            "\\REGISTRY\\USER\\S-1-5-21-1\\Environment",
            "\\Device\\HarddiskVolume1",
            "",
        ] {
            assert_eq!(view(p, true, 0), None, "{p}");
        }
    }

    #[test]
    fn the_users_classes_redirect_the_same_five_subkeys() {
        let user = "\\REGISTRY\\USER\\S-1-5-21-1";
        for name in CLASSES_REDIRECTED {
            let path = std::format!("{user}\\Software\\Classes\\{name}\\X");
            let want = std::format!("{user}\\Software\\Classes\\Wow6432Node\\{name}\\X");
            assert_eq!(view(&path, true, 0).as_deref(), Some(want.as_str()), "{name}");
            // The linked hive of the user's classes.
            let hive = std::format!("{user}_Classes\\{name}");
            let want = std::format!("{user}_Classes\\Wow6432Node\\{name}");
            assert_eq!(view(&hive, true, 0).as_deref(), Some(want.as_str()), "{name}");
        }
    }

    #[test]
    fn doubled_and_trailing_backslashes_are_ignored() {
        assert_eq!(
            view("\\REGISTRY\\MACHINE\\SOFTWARE\\\\Vendor\\", true, 0).as_deref(),
            Some("\\REGISTRY\\MACHINE\\SOFTWARE\\Wow6432Node\\Vendor")
        );
    }
}
