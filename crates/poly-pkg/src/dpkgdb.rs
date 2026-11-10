//! dpkg's database, `/var/lib/dpkg` (dpkg(1), "FILES"; dpkg-query(1);
//! deb-triggers(5)), written and read as text.
//!
//! **Writing it.** Poly's Debian base root is assembled by the build tool
//! from Debian's packages without running `dpkg` (it has to be made on any
//! host; ADR 0130). For `dpkg` and `apt` to install further packages into
//! that root they must find the packages that are there, as if `dpkg` had
//! installed them:
//!
//! - `status`: one paragraph per package, its `control` file's fields plus
//!   its state and, for a package with configuration files, `Conffiles`
//!   ([`status_paragraph`]). The state is `install ok unpacked`
//!   ([`State::Unpacked`]): the files are in place and no maintainer script
//!   has run, which is exactly what the build tool did. Debian's own
//!   `dpkg --configure -a`, run on Poly, then configures every package
//!   (alternatives, users and groups, caches, triggers) and records them as
//!   installed ([`unconfigured`] counts what is left to do);
//! - `info/<package>.list`: the paths of the package's files
//!   ([`list_text`]), and `info/<package>.<file>` for each of its other
//!   control files (the maintainer scripts, `md5sums`, `conffiles`,
//!   `triggers`, ...). A package that is `Multi-Arch: same` has its files
//!   named `<package>:<architecture>` ([`info_stem`]); `info/format` says
//!   so ([`INFO_FORMAT`]);
//! - `triggers/File` and `triggers/<name>`: which package is interested in
//!   which path or named trigger ([`interests`], [`triggers_file`]), so that
//!   installing an app later runs, for example, the icon cache's update.
//!
//! **Reading it.** The installer asks which version of a package is
//! installed ([`installed_version`]) and which launcher entries a package
//! brought ([`desktop_ids`]).

use crate::control::{self, Paragraph};
use alloc::string::{String, ToString};
use alloc::vec::Vec;

/// The contents of `info/format`: control files of `Multi-Arch: same`
/// packages carry the architecture in their names.
pub const INFO_FORMAT: &str = "1\n";

/// How far a package is (dpkg(1), "Package states").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    /// Its files are unpacked; its `postinst` has not run.
    Unpacked,
    /// Unpacked and configured.
    Installed,
}

impl State {
    /// The `Status` field: what is wanted, the error flag, the state.
    pub fn status(self) -> &'static str {
        match self {
            State::Unpacked => "install ok unpacked",
            State::Installed => "install ok installed",
        }
    }
}

/// The paragraph of dpkg's `status` file for a package in `state`, from its
/// `control` file. `conffiles`: its configuration files (absolute path, MD5
/// of the contents in hex), as its `conffiles` control file lists them.
/// `None` if the control file has no `Package` field.
pub fn status_paragraph(control_text: &str, state: State, conffiles: &[(String, String)]) -> Option<String> {
    let p = control::first(control_text)?;
    p.get("Package")?;
    let mut out = String::new();
    for (name, value) in &p.fields {
        // Never taken from a package: it is the database's own.
        if name.eq_ignore_ascii_case("Status") || name.eq_ignore_ascii_case("Conffiles") {
            continue;
        }
        push_field(&mut out, name, value);
        if name.eq_ignore_ascii_case("Package") {
            out.push_str(&alloc::format!("Status: {}\n", state.status()));
        }
    }
    if !conffiles.is_empty() {
        out.push_str("Conffiles:\n");
        for (path, md5) in conffiles {
            out.push_str(&alloc::format!(" {path} {md5}\n"));
        }
    }
    Some(out)
}

fn push_field(out: &mut String, name: &str, value: &str) {
    let mut lines = value.split('\n');
    out.push_str(name);
    out.push(':');
    match lines.next() {
        Some(first) if !first.is_empty() => {
            out.push(' ');
            out.push_str(first);
        }
        _ => {}
    }
    out.push('\n');
    for line in lines {
        out.push(' ');
        out.push_str(line);
        out.push('\n');
    }
}

/// The name a package's files have in `info/`: `<package>`, or
/// `<package>:<architecture>` for a `Multi-Arch: same` package.
pub fn info_stem(control: &Paragraph) -> Option<String> {
    let name = control.get("Package")?;
    let same = control.get("Multi-Arch").is_some_and(|m| m.trim() == "same");
    match control.get("Architecture").map(str::trim) {
        Some(arch) if same && !arch.is_empty() => Some(alloc::format!("{name}:{arch}")),
        _ => Some(name.to_string()),
    }
}

/// `info/<package>.list` for the paths of a package's `data.tar` (as the
/// archive names them, with or without `./` or `/` in front): `/.`, then
/// each path once, absolute.
pub fn list_text<'a>(paths: impl IntoIterator<Item = &'a str>) -> String {
    let mut out = String::from("/.\n");
    let mut seen: Vec<&str> = Vec::new();
    for path in paths {
        let path = path.trim_start_matches("./").trim_matches('/');
        if path.is_empty() || path == "." || seen.contains(&path) {
            continue;
        }
        seen.push(path);
        out.push('/');
        out.push_str(path);
        out.push('\n');
    }
    out
}

/// The configuration files a `conffiles` control file names (deb-conffiles(5):
/// one absolute path per line; a line with the `remove-on-upgrade` flag
/// names a file that is no longer shipped).
pub fn conffile_paths(conffiles: &str) -> Vec<String> {
    conffiles
        .lines()
        .map(str::trim)
        .filter(|l| l.starts_with('/') && !l.contains(' '))
        .map(ToString::to_string)
        .collect()
}

/// A trigger a package is interested in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Interest {
    /// A path (a file trigger) or a name (an explicit trigger).
    pub trigger: String,
    /// `interest-noawait`: the triggering package does not wait for it.
    pub noawait: bool,
}

/// The interests a `triggers` control file declares (deb-triggers(5):
/// `interest`, `interest-await`, `interest-noawait`; the `activate`
/// directives declare none).
pub fn interests(triggers: &str) -> Vec<Interest> {
    let mut out = Vec::new();
    for line in triggers.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        let mut words = line.split_whitespace();
        let noawait = match words.next() {
            Some("interest" | "interest-await") => false,
            Some("interest-noawait") => true,
            _ => continue,
        };
        if let Some(trigger) = words.next() {
            out.push(Interest { trigger: trigger.to_string(), noawait });
        }
    }
    out
}

/// What goes into dpkg's `triggers` directory for the interests of the
/// packages in `packages` (the package's name in `info/`, its interests):
/// file name and contents. `File` lists the file triggers (`<path>
/// <package>`), every other file is an explicit trigger's name and lists
/// the packages interested in it; `/noawait` follows a package that does
/// not make the triggering one wait.
pub fn triggers_files(packages: &[(String, Vec<Interest>)]) -> Vec<(String, String)> {
    let mut file = String::new();
    let mut named: Vec<(String, String)> = Vec::new();
    for (package, list) in packages {
        for i in list {
            let who = if i.noawait { alloc::format!("{package}/noawait") } else { package.clone() };
            if i.trigger.starts_with('/') {
                file.push_str(&alloc::format!("{} {who}\n", i.trigger));
            } else if !i.trigger.contains('/') && !matches!(i.trigger.as_str(), "File" | "Lock" | "Unincorp") {
                match named.iter_mut().find(|(n, _)| *n == i.trigger) {
                    Some((_, text)) => text.push_str(&alloc::format!("{who}\n")),
                    None => named.push((i.trigger.clone(), alloc::format!("{who}\n"))),
                }
            }
        }
    }
    let mut out = alloc::vec![(String::from("File"), file), (String::from("Unincorp"), String::new())];
    out.append(&mut named);
    out
}

/// The version of package `name` that `status` (dpkg's status file) has
/// installed and configured, if it has.
pub fn installed_version(status: &str, name: &str) -> Option<String> {
    // The file has thousands of paragraphs: find this one's without
    // parsing the others.
    let wanted = alloc::format!("Package: {name}");
    for block in status.split("\n\n") {
        let block = block.trim_start_matches('\n');
        if block.lines().next().map(str::trim_end) != Some(wanted.as_str()) {
            continue;
        }
        let p = control::first(block)?;
        let state = p.get("Status")?.split_whitespace().nth(2)?;
        if state == "installed" {
            return p.get("Version").map(ToString::to_string);
        }
    }
    None
}

/// How many packages of `status` are wanted but not configured yet
/// (unpacked, or left half way by a configuration that failed or was cut
/// off): what `dpkg --configure -a` will work through.
pub fn unconfigured(status: &str) -> usize {
    status
        .lines()
        .filter_map(|l| l.strip_prefix("Status:"))
        .filter(|s| {
            let mut words = s.split_whitespace();
            words.next() == Some("install")
                && matches!(
                    words.nth(1),
                    Some("unpacked" | "half-configured" | "triggers-awaited" | "triggers-pending")
                )
        })
        .count()
}

/// The launcher entries among a package's files (`info/<package>.list`):
/// the desktop file IDs in `/usr/share/applications`.
pub fn desktop_ids(list: &str) -> Vec<String> {
    list.lines()
        .filter_map(|l| l.trim().strip_prefix("/usr/share/applications/"))
        .filter(|id| id.ends_with(".desktop") && !id.contains('/'))
        .map(ToString::to_string)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONTROL: &str = "Package: libfoo1\nSource: foo\nVersion: 1.0-1\nArchitecture: amd64\n\
Multi-Arch: same\nDepends: libc6 (>= 2.34),\n libbar2\nDescription: foo library\n Long text.\n .\n More.\n";

    #[test]
    fn a_status_paragraph_is_the_control_file_plus_the_state() {
        let s = status_paragraph(CONTROL, State::Installed, &[]).unwrap();
        assert_eq!(
            s,
            "Package: libfoo1\nStatus: install ok installed\nSource: foo\nVersion: 1.0-1\nArchitecture: amd64\n\
Multi-Arch: same\nDepends: libc6 (>= 2.34),\n libbar2\nDescription: foo library\n Long text.\n .\n More.\n"
        );
        // What was written reads back as installed.
        assert_eq!(installed_version(&s, "libfoo1").as_deref(), Some("1.0-1"));
        assert_eq!(status_paragraph("Version: 1\n", State::Installed, &[]), None);
        // As the build tool leaves a package: unpacked, to be configured.
        let u = status_paragraph(CONTROL, State::Unpacked, &[]).unwrap();
        assert!(u.starts_with("Package: libfoo1\nStatus: install ok unpacked\n"));
        assert_eq!(installed_version(&u, "libfoo1"), None);
        assert_eq!((unconfigured(&u), unconfigured(&s)), (1, 0));
    }

    #[test]
    fn conffiles_are_listed_with_their_sums() {
        let conf = [("/etc/foo.conf".to_string(), "d41d8cd98f00b204e9800998ecf8427e".to_string())];
        let s = status_paragraph("Package: foo\nStatus: purge ok not-installed\nVersion: 2\n", State::Installed, &conf)
            .unwrap();
        assert_eq!(
            s,
            "Package: foo\nStatus: install ok installed\nVersion: 2\nConffiles:\n /etc/foo.conf d41d8cd98f00b204e9800998ecf8427e\n"
        );
        assert_eq!(
            conffile_paths("/etc/foo.conf\n\nremove-on-upgrade /etc/old.conf\n/etc/bar/baz\n"),
            ["/etc/foo.conf", "/etc/bar/baz"]
        );
    }

    #[test]
    fn multi_arch_same_packages_carry_their_architecture() {
        assert_eq!(info_stem(&control::first(CONTROL).unwrap()).unwrap(), "libfoo1:amd64");
        let plain = control::first("Package: tar\nArchitecture: amd64\nMulti-Arch: foreign\n").unwrap();
        assert_eq!(info_stem(&plain).unwrap(), "tar");
        assert_eq!(info_stem(&control::first("Version: 1\n").unwrap()), None);
    }

    #[test]
    fn the_file_list_is_absolute_and_starts_at_the_root() {
        let list =
            list_text(["./", "./usr", "./usr/bin/", "usr/bin/foo", "/usr/share/applications/foo.desktop", "./usr"]);
        assert_eq!(list, "/.\n/usr\n/usr/bin\n/usr/bin/foo\n/usr/share/applications/foo.desktop\n");
        assert_eq!(desktop_ids(&list), ["foo.desktop"]);
        assert!(desktop_ids("/.\n/usr/share/applications\n/usr/share/applications/kde/x.desktop\n").is_empty());
    }

    #[test]
    fn interests_become_the_triggers_directory() {
        let glib = interests(
            "# comment\ninterest-noawait /usr/lib/x86_64-linux-gnu/gio/modules\ninterest /usr/share/glib-2.0/schemas\nactivate-noawait ldconfig\n",
        );
        assert_eq!(glib.len(), 2);
        let libc = interests("interest-await ldconfig  # essential\n");
        assert_eq!(libc, [Interest { trigger: "ldconfig".into(), noawait: false }]);
        let base = interests("interest-noawait lib32\ninterest-noawait ldconfig\n");
        let files = triggers_files(&[
            ("libglib2.0-0t64:amd64".into(), glib),
            ("libc-bin".into(), libc),
            ("base-files".into(), base),
        ]);
        assert_eq!(
            files,
            [
                (
                    "File".to_string(),
                    "/usr/lib/x86_64-linux-gnu/gio/modules libglib2.0-0t64:amd64/noawait\n\
/usr/share/glib-2.0/schemas libglib2.0-0t64:amd64\n"
                        .to_string()
                ),
                ("Unincorp".to_string(), String::new()),
                ("ldconfig".to_string(), "libc-bin\nbase-files/noawait\n".to_string()),
                ("lib32".to_string(), "base-files/noawait\n".to_string()),
            ]
        );
    }

    #[test]
    fn only_a_configured_package_counts_as_installed() {
        let status = "Package: a\nStatus: install ok installed\nVersion: 1\n\n\
Package: ab\nStatus: install ok unpacked\nVersion: 2\n\n\
Package: gone\nStatus: deinstall ok config-files\nVersion: 3\n";
        assert_eq!(installed_version(status, "a").as_deref(), Some("1"));
        assert_eq!(installed_version(status, "ab"), None);
        assert_eq!(installed_version(status, "gone"), None);
        assert_eq!(installed_version(status, "b"), None);
        // One is unpacked; the removed one is not wanted and not counted.
        assert_eq!(unconfigured(status), 1);
        assert_eq!(unconfigured("Package: x\nStatus: install ok half-configured\n"), 1);
    }
}
