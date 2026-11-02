//! Files and directories: [`File`] (an open handle, closed when dropped),
//! [`OpenOptions`], [`Metadata`], [`read_dir`], and whole-file helpers, in
//! the shape of `std::fs` for code written against it.

use crate::abi::{self, Handle, Whence, open};
use crate::io::{Read, Write};
use crate::{Error, Result, io, sys};
use alloc::string::String;
use alloc::vec::Vec;

/// An open file (any handle: a regular file, a device, a pipe, a socket),
/// closed when dropped.
#[derive(Debug)]
pub struct File {
    handle: Handle,
}

impl File {
    /// Open `path` for reading.
    pub fn open(path: &str) -> Result<File> {
        File::open_with(path, open::READ, 0)
    }

    /// Create `path` (mode 0644) or cut it to length 0, for writing.
    pub fn create(path: &str) -> Result<File> {
        File::open_with(path, open::WRITE | open::CREATE | open::TRUNCATE, 0o644)
    }

    /// Open `path` with `flags` (`abi::open`) and `mode` for a file it
    /// creates.
    pub fn open_with(path: &str, flags: u32, mode: u16) -> Result<File> {
        sys::open(path.as_bytes(), flags, mode).map(|handle| File { handle })
    }

    /// Take ownership of `handle`: it is closed when the file is dropped.
    pub fn from_handle(handle: Handle) -> File {
        File { handle }
    }

    pub fn handle(&self) -> Handle {
        self.handle
    }

    /// Give up ownership of the handle without closing it.
    pub fn into_handle(self) -> Handle {
        let h = self.handle;
        core::mem::forget(self);
        h
    }

    pub fn write_all(&self, buf: &[u8]) -> Result<()> {
        io::write_all(self.handle, buf)
    }

    pub fn seek(&self, offset: i64, whence: Whence) -> Result<u64> {
        sys::seek(self.handle, offset, whence)
    }

    /// A second handle on the same open file (sharing its position).
    pub fn try_clone(&self) -> Result<File> {
        sys::dup(self.handle).map(File::from_handle)
    }

    pub fn metadata(&self) -> Result<Metadata> {
        sys::handle_info(self.handle).map(Metadata)
    }

    /// Set the file's length.
    pub fn set_len(&self, len: u64) -> Result<()> {
        sys::truncate(self.handle, len)
    }

    /// Write what is cached to the disk (every file system's: Poly has no
    /// per-file sync call yet).
    pub fn sync_all(&self) -> Result<()> {
        sys::sync();
        Ok(())
    }

    pub fn set_nonblocking(&self, on: bool) -> Result<()> {
        sys::set_nonblocking(self.handle, on)
    }
}

impl Read for File {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        sys::read(self.handle, buf)
    }
}

impl Read for &File {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        sys::read(self.handle, buf)
    }
}

impl Write for File {
    fn write(&mut self, buf: &[u8]) -> Result<usize> {
        sys::write(self.handle, buf)
    }
}

impl Write for &File {
    fn write(&mut self, buf: &[u8]) -> Result<usize> {
        sys::write(self.handle, buf)
    }
}

impl Drop for File {
    fn drop(&mut self) {
        let _ = sys::close(self.handle);
    }
}

/// How to open a file, as std's `OpenOptions`.
#[derive(Clone, Copy, Debug, Default)]
pub struct OpenOptions {
    flags: u32,
    mode: u16,
}

impl OpenOptions {
    pub fn new() -> OpenOptions {
        OpenOptions { flags: 0, mode: 0o644 }
    }

    fn set(&mut self, flag: u32, on: bool) -> &mut OpenOptions {
        if on {
            self.flags |= flag;
        } else {
            self.flags &= !flag;
        }
        self
    }

    pub fn read(&mut self, on: bool) -> &mut OpenOptions {
        self.set(open::READ, on)
    }

    pub fn write(&mut self, on: bool) -> &mut OpenOptions {
        self.set(open::WRITE, on)
    }

    pub fn append(&mut self, on: bool) -> &mut OpenOptions {
        self.set(open::APPEND | open::WRITE, on)
    }

    pub fn truncate(&mut self, on: bool) -> &mut OpenOptions {
        self.set(open::TRUNCATE, on)
    }

    pub fn create(&mut self, on: bool) -> &mut OpenOptions {
        self.set(open::CREATE, on)
    }

    /// Create it, failing with `AlreadyExists` if it is there.
    pub fn create_new(&mut self, on: bool) -> &mut OpenOptions {
        self.set(open::CREATE | open::EXCLUSIVE, on)
    }

    pub fn nonblocking(&mut self, on: bool) -> &mut OpenOptions {
        self.set(open::NONBLOCK, on)
    }

    /// The permission bits of a file it creates.
    pub fn mode(&mut self, mode: u16) -> &mut OpenOptions {
        self.mode = mode;
        self
    }

    pub fn open(&self, path: &str) -> Result<File> {
        File::open_with(path, self.flags, self.mode)
    }
}

/// What `stat` tells about a file.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Metadata(pub abi::FileInfo);

impl Metadata {
    pub fn len(&self) -> u64 {
        self.0.size
    }

    pub fn is_empty(&self) -> bool {
        self.0.size == 0
    }

    pub fn is_file(&self) -> bool {
        self.0.kind == abi::kind::FILE
    }

    pub fn is_dir(&self) -> bool {
        self.0.kind == abi::kind::DIRECTORY
    }

    /// Permission bits.
    pub fn mode(&self) -> u16 {
        self.0.mode
    }

    /// The device a device node stands for (`major << 32 | minor`), 0 for
    /// other files.
    pub fn device(&self) -> u64 {
        self.0.device
    }
}

/// The file at `path` (symbolic links followed).
pub fn metadata(path: &str) -> Result<Metadata> {
    sys::stat(path.as_bytes()).map(Metadata)
}

/// Whether `path` names something.
pub fn exists(path: &str) -> bool {
    metadata(path).is_ok()
}

/// The whole file at `path`.
pub fn read(path: &str) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    File::open(path)?.read_to_end(&mut out)?;
    Ok(out)
}

/// The whole file at `path` as text (`InvalidArgument` if it is not UTF-8).
pub fn read_to_string(path: &str) -> Result<String> {
    String::from_utf8(read(path)?).map_err(|_| Error::InvalidArgument)
}

/// Make the file at `path` hold `data` (created with mode 0644 if needed).
pub fn write(path: &str, data: &[u8]) -> Result<()> {
    File::create(path)?.write_all(data)
}

pub fn rename(from: &str, to: &str) -> Result<()> {
    sys::rename(from.as_bytes(), to.as_bytes())
}

pub fn remove_file(path: &str) -> Result<()> {
    sys::remove(path.as_bytes(), false)
}

/// Remove the empty directory at `path`.
pub fn remove_dir(path: &str) -> Result<()> {
    sys::remove(path.as_bytes(), true)
}

/// Remove `path` and everything in it.
pub fn remove_dir_all(path: &str) -> Result<()> {
    for e in read_dir(path)? {
        let p = join(path, &e.name);
        if e.kind == abi::kind::DIRECTORY { remove_dir_all(&p)? } else { remove_file(&p)? }
    }
    remove_dir(path)
}

/// Make the directory `path` (mode 0755).
pub fn create_dir(path: &str) -> Result<()> {
    sys::make_dir(path.as_bytes(), 0o755)
}

/// Make `path` and every missing parent (mode 0755).
pub fn create_dir_all(path: &str) -> Result<()> {
    match metadata(path) {
        Ok(m) if m.is_dir() => return Ok(()),
        Ok(_) => return Err(Error::AlreadyExists),
        Err(_) => {}
    }
    if let Some((parent, _)) = path.trim_end_matches('/').rsplit_once('/')
        && !parent.is_empty()
    {
        create_dir_all(parent)?;
    }
    match create_dir(path) {
        Err(Error::AlreadyExists) if metadata(path).is_ok_and(|m| m.is_dir()) => Ok(()),
        other => other,
    }
}

/// Set the permission bits of `path`.
pub fn set_mode(path: &str, mode: u16) -> Result<()> {
    sys::set_mode(path.as_bytes(), mode)
}

/// Make a named pipe at `path`.
pub fn make_fifo(path: &str, mode: u16) -> Result<()> {
    sys::make_fifo(path.as_bytes(), mode)
}

/// One entry of a directory.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DirEntry {
    pub name: String,
    /// `abi::kind`.
    pub kind: u8,
}

/// The entries of the directory at `path`, without `.` and `..`, in the
/// order the file system lists them; names that are not UTF-8 are left out.
pub fn read_dir(path: &str) -> Result<Vec<DirEntry>> {
    let dir = File::open_with(path, open::READ | open::DIRECTORY, 0)?;
    let mut out = Vec::new();
    let mut buf = alloc::vec![0u8; 4096];
    loop {
        let n = sys::read_dir(dir.handle(), &mut buf)?;
        if n == 0 {
            return Ok(out);
        }
        parse_dir(&buf[..n], &mut out);
    }
}

/// The records of one `read_dir` call.
fn parse_dir(mut buf: &[u8], out: &mut Vec<DirEntry>) {
    while let Some((r, rest)) = abi::DirRecord::parse(buf) {
        if r.name != b"."
            && r.name != b".."
            && let Ok(name) = core::str::from_utf8(r.name)
        {
            out.push(DirEntry { name: name.into(), kind: r.kind });
        }
        buf = rest;
    }
}

/// `dir/name`, with one `/` between them.
pub fn join(dir: &str, name: &str) -> String {
    let mut p = String::from(dir.trim_end_matches('/'));
    p.push('/');
    p.push_str(name.trim_start_matches('/'));
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn directory_records_skip_dot_entries() {
        let mut raw = Vec::new();
        for (kind, name) in
            [(abi::kind::DIRECTORY, &b"."[..]), (abi::kind::DIRECTORY, b".."), (abi::kind::FILE, b"a.desktop")]
        {
            abi::DirRecord { kind, name }.write(&mut raw);
        }
        let mut out = Vec::new();
        parse_dir(&raw, &mut out);
        assert_eq!(out, [DirEntry { name: "a.desktop".into(), kind: abi::kind::FILE }]);
    }

    #[test]
    fn paths_join() {
        assert_eq!(join("/run/user/0/", "/wayland-0"), "/run/user/0/wayland-0");
        assert_eq!(join("/tmp", "x"), "/tmp/x");
    }

    #[test]
    fn calls_fail_on_the_host() {
        assert_eq!(read("/etc/motd"), Err(Error::Unsupported));
        assert!(!exists("/"));
        assert!(OpenOptions::new().write(true).create_new(true).open("/x").is_err());
    }
}
