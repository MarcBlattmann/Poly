//! The few Unix-only host facilities xtask uses (file modes, symbolic
//! links, Unix sockets to QEMU), with stand-ins elsewhere so xtask builds
//! and runs on Windows too: building and booting the kernel work on any
//! host, while the tests that need a Linux host (root disks assembled from
//! its packages, screenshots and input over QEMU's Unix sockets) report
//! that instead of breaking the build.

use std::fs::Metadata;
use std::io;
use std::path::Path;

/// The message of every stand-in that cannot work on this host.
#[cfg(not(unix))]
const NEEDS_UNIX: &str = "this needs a Linux host (run it in the poly-dev container or WSL)";

/// A file's permission bits (`st_mode & 07777`).
#[cfg(unix)]
pub fn mode(meta: &Metadata) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    meta.permissions().mode() & 0o7777
}

/// A file's permission bits: what a Unix host would most likely show.
#[cfg(not(unix))]
pub fn mode(meta: &Metadata) -> u32 {
    match (meta.is_dir(), meta.permissions().readonly()) {
        (true, _) => 0o755,
        (false, true) => 0o444,
        (false, false) => 0o644,
    }
}

/// A file's modification time in seconds since the epoch.
#[cfg(unix)]
pub fn mtime(meta: &Metadata) -> i64 {
    use std::os::unix::fs::MetadataExt;
    meta.mtime()
}

/// A file's modification time in seconds since the epoch.
#[cfg(not(unix))]
pub fn mtime(meta: &Metadata) -> i64 {
    meta.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs() as i64)
}

/// Set a file's permission bits.
#[cfg(unix)]
pub fn set_mode(path: &Path, mode: u32) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
}

/// Set a file's permission bits: the host has none to set.
#[cfg(not(unix))]
pub fn set_mode(_path: &Path, _mode: u32) -> io::Result<()> {
    Ok(())
}

/// Make a symbolic link `link` to `target`.
#[cfg(unix)]
pub fn symlink(target: impl AsRef<Path>, link: &Path) -> io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

/// Make a symbolic link `link` to `target`.
#[cfg(not(unix))]
pub fn symlink(_target: impl AsRef<Path>, _link: &Path) -> io::Result<()> {
    Err(io::Error::new(io::ErrorKind::Unsupported, NEEDS_UNIX))
}

/// Whether this process runs as root (`/proc/self` belongs to its user).
#[cfg(unix)]
pub fn running_as_root() -> bool {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata("/proc/self").is_ok_and(|m| m.uid() == 0)
}

/// Whether this process runs as root: never, on this host.
#[cfg(not(unix))]
pub fn running_as_root() -> bool {
    false
}

#[cfg(unix)]
pub use std::os::unix::net::UnixStream;

/// A Unix socket stream: QEMU's QMP, monitor and VNC sockets. On this host
/// `connect` always fails, so no value of it ever exists.
#[cfg(not(unix))]
pub struct UnixStream(std::convert::Infallible);

#[cfg(not(unix))]
impl UnixStream {
    pub fn connect(_path: impl AsRef<Path>) -> io::Result<UnixStream> {
        Err(io::Error::new(io::ErrorKind::Unsupported, NEEDS_UNIX))
    }

    pub fn try_clone(&self) -> io::Result<UnixStream> {
        match self.0 {}
    }

    pub fn set_read_timeout(&self, _timeout: Option<std::time::Duration>) -> io::Result<()> {
        match self.0 {}
    }
}

#[cfg(not(unix))]
impl io::Read for UnixStream {
    fn read(&mut self, _buf: &mut [u8]) -> io::Result<usize> {
        match self.0 {}
    }
}

#[cfg(not(unix))]
impl io::Write for UnixStream {
    fn write(&mut self, _buf: &[u8]) -> io::Result<usize> {
        match self.0 {}
    }

    fn flush(&mut self) -> io::Result<()> {
        match self.0 {}
    }
}
