//! The root tree: its entries, the collector that gathers host files, and the gcc/header closure.

use super::*;

/// Modification time of the files the image gets that are not the host's
/// (configuration, the test files): 2023-11-14 22:13:20 UTC.
pub(super) const MTIME: i64 = 1_700_000_000;

/// The source of a regular file's contents.
#[derive(Clone, Debug)]
pub enum Source {
    Host(PathBuf),
    Bytes(Vec<u8>),
}

/// One entry of the image, by path (no leading `/`).
#[derive(Clone, Debug)]
pub enum Entry {
    Dir(u16),
    File { mode: u16, source: Source, mtime: i64 },
    Symlink(String),
}

/// The tree the image is made of.
#[derive(Debug, Default)]
pub struct RootTree {
    pub entries: BTreeMap<String, Entry>,
}

impl RootTree {
    pub(super) fn insert(&mut self, path: &str, entry: Entry) {
        let path = path.trim_start_matches('/').to_string();
        if path.is_empty() {
            return;
        }
        // Parent directories, unless something else is there already.
        let mut parent = Path::new(&path).parent();
        while let Some(dir) = parent.filter(|d| !d.as_os_str().is_empty()) {
            self.entries.entry(dir.to_string_lossy().into_owned()).or_insert(Entry::Dir(0o755));
            parent = dir.parent();
        }
        match (self.entries.get(&path), &entry) {
            // A directory asked for twice keeps the first mode.
            (Some(Entry::Dir(_)), Entry::Dir(_)) => {}
            _ => {
                self.entries.insert(path, entry);
            }
        }
    }

    /// A file the image makes itself.
    pub fn file(&mut self, path: &str, mode: u16, data: &[u8]) {
        self.insert(path, Entry::File { mode, source: Source::Bytes(data.to_vec()), mtime: MTIME });
    }

    /// A file of the host, at `path` in the image (Chromium's disk adds a
    /// whole unpacked tree this way, `chromium.rs`).
    pub fn host_file(&mut self, path: &str, mode: u16, host: &Path, mtime: i64) {
        self.insert(path, Entry::File { mode, source: Source::Host(host.to_path_buf()), mtime });
    }

    pub fn dir(&mut self, path: &str, mode: u16) {
        self.insert(path, Entry::Dir(mode));
    }

    pub fn symlink(&mut self, path: &str, target: &str) {
        self.insert(path, Entry::Symlink(target.to_string()));
    }

    /// Bytes of regular files and how many entries there are.
    pub(super) fn totals(&self) -> (u64, u64) {
        let mut bytes = 0;
        for entry in self.entries.values() {
            if let Entry::File { source, .. } = entry {
                bytes += match source {
                    Source::Host(p) => std::fs::metadata(p).map_or(0, |m| m.len()),
                    Source::Bytes(b) => b.len() as u64,
                };
            }
        }
        (bytes, self.entries.len() as u64)
    }
}

/// What the image holds and what was left out.
#[derive(Debug, Default)]
pub struct Contents {
    pub tree: RootTree,
    /// Programs found: "bash", "coreutils", "grep", "sed", "python3", "gcc".
    pub have: BTreeSet<&'static str>,
    /// What was left out and why.
    pub notes: Vec<String>,
}

/// Collects host files into a [`RootTree`], following symbolic links and
/// the shared objects ELF files need.
pub(crate) struct Collector {
    pub(crate) tree: RootTree,
    pub(super) search: SearchPath,
    /// Host paths already added (as given and resolved).
    seen: BTreeSet<PathBuf>,
    /// ELF files whose `DT_NEEDED` entries are still to be added.
    elves: VecDeque<PathBuf>,
}

/// `path` with its directory's symbolic links resolved (the last component
/// kept as it is, so a link stays a link).
fn host_location(path: &Path) -> Option<PathBuf> {
    let parent = path.parent()?;
    let name = path.file_name()?;
    Some(std::fs::canonicalize(parent).ok()?.join(name))
}

fn mtime_of(meta: &std::fs::Metadata) -> i64 {
    crate::hostfs::mtime(meta)
}

impl Collector {
    pub(super) fn new() -> Collector {
        Collector {
            tree: RootTree::default(),
            search: SearchPath::host(),
            seen: BTreeSet::new(),
            elves: VecDeque::new(),
        }
    }

    /// Add the host's file, link or directory at `path` (a directory with
    /// nothing in it). `Err` if it does not exist.
    pub(crate) fn add(&mut self, path: &Path) -> Result<(), String> {
        self.add_depth(path, 0)
    }

    /// Where the host's loader would find the shared object `name` (a
    /// library a program opens by name, which no `DT_NEEDED` entry names).
    pub(crate) fn find_library(&self, name: &str) -> Option<PathBuf> {
        self.search.find(name, &[]).map(|(path, _)| path)
    }

    fn add_depth(&mut self, path: &Path, depth: u32) -> Result<(), String> {
        if depth > 16 {
            return Err(format!("{}: too many levels of symbolic links", path.display()));
        }
        let at = host_location(path).ok_or_else(|| format!("{}: not found", path.display()))?;
        if !self.seen.insert(at.clone()) {
            return Ok(());
        }
        let meta = std::fs::symlink_metadata(&at).map_err(|e| format!("{}: {e}", at.display()))?;
        let image_path = at.to_string_lossy().into_owned();
        if meta.file_type().is_symlink() {
            let target = std::fs::read_link(&at).map_err(|e| format!("{}: {e}", at.display()))?;
            self.tree.symlink(&image_path, &target.to_string_lossy());
            let resolved = if target.is_absolute() { target } else { at.parent().unwrap().join(target) };
            return self.add_depth(&resolved, depth + 1);
        }
        if meta.is_dir() {
            self.tree.dir(&image_path, crate::hostfs::mode(&meta) as u16);
            return Ok(());
        }
        if !meta.is_file() {
            return Err(format!("{}: not a regular file, directory or link", at.display()));
        }
        let mode = crate::hostfs::mode(&meta) as u16;
        self.tree.insert(&image_path, Entry::File { mode, source: Source::Host(at.clone()), mtime: mtime_of(&meta) });
        if is_elf(&at) {
            self.elves.push_back(at);
        }
        Ok(())
    }

    /// Add the shared objects the host's ELF file at `path` needs (not the
    /// file itself: the caller puts it somewhere else).
    pub(crate) fn add_elf_needs(&mut self, path: &Path) {
        self.elves.push_back(path.to_path_buf());
    }

    /// Add a directory and everything under it, leaving out the names in
    /// `skip` (at any depth) and `__pycache__` directories only if asked.
    pub(crate) fn add_tree(&mut self, dir: &Path, skip: &[&str]) -> Result<(), String> {
        self.add(dir)?;
        let at = std::fs::canonicalize(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        let mut entries: Vec<_> = std::fs::read_dir(&at)
            .map_err(|e| format!("{}: {e}", at.display()))?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .collect();
        entries.sort();
        for path in entries {
            let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            if skip.contains(&name.as_str()) {
                continue;
            }
            let meta = std::fs::symlink_metadata(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            if meta.is_dir() {
                self.add_tree(&path, skip)?;
            } else {
                self.add(&path)?;
            }
        }
        Ok(())
    }

    /// Add the shared objects (and interpreters) the ELF files added so far
    /// need, until nothing is missing.
    pub(super) fn add_needed(&mut self) -> Result<(), String> {
        while let Some(path) = self.elves.pop_front() {
            let bytes = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            let Ok(elf) = Elf::parse(&bytes) else { continue };
            if let Some(interp) = elf.interp() {
                let interp = PathBuf::from(String::from_utf8_lossy(interp).into_owned());
                self.add(&interp).map_err(|e| format!("the interpreter of {}: {e}", path.display()))?;
            }
            let Ok(Some(dynamic)) = Dynamic::parse(&elf) else { continue };
            let own = own_paths(&dynamic);
            for needed in dynamic.needed() {
                let name = String::from_utf8_lossy(needed).into_owned();
                let (found, _) = self.search.find(&name, &own).ok_or_else(|| {
                    format!("{name} (needed by {}) is not in the host's library directories", path.display())
                })?;
                self.add(&found)?;
            }
        }
        Ok(())
    }
}

/// Whether the file at `path` starts with the ELF magic.
fn is_elf(path: &Path) -> bool {
    use std::io::Read;
    let mut magic = [0u8; 4];
    std::fs::File::open(path).and_then(|mut f| f.read_exact(&mut magic)).is_ok() && magic == *b"\x7fELF"
}

/// The first of `names` that exists in `/usr/bin` or `/bin`.
pub(super) fn find_program(names: &[&str]) -> Option<PathBuf> {
    ["/usr/bin", "/bin"].iter().flat_map(|d| names.iter().map(move |n| Path::new(d).join(n))).find(|p| p.exists())
}

/// Where the host's GCC keeps its programs (`cc1`, `collect2`) and its
/// libraries (start files, libgcc, its headers): `/usr/{libexec,lib}/gcc/
/// <triple>/<version>`, the version the driver's name ends in if several.
pub(super) fn gcc_dirs(driver: &Path) -> Option<(PathBuf, PathBuf)> {
    let name = driver.file_name()?.to_string_lossy().into_owned();
    let mut found: Vec<(PathBuf, PathBuf)> = Vec::new();
    for base in ["/usr/libexec/gcc", "/usr/lib/gcc"] {
        let Ok(triples) = std::fs::read_dir(base) else { continue };
        for triple in triples.filter_map(|e| e.ok()) {
            let Ok(versions) = std::fs::read_dir(triple.path()) else { continue };
            for version in versions.filter_map(|e| e.ok()) {
                let exec = version.path();
                if !exec.join("cc1").is_file() {
                    continue;
                }
                let lib = Path::new("/usr/lib/gcc").join(triple.file_name()).join(version.file_name());
                if lib.join("crtbegin.o").is_file() {
                    found.push((exec, lib));
                }
            }
        }
    }
    found.sort();
    let suffix = |dirs: &(PathBuf, PathBuf)| {
        let v = dirs.1.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        name.ends_with(&format!("-{v}"))
    };
    found.iter().find(|d| suffix(d)).cloned().or_else(|| found.last().cloned())
}

/// The headers `roots` include, directly or not, in the compiler's search
/// directories `dirs`: every file a `#include` or `#include_next` line can
/// name (conditionals are not evaluated, so this is a superset).
pub(super) fn header_closure(roots: &[&str], dirs: &[PathBuf]) -> Vec<PathBuf> {
    let mut seen: BTreeSet<PathBuf> = BTreeSet::new();
    let mut queue: VecDeque<PathBuf> = VecDeque::new();
    let resolve = |name: &str, from: Option<&Path>, out: &mut Vec<PathBuf>| {
        if let Some(dir) = from {
            let p = dir.join(name);
            if p.is_file() {
                out.push(p);
            }
        }
        for dir in dirs {
            let p = dir.join(name);
            if p.is_file() {
                out.push(p);
            }
        }
    };
    let mut first = Vec::new();
    for root in roots {
        resolve(root, None, &mut first);
    }
    for p in first {
        if seen.insert(p.clone()) {
            queue.push_back(p);
        }
    }
    while let Some(file) = queue.pop_front() {
        let Ok(text) = std::fs::read(&file) else { continue };
        let text = String::from_utf8_lossy(&text);
        for line in text.lines() {
            let Some(rest) = line.trim_start().strip_prefix('#') else { continue };
            let rest = rest.trim_start();
            let Some(rest) = rest.strip_prefix("include_next").or_else(|| rest.strip_prefix("include")) else {
                continue;
            };
            let rest = rest.trim();
            let (name, quoted) = if let Some(r) = rest.strip_prefix('<') {
                (r.split('>').next().unwrap_or_default(), false)
            } else if let Some(r) = rest.strip_prefix('"') {
                (r.split('"').next().unwrap_or_default(), true)
            } else {
                continue;
            };
            if name.is_empty() {
                continue;
            }
            let mut out = Vec::new();
            resolve(name, quoted.then(|| file.parent()).flatten(), &mut out);
            for p in out {
                if seen.insert(p.clone()) {
                    queue.push_back(p);
                }
            }
        }
    }
    seen.into_iter().collect()
}

/// The absolute paths a linker script (`libc.so`) names.
pub(super) fn script_paths(path: &Path) -> Vec<PathBuf> {
    let Ok(text) = std::fs::read_to_string(path) else { return Vec::new() };
    text.split(|c: char| c.is_whitespace() || c == '(' || c == ')')
        .filter(|w| w.starts_with('/') && !w.starts_with("/*"))
        .map(PathBuf::from)
        .collect()
}
