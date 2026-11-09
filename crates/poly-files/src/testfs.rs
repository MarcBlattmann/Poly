//! Test file systems: [`MemFs`], an in-memory fake with permission bits,
//! symbolic links, modification times and several devices, and [`StdFs`],
//! `std::fs` over a fresh temp directory that stands in for `/`.

use alloc::collections::BTreeMap;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::cell::RefCell;

use crate::fs::{Fs, FsError, Kind, Meta, RawEntry};
use crate::path::{file_name, join, normalize, parent};

#[derive(Clone, Debug)]
enum Node {
    File { data: Vec<u8>, mode: u16, modified: Option<u64> },
    Dir { mode: u16 },
    Link(String),
}

/// An in-memory file system.
#[derive(Debug)]
pub struct MemFs {
    nodes: RefCell<BTreeMap<String, Node>>,
    /// Paths that are mount points of other devices.
    devices: RefCell<Vec<String>>,
}

impl MemFs {
    pub fn new() -> Self {
        let mut nodes = BTreeMap::new();
        nodes.insert("/".to_string(), Node::Dir { mode: 0o755 });
        MemFs { nodes: RefCell::new(nodes), devices: RefCell::new(Vec::new()) }
    }

    /// Makes `path` (an existing folder) the root of another device.
    pub fn add_device(&self, path: &str) {
        self.devices.borrow_mut().push(normalize(path));
    }

    pub fn symlink(&self, path: &str, target: &str) {
        self.nodes.borrow_mut().insert(normalize(path), Node::Link(target.into()));
    }

    pub fn set_modified(&self, path: &str, t: u64) {
        let p = self.resolve(path, true).unwrap();
        if let Some(Node::File { modified, .. }) = self.nodes.borrow_mut().get_mut(&p) {
            *modified = Some(t);
        }
    }

    /// Writes files (and their folders) in one go.
    pub fn with(self, files: &[(&str, &str)]) -> Self {
        for (path, data) in files {
            if let Some(dir) = path.strip_suffix('/') {
                crate::fs::create_dir_all(&self, dir).unwrap();
            } else {
                crate::fs::create_dir_all(&self, parent(path).unwrap()).unwrap();
                self.write(path, data.as_bytes()).unwrap();
            }
        }
        self
    }

    fn device(&self, path: &str) -> u64 {
        let devs = self.devices.borrow();
        let mut best = (0, 0);
        for (i, d) in devs.iter().enumerate() {
            if crate::path::is_inside(path, d) && d.len() >= best.1 {
                best = (i as u64 + 1, d.len());
            }
        }
        best.0
    }

    /// The path `path` names with symbolic links resolved (the last
    /// component only if `follow_last`); the last component need not exist.
    fn resolve(&self, path: &str, follow_last: bool) -> Result<String, FsError> {
        let nodes = self.nodes.borrow();
        let mut cur = String::from("/");
        let mut rest: Vec<String> =
            normalize(path).split('/').filter(|c| !c.is_empty()).rev().map(String::from).collect();
        let mut hops = 0;
        while let Some(c) = rest.pop() {
            let cand = join(&cur, &c);
            match nodes.get(&cand) {
                Some(Node::Link(t)) if follow_last || !rest.is_empty() => {
                    hops += 1;
                    if hops > 40 {
                        return Err(FsError::Other);
                    }
                    let target = join(&cur, t);
                    rest.extend(target.split('/').filter(|c| !c.is_empty()).rev().map(String::from));
                    cur = String::from("/");
                }
                Some(Node::File { .. } | Node::Link(_)) if !rest.is_empty() => return Err(FsError::NotADirectory),
                None if !rest.is_empty() => return Err(FsError::NotFound),
                _ => cur = cand,
            }
        }
        Ok(cur)
    }

    fn meta(&self, p: &str, node: &Node) -> Meta {
        let device = self.device(p);
        match node {
            Node::File { data, mode, modified } => {
                Meta { kind: Kind::File, size: data.len() as u64, mode: *mode, device, modified: *modified }
            }
            Node::Dir { mode } => Meta { kind: Kind::Dir, size: 0, mode: *mode, device, modified: None },
            Node::Link(t) => Meta { kind: Kind::Symlink, size: t.len() as u64, mode: 0o777, device, modified: None },
        }
    }

    /// Checks that the parent of resolved path `p` is a folder.
    fn dir_of(&self, p: &str) -> Result<(), FsError> {
        let parent = parent(p).ok_or(FsError::Invalid)?;
        match self.nodes.borrow().get(parent) {
            Some(Node::Dir { .. }) => Ok(()),
            Some(_) => Err(FsError::NotADirectory),
            None => Err(FsError::NotFound),
        }
    }

    fn children(&self, dir: &str) -> Vec<String> {
        let prefix = if dir == "/" { "/".to_string() } else { alloc::format!("{dir}/") };
        self.nodes
            .borrow()
            .keys()
            .filter(|k| k.len() > prefix.len() && k.starts_with(&prefix) && !k[prefix.len()..].contains('/'))
            .cloned()
            .collect()
    }
}

impl Fs for MemFs {
    fn read_dir(&self, path: &str) -> Result<Vec<RawEntry>, FsError> {
        let p = self.resolve(path, true)?;
        if !matches!(self.nodes.borrow().get(&p), Some(Node::Dir { .. })) {
            return Err(FsError::NotADirectory);
        }
        let nodes = self.nodes.borrow();
        Ok(self
            .children(&p)
            .into_iter()
            .map(|c| {
                let kind = match nodes.get(&c) {
                    Some(Node::Dir { .. }) => Kind::Dir,
                    Some(Node::Link(_)) => Kind::Symlink,
                    _ => Kind::File,
                };
                RawEntry { name: file_name(&c).to_string(), kind }
            })
            .collect())
    }

    fn stat(&self, path: &str) -> Result<Meta, FsError> {
        let p = self.resolve(path, true)?;
        let nodes = self.nodes.borrow();
        let n = nodes.get(&p).ok_or(FsError::NotFound)?;
        Ok(self.meta(&p, n))
    }

    fn lstat(&self, path: &str) -> Result<Meta, FsError> {
        let p = self.resolve(path, false)?;
        let nodes = self.nodes.borrow();
        let n = nodes.get(&p).ok_or(FsError::NotFound)?;
        Ok(self.meta(&p, n))
    }

    fn read_head(&self, path: &str, max: usize) -> Result<Vec<u8>, FsError> {
        let p = self.resolve(path, true)?;
        match self.nodes.borrow().get(&p) {
            None => Err(FsError::NotFound),
            Some(Node::File { data, .. }) => Ok(data[..data.len().min(max)].to_vec()),
            _ => Err(FsError::IsADirectory),
        }
    }

    fn write(&self, path: &str, data: &[u8]) -> Result<(), FsError> {
        let p = self.resolve(path, true)?;
        self.dir_of(&p)?;
        let mut nodes = self.nodes.borrow_mut();
        let mode = match nodes.get(&p) {
            Some(Node::Dir { .. }) => return Err(FsError::IsADirectory),
            Some(Node::File { mode, .. }) => *mode,
            _ => 0o644,
        };
        nodes.insert(p, Node::File { data: data.to_vec(), mode, modified: None });
        Ok(())
    }

    fn create_new(&self, path: &str, data: &[u8]) -> Result<(), FsError> {
        if self.exists(path) {
            return Err(FsError::AlreadyExists);
        }
        self.write(path, data)
    }

    fn create_dir(&self, path: &str) -> Result<(), FsError> {
        let p = self.resolve(path, false)?;
        if self.exists(&p) {
            return Err(FsError::AlreadyExists);
        }
        self.dir_of(&p)?;
        self.nodes.borrow_mut().insert(p, Node::Dir { mode: 0o755 });
        Ok(())
    }

    fn rename(&self, from: &str, to: &str) -> Result<(), FsError> {
        let (from, to) = (self.resolve(from, false)?, self.resolve(to, false)?);
        let src = self.lstat(&from)?;
        self.dir_of(&to)?;
        if self.device(&from) != self.device(parent(&to).unwrap_or("/")) {
            return Err(FsError::CrossDevice);
        }
        if src.kind == Kind::Dir && crate::path::is_inside(&to, &from) && to != from {
            return Err(FsError::Invalid);
        }
        if let Ok(dst) = self.lstat(&to)
            && dst.kind == Kind::Dir
            && !self.children(&to).is_empty()
        {
            return Err(FsError::NotEmpty);
        }
        let mut nodes = self.nodes.borrow_mut();
        let prefix = alloc::format!("{from}/");
        let moved: Vec<String> = nodes.keys().filter(|k| **k == from || k.starts_with(&prefix)).cloned().collect();
        for k in moved {
            if let Some(n) = nodes.remove(&k) {
                nodes.insert(alloc::format!("{to}{}", &k[from.len()..]), n);
            }
        }
        Ok(())
    }

    fn remove_file(&self, path: &str) -> Result<(), FsError> {
        let p = self.resolve(path, false)?;
        let mut nodes = self.nodes.borrow_mut();
        match nodes.get(&p) {
            None => Err(FsError::NotFound),
            Some(Node::Dir { .. }) => Err(FsError::IsADirectory),
            Some(_) => {
                nodes.remove(&p);
                Ok(())
            }
        }
    }

    fn remove_dir(&self, path: &str) -> Result<(), FsError> {
        let p = self.resolve(path, false)?;
        match self.nodes.borrow().get(&p) {
            None => return Err(FsError::NotFound),
            Some(Node::Dir { .. }) => {}
            Some(_) => return Err(FsError::NotADirectory),
        }
        if !self.children(&p).is_empty() {
            return Err(FsError::NotEmpty);
        }
        self.nodes.borrow_mut().remove(&p);
        Ok(())
    }

    fn set_mode(&self, path: &str, new: u16) -> Result<(), FsError> {
        let p = self.resolve(path, true)?;
        match self.nodes.borrow_mut().get_mut(&p) {
            Some(Node::File { mode, .. } | Node::Dir { mode }) => {
                *mode = new;
                Ok(())
            }
            _ => Err(FsError::NotFound),
        }
    }
}

/// `std::fs` over a fresh temp directory: Poly path `/a/b` is
/// `<temp>/a/b`. Removed when dropped.
#[derive(Debug)]
pub struct StdFs {
    root: std::path::PathBuf,
}

impl StdFs {
    pub fn new() -> Self {
        static COUNTER: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);
        let n = COUNTER.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
        let root = std::env::temp_dir().join(alloc::format!("poly-files-test-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        StdFs { root }
    }

    fn real(&self, path: &str) -> std::path::PathBuf {
        let p = normalize(path);
        self.root.join(p.trim_start_matches('/'))
    }
}

impl Drop for StdFs {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn err(e: std::io::Error) -> FsError {
    use std::io::ErrorKind as E;
    match e.kind() {
        E::NotFound => FsError::NotFound,
        E::PermissionDenied => FsError::PermissionDenied,
        E::AlreadyExists => FsError::AlreadyExists,
        E::NotADirectory => FsError::NotADirectory,
        E::IsADirectory => FsError::IsADirectory,
        E::DirectoryNotEmpty => FsError::NotEmpty,
        E::CrossesDevices => FsError::CrossDevice,
        E::StorageFull => FsError::NoSpace,
        E::InvalidInput => FsError::Invalid,
        _ => FsError::Other,
    }
}

fn kind(t: std::fs::FileType) -> Kind {
    if t.is_symlink() {
        Kind::Symlink
    } else if t.is_dir() {
        Kind::Dir
    } else if t.is_file() {
        Kind::File
    } else {
        Kind::Other
    }
}

fn meta(m: std::fs::Metadata) -> Meta {
    #[cfg(unix)]
    let mode = {
        use std::os::unix::fs::PermissionsExt;
        (m.permissions().mode() & 0o7777) as u16
    };
    #[cfg(not(unix))]
    let mode = if m.permissions().readonly() { 0o444 } else { 0o644 };
    let modified = m.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs());
    Meta { kind: kind(m.file_type()), size: m.len(), mode, device: 0, modified }
}

impl Fs for StdFs {
    fn read_dir(&self, path: &str) -> Result<Vec<RawEntry>, FsError> {
        let mut out = Vec::new();
        for e in std::fs::read_dir(self.real(path)).map_err(err)? {
            let e = e.map_err(err)?;
            let name = e.file_name().to_string_lossy().into_owned();
            out.push(RawEntry { name, kind: kind(e.file_type().map_err(err)?) });
        }
        Ok(out)
    }

    fn stat(&self, path: &str) -> Result<Meta, FsError> {
        std::fs::metadata(self.real(path)).map(meta).map_err(err)
    }

    fn lstat(&self, path: &str) -> Result<Meta, FsError> {
        std::fs::symlink_metadata(self.real(path)).map(meta).map_err(err)
    }

    fn read_head(&self, path: &str, max: usize) -> Result<Vec<u8>, FsError> {
        use std::io::Read;
        let f = std::fs::File::open(self.real(path)).map_err(err)?;
        let mut out = Vec::new();
        f.take(max as u64).read_to_end(&mut out).map_err(err)?;
        Ok(out)
    }

    fn write(&self, path: &str, data: &[u8]) -> Result<(), FsError> {
        std::fs::write(self.real(path), data).map_err(err)
    }

    fn create_new(&self, path: &str, data: &[u8]) -> Result<(), FsError> {
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new().write(true).create_new(true).open(self.real(path)).map_err(err)?;
        f.write_all(data).map_err(err)
    }

    fn create_dir(&self, path: &str) -> Result<(), FsError> {
        std::fs::create_dir(self.real(path)).map_err(err)
    }

    fn rename(&self, from: &str, to: &str) -> Result<(), FsError> {
        std::fs::rename(self.real(from), self.real(to)).map_err(err)
    }

    fn remove_file(&self, path: &str) -> Result<(), FsError> {
        std::fs::remove_file(self.real(path)).map_err(err)
    }

    fn remove_dir(&self, path: &str) -> Result<(), FsError> {
        std::fs::remove_dir(self.real(path)).map_err(err)
    }

    fn set_mode(&self, path: &str, mode: u16) -> Result<(), FsError> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(self.real(path), std::fs::Permissions::from_mode(u32::from(mode))).map_err(err)
        }
        #[cfg(not(unix))]
        {
            let _ = (path, mode);
            Err(FsError::Unsupported)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memfs_basics() {
        let fs = MemFs::new().with(&[("/a/b.txt", "hi"), ("/a/c/", "")]);
        let mut names: Vec<String> = fs.read_dir("/a").unwrap().into_iter().map(|e| e.name).collect();
        names.sort();
        assert_eq!(names, ["b.txt", "c"]);
        fs.symlink("/a/l", "b.txt");
        assert_eq!(fs.stat("/a/l").unwrap().kind, Kind::File);
        assert_eq!(fs.lstat("/a/l").unwrap().kind, Kind::Symlink);
        fs.rename("/a", "/z").unwrap();
        assert_eq!(fs.read_to_string("/z/b.txt").unwrap(), "hi");
        assert_eq!(fs.remove_dir("/z"), Err(FsError::NotEmpty));
        fs.add_device("/z/c");
        assert_eq!(fs.rename("/z/b.txt", "/z/c/b.txt"), Err(FsError::CrossDevice));
    }

    #[test]
    fn stdfs_basics() {
        let fs = StdFs::new();
        fs.create_dir("/d").unwrap();
        fs.write("/d/f", b"data").unwrap();
        assert_eq!(fs.create_new("/d/f", b"x"), Err(FsError::AlreadyExists));
        assert_eq!(fs.stat("/d/f").unwrap().size, 4);
        assert_eq!(fs.read_head("/d/f", 2).unwrap(), b"da");
        assert_eq!(fs.read_dir("/d").unwrap(), [RawEntry { name: "f".into(), kind: Kind::File }]);
        assert_eq!(fs.stat("/nope"), Err(FsError::NotFound));
        assert!(fs.stat("/d/f").unwrap().modified.is_some());
    }
}
