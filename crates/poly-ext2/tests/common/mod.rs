//! Helpers shared by the integration tests: an in-memory disk that can
//! record its writes, a clock, the host's e2fsprogs, and tree walking over
//! any `FileSystem`.

#![allow(dead_code)]

use poly_block::IoError;
use poly_ext2::BlockIo;
use poly_vfs::{Clock, DirEntry, Errno, FileSystem, FileType, Ino, Timespec};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::sync::atomic::{AtomicI64, Ordering};

/// What a recording disk saw.
#[derive(Clone, Debug)]
pub enum Event {
    Write(u64, Vec<u8>),
    Flush,
}

/// A disk in a `Vec`, with a configurable block size; optionally records
/// every write and flush (for crash tests).
pub struct MemDisk {
    pub data: Mutex<Vec<u8>>,
    pub sector: u32,
    pub log: Option<Mutex<Vec<Event>>>,
    pub read_only: bool,
}

impl MemDisk {
    pub fn new(data: Vec<u8>, sector: u32) -> MemDisk {
        MemDisk { data: Mutex::new(data), sector, log: None, read_only: false }
    }

    pub fn recording(data: Vec<u8>, sector: u32) -> MemDisk {
        MemDisk { data: Mutex::new(data), sector, log: Some(Mutex::new(Vec::new())), read_only: false }
    }

    pub fn image(&self) -> Vec<u8> {
        self.data.lock().unwrap().clone()
    }

    pub fn events(&self) -> Vec<Event> {
        self.log.as_ref().map(|l| l.lock().unwrap().clone()).unwrap_or_default()
    }
}

impl BlockIo for MemDisk {
    fn block_size(&self) -> u32 {
        self.sector
    }
    fn blocks(&self) -> u64 {
        self.data.lock().unwrap().len() as u64 / self.sector as u64
    }
    fn read_only(&self) -> bool {
        self.read_only
    }
    fn read(&self, first: u64, buf: &mut [u8]) -> Result<(), IoError> {
        let d = self.data.lock().unwrap();
        let at = (first * self.sector as u64) as usize;
        if !buf.len().is_multiple_of(self.sector as usize) || at + buf.len() > d.len() {
            return Err(IoError::OutOfRange);
        }
        buf.copy_from_slice(&d[at..at + buf.len()]);
        Ok(())
    }
    fn write(&self, first: u64, buf: &[u8]) -> Result<(), IoError> {
        if self.read_only {
            return Err(IoError::ReadOnly);
        }
        let mut d = self.data.lock().unwrap();
        let at = (first * self.sector as u64) as usize;
        if !buf.len().is_multiple_of(self.sector as usize) || at + buf.len() > d.len() {
            return Err(IoError::OutOfRange);
        }
        d[at..at + buf.len()].copy_from_slice(buf);
        if let Some(l) = &self.log {
            l.lock().unwrap().push(Event::Write(at as u64, buf.to_vec()));
        }
        Ok(())
    }
    fn flush(&self) -> Result<(), IoError> {
        if let Some(l) = &self.log {
            l.lock().unwrap().push(Event::Flush);
        }
        Ok(())
    }
}

/// A clock that moves one second per reading, from 1.7e9.
pub struct Ticker(AtomicI64);

impl Ticker {
    pub fn new() -> std::sync::Arc<Ticker> {
        std::sync::Arc::new(Ticker(AtomicI64::new(1_700_000_000)))
    }
}

impl Clock for Ticker {
    fn now(&self) -> Timespec {
        Timespec::new(self.0.fetch_add(1, Ordering::Relaxed), 123_456_789)
    }
}

/// A small deterministic random number generator (xorshift64*).
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Rng {
        Rng(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1)
    }
    pub fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }
    pub fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
    pub fn chance(&mut self, percent: u64) -> bool {
        self.below(100) < percent
    }
}

pub fn pattern(seed: u64, len: usize) -> Vec<u8> {
    let mut r = Rng::new(seed);
    (0..len).map(|_| r.next() as u8).collect()
}

// ----- the host's e2fsprogs -----

fn find_tool(name: &str) -> Option<PathBuf> {
    let candidates = [PathBuf::from(name), Path::new("/sbin").join(name), Path::new("/usr/sbin").join(name)];
    candidates
        .into_iter()
        .find(|c| Command::new(c).arg("-V").stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok())
}

pub struct Tools {
    pub mke2fs: PathBuf,
    pub debugfs: PathBuf,
    pub e2fsck: PathBuf,
}

/// The host's e2fsprogs, if installed. Tests print a note and skip the
/// checks that need them otherwise.
pub fn tools() -> Option<Tools> {
    Some(Tools { mke2fs: find_tool("mke2fs")?, debugfs: find_tool("debugfs")?, e2fsck: find_tool("e2fsck")? })
}

/// A directory removed when dropped.
pub struct TempDir(pub PathBuf);

impl TempDir {
    pub fn new(tag: &str) -> TempDir {
        let p = std::env::temp_dir().join(format!("poly-ext2-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        TempDir(p)
    }
    pub fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

impl Tools {
    /// `mke2fs -t ext2` on a new image of `size` bytes; `args` go before the
    /// image name.
    pub fn mkfs(&self, dir: &TempDir, size: u64, args: &[&str]) -> Vec<u8> {
        self.mkfs_type(dir, size, "ext2", args)
    }

    /// `mke2fs -t <fs_type>` (`ext4`: the host's default ext4 features).
    pub fn mkfs_type(&self, dir: &TempDir, size: u64, fs_type: &str, args: &[&str]) -> Vec<u8> {
        let img = dir.path("mkfs.img");
        let _ = std::fs::remove_file(&img);
        std::fs::File::create(&img).unwrap().set_len(size).unwrap();
        let out = Command::new(&self.mke2fs)
            .args(["-q", "-F", "-t", fs_type])
            .args(args)
            .arg(&img)
            .output()
            .unwrap();
        assert!(out.status.success(), "mke2fs {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        std::fs::read(&img).unwrap()
    }

    /// Runs debugfs commands on an image (writable), returns its output.
    pub fn debugfs(&self, dir: &TempDir, image: &mut Vec<u8>, cmds: &str) -> String {
        let img = dir.path("debugfs.img");
        std::fs::write(&img, &*image).unwrap();
        let script = dir.path("debugfs.cmds");
        std::fs::write(&script, cmds).unwrap();
        let out = Command::new(&self.debugfs).arg("-w").arg("-f").arg(&script).arg(&img).output().unwrap();
        *image = std::fs::read(&img).unwrap();
        String::from_utf8_lossy(&out.stdout).into_owned() + &String::from_utf8_lossy(&out.stderr)
    }

    /// `e2fsck` with `flags` (`-fn` checks only); exit code and output.
    pub fn fsck(&self, dir: &TempDir, image: &mut Vec<u8>, flags: &str) -> (i32, String) {
        let img = dir.path("fsck.img");
        std::fs::write(&img, &*image).unwrap();
        let out = Command::new(&self.e2fsck).arg(flags).arg(&img).output().unwrap();
        *image = std::fs::read(&img).unwrap();
        let text = String::from_utf8_lossy(&out.stdout).into_owned() + &String::from_utf8_lossy(&out.stderr);
        (out.status.code().unwrap_or(-1), text)
    }

    /// Asserts that e2fsck finds nothing to fix.
    pub fn assert_clean(&self, dir: &TempDir, image: &[u8], what: &str) {
        let mut img = image.to_vec();
        let (code, out) = self.fsck(dir, &mut img, "-fn");
        assert_eq!(code, 0, "e2fsck -fn after {what}:\n{out}");
    }

    /// Dumps the whole tree to a host directory with `rdump`.
    pub fn rdump(&self, dir: &TempDir, image: &[u8], to: &Path) {
        let _ = std::fs::remove_dir_all(to);
        std::fs::create_dir_all(to).unwrap();
        let mut img = image.to_vec();
        let out = self.debugfs(dir, &mut img, &format!("rdump / {}\n", to.display()));
        // Without root, rdump cannot give the files their owners and says so
        // for each one; the tests compare contents and modes, not owners.
        let errors: Vec<&str> =
            out.lines().filter(|l| l.contains("rdump: ") && !l.contains("while changing ownership")).collect();
        assert!(errors.is_empty(), "rdump failed: {out}");
    }
}

// ----- trees over FileSystem -----

/// A node as the tests compare it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Node {
    File { perm: u16, data: Vec<u8> },
    Dir { perm: u16 },
    Symlink(Vec<u8>),
    Special { kind: FileType, perm: u16, rdev: (u32, u32) },
}

pub fn read_all(fs: &dyn FileSystem, ino: Ino) -> Result<Vec<u8>, Errno> {
    let size = fs.getattr(ino)?.size as usize;
    let mut buf = vec![0u8; size];
    let mut done = 0;
    while done < size {
        let n = fs.read(ino, done as u64, &mut buf[done..])?;
        if n == 0 {
            break;
        }
        done += n;
    }
    buf.truncate(done);
    Ok(buf)
}

pub fn list(fs: &dyn FileSystem, dir: Ino) -> Result<Vec<(Vec<u8>, Ino, FileType)>, Errno> {
    let mut out = Vec::new();
    fs.readdir(dir, 0, &mut |e: &DirEntry<'_>| {
        if e.name != b"." && e.name != b".." {
            out.push((e.name.to_vec(), e.ino, e.kind));
        }
        true
    })?;
    out.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(out)
}

/// Every node below `dir`, by path; bounded so a corrupted tree with
/// cycles still ends.
pub fn walk(
    fs: &dyn FileSystem,
    dir: Ino,
    prefix: &str,
    out: &mut BTreeMap<String, Node>,
    budget: &mut usize,
) -> Result<(), Errno> {
    for (name, ino, _) in list(fs, dir)? {
        if *budget == 0 {
            return Ok(());
        }
        *budget -= 1;
        let path = format!("{prefix}/{}", String::from_utf8_lossy(&name));
        let a = fs.getattr(ino)?;
        let node = match a.kind {
            FileType::Regular => Node::File { perm: a.perm, data: read_all(fs, ino)? },
            FileType::Directory => Node::Dir { perm: a.perm },
            FileType::Symlink => Node::Symlink(fs.readlink(ino)?),
            k => Node::Special { kind: k, perm: a.perm, rdev: (a.rdev.major, a.rdev.minor) },
        };
        let is_dir = matches!(node, Node::Dir { .. });
        out.insert(path.clone(), node);
        if is_dir && prefix.matches('/').count() < 12 {
            walk(fs, ino, &path, out, budget)?;
        }
    }
    Ok(())
}

pub fn tree(fs: &dyn FileSystem) -> BTreeMap<String, Node> {
    let mut out = BTreeMap::new();
    let mut budget = usize::MAX;
    walk(fs, fs.root(), "", &mut out, &mut budget).unwrap();
    out
}

/// The tree in a host directory, as `tree` would show it (symbolic links
/// not followed; special files skipped).
pub fn host_tree(root: &Path) -> BTreeMap<String, Node> {
    use std::os::unix::fs::PermissionsExt;
    fn go(root: &Path, dir: &Path, out: &mut BTreeMap<String, Node>) {
        let mut entries: Vec<_> = std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().path()).collect();
        entries.sort();
        for p in entries {
            let rel = format!("/{}", p.strip_prefix(root).unwrap().display());
            let m = std::fs::symlink_metadata(&p).unwrap();
            let perm = (m.permissions().mode() & 0o7777) as u16;
            if m.file_type().is_symlink() {
                use std::os::unix::ffi::OsStrExt;
                out.insert(rel, Node::Symlink(std::fs::read_link(&p).unwrap().as_os_str().as_bytes().to_vec()));
            } else if m.is_dir() {
                out.insert(rel, Node::Dir { perm });
                go(root, &p, out);
            } else if m.is_file() {
                out.insert(rel, Node::File { perm, data: std::fs::read(&p).unwrap() });
            }
        }
    }
    let mut out = BTreeMap::new();
    go(root, root, &mut out);
    out
}

/// The inode `path` names (no symbolic links followed).
pub fn resolve(fs: &dyn FileSystem, path: &str) -> Result<Ino, Errno> {
    let mut ino = fs.root();
    for c in path.split('/').filter(|c| !c.is_empty()) {
        ino = fs.lookup(ino, c.as_bytes())?;
    }
    Ok(ino)
}

/// Splits "a/b/c" into the directory inode of "a/b" and "c".
pub fn parent<'p>(fs: &dyn FileSystem, path: &'p str) -> Result<(Ino, &'p str), Errno> {
    let (dir, name) = path.rsplit_once('/').unwrap_or(("", path));
    Ok((resolve(fs, dir)?, name))
}
