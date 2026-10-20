//! The shared libraries a dynamically linked program needs, found on the
//! host the way ld.so(8) describes the search: the object's `DT_RPATH` or
//! `DT_RUNPATH`, the directories of `/etc/ld.so.conf`, then `/lib64`,
//! `/usr/lib64`, `/lib`, `/usr/lib`. Only the files' ELF headers are read
//! (`poly_elf::Dynamic`); nothing is run on the host.
//!
//! Used by the dynamic linking boot tests (`dynamic.rs`) and by the agent
//! pipeline's app tests (`tools/pipeline/src/apps.rs`, which includes this
//! file with `#[path]`), so both put the same libraries into their images.
//! Only `std` and `poly_elf` are used here.

use poly_elf::{Dynamic, Elf};
use std::collections::{BTreeMap, VecDeque};
use std::path::{Path, PathBuf};

/// The directories an object asks to be searched first: `DT_RUNPATH`, or
/// `DT_RPATH` without one (ld.so(8)). `$ORIGIN` entries are left out: they
/// only name directories of the image, not of the host.
pub fn own_paths(d: &Dynamic<'_>) -> Vec<PathBuf> {
    let list = d.runpath().or_else(|| d.rpath()).unwrap_or_default();
    String::from_utf8_lossy(list)
        .split(':')
        .filter(|p| !p.is_empty() && !p.contains("$ORIGIN") && !p.contains("${ORIGIN}"))
        .map(PathBuf::from)
        .collect()
}

/// The host's library directories, in search order.
#[derive(Debug)]
pub struct SearchPath {
    pub dirs: Vec<PathBuf>,
}

impl SearchPath {
    /// `/etc/ld.so.conf` (and the files it includes), then the default
    /// directories.
    pub fn host() -> SearchPath {
        let mut dirs = Vec::new();
        read_ld_so_conf(Path::new("/etc/ld.so.conf"), &mut dirs, 0);
        dirs.extend(["/lib64", "/usr/lib64", "/lib", "/usr/lib"].map(PathBuf::from));
        SearchPath { dirs }
    }

    /// The first x86-64 ELF file called `name` in `first` or the search
    /// path, with its contents.
    pub fn find(&self, name: &str, first: &[PathBuf]) -> Option<(PathBuf, Vec<u8>)> {
        if name.contains('/') {
            let bytes = std::fs::read(name).ok()?;
            return Elf::parse(&bytes).is_ok().then(|| (PathBuf::from(name), bytes));
        }
        first.iter().chain(&self.dirs).find_map(|dir| {
            let path = dir.join(name);
            let bytes = std::fs::read(&path).ok()?;
            Elf::parse(&bytes).is_ok().then_some((path, bytes))
        })
    }
}

/// Directories listed in an `ld.so.conf`-style file: one per line, `#`
/// comments, `include <glob>` lines naming more such files (only `*` in
/// the last component is expanded).
pub fn read_ld_so_conf(file: &Path, dirs: &mut Vec<PathBuf>, depth: u32) {
    let Ok(text) = std::fs::read_to_string(file) else { return };
    if depth > 4 {
        return;
    }
    for line in text.lines() {
        let line = line.split('#').next().unwrap_or_default().trim();
        if let Some(pattern) = line.strip_prefix("include") {
            let pattern = Path::new(pattern.trim());
            let dir = pattern.parent().unwrap_or(Path::new("/"));
            let glob = pattern.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let (prefix, suffix) = glob.split_once('*').unwrap_or((&glob, ""));
            let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
                .map(|entries| {
                    entries
                        .filter_map(|e| e.ok().map(|e| e.path()))
                        .filter(|p| {
                            let n = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                            if glob.contains('*') {
                                n.starts_with(prefix) && n.ends_with(suffix) && n.len() >= prefix.len() + suffix.len()
                            } else {
                                n == glob
                            }
                        })
                        .collect()
                })
                .unwrap_or_default();
            files.sort();
            for f in files {
                read_ld_so_conf(&f, dirs, depth + 1);
            }
        } else if line.starts_with('/') {
            dirs.push(PathBuf::from(line));
        }
    }
}

/// The `DT_NEEDED` closure of `objects` (`(name, ELF)`: programs, plugins),
/// breadth first, as the dynamic linker loads it: soname -> where it was
/// found on the host. The interpreter (`skip`, by file name) is left out;
/// statically linked objects need nothing.
pub fn needed_closure(objects: &[(String, Vec<u8>)], skip: &str) -> Result<BTreeMap<String, PathBuf>, String> {
    let search = SearchPath::host();
    let mut queue: VecDeque<(String, Vec<u8>)> = objects.iter().cloned().collect();
    let mut found: BTreeMap<String, PathBuf> = BTreeMap::new();
    while let Some((who, elf)) = queue.pop_front() {
        let parsed = Elf::parse(&elf).map_err(|e| format!("{who}: {e:?}"))?;
        let Some(dynamic) = Dynamic::parse(&parsed).map_err(|e| format!("{who}: {e:?}"))? else { continue };
        let own = own_paths(&dynamic);
        for needed in dynamic.needed() {
            let name = String::from_utf8_lossy(needed).into_owned();
            if found.contains_key(&name) || name == skip {
                continue;
            }
            let (path, bytes) = search
                .find(&name, &own)
                .ok_or_else(|| format!("{name} (needed by {who}) is not in the host's library directories"))?;
            found.insert(name.clone(), path);
            queue.push_back((name, bytes));
        }
    }
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ld_so_conf_lines_and_includes() {
        let dir = std::env::temp_dir().join(format!("poly-ldconf-{}-{}", env!("CARGO_PKG_NAME"), std::process::id()));
        let conf_d = dir.join("conf.d");
        std::fs::create_dir_all(&conf_d).unwrap();
        std::fs::write(conf_d.join("b.conf"), "/opt/b\n").unwrap();
        std::fs::write(conf_d.join("a.conf"), "# a comment\n/opt/a # trailing\n\n").unwrap();
        std::fs::write(conf_d.join("ignored.txt"), "/opt/no\n").unwrap();
        let main = dir.join("ld.so.conf");
        std::fs::write(&main, format!("/first\ninclude {}/*.conf\nrelative/dir\n/last\n", conf_d.display())).unwrap();
        let mut dirs = Vec::new();
        read_ld_so_conf(&main, &mut dirs, 0);
        let _ = std::fs::remove_dir_all(&dir);
        let want: Vec<PathBuf> = ["/first", "/opt/a", "/opt/b", "/last"].iter().map(PathBuf::from).collect();
        assert_eq!(dirs, want);
    }
}
