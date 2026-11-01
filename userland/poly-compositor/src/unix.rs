//! The Linux calls the compositor makes: shared memory mappings, messages
//! with file descriptors on Unix sockets (`SCM_RIGHTS`, unix(7),
//! cmsg(3)), `poll`, and starting and waiting for a client.

use poly_wayland::server::{Host, ShmMap};
use poly_wayland::wire::Fd;
use std::ffi::CString;
use std::io;
use std::os::fd::RawFd;
use std::sync::atomic::{AtomicI32, Ordering};

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

/// The [`Host`] on Linux: `mmap`, `close`, the keymap file, and dma-bufs.
#[derive(Debug, Default)]
pub struct UnixHost {
    /// The keymap file (read-only, unlinked) and its size; each
    /// `wl_keyboard` gets a duplicate of the descriptor. Shared with the
    /// compositor, which replaces it when the layout changes (ADR 0117).
    pub keymap: SharedKeymap,
    /// The `dev_t` of the DRM render node clients allocate dma-bufs on, if
    /// the compositor takes them (it runs on KMS).
    pub dmabuf_device: Option<u64>,
}

/// The device number of a device node (`st_rdev`), if it is one.
pub fn device_number(path: &str) -> Option<u64> {
    let c = CString::new(path).ok()?;
    // SAFETY: stat is plain data; all-zero is a valid value to overwrite.
    let mut st: libc::stat = unsafe { std::mem::zeroed() };
    // SAFETY: a NUL-terminated path and a stat buffer to fill.
    let r = unsafe { libc::stat(c.as_ptr(), &mut st) };
    (r == 0 && st.st_rdev != 0).then_some(st.st_rdev)
}

/// A read-only descriptor holding `bytes`: a sealed memfd, else an unlinked
/// file in `/tmp`.
pub fn read_only_file(name: &str, bytes: &[u8]) -> Option<RawFd> {
    let c = CString::new(name).ok()?;
    // SAFETY: a NUL-terminated name and valid flags.
    let fd = unsafe { libc::memfd_create(c.as_ptr(), libc::MFD_CLOEXEC | libc::MFD_ALLOW_SEALING) };
    let fd = if fd >= 0 {
        fd
    } else {
        let path = format!("/tmp/{name}-{}", std::process::id());
        let p = CString::new(path.as_str()).ok()?;
        // SAFETY: a NUL-terminated path.
        let fd =
            unsafe { libc::open(p.as_ptr(), libc::O_RDWR | libc::O_CREAT | libc::O_EXCL | libc::O_CLOEXEC, 0o600) };
        let _ = std::fs::remove_file(&path);
        if fd < 0 {
            return None;
        }
        fd
    };
    let mut at = 0;
    while at < bytes.len() {
        // SAFETY: writing from a live buffer of the length given.
        let n = unsafe { libc::write(fd, bytes[at..].as_ptr().cast(), bytes.len() - at) };
        if n <= 0 {
            close(fd);
            return None;
        }
        at += n as usize;
    }
    // Sealing is best effort (a file in /tmp cannot be sealed).
    // SAFETY: fcntl on a descriptor we own.
    unsafe { libc::fcntl(fd, libc::F_ADD_SEALS, libc::F_SEAL_SHRINK | libc::F_SEAL_GROW | libc::F_SEAL_WRITE) };
    let path = format!("/proc/self/fd/{fd}");
    let p = CString::new(path).ok()?;
    // A read-only descriptor of it, as the protocol wants.
    // SAFETY: a NUL-terminated path.
    let ro = unsafe { libc::open(p.as_ptr(), libc::O_RDONLY | libc::O_CLOEXEC) };
    if ro >= 0 {
        close(fd);
        return Some(ro);
    }
    Some(fd)
}

impl Host for UnixHost {
    fn dmabuf_device(&mut self) -> Option<u64> {
        self.dmabuf_device
    }

    fn map_dmabuf(&mut self, fd: Fd) -> Option<Box<dyn ShmMap>> {
        // SAFETY: lseek on a descriptor the client sent.
        let size = unsafe { libc::lseek(fd.0, 0, libc::SEEK_END) };
        if size <= 0 {
            close(fd.0);
            return None;
        }
        self.map_shm(fd, size as usize)
    }

    fn dmabuf_table(&mut self, table: &[u8]) -> Option<Fd> {
        read_only_file("poly-dmabuf-formats", table).map(Fd)
    }

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
    spawn_keeping(argv, env, &[])
}

/// The program `name` runs: itself if it has a `/`, else the first
/// executable file of that name in the directories of `PATH` in `env`
/// (`/usr/local/bin:/usr/bin:/bin` without one), as execvp(3) searches
/// (desktop files name programs that way, ADR 0118).
pub fn resolve_program(name: &str, env: &[String]) -> String {
    if name.contains('/') || name.is_empty() {
        return name.to_string();
    }
    let path = env.iter().find_map(|v| v.strip_prefix("PATH=")).unwrap_or("/usr/local/bin:/usr/bin:/bin");
    use std::os::unix::fs::PermissionsExt;
    path.split(':')
        .filter(|d| !d.is_empty())
        .map(|d| format!("{d}/{name}"))
        .find(|p| std::fs::metadata(p).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0))
        .unwrap_or_else(|| name.to_string())
}

/// [`spawn`], the descriptors `keep` inherited by the program (their
/// close-on-exec flag cleared in the child).
pub fn spawn_keeping(argv: &[String], env: &[String], keep: &[RawFd]) -> io::Result<libc::pid_t> {
    let cstrings = |v: &[String]| -> io::Result<Vec<CString>> {
        v.iter().map(|s| CString::new(s.as_str()).map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))).collect()
    };
    let (args, envs) = (cstrings(argv)?, cstrings(env)?);
    let program = CString::new(resolve_program(argv.first().map_or("", String::as_str), env))
        .map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
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
            for &fd in keep {
                // SAFETY: fcntl on a descriptor of this process; it only
                // clears FD_CLOEXEC (async-signal-safe after fork).
                unsafe { libc::fcntl(fd, libc::F_SETFD, 0) };
            }
            // SAFETY: null-terminated arrays of pointers to live C strings.
            unsafe {
                libc::execve(program.as_ptr(), arg_ptrs.as_ptr(), env_ptrs.as_ptr());
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

/// The write end of [`ChildSignal`]'s pipe, for the signal handler; -1
/// before [`ChildSignal::install`].
static CHILD_PIPE: AtomicI32 = AtomicI32::new(-1);

/// `SIGCHLD`'s handler: one byte into the pipe (write(2) is
/// async-signal-safe, signal-safety(7)); errno is left as it was.
extern "C" fn on_sigchld(_: libc::c_int) {
    let fd = CHILD_PIPE.load(Ordering::Relaxed);
    if fd < 0 {
        return;
    }
    // SAFETY: errno is this thread's and is put back after the write; the
    // byte is a live one-byte buffer.
    unsafe {
        let errno = libc::__errno_location();
        let saved = *errno;
        // A full pipe wakes the loop already: the byte may be dropped.
        libc::write(fd, [1u8].as_ptr().cast(), 1);
        *errno = saved;
    }
}

/// Wakes the main loop when a child process ends: `SIGCHLD`'s handler
/// writes to a pipe whose read end is polled with the clients (the
/// self-pipe way, so a child that ends just before `poll` still wakes
/// it), and the loop then reaps what ended ([`try_wait`]). Apps started
/// from the launcher are reaped at once, not when something else wakes the
/// compositor. Children do not keep the handler past `execve` (signal(7)).
#[derive(Debug)]
pub struct ChildSignal {
    read: RawFd,
}

impl ChildSignal {
    /// Make the pipe and install the handler (`SA_NOCLDSTOP`: stopped and
    /// continued children do not count; `SA_RESTART` for the calls that
    /// restart, sigaction(2)). Once per process.
    pub fn install() -> io::Result<ChildSignal> {
        let mut fds = [0 as RawFd; 2];
        // SAFETY: `fds` has room for the two descriptors pipe2 writes.
        if unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC | libc::O_NONBLOCK) } < 0 {
            return Err(io::Error::last_os_error());
        }
        CHILD_PIPE.store(fds[1], Ordering::Relaxed);
        // SAFETY: a zeroed sigaction is valid; the handler is an extern "C"
        // function that makes only async-signal-safe calls.
        let r = unsafe {
            let mut sa: libc::sigaction = std::mem::zeroed();
            sa.sa_sigaction = on_sigchld as *const () as libc::sighandler_t;
            sa.sa_flags = libc::SA_RESTART | libc::SA_NOCLDSTOP;
            libc::sigemptyset(&mut sa.sa_mask);
            libc::sigaction(libc::SIGCHLD, &sa, std::ptr::null_mut())
        };
        if r < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(ChildSignal { read: fds[0] })
    }

    /// The descriptor to poll for `POLLIN`.
    pub fn fd(&self) -> RawFd {
        self.read
    }

    /// Empty the pipe (after it polled readable).
    pub fn clear(&self) {
        let mut buf = [0u8; 64];
        // SAFETY: reading into a live buffer of the length given, from our
        // own non-blocking pipe (it stops with EAGAIN once empty).
        while unsafe { libc::read(self.read, buf.as_mut_ptr().cast(), buf.len()) } > 0 {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A child that ends makes the pipe readable at once, and is reaped.
    #[test]
    fn a_child_that_ends_wakes_the_loop() {
        let sig = ChildSignal::install().expect("pipe and handler");
        let mut fds = [libc::pollfd { fd: sig.fd(), events: libc::POLLIN, revents: 0 }];
        poll(&mut fds, 0).unwrap();
        assert_eq!(fds[0].revents, 0, "readable before any child ended");
        let pid = spawn(&["/bin/sh".into(), "-c".into(), "exit 7".into()], &[]).expect("spawn");
        // Woken by the child's end, long before the hour's timeout.
        let start = std::time::Instant::now();
        loop {
            poll(&mut fds, 3_600_000).unwrap();
            if fds[0].revents & libc::POLLIN != 0 {
                break;
            }
            assert!(start.elapsed() < std::time::Duration::from_secs(60), "SIGCHLD never woke the poll");
        }
        sig.clear();
        let status = loop {
            if let Some(s) = try_wait(pid) {
                break s;
            }
            std::thread::yield_now();
        };
        assert_eq!(status, 7);
        // Emptied: not readable until another child ends.
        fds[0].revents = 0;
        poll(&mut fds, 0).unwrap();
        assert_eq!(fds[0].revents, 0, "still readable after clear");
    }
}
