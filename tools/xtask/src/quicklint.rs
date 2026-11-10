//! `cargo xtask ci lint --quick`: what to lint when only the files changed
//! against `origin/main` are looked at (ADR 0174).
//!
//! The full lint formats and runs clippy over six Cargo workspaces. The
//! quick one does so for the host packages and the other workspaces that
//! have a changed file (committed since the merge base with `origin/main`,
//! modified or untracked), plus the source checks. It does not look at
//! the crates that depend on a changed crate: the full lint, which CI
//! runs, is the gate.

use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;

type Result<T = ()> = std::result::Result<T, String>;

/// What a lint run covers.
#[derive(Debug, Default, PartialEq)]
pub struct Plan {
    /// The whole host workspace (`packages` is not looked at).
    pub all_host: bool,
    /// Host workspace packages, by name.
    pub packages: Vec<String>,
    /// xtask for Windows (clippy with `--target x86_64-pc-windows-msvc`).
    pub xtask_windows: bool,
    pub kernel: bool,
    pub native: bool,
    pub aarch64: bool,
    pub ntdll: bool,
    pub win32: bool,
    pub wintrace: bool,
}

impl Plan {
    /// The full lint.
    pub fn everything() -> Plan {
        Plan {
            all_host: true,
            packages: Vec::new(),
            xtask_windows: true,
            kernel: true,
            native: true,
            aarch64: true,
            ntdll: true,
            win32: true,
            wintrace: true,
        }
    }

    /// The units in words, for the line the quick lint prints.
    pub fn describe(&self) -> String {
        let mut units: Vec<String> = Vec::new();
        if self.all_host {
            units.push("the host workspace".into());
        }
        units.extend(self.packages.iter().cloned());
        for (on, name) in [
            (self.xtask_windows, "xtask for Windows"),
            (self.kernel, "the kernel"),
            (self.native, "the native programs"),
            (self.aarch64, "the aarch64 stub"),
            (self.ntdll, "ntdll (fmt)"),
            (self.win32, "win32 (fmt)"),
            (self.wintrace, "wintrace (fmt)"),
        ] {
            if on {
                units.push(name.into());
            }
        }
        if units.is_empty() { "no crate (only the source checks)".into() } else { units.join(", ") }
    }
}

/// Files that configure every build: when one changed, only the full lint will do.
const WORKSPACE_WIDE: [&str; 6] =
    ["Cargo.toml", "Cargo.lock", "rust-toolchain.toml", "rustfmt.toml", "clippy.toml", ".cargo/config.toml"];

/// The plan for the `changed` files (paths from the repository's root, `/`
/// between their parts), given the host workspace's `members`, `(directory,
/// package name)`; or why the full lint has to run.
pub fn plan(changed: &[String], members: &[(String, String)]) -> std::result::Result<Plan, String> {
    let mut plan = Plan::default();
    let under = |file: &str, dir: &str| file.strip_prefix(dir).is_some_and(|rest| rest.starts_with('/'));
    for file in changed.iter().map(String::as_str) {
        if WORKSPACE_WIDE.contains(&file) {
            return Err(format!("{file} changed"));
        }
        // The workspaces of their own first: they lie inside member directories.
        if under(file, "kernel/src/arch/aarch64") {
            plan.aarch64 = true;
        } else if under(file, "kernel") {
            plan.kernel = true;
        } else if under(file, "userland/native") {
            plan.native = true;
        } else if under(file, "personalities/nt/ntdll") {
            plan.ntdll = true;
        } else if under(file, "personalities/nt/win32") {
            plan.win32 = true;
        } else if under(file, "tools/wintrace") {
            plan.wintrace = true;
        } else if let Some((_, package)) =
            members.iter().filter(|(dir, _)| under(file, dir)).max_by_key(|(dir, _)| dir.len())
        {
            if !plan.packages.contains(package) {
                plan.packages.push(package.clone());
            }
            plan.xtask_windows |= package == "xtask";
        }
    }
    plan.packages.sort();
    Ok(plan)
}

/// The member directories of the workspace in `cargo_toml` (the strings of
/// its `members = [...]`).
fn member_dirs(cargo_toml: &str) -> Vec<String> {
    let Some(at) = cargo_toml.find("members") else { return Vec::new() };
    let rest = &cargo_toml[at..];
    let (Some(open), Some(close)) = (rest.find('['), rest.find(']')) else { return Vec::new() };
    if close < open {
        return Vec::new();
    }
    rest[open + 1..close]
        .lines()
        .map(|l| l.split('#').next().unwrap_or(""))
        .flat_map(|l| l.split(','))
        .map(|s| s.trim().trim_matches('"').to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

/// The package name in `cargo_toml`: the first `name = "..."` after `[package]`.
fn package_name(cargo_toml: &str) -> Option<String> {
    let at = cargo_toml.find("[package]")?;
    cargo_toml[at..].lines().skip(1).take_while(|l| !l.trim_start().starts_with('[')).find_map(|l| {
        let (key, value) = l.split_once('=')?;
        (key.trim() == "name").then(|| value.trim().trim_matches('"').to_string())
    })
}

/// The host workspace's members: `(directory, package name)`.
pub fn members(root: &Path) -> Result<Vec<(String, String)>> {
    let read = |p: std::path::PathBuf| std::fs::read_to_string(&p).map_err(|e| format!("{}: {e}", p.display()));
    let dirs = member_dirs(&read(root.join("Cargo.toml"))?);
    if dirs.is_empty() || dirs.iter().any(|d| d.contains('*')) {
        return Err("Cargo.toml: the members could not be read as a plain list".into());
    }
    dirs.into_iter()
        .map(|dir| {
            let name = package_name(&read(root.join(&dir).join("Cargo.toml"))?)
                .ok_or_else(|| format!("{dir}/Cargo.toml: no package name"))?;
            Ok((dir, name))
        })
        .collect()
}

fn git(root: &Path, args: &[&str]) -> Result<Vec<u8>> {
    let out = Command::new("git").current_dir(root).args(args).output().map_err(|e| format!("git {}: {e}", args[0]))?;
    if out.status.success() {
        Ok(out.stdout)
    } else {
        Err(format!("git {} failed: {}", args.join(" "), String::from_utf8_lossy(&out.stderr).trim()))
    }
}

/// The NUL-separated records of git's `-z` output.
fn records(bytes: &[u8]) -> impl Iterator<Item = &[u8]> {
    bytes.split(|b| *b == 0).filter(|r| !r.is_empty())
}

/// `(blob hash, path)` of a record `<mode> <a> <b>\t<path>`, the hash
/// being word `hash_word` before the tab: `git ls-tree -r` has `<mode>
/// blob <hash>`, `git ls-files -s` has `<mode> <hash> <stage>`.
fn blob_and_path(record: &[u8], hash_word: usize) -> Option<(&[u8], &[u8])> {
    let tab = record.iter().position(|b| *b == b'\t')?;
    let hash = record[..tab].split(|b| *b == b' ').nth(hash_word)?;
    Some((hash, &record[tab + 1..]))
}

/// The paths whose blob differs between `manifest` (`git ls-files -s -z`
/// of the work tree as it is, which polyd makes, `POLYD_WANT`) and `tree`
/// (`git ls-tree -r -z` of the merge base), or that only one of them has.
fn changed_by_manifest(manifest: &[u8], tree: &[u8]) -> Vec<String> {
    let mut base: BTreeMap<&[u8], &[u8]> =
        records(tree).filter_map(|r| blob_and_path(r, 2)).map(|(hash, path)| (path, hash)).collect();
    let mut changed: Vec<String> = Vec::new();
    for (hash, path) in records(manifest).filter_map(|r| blob_and_path(r, 1)) {
        if base.remove(path) != Some(hash) {
            changed.push(String::from_utf8_lossy(path).into_owned());
        }
    }
    changed.extend(base.keys().map(|p| String::from_utf8_lossy(p).into_owned()));
    changed.sort();
    changed
}

/// The files that differ from the merge base with `origin/main`:
/// committed since, modified, or untracked (and not ignored).
///
/// Under polyd the work tree's state comes from its manifest: git's own
/// comparison reads every file whose time the index does not know, which
/// over the Windows bind mount is every file, at 25 ms each.
pub fn changed(root: &Path) -> Result<Vec<String>> {
    let base = git(root, &["merge-base", "origin/main", "HEAD"]).map_err(|e| {
        format!("lint --quick compares with origin/main ({e}); run the full lint, or `git fetch origin`")
    })?;
    let base = String::from_utf8_lossy(&base).trim().to_string();
    if let Some(manifest) = std::env::var_os("POLYD_WANT").and_then(|p| std::fs::read(p).ok()) {
        return Ok(changed_by_manifest(&manifest, &git(root, &["ls-tree", "-r", "-z", &base])?));
    }
    let mut files: Vec<String> = Vec::new();
    for args in [&["diff", "--name-only", "-z", &base][..], &["ls-files", "-o", "--exclude-standard", "-z"]] {
        files.extend(records(&git(root, args)?).map(|p| String::from_utf8_lossy(p).into_owned()));
    }
    files.sort();
    files.dedup();
    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn members() -> Vec<(String, String)> {
        [
            ("crates/poly-abi", "poly-abi"),
            ("personalities/nt", "poly-nt"),
            ("tools/xtask", "xtask"),
            ("tools/coverage", "poly-coverage"),
        ]
        .map(|(d, n)| (d.to_string(), n.to_string()))
        .to_vec()
    }

    fn plan_of(files: &[&str]) -> std::result::Result<Plan, String> {
        plan(&files.iter().map(|f| f.to_string()).collect::<Vec<_>>(), &members())
    }

    #[test]
    fn only_the_touched_units_are_planned() {
        let p = plan_of(&["crates/poly-abi/src/lib.rs", "tools/coverage/src/main.rs", "crates/poly-abi/Cargo.toml"])
            .unwrap();
        assert_eq!(p, Plan { packages: vec!["poly-abi".into(), "poly-coverage".into()], ..Plan::default() });

        // A file of no crate plans nothing, and a directory name is not a prefix of another's.
        assert_eq!(
            plan_of(&["docs/adr/0174-x.md", "crates/poly-abi-extra/src/lib.rs", "kernel.md"]).unwrap(),
            Plan::default()
        );
        assert_eq!(Plan::default().describe(), "no crate (only the source checks)");
    }

    #[test]
    fn workspaces_inside_member_directories_are_their_own_units() {
        let p = plan_of(&[
            "personalities/nt/src/lib.rs",
            "personalities/nt/ntdll/src/lib.rs",
            "personalities/nt/win32/user32/src/lib.rs",
            "kernel/src/main.rs",
            "kernel/src/arch/aarch64/src/lib.rs",
            "userland/native/hello/src/main.rs",
            "tools/wintrace/proxy/common.rs",
            "tools/xtask/src/main.rs",
        ])
        .unwrap();
        assert_eq!(
            p,
            Plan {
                all_host: false,
                packages: vec!["poly-nt".into(), "xtask".into()],
                xtask_windows: true,
                kernel: true,
                native: true,
                aarch64: true,
                ntdll: true,
                win32: true,
                wintrace: true,
            }
        );
        assert!(p.describe().starts_with("poly-nt, xtask, xtask for Windows, the kernel"));
    }

    #[test]
    fn a_workspace_wide_file_needs_the_full_lint() {
        assert_eq!(plan_of(&["crates/poly-abi/src/lib.rs", "Cargo.lock"]), Err("Cargo.lock changed".into()));
        assert_eq!(plan_of(&[".cargo/config.toml"]), Err(".cargo/config.toml changed".into()));
        // A crate's own manifest does not.
        assert!(plan_of(&["tools/xtask/Cargo.toml"]).is_ok());
    }

    #[test]
    fn reads_the_members_and_the_package_name() {
        let toml = "[workspace]\nresolver = \"3\"\nmembers = [\n    \"crates/a\", # one\n    \"tools/b\",\n]\n\n[workspace.package]\nedition = \"2024\"\n";
        assert_eq!(member_dirs(toml), ["crates/a", "tools/b"]);
        assert!(member_dirs("[package]\nname = \"x\"\n").is_empty());
        assert_eq!(
            package_name("[package]\nname = \"poly-coverage\"\nversion = \"0.0.1\"\n\n[[bin]]\nname = \"coverage\"\n")
                .as_deref(),
            Some("poly-coverage")
        );
        assert_eq!(package_name("[lib]\nname = \"x\"\n"), None);
    }

    /// The real workspace: every member has a name, xtask among them.
    #[test]
    fn the_workspace_members_are_read() {
        let m = super::members(&crate::root()).unwrap();
        assert!(m.contains(&("tools/xtask".to_string(), "xtask".to_string())), "{m:?}");
        assert!(m.len() > 50);
    }

    #[test]
    fn compares_the_manifest_with_the_base_tree() {
        let tree =
            b"100644 blob aaaa\tsame.rs\x00100644 blob bbbb\tedited.rs\x00100755 blob cccc\tgone dir/gone.rs\x00";
        let manifest = b"100644 aaaa 0\tsame.rs\x00100644 dddd 0\tedited.rs\x00100644 eeee 0\tnew.rs\x00";
        assert_eq!(changed_by_manifest(manifest, tree), ["edited.rs", "gone dir/gone.rs", "new.rs"]);
        assert!(changed_by_manifest(b"", b"").is_empty());
    }
}
