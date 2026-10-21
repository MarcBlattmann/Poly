//! `TIOCGPTPEER` (ioctl_tty(2)): a pseudo-terminal master opens its slave
//! and returns the new descriptor, as an open of `/dev/pts/<n>` with the
//! flags in the argument would (ioctl's spec sheet: `EIO` while the slave
//! is locked; `O_NOCTTY` and the other open flags as for open(2)).

use super::{Thread, vfs_errno};
use alloc::format;
use poly_personality_linux::errno::Errno;
use poly_vfs::OpenFlags;

/// The flags `TIOCGPTPEER` passes on to the open (access mode,
/// `O_NOCTTY`, `O_NONBLOCK`, `O_CLOEXEC`; `asm-generic/fcntl.h`).
const PEER_FLAGS: u32 = 0o3 | 0o400 | 0o4000 | 0o2000000;

impl Thread<'_> {
    /// `ioctl(fd, TIOCGPTPEER, flags)`; `None` if `fd` is not a pty master
    /// (the request then goes to the device, which refuses it).
    pub(super) fn pty_peer(&mut self, fd: i32, flags: u64) -> Option<Result<u64, Errno>> {
        let file = self.process.files.file(fd).ok()?;
        let index = file.device()?.pty_index()?;
        let flags = OpenFlags::from_bits(flags as u32 & PEER_FLAGS);
        let path = format!("/dev/pts/{index}");
        Some(self.process.files.open(path.as_bytes(), flags, 0).map(|fd| fd as u64).map_err(vfs_errno))
    }
}
