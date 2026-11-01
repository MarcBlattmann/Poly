//! The Linux calls the compositor makes: shared memory mappings, messages
//! with file descriptors on Unix sockets (`SCM_RIGHTS`, unix(7),
//! cmsg(3)), `poll`, and starting and waiting for a client.

use poly_wayland::server::{Host, ShmMap};
use poly_wayland::wire::Fd;
use std::ffi::CString;
use std::io;
use std::os::fd::RawFd;

/// The most file descriptors one message may carry (as many as the
/// receive buffer below has room for).
pub const MAX_FDS: usize = 28;

/// A client's shared memory, mapped read-only and shared.
#[derive(Debug)]
pub struct Mapping {
    ptr: *mut u8,
    len: usize,
    fd: RawFd,
}

impl Mapping {
    fn map(fd: RawFd, len: usize) -> Option<Mapping> {
        // SAFETY: a new mapping chosen by the kernel (no fixed address), of a
        // file the client sent; nothing else refers to that range.
        let ptr = unsafe { libc::mmap(std::ptr::null_mut(), len, libc::PROT_READ, libc::MAP_SHARED, fd, 0) };
        if ptr == libc::MAP_FAILED {
            return None;
        }
        Some(Mapping { ptr: ptr.cast(), len, fd })
    }

    fn unmap(&mut self) {
        if self.len > 0 {
            // SAFETY: `ptr`/`len` are exactly what mmap returned; no
            // reference into the range outlives this call (reads copy out).
            unsafe { libc::munmap(self.ptr.cast(), self.len) };
            self.len = 0;
        }
    }
}

impl ShmMap for Mapping {
    fn len(&self) -> usize {
        self.len
    }

    fn resize(&mut self, size: usize) -> bool {
        match Mapping::map(self.fd, size) {
            Some(new) => {
                self.unmap();
                self.ptr = new.ptr;
                self.len = new.len;
                std::mem::forget(new);
                true
            }
            None => false,
        }
    }

    fn read(&self, offset: usize, dst: &mut [u8]) -> bool {
        if offset.checked_add(dst.len()).is_none_or(|end| end > self.len) {
            return false;
        }
        // SAFETY: the range is inside the mapping (checked above), which
        // stays mapped while `self` lives. The client may write to it at the
        // same time: a torn copy only shows a half-drawn frame, which the
        // protocol leaves to the client (it must not touch a committed
        // buffer before the release).
        unsafe { std::ptr::copy_nonoverlapping(self.ptr.add(offset), dst.as_mut_ptr(), dst.len()) };
        true
    }
}

impl Drop for Mapping {
    fn drop(&mut self) {
        self.unmap();
        close(self.fd);
    }
}

pub fn close(fd: RawFd) {
    // SAFETY: closing a descriptor the compositor owns and no longer uses.
    unsafe { libc::close(fd) };
}

/// The keymap file (read-only, unlinked) and its size, shared between the
/// host and the compositor, which replaces it when the layout changes
/// (ADR 0117).
pub type SharedKeymap = std::rc::Rc<std::cell::Cell<Option<(RawFd, u32)>>>;

/// The [`Host`] on Linux: `mmap`, `close`, and the keymap file.
#[derive(Debug, Default)]
pub struct UnixHost {
    /// Each `wl_keyboard` gets a duplicate of the descriptor.
    pub keymap: SharedKeymap,
}

impl Host for UnixHost {
    fn keymap(&mut self) -> Option<(Fd, u32)> {
        let (fd, size) = self.keymap.get()?;
        // SAFETY: duplicating a descriptor the host keeps open.
        let dup = unsafe { libc::fcntl(fd, libc::F_DUPFD_CLOEXEC, 0) };
        (dup >= 0).then_some((Fd(dup), size))
    }

    fn map_shm(&mut self, fd: Fd, size: usize) -> Option<Box<dyn ShmMap>> {
        match Mapping::map(fd.0, size) {
            Some(m) => Some(Box::new(m)),
            None => {
                close(fd.0);
                None
            }
        }
    }

    fn close_fd(&mut self, fd: Fd) {
        close(fd.0);
    }
}

/// Room for a `cmsghdr` and `MAX_FDS` descriptors, aligned for it.
#[repr(C, align(8))]
struct ControlBuffer([u8; 256]);

/// Read what one `recvmsg` gives: bytes into `buf`, descriptors into `fds`.
/// 0 bytes is the end of the connection.
pub fn recv(sock: RawFd, buf: &mut [u8], fds: &mut Vec<Fd>) -> io::Result<usize> {
    let mut iov = libc::iovec { iov_base: buf.as_mut_ptr().cast(), iov_len: buf.len() };
    let mut control = ControlBuffer([0; 256]);
    // SAFETY: msghdr is plain data; all-zero is a valid empty header.
    let mut msg: libc::msghdr = unsafe { std::mem::zeroed() };
    msg.msg_iov = &mut iov;
    msg.msg_iovlen = 1;
    msg.msg_control = control.0.as_mut_ptr().cast();
    msg.msg_controllen = control.0.len() as _;
    // SAFETY: `msg` points at `iov` (over `buf`) and `control`, which live
    // across the call and are as long as the header says.
    let n = unsafe { libc::recvmsg(sock, &mut msg, 0) };
    if n < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: the kernel filled `control` up to `msg_controllen`; the CMSG
    // macros walk only inside it.
    unsafe {
        let mut cmsg = libc::CMSG_FIRSTHDR(&msg);
        while !cmsg.is_null() {
            if (*cmsg).cmsg_level == libc::SOL_SOCKET && (*cmsg).cmsg_type == libc::SCM_RIGHTS {
                let data = libc::CMSG_DATA(cmsg);
                let count = ((*cmsg).cmsg_len as usize - libc::CMSG_LEN(0) as usize) / size_of::<RawFd>();
                for i in 0..count {
                    let fd = std::ptr::read_unaligned(data.cast::<RawFd>().add(i));
                    fds.push(Fd(fd));
                }
            }
            cmsg = libc::CMSG_NXTHDR(&msg, cmsg);
        }
    }
    Ok(n as usize)
}

/// Send `bytes` and `fds` (with the first byte); returns how many bytes went.
pub fn send(sock: RawFd, bytes: &[u8], fds: &[Fd]) -> io::Result<usize> {
    let mut iov = libc::iovec { iov_base: bytes.as_ptr() as *mut _, iov_len: bytes.len() };
    let mut control = ControlBuffer([0; 256]);
    // SAFETY: as in `recv`.
    let mut msg: libc::msghdr = unsafe { std::mem::zeroed() };
    msg.msg_iov = &mut iov;
    msg.msg_iovlen = 1;
    let fds = &fds[..fds.len().min(MAX_FDS)];
    if !fds.is_empty() {
        // SAFETY: CMSG_SPACE/LEN are pure arithmetic; the header and the
        // descriptors fit in `control` (MAX_FDS * 4 + header < 256).
        unsafe {
            let space = libc::CMSG_SPACE((fds.len() * size_of::<RawFd>()) as u32) as usize;
            msg.msg_control = control.0.as_mut_ptr().cast();
            msg.msg_controllen = space as _;
            let cmsg = libc::CMSG_FIRSTHDR(&msg);
            (*cmsg).cmsg_level = libc::SOL_SOCKET;
            (*cmsg).cmsg_type = libc::SCM_RIGHTS;
            (*cmsg).cmsg_len = libc::CMSG_LEN((fds.len() * size_of::<RawFd>()) as u32) as _;
            let data = libc::CMSG_DATA(cmsg).cast::<RawFd>();
            for (i, fd) in fds.iter().enumerate() {
                std::ptr::write_unaligned(data.add(i), fd.0);
            }
        }
    }
    // SAFETY: `msg` points at live buffers of the lengths it gives.
    let n = unsafe { libc::sendmsg(sock, &msg, libc::MSG_NOSIGNAL) };
    if n < 0 { Err(io::Error::last_os_error()) } else { Ok(n as usize) }
}

/// `poll(2)` on `fds`; `timeout_ms` -1 waits forever. Interrupted by a
/// signal: no events.
pub fn poll(fds: &mut [libc::pollfd], timeout_ms: i32) -> io::Result<()> {
    // SAFETY: `fds` is a live slice of pollfd of the length given.
    let r = unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, timeout_ms) };
    if r < 0 {
        let e = io::Error::last_os_error();
        if e.kind() == io::ErrorKind::Interrupted {
            fds.iter_mut().for_each(|f| f.revents = 0);
            return Ok(());
        }
        return Err(e);
    }
    Ok(())
}

/// `CLOCK_MONOTONIC` in nanoseconds.
pub fn monotonic_ns() -> u64 {
    let mut ts = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    // SAFETY: `ts` is a valid timespec to write.
    unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts) };
    ts.tv_sec as u64 * 1_000_000_000 + ts.tv_nsec as u64
}

/// Start `argv[0]` with `argv` and `env` (`NAME=value`) in a child process
/// (fork and execve); returns its pid.
pub fn spawn(argv: &[String], env: &[String]) -> io::Result<libc::pid_t> {
    let cstrings = |v: &[String]| -> io::Result<Vec<CString>> {
        v.iter().map(|s| CString::new(s.as_str()).map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))).collect()
    };
    let (args, envs) = (cstrings(argv)?, cstrings(env)?);
    let mut arg_ptrs: Vec<*const libc::c_char> = args.iter().map(|s| s.as_ptr()).collect();
    arg_ptrs.push(std::ptr::null());
    let mut env_ptrs: Vec<*const libc::c_char> = envs.iter().map(|s| s.as_ptr()).collect();
    env_ptrs.push(std::ptr::null());
    // SAFETY: the compositor is single-threaded, so the child may run any
    // code; it only calls execve (with arrays built before the fork) and
    // _exit.
    let pid = unsafe { libc::fork() };
    match pid {
        -1 => Err(io::Error::last_os_error()),
        0 => {
            // SAFETY: null-terminated arrays of pointers to live C strings.
            unsafe {
                libc::execve(arg_ptrs[0], arg_ptrs.as_ptr(), env_ptrs.as_ptr());
                libc::_exit(127)
            }
        }
        pid => Ok(pid),
    }
}

/// The exit status of child `pid` if it ended (`waitpid` without waiting):
/// its exit code, or 128 plus the signal that killed it.
pub fn try_wait(pid: libc::pid_t) -> Option<i32> {
    let mut status = 0;
    // SAFETY: `status` is a valid int to write.
    let r = unsafe { libc::waitpid(pid, &mut status, libc::WNOHANG) };
    if r != pid {
        return None;
    }
    if libc::WIFEXITED(status) {
        Some(libc::WEXITSTATUS(status))
    } else if libc::WIFSIGNALED(status) {
        Some(128 + libc::WTERMSIG(status))
    } else {
        None
    }
}
