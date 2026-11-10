//! Running `apt-get` (apt-get(8), apt.conf(5)) for the installer: the
//! command lines and the progress lines `apt` writes for a front end.
//!
//! `apt-get install /path/to/package.deb` installs a package file together
//! with what it depends on from the configured archives, running `dpkg` for
//! each package (their maintainer scripts and triggers included). With
//! `APT::Status-Fd` set to a descriptor number, `apt` writes one line there
//! for each step, fields separated by colons:
//!
//! - `dlstatus:<n>:<percent>:<text>` while it downloads;
//! - `pmstatus:<package>:<percent>:<text>` while `dpkg` unpacks and
//!   configures;
//! - `pmerror:<package>:<percent>:<text>` for a package that failed;
//! - `pmconffile:...` where a changed configuration file needs a decision
//!   (none is asked for: the installer runs `dpkg` non-interactively, which
//!   keeps the file that is there).

use alloc::string::{String, ToString};
use alloc::vec::Vec;

/// The programs.
pub const APT_GET: &str = "/usr/bin/apt-get";
pub const DPKG: &str = "/usr/bin/dpkg";

/// dpkg's status file (dpkg(1), "FILES").
pub const DPKG_STATUS: &str = "/var/lib/dpkg/status";
/// Where dpkg keeps each package's control files and file list.
pub const DPKG_INFO: &str = "/var/lib/dpkg/info";

/// Where `apt` keeps the package lists `apt-get update` downloads.
pub const LISTS_DIR: &str = "/var/lib/apt/lists";

/// The environment `apt-get` and what it starts run in: no questions asked
/// (debconf(7)), a `PATH` with the system administration directories the
/// maintainer scripts expect (Debian Policy 6.1), messages in English as
/// the installer reads them.
pub const ENVIRONMENT: [(&str, &str); 5] = [
    ("DEBIAN_FRONTEND", "noninteractive"),
    ("PATH", "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin"),
    ("LC_ALL", "C.UTF-8"),
    ("HOME", "/root"),
    ("TERM", "dumb"),
];

fn base(status_fd: u32) -> Vec<String> {
    alloc::vec![
        APT_GET.to_string(),
        "-y".to_string(),
        "-q".to_string(),
        "-o".to_string(),
        alloc::format!("APT::Status-Fd={status_fd}"),
        // Another package run (a second installer window) holds dpkg's
        // lock: wait for it instead of failing at once.
        "-o".to_string(),
        "DPkg::Lock::Timeout=120".to_string(),
        // A configuration file the package changed: keep the one in use
        // (dpkg(1), --force-confdef and --force-confold) instead of asking.
        "-o".to_string(),
        "Dpkg::Options::=--force-confdef".to_string(),
        "-o".to_string(),
        "Dpkg::Options::=--force-confold".to_string(),
    ]
}

/// `dpkg --configure -a`: configure every package that is unpacked and not
/// configured (dpkg(1)): on a base root the build tool made, all of them,
/// once. A configuration file a script would ask about keeps its contents.
pub fn configure() -> Vec<String> {
    [DPKG, "--force-confdef", "--force-confold", "--configure", "-a"].map(String::from).to_vec()
}

/// The package a line of dpkg's output starts to configure (`Setting up
/// <package> (<version>) ...`), if it is such a line.
pub fn setting_up(line: &str) -> Option<&str> {
    line.strip_prefix("Setting up ")?.split_whitespace().next()
}

/// `apt-get update`: download the archives' package lists.
pub fn update(status_fd: u32) -> Vec<String> {
    let mut argv = base(status_fd);
    argv.push("update".to_string());
    argv
}

/// `apt-get install <file>`: install the package file at the absolute path
/// `deb` with its dependencies.
pub fn install(deb: &str, status_fd: u32) -> Vec<String> {
    let mut argv = base(status_fd);
    argv.push("install".to_string());
    argv.push("--".to_string());
    argv.push(deb.to_string());
    argv
}

/// Whether a file name in [`LISTS_DIR`] is a downloaded package list.
pub fn is_package_list(name: &str) -> bool {
    name.ends_with("_Packages") || name.contains("_Packages.")
}

/// A progress line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Progress {
    /// Downloading: how far (0 to 100) and what.
    Download { percent: u8, text: String },
    /// Unpacking and configuring: how far (0 to 100) and what.
    Install { percent: u8, text: String },
    /// A package failed.
    Error { package: String, text: String },
}

/// Read a line of the status descriptor; `None` for anything else.
pub fn parse_status(line: &str) -> Option<Progress> {
    let mut parts = line.trim_end().splitn(4, ':');
    let (kind, subject, percent, text) = (parts.next()?, parts.next()?, parts.next()?, parts.next()?);
    let pct = || -> Option<u8> {
        let whole = percent.trim().split('.').next()?;
        Some(whole.parse::<u32>().ok()?.min(100) as u8)
    };
    match kind {
        "dlstatus" => Some(Progress::Download { percent: pct()?, text: text.to_string() }),
        "pmstatus" => Some(Progress::Install { percent: pct()?, text: text.to_string() }),
        "pmerror" => Some(Progress::Error { package: subject.to_string(), text: text.to_string() }),
        _ => None,
    }
}

/// What `dpkg` says went wrong in `lines` of its output: the package it
/// could not process (`dpkg: error processing package <name> (...)`, the
/// first: later ones are packages that depend on it) and the line before,
/// which is what the package's script said last.
pub fn dpkg_error(lines: &[&str]) -> Option<String> {
    let at = lines.iter().position(|l| l.starts_with("dpkg: error processing package "))?;
    let package = lines[at].split_whitespace().nth(4).unwrap_or("a package");
    let said = at.checked_sub(1).map(|i| lines[i].trim()).filter(|l| !l.is_empty() && !l.starts_with("Setting up "));
    Some(match said {
        Some(said) => alloc::format!("{package} could not be set up ({said})"),
        None => alloc::format!("{package} could not be set up"),
    })
}

/// The error among `apt`'s output lines, as it says it (`E: ...`, the
/// last one; apt-get(8) writes its errors so), without the prefix.
pub fn error_line<'a>(lines: impl IntoIterator<Item = &'a str>) -> Option<&'a str> {
    lines.into_iter().filter_map(|l| l.trim().strip_prefix("E: ")).filter(|l| !l.is_empty()).last()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_commands_ask_no_questions() {
        let i = install("/home/poly/Downloads/app.deb", 3);
        assert_eq!(i[0], APT_GET);
        assert!(i.contains(&"-y".to_string()));
        assert!(i.contains(&"APT::Status-Fd=3".to_string()));
        assert_eq!(&i[i.len() - 3..], ["install", "--", "/home/poly/Downloads/app.deb"]);
        assert_eq!(update(7).last().map(String::as_str), Some("update"));
        assert!(update(7).contains(&"APT::Status-Fd=7".to_string()));
        assert!(ENVIRONMENT.contains(&("DEBIAN_FRONTEND", "noninteractive")));
        assert_eq!(configure(), ["/usr/bin/dpkg", "--force-confdef", "--force-confold", "--configure", "-a"]);
        assert_eq!(setting_up("Setting up libc-bin (2.41-12+deb13u4) ..."), Some("libc-bin"));
        assert_eq!(setting_up("Processing triggers for libc-bin (2.41-12+deb13u4) ..."), None);
    }

    #[test]
    fn progress_lines_are_read() {
        assert_eq!(
            parse_status("dlstatus:1:42.8571:Retrieving file 2 of 3\n"),
            Some(Progress::Download { percent: 42, text: "Retrieving file 2 of 3".into() })
        );
        assert_eq!(
            parse_status("pmstatus:galculator:66.6667:Unpacking galculator (amd64)"),
            Some(Progress::Install { percent: 66, text: "Unpacking galculator (amd64)".into() })
        );
        // The text may hold colons.
        assert_eq!(
            parse_status("pmstatus:dpkg-exec:0:Running dpkg: step 1"),
            Some(Progress::Install { percent: 0, text: "Running dpkg: step 1".into() })
        );
        assert_eq!(
            parse_status("pmerror:foo:50:trying to overwrite '/usr/bin/x'"),
            Some(Progress::Error { package: "foo".into(), text: "trying to overwrite '/usr/bin/x'".into() })
        );
        assert_eq!(
            parse_status("pmstatus:foo:150:over"),
            Some(Progress::Install { percent: 100, text: "over".into() })
        );
        assert_eq!(parse_status("Reading package lists..."), None);
        assert_eq!(parse_status("pmstatus:foo:abc:x"), None);
        assert_eq!(parse_status("pmconffile:/etc/x:a:b"), None);
    }

    #[test]
    fn the_last_error_is_what_went_wrong() {
        let log =
            ["Reading package lists...", "E: Unable to locate package x", "W: a warning", "E: Sub-process failed "];
        assert_eq!(error_line(log), Some("Sub-process failed"));
        assert_eq!(error_line(["all fine"]), None);
    }

    #[test]
    fn dpkg_names_the_package_that_failed_first() {
        let log = [
            "Setting up base-files (13.8+deb13u7) ...",
            "rmdir: failed to remove '/var/run': Directory not empty",
            "dpkg: error processing package base-files (--configure):",
            " installed base-files package post-installation script subprocess returned error exit status 1",
            "dpkg: error processing package bash (--configure):",
            " dependency problems - leaving unconfigured",
        ];
        assert_eq!(
            dpkg_error(&log).unwrap(),
            "base-files could not be set up (rmdir: failed to remove '/var/run': Directory not empty)"
        );
        assert_eq!(dpkg_error(&log[2..3]).unwrap(), "base-files could not be set up");
        assert_eq!(
            dpkg_error(&["Setting up x (1) ...", "dpkg: error processing package x (--configure):"]).unwrap(),
            "x could not be set up"
        );
        assert_eq!(dpkg_error(&["Setting up x (1) ..."]), None);
    }

    #[test]
    fn package_lists_are_told_from_the_rest_of_the_directory() {
        assert!(is_package_list("snapshot.debian.org_archive_debian_x_dists_trixie_main_binary-amd64_Packages"));
        assert!(is_package_list("deb.debian.org_debian_dists_trixie_main_binary-amd64_Packages.lz4"));
        assert!(!is_package_list("snapshot.debian.org_archive_debian_x_dists_trixie_InRelease"));
        assert!(!is_package_list("lock"));
        assert!(!is_package_list("partial"));
    }
}
