//! App tests: an unchanged, dynamically linked program of this (real Linux)
//! machine, run with fixed arguments, stdin, environment and files, on the
//! reference system and on Poly, and compared like a corpus test
//! (docs/AGENT_PIPELINE.md, "App tests"; ADR 0056).
//!
//! A test is a JSON file `<name>.app` (`tests/linux/apps/`, or an entry's
//! tests directory):
//!
//! ```json
//! {
//!   "description": "cat -n numbers the lines of a file",
//!   "steps": [["cat", "-n", "in.txt"]],
//!   "stdin": "optional text",
//!   "env": ["OPTIONAL=extra"],
//!   "files": [
//!     {"path": "in.txt", "text": "b\na\n"},
//!     {"path": "sub", "dir": true},
//!     {"path": "run.sh", "text": "...", "mode": "755"},
//!     {"path": "link", "symlink": "in.txt"}
//!   ]
//! }
//! ```
//!
//! Each step names a program of the host (`/usr/bin`, `/bin`, `/usr/sbin`,
//! `/sbin`) and its arguments; it runs as `/bin/<name>` with argv[0]
//! `<name>`. Every step but the last runs as a child that is waited for;
//! the last one replaces the launcher, so its exit status is the test's.
//!
//! From the definition the tool makes one file tree, the same for both
//! systems: the launcher (`tests/linux/apps/launch.c`) as `/test-binary`,
//! its configuration and stdin in `/poly-app/`, the programs in `/bin`,
//! their program interpreter and the shared libraries they need (found as
//! `tools/xtask/src/libsearch.rs` does for the dynamic linking boot tests)
//! at the paths they have on the host, `/etc/passwd` and `/etc/group` with
//! only root, empty `/dev`, `/proc`, `/sys`, `/tmp`, and the test's files
//! under `/work` (the working directory). Everything belongs to root and
//! has the modification time [`MTIME`]; directories are 0755, files 0644
//! unless the definition says otherwise.
//!
//! On Poly the tree is the initramfs (the kernel mounts its `/dev`, `/proc`
//! and `/sys` over the empty directories). On Linux it is written to a
//! tmpfs where possible (what an initramfs is on Linux) and the launcher
//! runs chrooted into it, in a mount namespace of its own with the host's
//! `/dev` and `/proc` bound in (`ptrace.rs`; `/sys` stays empty there),
//! and [`proc_overlays`] over some of the host's `/proc` files.
//!
//! The environment is fixed: [`ENV`] plus the definition's `env`.

use crate::cpio;
use crate::libsearch;
use crate::sha256;
use crate::util::{self, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::ffi::CString;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

/// File name extension of app test definitions.
pub const EXTENSION: &str = "app";
/// The launcher's source, relative to the repository.
pub const LAUNCHER_SOURCE: &str = "tests/linux/apps/launch.c";
/// The launcher's binary in a corpus build directory.
pub const LAUNCHER_BINARY: &str = "poly-launch";
/// Modification time of everything in the tree: 2023-11-14 22:13:20 UTC,
/// more than six months before any run, so `ls -l` shows the year on
/// every system.
pub const MTIME: u32 = 1_700_000_000;
/// The environment every program gets, before the definition's `env`.
/// `GLIBC_TUNABLES` hides SSE4.2 from glibc (tunables, glibc manual), so
/// the dynamic linker does not search the `glibc-hwcaps/x86-64-v2..v4`
/// library directories on a newer host CPU: its search is then the one a
/// baseline x86-64 CPU (QEMU's default model, under which Poly runs) gets.
pub const ENV: &[&str] = &["PATH=/bin", "HOME=/work", "LANG=C", "TZ=UTC", "GLIBC_TUNABLES=glibc.cpu.hwcaps=-SSE4_2"];
/// The working directory, where the definition's files go.
pub const WORK: &str = "work";
/// Where the host's programs are looked up.
const PROGRAM_DIRS: &[&str] = &["/usr/bin", "/bin", "/usr/sbin", "/sbin"];
/// Launcher files in the tree.
pub const CONFIG_PATH: &str = "poly-app/launch";
pub const STDIN_PATH: &str = "poly-app/stdin";
pub const STDOUT_PATH: &str = "poly-app/stdout";
pub const STDERR_PATH: &str = "poly-app/stderr";

/// One file of a definition, relative to `/work`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileDef {
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub dir: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub symlink: Option<String>,
    /// Octal permission bits (`"755"`); files 644 and directories 755 by default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
}

/// An app test definition (`<name>.app`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppDef {
    pub description: String,
    /// `[program, args...]` per step.
    pub steps: Vec<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stdin: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub env: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<FileDef>,
}

fn relative_ok(path: &str) -> bool {
    !path.is_empty()
        && !path.starts_with('/')
        && !path.ends_with('/')
        && path.split('/').all(|c| !c.is_empty() && c != "." && c != "..")
}

fn parse_mode(mode: &Option<String>, default: u32) -> Result<u32> {
    match mode {
        None => Ok(default),
        Some(m) => match u32::from_str_radix(m, 8) {
            Ok(v) if v <= 0o7777 => Ok(v),
            _ => Err(format!("mode {m:?}: octal permission bits expected")),
        },
    }
}

impl AppDef {
    pub fn load(path: &Path) -> Result<AppDef> {
        let def: AppDef = util::read_json(path)?;
        def.check().map_err(|e| format!("{}: {e}", path.display()))?;
        Ok(def)
    }

    pub fn check(&self) -> Result {
        if self.steps.is_empty() || self.steps.len() > 16 {
            return Err("between 1 and 16 steps expected".into());
        }
        for step in &self.steps {
            let Some(program) = step.first() else { return Err("a step without a program".into()) };
            if program.is_empty() || program.contains('/') {
                return Err(format!("program {program:?}: a command name of the host expected, not a path"));
            }
        }
        for v in &self.env {
            if !v.contains('=') || v.contains('\0') {
                return Err(format!("env {v:?}: NAME=value expected"));
            }
        }
        let mut seen = BTreeSet::new();
        for f in &self.files {
            if !relative_ok(&f.path) {
                return Err(format!("file {:?}: a relative path inside /work expected", f.path));
            }
            if !seen.insert(f.path.clone()) {
                return Err(format!("file {:?} given twice", f.path));
            }
            let kinds = usize::from(f.text.is_some()) + usize::from(f.dir) + usize::from(f.symlink.is_some());
            if kinds != 1 {
                return Err(format!("file {:?}: exactly one of text, dir and symlink", f.path));
            }
            parse_mode(&f.mode, 0)?;
        }
        if self.steps.iter().flatten().chain(&self.env).any(|s| s.contains('\0')) {
            return Err("arguments cannot contain NUL".into());
        }
        Ok(())
    }

    /// The programs the steps name, each once, in order.
    pub fn programs(&self) -> Vec<&str> {
        let mut out: Vec<&str> = Vec::new();
        for step in &self.steps {
            if !out.contains(&step[0].as_str()) {
                out.push(&step[0]);
            }
        }
        out
    }

    /// The launcher's configuration (`/poly-app/launch`, see launch.c).
    pub fn launch_config(&self) -> Vec<u8> {
        let mut words: Vec<String> = vec!["poly-launch/1".into(), format!("/{WORK}"), self.steps.len().to_string()];
        for step in &self.steps {
            words.push(format!("/bin/{}", step[0]));
            words.push(step.len().to_string());
            words.extend(step.iter().cloned());
        }
        let env: Vec<String> = ENV.iter().map(|s| s.to_string()).chain(self.env.iter().cloned()).collect();
        words.push(env.len().to_string());
        words.extend(env);
        let mut out = Vec::new();
        for w in words {
            out.extend_from_slice(w.as_bytes());
            out.push(0);
        }
        out
    }
}

/// What a tree node is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind {
    Dir(u32),
    File(u32, Vec<u8>),
    Symlink(String),
}

/// One node: path without the leading `/`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Node {
    pub path: String,
    pub kind: Kind,
}

/// The file tree of an app test, in creation order (parents first).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Tree {
    pub nodes: Vec<Node>,
}

impl Tree {
    fn has(&self, path: &str) -> bool {
        self.nodes.iter().any(|n| n.path == path)
    }

    /// Add `path`, with its missing parent directories (0755) first.
    fn add(&mut self, path: &str, kind: Kind) -> Result {
        if self.has(path) {
            return Err(format!("/{path} is in the tree twice"));
        }
        let mut parent = String::new();
        let parts: Vec<&str> = path.split('/').collect();
        for p in &parts[..parts.len() - 1] {
            if !parent.is_empty() {
                parent.push('/');
            }
            parent.push_str(p);
            match self.nodes.iter().find(|n| n.path == parent) {
                None => self.nodes.push(Node { path: parent.clone(), kind: Kind::Dir(0o755) }),
                Some(Node { kind: Kind::Dir(_), .. }) => {}
                Some(_) => return Err(format!("/{parent} is not a directory")),
            }
        }
        self.nodes.push(Node { path: path.into(), kind });
        Ok(())
    }

    /// The archive for Poly's initramfs: the root's own entry first.
    pub fn cpio(&self) -> Result<Vec<u8>> {
        let mut entries = vec![cpio::Entry::dir_mode(".", 0o755).at(MTIME)];
        for n in &self.nodes {
            entries.push(
                match &n.kind {
                    Kind::Dir(mode) => cpio::Entry::dir_mode(&n.path, *mode),
                    Kind::File(mode, data) => cpio::Entry::file(&n.path, *mode, data.clone()),
                    Kind::Symlink(target) => cpio::Entry::symlink(&n.path, target),
                }
                .at(MTIME),
            );
        }
        cpio::write(&entries)
    }

    /// Write the tree into the empty directory `root` (created), with the
    /// same modes and times.
    pub fn materialise(&self, root: &Path) -> Result {
        use std::os::unix::fs::PermissionsExt;
        std::fs::create_dir(root).map_err(|e| format!("{}: {e}", root.display()))?;
        let perms = |p: &Path, mode: u32| {
            std::fs::set_permissions(p, std::fs::Permissions::from_mode(mode))
                .map_err(|e| format!("{}: {e}", p.display()))
        };
        for n in &self.nodes {
            let at = root.join(&n.path);
            match &n.kind {
                Kind::Dir(mode) => {
                    std::fs::create_dir(&at).map_err(|e| format!("{}: {e}", at.display()))?;
                    perms(&at, *mode)?;
                }
                Kind::File(mode, data) => {
                    std::fs::write(&at, data).map_err(|e| format!("{}: {e}", at.display()))?;
                    perms(&at, *mode)?;
                }
                Kind::Symlink(target) => {
                    std::os::unix::fs::symlink(target, &at).map_err(|e| format!("{}: {e}", at.display()))?
                }
            }
        }
        perms(root, 0o755)?;
        // Times last, children before their directory, the root at the end.
        for n in self.nodes.iter().rev() {
            set_mtime(&root.join(&n.path))?;
        }
        set_mtime(root)
    }

    /// The files' hashes, for the record (`<sha256> <path>` lines).
    pub fn listing(&self) -> String {
        let mut out = String::new();
        for n in &self.nodes {
            let what = match &n.kind {
                Kind::Dir(m) => format!("dir {m:o}"),
                Kind::File(m, d) => format!("file {m:o} {}", sha256::hex(d)),
                Kind::Symlink(t) => format!("symlink {t}"),
            };
            out.push_str(&format!("/{} {what}\n", n.path));
        }
        out
    }
}

/// Set the modification and access time of `path` (not following a
/// symbolic link) to [`MTIME`].
fn set_mtime(path: &Path) -> Result {
    let c = CString::new(path.as_os_str().as_bytes()).map_err(|e| e.to_string())?;
    let t = libc::timespec { tv_sec: libc::time_t::from(MTIME), tv_nsec: 0 };
    let times = [t, t];
    // SAFETY: `c` is a NUL-terminated path and `times` an array of two
    // timespecs, as utimensat(2) takes; both outlive the call.
    let r = unsafe { libc::utimensat(libc::AT_FDCWD, c.as_ptr(), times.as_ptr(), libc::AT_SYMLINK_NOFOLLOW) };
    if r < 0 {
        return Err(format!("utimensat {}: {}", path.display(), std::io::Error::last_os_error()));
    }
    Ok(())
}

/// The host program `name`, from [`PROGRAM_DIRS`].
pub fn find_program(name: &str) -> Option<PathBuf> {
    PROGRAM_DIRS.iter().map(|d| Path::new(d).join(name)).find(|p| p.is_file())
}

/// Links named `name` to `/<at>` in the directories ld.so(8) searches by
/// default, where nothing is yet (as the dynamic linking boot tests do,
/// `tools/xtask/src/dynamic.rs`).
fn default_links(tree: &mut Tree, name: &str, at: &str) -> Result {
    for dir in ["lib", "usr/lib", "lib64", "usr/lib64"] {
        let link = format!("{dir}/{name}");
        if link != at && !tree.has(&link) {
            tree.add(&link, Kind::Symlink(format!("/{at}")))?;
        }
    }
    Ok(())
}

/// Build the tree of `def` with the launcher binary `launcher`. `Err` with
/// the reason if a program is not on this host or is not dynamically
/// linked against a C library the tool can collect.
pub fn build_tree(def: &AppDef, launcher: &[u8]) -> Result<Tree> {
    def.check()?;
    let mut tree = Tree::default();
    for dir in ["bin", "dev", "etc", "poly-app", "proc", "sys"] {
        tree.add(dir, Kind::Dir(0o755))?;
    }
    tree.add("tmp", Kind::Dir(0o1777))?;
    tree.add("test-binary", Kind::File(0o755, launcher.to_vec()))?;
    tree.add(CONFIG_PATH, Kind::File(0o644, def.launch_config()))?;
    tree.add(STDIN_PATH, Kind::File(0o644, def.stdin.clone().unwrap_or_default().into_bytes()))?;
    tree.add("etc/passwd", Kind::File(0o644, b"root:x:0:0:root:/root:/bin/sh\n".to_vec()))?;
    tree.add("etc/group", Kind::File(0o644, b"root:x:0:\n".to_vec()))?;

    let mut objects: Vec<(String, Vec<u8>)> = Vec::new();
    let mut interp: Option<String> = None;
    for name in def.programs() {
        let path = find_program(name).ok_or_else(|| format!("{name} is not installed on this host"))?;
        let elf = util::read(&path)?;
        let parsed =
            poly_elf::Elf::parse(&elf).map_err(|e| format!("{}: not an x86-64 ELF program: {e:?}", path.display()))?;
        let Some(i) = parsed.interp() else {
            return Err(format!("{} is statically linked; app tests run dynamically linked programs", path.display()));
        };
        let i = String::from_utf8_lossy(i).into_owned();
        if interp.get_or_insert_with(|| i.clone()) != &i {
            return Err(format!("the programs name two interpreters ({i} and {})", interp.unwrap_or_default()));
        }
        tree.add(&format!("bin/{name}"), Kind::File(0o755, elf.clone()))?;
        objects.push((name.to_string(), elf));
    }
    let interp = interp.ok_or("no programs")?;
    let interp_name = Path::new(&interp).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let interp_at = interp.trim_start_matches('/').to_string();
    tree.add(&interp_at, Kind::File(0o755, util::read(Path::new(&interp))?))?;
    let found = libsearch::needed_closure(&objects, &interp_name)?;
    if !found.contains_key("libc.so.6") {
        return Err("the programs do not need libc.so.6: not a glibc host".into());
    }
    for (name, path) in &found {
        let at = path.parent().unwrap_or(Path::new("/")).join(name);
        let at = at.to_string_lossy().trim_start_matches('/').to_string();
        tree.add(&at, Kind::File(0o755, util::read(path)?))?;
    }
    for (name, path) in &found {
        let at = path.parent().unwrap_or(Path::new("/")).join(name);
        default_links(&mut tree, name, at.to_string_lossy().trim_start_matches('/'))?;
    }
    // libc names the interpreter by its own name: a link where libc is.
    if let Some(libc) = found.get("libc.so.6") {
        let at = libc.parent().unwrap_or(Path::new("/")).join(&interp_name);
        let at = at.to_string_lossy().trim_start_matches('/').to_string();
        if at != interp_at && !tree.has(&at) {
            tree.add(&at, Kind::Symlink(interp.clone()))?;
        }
    }

    tree.add(WORK, Kind::Dir(0o755))?;
    for f in &def.files {
        let path = format!("{WORK}/{}", f.path);
        let kind = match (&f.text, f.dir, &f.symlink) {
            (Some(text), _, _) => Kind::File(parse_mode(&f.mode, 0o644)?, text.clone().into_bytes()),
            (_, true, _) => Kind::Dir(parse_mode(&f.mode, 0o755)?),
            (_, _, Some(target)) => Kind::Symlink(target.clone()),
            _ => unreachable!("checked"),
        };
        // A directory the definition lists after a file in it already exists.
        if let Some(n) = tree.nodes.iter_mut().find(|n| n.path == path)
            && matches!((&n.kind, &kind), (Kind::Dir(_), Kind::Dir(_)))
        {
            n.kind = kind;
            continue;
        }
        tree.add(&path, kind)?;
    }
    Ok(tree)
}

/// Directories app trees are written to on the reference system: a tmpfs
/// that allows programs to run if there is one (`$POLY_APP_TREE_DIR`,
/// `/dev/shm`), else the temporary directory.
pub fn tree_base() -> PathBuf {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(d) = std::env::var_os("POLY_APP_TREE_DIR") {
        candidates.push(PathBuf::from(d));
    }
    candidates.push(PathBuf::from("/dev/shm"));
    let tmp = std::env::temp_dir();
    candidates
        .iter()
        .find(|d| is_tmpfs(d) && allows_exec(d) && writable(d))
        .cloned()
        .or_else(|| candidates.iter().find(|d| allows_exec(d) && writable(d)).cloned())
        .unwrap_or(tmp)
}

fn statvfs_flags(dir: &Path) -> Option<u64> {
    let c = CString::new(dir.as_os_str().as_bytes()).ok()?;
    // SAFETY: statvfs is plain integers; all zeroes is a valid value.
    let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
    // SAFETY: `c` is a NUL-terminated path; statvfs(3) fills `st`.
    (unsafe { libc::statvfs(c.as_ptr(), &mut st) } == 0).then_some(st.f_flag)
}

fn allows_exec(dir: &Path) -> bool {
    statvfs_flags(dir).is_some_and(|f| f & libc::ST_NOEXEC == 0)
}

fn writable(dir: &Path) -> bool {
    let Ok(c) = CString::new(dir.as_os_str().as_bytes()) else { return false };
    // SAFETY: `c` is a NUL-terminated path.
    dir.is_dir() && unsafe { libc::access(c.as_ptr(), libc::W_OK | libc::X_OK) } == 0
}

/// Whether `dir` is on a tmpfs (statfs(2), `TMPFS_MAGIC` of linux/magic.h).
pub fn is_tmpfs(dir: &Path) -> bool {
    const TMPFS_MAGIC: i64 = 0x0102_1994;
    let Ok(c) = CString::new(dir.as_os_str().as_bytes()) else { return false };
    // SAFETY: statfs is plain integers; all zeroes is a valid value.
    let mut st: libc::statfs = unsafe { std::mem::zeroed() };
    // SAFETY: `c` is a NUL-terminated path; statfs(2) fills `st`.
    (unsafe { libc::statfs(c.as_ptr(), &mut st) } == 0) && st.f_type as i64 == TMPFS_MAGIC
}

/// A fresh directory name for one run's tree under [`tree_base`].
pub fn fresh_tree_dir(test: &str) -> PathBuf {
    use std::sync::atomic::{AtomicU32, Ordering};
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let safe: String = test.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '_' }).collect();
    tree_base().join(format!("poly-app-{}-{n}-{safe}", std::process::id()))
}

/// File systems the reference hides from `/proc/filesystems`: Poly is a
/// Linux without SELinux, and the SELinux library that `ls`, `cp`, `mkdir`
/// and `sed` link probes `/proc/filesystems` for `selinuxfs` (and then
/// reads `/proc/mounts`), so the reference must look like a kernel built
/// without it, whatever the host's kernel is.
pub const HIDDEN_FILESYSTEMS: &[&str] = &["selinuxfs"];

/// Files the reference run binds over the host's `/proc` in the tree at
/// `tree`: a copy of `/proc/filesystems` without [`HIDDEN_FILESYSTEMS`],
/// written next to the tree (`<tree>.filesystems`, removed with it).
pub fn proc_overlays(tree: &Path) -> Result<Vec<(PathBuf, String)>> {
    let host = util::read_string(Path::new("/proc/filesystems"))?;
    let kept: String = host
        .lines()
        .filter(|l| !HIDDEN_FILESYSTEMS.contains(&l.split('\t').next_back().unwrap_or_default().trim()))
        .map(|l| format!("{l}\n"))
        .collect();
    let file = tree.with_extension("filesystems");
    util::write(&file, kept)?;
    Ok(vec![(file, "proc/filesystems".into())])
}

/// The `.app` files in `dir`, sorted.
pub fn definitions(dir: &Path) -> Result<Vec<PathBuf>> {
    let Ok(read) = std::fs::read_dir(dir) else { return Ok(Vec::new()) };
    let mut out = Vec::new();
    for e in read {
        let path = e.map_err(|e| e.to_string())?.path();
        if path.extension().is_some_and(|x| x == EXTENSION) && path.is_file() {
            out.push(path);
        }
    }
    out.sort();
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn def(json: &str) -> Result<AppDef> {
        let d: AppDef = serde_json::from_str(json).map_err(|e| e.to_string())?;
        d.check()?;
        Ok(d)
    }

    #[test]
    fn definitions_are_checked() {
        assert!(def(r#"{"description": "x", "steps": [["cat", "a"]]}"#).is_ok());
        assert!(def(r#"{"description": "x", "steps": []}"#).is_err());
        assert!(def(r#"{"description": "x", "steps": [["/bin/cat"]]}"#).is_err(), "a path");
        assert!(def(r#"{"description": "x", "steps": [["cat"]], "bogus": 1}"#).is_err(), "unknown field");
        for bad in ["/abs", "../up", "a/./b", "a/"] {
            let j =
                format!(r#"{{"description": "x", "steps": [["cat"]], "files": [{{"path": "{bad}", "text": ""}}]}}"#);
            assert!(def(&j).is_err(), "{bad}");
        }
        let two = r#"{"description": "x", "steps": [["cat"]], "files": [{"path": "a", "text": "", "dir": true}]}"#;
        assert!(def(two).is_err(), "two kinds");
        let mode = r#"{"description": "x", "steps": [["cat"]], "files": [{"path": "a", "text": "", "mode": "9"}]}"#;
        assert!(def(mode).is_err(), "not octal");
        assert!(def(r#"{"description": "x", "steps": [["cat"]], "env": ["NOEQUALS"]}"#).is_err());
    }

    #[test]
    fn launch_configuration_layout() {
        let d = def(r#"{"description": "x", "steps": [["cp", "a", "b"], ["cat", "b"]], "env": ["X=1"]}"#).unwrap();
        let words: Vec<String> =
            d.launch_config().split(|&b| b == 0).map(|w| String::from_utf8_lossy(w).into_owned()).collect();
        assert_eq!(
            words,
            [
                "poly-launch/1",
                "/work",
                "2",
                "/bin/cp",
                "3",
                "cp",
                "a",
                "b",
                "/bin/cat",
                "2",
                "cat",
                "b",
                "6",
                "PATH=/bin",
                "HOME=/work",
                "LANG=C",
                "TZ=UTC",
                "GLIBC_TUNABLES=glibc.cpu.hwcaps=-SSE4_2",
                "X=1",
                ""
            ]
        );
        assert_eq!(d.programs(), ["cp", "cat"]);
    }

    fn sample_tree() -> Tree {
        let mut t = Tree::default();
        t.add("work/sub/f", Kind::File(0o600, b"x".to_vec())).unwrap();
        t.add("work/l", Kind::Symlink("sub/f".into())).unwrap();
        t.add("tmp", Kind::Dir(0o1777)).unwrap();
        t
    }

    #[test]
    fn trees_make_parents_first() {
        let t = sample_tree();
        let paths: Vec<&str> = t.nodes.iter().map(|n| n.path.as_str()).collect();
        assert_eq!(paths, ["work", "work/sub", "work/sub/f", "work/l", "tmp"]);
        let mut t2 = t.clone();
        assert!(t2.add("work/sub/f", Kind::Dir(0o755)).is_err(), "twice");
        assert!(t2.add("work/sub/f/g", Kind::Dir(0o755)).is_err(), "under a file");
        let entries = cpio::read(&t.cpio().unwrap()).unwrap();
        assert_eq!(entries[0].name, ".");
        assert!(entries.iter().all(|e| e.mtime == MTIME));
        assert_eq!(entries[3].mode, cpio::S_IFREG | 0o600);
        assert_eq!(entries[4].mode, cpio::S_IFLNK | 0o777);
        assert_eq!(entries[4].data, b"sub/f");
        assert_eq!(entries[5].mode, cpio::S_IFDIR | 0o1777);
    }

    #[test]
    fn materialised_trees_have_modes_and_times() {
        use std::os::unix::fs::MetadataExt;
        let base = std::env::temp_dir().join(format!("poly-apps-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let root = base.join("tree");
        sample_tree().materialise(&root).unwrap();
        let f = std::fs::metadata(root.join("work/sub/f")).unwrap();
        assert_eq!((f.mode() & 0o7777, f.mtime()), (0o600, i64::from(MTIME)));
        let l = std::fs::symlink_metadata(root.join("work/l")).unwrap();
        assert_eq!(l.mtime(), i64::from(MTIME));
        assert_eq!(std::fs::read_link(root.join("work/l")).unwrap(), Path::new("sub/f"));
        let d = std::fs::metadata(root.join("work")).unwrap();
        assert_eq!((d.mode() & 0o7777, d.mtime()), (0o755, i64::from(MTIME)), "directory time set last");
        assert_eq!(std::fs::metadata(root.join("tmp")).unwrap().mode() & 0o7777, 0o1777);
        assert_eq!(std::fs::metadata(&root).unwrap().mtime(), i64::from(MTIME));
        let _ = std::fs::remove_dir_all(&base);
    }

    /// A real host program, where there is one: its tree holds it, the
    /// interpreter and libc.
    #[test]
    fn trees_of_host_programs() {
        if find_program("true").is_none() {
            eprintln!("skipped: no true(1) on this host");
            return;
        }
        let d = def(r#"{"description": "x", "steps": [["true"]], "files": [{"path": "d/a", "text": "1"}]}"#).unwrap();
        let t = match build_tree(&d, b"launcher") {
            Ok(t) => t,
            Err(e) => {
                eprintln!("skipped: {e}");
                return;
            }
        };
        assert!(t.has("bin/true") && t.has("test-binary") && t.has(CONFIG_PATH) && t.has("work/d/a"));
        assert!(t.nodes.iter().any(|n| n.path.ends_with("/libc.so.6") && matches!(n.kind, Kind::File(..))));
        assert!(t.listing().contains("/test-binary file 755 "));
    }
}
