//! The compositor's one OS layer, on Poly's native calls through `poly-rt`
//! (ADR 0125): shared memory mappings, messages with handles on Unix
//! sockets, `poll`, devices (open, `control`, read, map), and starting,
//! watching and reaping children. The rest of the compositor reaches the
//! system only through this module, `poly_rt::fs` and `poly_rt::net`.
//!
//! Handles are the kernel's descriptors (the same numbers a Linux client's
//! file descriptors have), so they are kept as [`RawFd`] here, as the
//! Wayland server's [`Fd`] does.

use crate::io::{self, ErrorKind};
use alloc::boxed::Box;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::cell::{Cell, RefCell, UnsafeCell};
use poly_rt::abi::{self, events, open};
use poly_rt::fs::File;
use poly_rt::process::{Child, Command};
use poly_rt::sys;
use poly_wayland::server::{Host, ShmMap};
use poly_wayland::wire::Fd;

/// A handle, as a descriptor number.
pub type RawFd = i32;

/// A process id.
pub type Pid = u32;

/// The most handles one message may carry.
pub const MAX_FDS: usize = 28;

fn h(fd: RawFd) -> u32 {
    fd as u32
}

/// A client's shared memory, mapped read-only and shared.
#[derive(Debug)]
pub struct Mapping {
    ptr: *mut u8,
    len: usize,
    fd: RawFd,
}

impl Mapping {
    fn map(fd: RawFd, len: usize) -> Option<Mapping> {
        let ptr = poly_rt::mem::map_handle(h(fd), 0, len, abi::prot::READ, true).ok()?;
        Some(Mapping { ptr, len, fd })
    }

    fn unmap(&mut self) {
        if self.len > 0 {
            // SAFETY: `ptr`/`len` are exactly what `map_handle` returned; no
            // reference into the range outlives this call (reads copy out).
            let _ = unsafe { poly_rt::mem::unmap(self.ptr, self.len) };
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
                core::mem::forget(new);
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
        unsafe { core::ptr::copy_nonoverlapping(self.ptr.add(offset), dst.as_mut_ptr(), dst.len()) };
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
    let _ = sys::close(h(fd));
}

/// A second handle on `fd`'s open file.
pub fn dup(fd: RawFd) -> Option<RawFd> {
    sys::dup(h(fd)).ok().map(|d| d as RawFd)
}

/// The keymap file (read-only, unlinked) and its size, shared between the
/// host and the compositor, which replaces it when the layout changes
/// (ADR 0117).
pub type SharedKeymap = alloc::rc::Rc<Cell<Option<(RawFd, u32)>>>;

/// The [`Host`]: mappings, `close`, the keymap file, and dma-bufs.
#[derive(Debug, Default)]
pub struct UnixHost {
    /// The keymap file (read-only, unlinked) and its size; each
    /// `wl_keyboard` gets a duplicate of the handle. Shared with the
    /// compositor, which replaces it when the layout changes (ADR 0117).
    pub keymap: SharedKeymap,
    /// The `dev_t` of the DRM render node clients allocate dma-bufs on, if
    /// the compositor takes them (it runs on KMS).
    pub dmabuf_device: Option<u64>,
}

/// A device number as Linux's `dev_t` (glibc's `makedev`), the form
/// clients compare with their `stat`'s `st_rdev`.
pub fn linux_dev_t(major: u32, minor: u32) -> u64 {
    let (major, minor) = (u64::from(major), u64::from(minor));
    (major & 0xfff) << 8 | (major & !0xfff) << 32 | (minor & 0xff) | (minor & !0xff) << 12
}

/// The device number of a device node, as a Linux `dev_t`, if it is one.
pub fn device_number(path: &str) -> Option<u64> {
    let d = poly_rt::fs::metadata(path).ok()?.device();
    (d != 0).then(|| linux_dev_t((d >> 32) as u32, d as u32))
}

/// A read-only handle holding `bytes`: a sealed memory object, reopened
/// read-only through `/proc/self/fd` (else the object itself).
pub fn read_only_file(_name: &str, bytes: &[u8]) -> Option<RawFd> {
    let object = poly_rt::mem::memory_object(0).ok()?;
    object.write_all(bytes).ok()?;
    use abi::seal;
    let _ = poly_rt::mem::seal_object(&object, seal::SHRINK | seal::GROW | seal::WRITE);
    let path = format!("/proc/self/fd/{}", object.handle());
    match File::open(&path) {
        Ok(ro) => Some(ro.into_handle() as RawFd),
        Err(_) => Some(object.into_handle() as RawFd),
    }
}

impl Host for UnixHost {
    fn dmabuf_device(&mut self) -> Option<u64> {
        self.dmabuf_device
    }

    fn map_dmabuf(&mut self, fd: Fd) -> Option<Box<dyn ShmMap>> {
        let size = sys::seek(h(fd.0), 0, abi::Whence::End).unwrap_or(0);
        if size == 0 {
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
        dup(fd).map(|d| (Fd(d), size))
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

/// Read what one receive gives: bytes into `buf`, handles into `fds`. 0
/// bytes is the end of the connection.
pub fn recv(sock: RawFd, buf: &mut [u8], fds: &mut Vec<Fd>) -> io::Result<usize> {
    let mut got = [0u32; MAX_FDS];
    let (n, count) = sys::recv(h(sock), buf, &mut got)?;
    fds.extend(got[..count].iter().map(|&g| Fd(g as RawFd)));
    Ok(n)
}

/// Send `bytes` and `fds` (with the first byte); returns how many bytes went.
pub fn send(sock: RawFd, bytes: &[u8], fds: &[Fd]) -> io::Result<usize> {
    let handles: Vec<u32> = fds.iter().take(MAX_FDS).map(|f| h(f.0)).collect();
    Ok(sys::send(h(sock), bytes, &handles)?)
}

/// One entry to poll, as `poll(2)`'s `struct pollfd` (a negative `fd` is
/// skipped).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PollFd {
    pub fd: RawFd,
    pub events: u16,
    pub revents: u16,
}

/// Readable (`POLLIN`).
pub const POLLIN: u16 = events::READABLE;

impl PollFd {
    pub fn readable(fd: RawFd) -> PollFd {
        PollFd { fd, events: POLLIN, revents: 0 }
    }
}

/// Wait on `fds`; `timeout_ms` -1 waits forever. An interrupted wait
/// reports no events.
pub fn poll(fds: &mut [PollFd], timeout_ms: i32) -> io::Result<()> {
    let mut entries: Vec<abi::PollEntry> = fds
        .iter()
        .map(|p| abi::PollEntry {
            handle: if p.fd < 0 { abi::NO_HANDLE } else { h(p.fd) },
            events: p.events,
            revents: 0,
        })
        .collect();
    let timeout = (timeout_ms >= 0).then(|| core::time::Duration::from_millis(timeout_ms as u64));
    poly_rt::poll::poll(&mut entries, timeout)?;
    for (p, e) in fds.iter_mut().zip(&entries) {
        p.revents = e.revents;
    }
    Ok(())
}

/// The monotonic clock in nanoseconds.
pub fn monotonic_ns() -> u64 {
    poly_rt::time::Instant::now().as_nanos()
}

/// Seconds since 1970 on the wall clock.
pub fn realtime_secs() -> i64 {
    poly_rt::time::since_epoch().as_secs() as i64
}

/// This process's id.
pub fn getpid() -> Pid {
    poly_rt::process::id()
}

/// Write the disks back and restart (`restart`) or power off the machine
/// (the native `power` call, ADR 0126); returns only with the reason it
/// could not.
pub fn power(restart: bool) -> io::Error {
    let e = if restart { poly_rt::system::restart() } else { poly_rt::system::power_off() };
    e.into()
}

/// The user id the compositor runs as (native processes run as root).
pub fn getuid() -> u32 {
    poly_rt::process::uid()
}

// ---- devices -------------------------------------------------------------

/// Open a device node: read-only or read-write, non-blocking or not.
pub fn open_device(path: &str, write: bool, nonblock: bool) -> io::Result<RawFd> {
    let mut flags = open::READ;
    if write {
        flags |= open::WRITE;
    }
    if nonblock {
        flags |= open::NONBLOCK;
    }
    let f = File::open_with(path, flags, 0).map_err(|e| io::Error::new(e, format!("{path}: {e}")))?;
    Ok(f.into_handle() as RawFd)
}

/// A device request (an ioctl) on `fd`: the device's own request number and
/// argument.
///
/// # Safety
///
/// `arg` must be what the request takes: a pointer to memory of the size
/// and layout the device reads and writes, or a value.
pub unsafe fn control(fd: RawFd, request: u32, arg: u64) -> io::Result<u64> {
    // SAFETY: passed on from the caller.
    Ok(unsafe { poly_rt::io::control(h(fd), request, arg) }?)
}

/// Read from `fd`; `WouldBlock` and interruptions read nothing (0).
pub fn read(fd: RawFd, buf: &mut [u8]) -> io::Result<usize> {
    match sys::read(h(fd), buf) {
        Ok(n) => Ok(n),
        Err(ErrorKind::WouldBlock | ErrorKind::Interrupted) => Ok(0),
        Err(e) => Err(e.into()),
    }
}

/// Write to `fd`.
pub fn write(fd: RawFd, buf: &[u8]) -> io::Result<usize> {
    Ok(sys::write(h(fd), buf)?)
}

/// Map `len` bytes of `fd`'s file (a DRM buffer, a frame buffer) from
/// `offset`, readable, writable and shared.
pub fn map_shared(fd: RawFd, offset: u64, len: usize) -> io::Result<*mut u8> {
    Ok(poly_rt::mem::map_handle(h(fd), offset, len, abi::prot::READ | abi::prot::WRITE, true)?)
}

/// Unmap what [`map_shared`] mapped.
///
/// # Safety
///
/// Nothing may use the memory afterwards.
pub unsafe fn unmap(ptr: *mut u8, len: usize) {
    // SAFETY: passed on from the caller.
    let _ = unsafe { poly_rt::mem::unmap(ptr, len) };
}

/// A pipe: its read and write ends.
pub fn pipe() -> io::Result<(RawFd, RawFd)> {
    let [r, w] = sys::pipe()?;
    Ok((r as RawFd, w as RawFd))
}

// ---- children -------------------------------------------------------------

/// A value the single-threaded compositor keeps in a `static`.
struct Single<T>(UnsafeCell<T>);

// SAFETY: the compositor is one thread (poly-rt starts no others), so
// the cell is never reached from two threads.
unsafe impl<T> Sync for Single<T> {}

impl<T> Single<T> {
    /// Run `f` on the value. Not reentrant: `f` must not call back in.
    fn with<R>(&self, f: impl FnOnce(&mut T) -> R) -> R {
        // SAFETY: one thread (above), and `with` is never nested, so this
        // is the only reference while `f` runs.
        f(unsafe { &mut *self.0.get() })
    }
}

/// The children started and not reaped yet, each with its process handle
/// (readable once it ended), which the main loop polls: what `SIGCHLD`
/// and a self-pipe did on Linux.
static CHILDREN: Single<Vec<(Pid, File)>> = Single(UnsafeCell::new(Vec::new()));

/// Start `argv[0]` with `argv` and `env` (`NAME=value`) as a child; returns
/// its pid.
pub fn spawn(argv: &[String], env: &[String]) -> io::Result<Pid> {
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
    path.split(':')
        .filter(|d| !d.is_empty())
        .map(|d| format!("{d}/{name}"))
        .find(|p| poly_rt::fs::metadata(p).is_ok_and(|m| m.is_file() && m.mode() & 0o111 != 0))
        .unwrap_or_else(|| name.to_string())
}

/// [`spawn`], the handles `keep` given to the program under their own
/// numbers.
pub fn spawn_keeping(argv: &[String], env: &[String], keep: &[RawFd]) -> io::Result<Pid> {
    let program = resolve_program(argv.first().map_or("", String::as_str), env);
    let mut cmd = Command::new(&program);
    if let Some(name) = argv.first() {
        cmd.arg0(name);
    }
    cmd.args(argv.iter().skip(1).map(String::as_str)).envs(env.iter().map(String::as_str));
    for &fd in keep {
        cmd.keep(h(fd));
    }
    let child = cmd.spawn().map_err(|e| io::Error::new(e, format!("{program}: {e}")))?;
    let pid = child.id();
    if let Ok(handle) = child.process_handle() {
        CHILDREN.with(|c| c.push((pid, handle)));
    }
    Ok(pid)
}

/// The exit status of child `pid` if it ended (reaping it): its exit code,
/// or 128 plus the signal that ended it (11, `SIGSEGV`, for a native
/// child's fault).
pub fn try_wait(pid: Pid) -> Option<i32> {
    let status = Child::from_pid(pid).try_wait().ok().flatten()?;
    CHILDREN.with(|c| c.retain(|(p, _)| *p != pid));
    Some(match status {
        abi::ExitStatus::Exited(code) => i32::from(code),
        abi::ExitStatus::Signaled(s) => 128 + i32::from(s),
        abi::ExitStatus::Faulted(_) => 128 + 11,
    })
}

/// Ask child `pid` to end (a Linux child gets `SIGTERM`).
pub fn terminate(pid: Pid) {
    let _ = Child::from_pid(pid).terminate();
}

/// Whether a process with id `pid` exists.
pub fn alive(pid: Pid) -> bool {
    poly_rt::process::alive(pid)
}

/// The process handles of the children not reaped yet, to poll: one is
/// readable once its child ended, and stays so until [`try_wait`] reaps it.
pub fn child_poll_fds() -> Vec<PollFd> {
    CHILDREN.with(|c| c.iter().map(|(_, f)| PollFd::readable(f.handle() as RawFd)).collect())
}

/// Requests from outside (a client asking for a picture of the output),
/// one per line on a named pipe: what `SIGUSR1` did on Linux. The pipe is
/// open for reading and writing, so it never reports an end.
#[derive(Debug)]
pub struct Control {
    path: String,
    file: File,
    partial: RefCell<Vec<u8>>,
}

impl Control {
    /// Make the named pipe at `path` and open it.
    pub fn open(path: &str) -> io::Result<Control> {
        let _ = poly_rt::fs::remove_file(path);
        poly_rt::fs::make_fifo(path, 0o622).map_err(|e| io::Error::new(e, format!("{path}: {e}")))?;
        let file = File::open_with(path, open::READ | open::WRITE | open::NONBLOCK, 0)
            .map_err(|e| io::Error::new(e, format!("{path}: {e}")))?;
        Ok(Control { path: path.into(), file, partial: RefCell::new(Vec::new()) })
    }

    pub fn fd(&self) -> RawFd {
        self.file.handle() as RawFd
    }

    /// The whole lines that came in.
    pub fn take_lines(&self) -> Vec<String> {
        let mut buf = [0u8; 256];
        let mut partial = self.partial.borrow_mut();
        while let Ok(n) = sys::read(self.file.handle(), &mut buf) {
            if n == 0 {
                break;
            }
            partial.extend_from_slice(&buf[..n]);
        }
        let mut lines = Vec::new();
        while let Some(at) = partial.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = partial.drain(..=at).collect();
            lines.push(String::from_utf8_lossy(&line[..at]).into_owned());
        }
        lines
    }
}

impl Drop for Control {
    fn drop(&mut self) {
        let _ = poly_rt::fs::remove_file(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_numbers_as_linux_dev_t() {
        // glibc's makedev: DRM's render node 226:128, the console 5:1.
        assert_eq!(linux_dev_t(226, 128), 0xe280);
        assert_eq!(linux_dev_t(5, 1), 0x501);
        assert_eq!(linux_dev_t(0x1234, 0x56789), 0x1000_5672_3489);
    }

    /// Programs without a `/` are searched in `PATH`, as desktop files
    /// name them (nothing is found on the host).
    #[test]
    fn programs_are_found_in_path() {
        let env = ["PATH=/nonexistent:/bin:/usr/bin".to_string()];
        assert_eq!(resolve_program("/x/y", &env), "/x/y");
        assert_eq!(resolve_program("no-such-program-here", &env), "no-such-program-here");
    }
}
