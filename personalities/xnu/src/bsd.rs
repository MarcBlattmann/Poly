//! The BSD layer of the XNU personality: files, memory mappings, sysctl,
//! process ids, signals and child processes (ADR 0237, ADR 0246).
//!
//! The kernel provides the services through [`BsdKernel`] (one value for
//! the calling thread, for one call); this module decodes the arguments,
//! checks them the way the spec sheets (`specs/macos/bsd/*.yaml`) record,
//! lays out Darwin's structures, and encodes the results: on success the
//! carry flag is clear and `rax` (and for some calls `rdx`) holds the
//! result; on failure the carry flag is set and `rax` holds the Darwin
//! errno. Calls not here go on to the Mach core's table
//! ([`crate::call`]) or are `ENOSYS`.
//!
//! Numbers are those of the macOS SDK's `<sys/syscall.h>`; constants and
//! layouts cite their headers or sheets. No XNU or Darwin source was read.

pub mod errno;
pub mod file;
pub mod signal;
pub mod sysctl;

use crate::{Class, ENOSYS, decode};
use alloc::vec::Vec;
use errno::*;
use file::{DirEntry, OpenHow, Stat, Status};
use poly_abi::{Outcome, RFLAGS_CF, SyscallRegs};
use signal::{Action, Signals};

/// BSD system call numbers (macOS SDK `<sys/syscall.h>`).
pub mod nr {
    pub const EXIT: u64 = 1;
    pub const READ: u64 = 3;
    pub const WRITE: u64 = 4;
    pub const OPEN: u64 = 5;
    pub const CLOSE: u64 = 6;
    pub const WAIT4: u64 = 7;
    pub const CHDIR: u64 = 12;
    pub const FCHDIR: u64 = 13;
    pub const GETPID: u64 = 20;
    pub const GETUID: u64 = 24;
    pub const GETEUID: u64 = 25;
    pub const KILL: u64 = 37;
    pub const GETPPID: u64 = 39;
    pub const PIPE: u64 = 42;
    pub const GETEGID: u64 = 43;
    pub const SIGACTION: u64 = 46;
    pub const GETGID: u64 = 47;
    pub const SIGPROCMASK: u64 = 48;
    pub const SIGPENDING: u64 = 52;
    pub const MUNMAP: u64 = 73;
    pub const DUP2: u64 = 90;
    pub const FCNTL: u64 = 92;
    pub const PREAD: u64 = 153;
    pub const PWRITE: u64 = 154;
    pub const CSOPS: u64 = 169;
    pub const CSOPS_AUDITTOKEN: u64 = 170;
    pub const MMAP: u64 = 197;
    pub const LSEEK: u64 = 199;
    pub const SYSCTL: u64 = 202;
    pub const POSIX_SPAWN: u64 = 244;
    pub const SYSCTLBYNAME: u64 = 274;
    pub const ISSETUGID: u64 = 327;
    pub const STAT64: u64 = 338;
    pub const FSTAT64: u64 = 339;
    pub const LSTAT64: u64 = 340;
    pub const GETDIRENTRIES64: u64 = 344;
    pub const THREAD_SELFID: u64 = 372;
    pub const OPENAT: u64 = 463;
    pub const FSTATAT64: u64 = 470;
    pub const GETENTROPY: u64 = 500;
}

/// `PATH_MAX` (`<sys/syslimits.h>`).
pub const PATH_MAX: usize = 1024;
/// The most arguments plus environment strings `posix_spawn` copies.
pub const MAX_ARGS: usize = 4096;
/// The most bytes of them.
pub const MAX_ARG_BYTES: usize = 1 << 20;

/// Where a stat call looks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StatOf<'a> {
    Fd(i32),
    /// A path from `dirfd` ([`file::AT_FDCWD`]: the working directory).
    Path {
        dirfd: i32,
        path: &'a [u8],
        follow: bool,
    },
}

/// A memory mapping to make (checked and aligned already).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MapRequest {
    /// The address: a hint, or exact with `fixed`.
    pub addr: u64,
    /// A whole number of pages.
    pub len: u64,
    /// `PROT_*` bits (`<sys/mman.h>`: read 1, write 2, exec 4; the same as
    /// Linux's).
    pub prot: u32,
    pub shared: bool,
    pub fixed: bool,
    /// The file and page-aligned offset, `None` for anonymous memory.
    pub file: Option<(i32, u64)>,
}

/// What a waited-for child did, as `wait4` encodes it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChildStatus {
    Exited(u8),
    Signaled(u32),
}

impl ChildStatus {
    /// The wait status word: the exit code in bits 8 to 15, or the signal
    /// in the low 7 bits (the wait4 sheet).
    pub const fn word(self) -> u32 {
        match self {
            ChildStatus::Exited(c) => (c as u32) << 8,
            ChildStatus::Signaled(s) => s & 0x7f,
        }
    }
}

/// What the BSD calls need from the kernel, for the calling thread. Errors
/// are Darwin errno values ([`errno::from_linux`] maps the kernel's).
pub trait BsdKernel {
    /// Copy from user memory; `Err` if it is not readable.
    fn copy_in(&self, addr: u64, buf: &mut [u8]) -> Result<(), ()>;
    /// Copy to user memory; `Err` if it is not writable.
    fn copy_out(&mut self, addr: u64, buf: &[u8]) -> Result<(), ()>;

    /// End the process with exit status `status`; does not come back to
    /// user mode.
    fn exit(&mut self, status: u8);

    fn write(&mut self, fd: i32, buf: &[u8], offset: Option<u64>) -> Result<usize, u32>;
    /// Read into `buf` at the descriptor's offset or at `offset`; blocks.
    fn read(&mut self, fd: i32, buf: &mut [u8], offset: Option<u64>) -> Result<usize, u32>;
    fn open(&mut self, dirfd: i32, path: &[u8], how: &OpenHow) -> Result<i32, u32>;
    fn close(&mut self, fd: i32) -> Result<(), u32>;
    /// `whence`: 0 set, 1 current, 2 end.
    fn lseek(&mut self, fd: i32, offset: i64, whence: u32) -> Result<u64, u32>;
    fn stat(&mut self, of: StatOf<'_>) -> Result<Stat, u32>;
    /// Read directory entries from `fd`'s position on; `emit` returns
    /// whether the entry was taken (the position moves past taken ones).
    fn read_dir(&mut self, fd: i32, emit: &mut dyn FnMut(&DirEntry<'_>) -> bool) -> Result<(), u32>;
    /// A copy of `fd` at the lowest free descriptor `>= min`.
    fn dup(&mut self, fd: i32, min: i32, cloexec: bool) -> Result<i32, u32>;
    fn dup2(&mut self, old: i32, new: i32) -> Result<i32, u32>;
    fn cloexec(&self, fd: i32) -> Result<bool, u32>;
    fn set_cloexec(&mut self, fd: i32, on: bool) -> Result<(), u32>;
    fn status(&self, fd: i32) -> Result<Status, u32>;
    fn set_status(&mut self, fd: i32, append: bool, nonblock: bool) -> Result<(), u32>;
    /// The absolute path of the file open on `fd`.
    fn path_of(&self, fd: i32) -> Result<Vec<u8>, u32>;
    fn chdir(&mut self, path: &[u8]) -> Result<(), u32>;
    fn fchdir(&mut self, fd: i32) -> Result<(), u32>;
    /// A pipe: (read end, write end).
    fn pipe(&mut self) -> Result<(i32, i32), u32>;

    fn mmap(&mut self, req: &MapRequest) -> Result<u64, u32>;
    /// Unmap whole pages; nothing mapped there is no error.
    fn munmap(&mut self, addr: u64, len: u64) -> Result<(), u32>;
    fn page_size(&self) -> u64;

    fn pid(&self) -> u32;
    fn ppid(&self) -> u32;
    fn thread_id(&self) -> u64;
    /// (uid, gid) the process runs as (real and effective are the same).
    fn ids(&self) -> (u32, u32);
    /// Whether a process `pid` exists.
    fn process_exists(&self, pid: u32) -> bool;
    fn random(&mut self, buf: &mut [u8]);
    fn system(&self) -> sysctl::System;

    /// Work on the calling process's signal state.
    fn with_signals(&mut self, f: &mut dyn FnMut(&mut Signals));
    /// End the calling process as killed by `sig` (a default action).
    fn exit_signal(&mut self, sig: u32);
    /// Send `sig` (1 to 31) to process `pid` (not the caller): `ESRCH`,
    /// `EPERM`, or done as that process's state says.
    fn kill(&mut self, pid: u32, sig: u32) -> Result<(), u32>;
    /// Start `path` as a child with these arguments and environment;
    /// returns its pid.
    fn spawn(&mut self, path: &[u8], argv: &[Vec<u8>], envp: &[Vec<u8>]) -> Result<u32, u32>;
    /// Wait for the child `pid` (-1: any). `Ok(None)`: `nohang` and nothing
    /// to report. `ECHILD` without such a child.
    fn wait(&mut self, pid: i32, nohang: bool) -> Result<Option<(u32, ChildStatus)>, u32>;
}

/// A call's result before the registers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ret {
    pub value: u64,
    /// The second result register (`rdx`), for the calls that have one.
    pub second: Option<u64>,
}

fn ok(value: u64) -> Result<Ret, u32> {
    Ok(Ret { value, second: None })
}

fn read_u64(k: &dyn BsdKernel, addr: u64) -> Result<u64, u32> {
    let mut b = [0u8; 8];
    k.copy_in(addr, &mut b).map_err(|()| EFAULT)?;
    Ok(u64::from_le_bytes(b))
}

fn read_u32(k: &dyn BsdKernel, addr: u64) -> Result<u32, u32> {
    let mut b = [0u8; 4];
    k.copy_in(addr, &mut b).map_err(|()| EFAULT)?;
    Ok(u32::from_le_bytes(b))
}

fn put(k: &mut dyn BsdKernel, addr: u64, bytes: &[u8]) -> Result<(), u32> {
    k.copy_out(addr, bytes).map_err(|()| EFAULT)
}

/// A NUL-terminated string of at most `max` bytes (without the NUL):
/// `EFAULT` if unreadable, `ENAMETOOLONG` if longer.
pub fn read_cstr(k: &dyn BsdKernel, addr: u64, max: usize) -> Result<Vec<u8>, u32> {
    let mut out = Vec::new();
    let mut chunk = [0u8; 64];
    loop {
        let at = addr.wrapping_add(out.len() as u64);
        // Up to the end of the page, so a string at the end of a mapping
        // reads.
        let n = (4096 - (at % 4096) as usize).min(chunk.len());
        k.copy_in(at, &mut chunk[..n]).map_err(|()| EFAULT)?;
        if let Some(z) = chunk[..n].iter().position(|&b| b == 0) {
            out.extend_from_slice(&chunk[..z]);
            return if out.len() > max { Err(ENAMETOOLONG) } else { Ok(out) };
        }
        out.extend_from_slice(&chunk[..n]);
        if out.len() > max {
            return Err(ENAMETOOLONG);
        }
    }
}

/// A NULL-terminated array of strings (argv, envp); `addr` 0 is empty.
fn read_strings(k: &dyn BsdKernel, addr: u64, total: &mut usize) -> Result<Vec<Vec<u8>>, u32> {
    let mut out = Vec::new();
    if addr == 0 {
        return Ok(out);
    }
    loop {
        let p = read_u64(k, addr.wrapping_add(8 * out.len() as u64))?;
        if p == 0 {
            return Ok(out);
        }
        if out.len() >= MAX_ARGS {
            return Err(E2BIG);
        }
        let s = read_cstr(k, p, MAX_ARG_BYTES).map_err(|e| if e == ENAMETOOLONG { E2BIG } else { e })?;
        *total += s.len() + 1;
        if *total > MAX_ARG_BYTES {
            return Err(E2BIG);
        }
        out.push(s);
    }
}

/// `E2BIG` (`<sys/errno.h>`).
pub const E2BIG: u32 = 7;

/// Run BSD call `number` with arguments `a`; `None` if this layer does not
/// have it.
pub fn call(k: &mut dyn BsdKernel, number: u64, a: &[u64; 8]) -> Option<Result<Ret, u32>> {
    use nr::*;
    let fd = a[0] as i32;
    let r = match number {
        EXIT => {
            k.exit(a[0] as u8);
            ok(0)
        }
        READ | PREAD => rw_read(k, fd, a[1], a[2], (number == PREAD).then_some(a[3])),
        WRITE | PWRITE => rw_write(k, fd, a[1], a[2], (number == PWRITE).then_some(a[3])),
        OPEN => open(k, file::AT_FDCWD, a[0], a[1] as u32, a[2] as u32),
        OPENAT => open(k, a[0] as i32, a[1], a[2] as u32, a[3] as u32),
        CLOSE => k.close(fd).and_then(|()| ok(0)),
        LSEEK => {
            if a[2] > 2 {
                Err(EINVAL)
            } else {
                k.lseek(fd, a[1] as i64, a[2] as u32).and_then(ok)
            }
        }
        STAT64 | LSTAT64 => stat_path(k, file::AT_FDCWD, a[0], a[1], number == STAT64),
        FSTATAT64 => {
            let flags = a[3] as u32;
            if flags & !file::AT_SYMLINK_NOFOLLOW != 0 {
                Err(EINVAL)
            } else {
                stat_path(k, a[0] as i32, a[1], a[2], flags & file::AT_SYMLINK_NOFOLLOW == 0)
            }
        }
        FSTAT64 => k.stat(StatOf::Fd(fd)).and_then(|s| put(k, a[1], &s.encode())).and_then(|()| ok(0)),
        GETDIRENTRIES64 => getdirentries(k, fd, a[1], a[2], a[3]),
        DUP2 => {
            if (a[1] as i32) < 0 {
                Err(EBADF)
            } else {
                k.dup2(fd, a[1] as i32).and_then(|n| ok(n as u64))
            }
        }
        FCNTL => fcntl(k, fd, a[1] as u32, a[2]),
        CHDIR => read_cstr(k, a[0], PATH_MAX).and_then(|p| k.chdir(&p)).and_then(|()| ok(0)),
        FCHDIR => k.fchdir(fd).and_then(|()| ok(0)),
        PIPE => k.pipe().map(|(r, w)| Ret { value: r as u64, second: Some(w as u64) }),
        MMAP => mmap(k, a),
        MUNMAP => munmap(k, a[0], a[1]),
        GETPID => ok(u64::from(k.pid())),
        GETPPID => ok(u64::from(k.ppid())),
        GETUID | GETEUID => ok(u64::from(k.ids().0)),
        GETGID | GETEGID => ok(u64::from(k.ids().1)),
        ISSETUGID => ok(0),
        THREAD_SELFID => ok(k.thread_id()),
        CSOPS | CSOPS_AUDITTOKEN => csops(k, a[0] as i32, a[1] as u32, a[2], a[3]),
        GETENTROPY => getentropy(k, a[0], a[1]),
        SYSCTL => sysctl(k, a),
        SYSCTLBYNAME => sysctlbyname(k, a),
        SIGACTION => sigaction(k, a[0] as u32, a[1], a[2]),
        SIGPROCMASK => sigprocmask(k, a[0] as u32, a[1], a[2]),
        SIGPENDING => {
            let mut pending = 0;
            k.with_signals(&mut |s| pending = s.pending);
            put(k, a[0], &pending.to_le_bytes()).and_then(|()| ok(0))
        }
        KILL => kill(k, a[0] as i32, a[1] as i32),
        POSIX_SPAWN => posix_spawn(k, a),
        WAIT4 => wait4(k, a[0] as i32, a[1], a[2] as u32, a[3]),
        _ => return None,
    };
    Some(r)
}

/// The most bytes one read or write moves at a time through the kernel.
const IO_CHUNK: usize = 1 << 20;

fn rw_read(k: &mut dyn BsdKernel, fd: i32, buf: u64, len: u64, offset: Option<u64>) -> Result<Ret, u32> {
    if offset.is_some_and(|o| (o as i64) < 0) {
        return Err(EINVAL);
    }
    let n = (len as usize).min(IO_CHUNK);
    let mut data = alloc::vec![0u8; n];
    let got = k.read(fd, &mut data, offset)?;
    if got > 0 {
        put(k, buf, &data[..got])?;
    }
    ok(got as u64)
}

fn rw_write(k: &mut dyn BsdKernel, fd: i32, buf: u64, len: u64, offset: Option<u64>) -> Result<Ret, u32> {
    if offset.is_some_and(|o| (o as i64) < 0) {
        return Err(EINVAL);
    }
    let mut done = 0u64;
    while done < len {
        let n = ((len - done) as usize).min(IO_CHUNK);
        let mut data = alloc::vec![0u8; n];
        if k.copy_in(buf.wrapping_add(done), &mut data).is_err() {
            return if done == 0 { Err(EFAULT) } else { ok(done) };
        }
        let w = match k.write(fd, &data, offset.map(|o| o + done)) {
            Ok(w) => w,
            Err(e) if done == 0 => return Err(e),
            Err(_) => break,
        };
        done += w as u64;
        if w < n {
            break;
        }
    }
    if len == 0 {
        // A count of 0 still checks the descriptor.
        k.write(fd, &[], offset)?;
    }
    ok(done)
}

fn open(k: &mut dyn BsdKernel, dirfd: i32, path: u64, flags: u32, mode: u32) -> Result<Ret, u32> {
    let path = read_cstr(k, path, PATH_MAX)?;
    if path.is_empty() {
        return Err(ENOENT);
    }
    let how = OpenHow::decode(flags, mode).ok_or(EINVAL)?;
    k.open(dirfd, &path, &how).and_then(|fd| ok(fd as u64))
}

fn stat_path(k: &mut dyn BsdKernel, dirfd: i32, path: u64, buf: u64, follow: bool) -> Result<Ret, u32> {
    let path = read_cstr(k, path, PATH_MAX)?;
    if path.is_empty() {
        return Err(ENOENT);
    }
    let st = k.stat(StatOf::Path { dirfd, path: &path, follow })?;
    put(k, buf, &st.encode())?;
    ok(0)
}

fn getdirentries(k: &mut dyn BsdKernel, fd: i32, buf: u64, size: u64, basep: u64) -> Result<Ret, u32> {
    let st = k.stat(StatOf::Fd(fd))?;
    if size < file::MIN_DIRBUF || !st.is_dir() {
        return Err(EINVAL);
    }
    // basep must be writable (checked before anything moves).
    let old = read_u64(k, basep)?;
    put(k, basep, &old.to_le_bytes())?;
    let room = (size as usize).min(IO_CHUNK);
    let mut out = Vec::new();
    k.read_dir(fd, &mut |e| {
        let left = room - out.len();
        file::push_record(&mut out, left, e)
    })?;
    if !out.is_empty() {
        put(k, buf, &out)?;
    }
    let base = if out.is_empty() { file::DIR_END_BASE } else { 0 };
    put(k, basep, &base.to_le_bytes())?;
    ok(out.len() as u64)
}

fn fcntl(k: &mut dyn BsdKernel, fd: i32, cmd: u32, arg: u64) -> Result<Ret, u32> {
    use file::f;
    match cmd {
        f::DUPFD | f::DUPFD_CLOEXEC => {
            let min = arg as i32;
            if min < 0 {
                return Err(EINVAL);
            }
            k.dup(fd, min, cmd == f::DUPFD_CLOEXEC).and_then(|n| ok(n as u64))
        }
        f::GETFD => k.cloexec(fd).and_then(|c| ok(u64::from(c))),
        f::SETFD => k.set_cloexec(fd, arg & f::FD_CLOEXEC != 0).and_then(|()| ok(0)),
        f::GETFL => k.status(fd).and_then(|s| ok(s.bits())),
        f::SETFL => {
            let a = arg as u32;
            k.set_status(fd, a & file::o::APPEND != 0, a & file::o::NONBLOCK != 0).and_then(|()| ok(0))
        }
        f::GETPATH => {
            let mut p = k.path_of(fd)?;
            if p.len() >= f::MAXPATHLEN {
                return Err(ENAMETOOLONG);
            }
            p.push(0);
            put(k, arg, &p)?;
            ok(0)
        }
        _ => {
            // The descriptor is checked first, for any command.
            k.status(fd)?;
            Err(ENODEV)
        }
    }
}

/// `<sys/mman.h>` flags.
pub mod map {
    pub const SHARED: u32 = 1;
    pub const PRIVATE: u32 = 2;
    pub const FIXED: u32 = 0x10;
    pub const ANON: u32 = 0x1000;
}

fn mmap(k: &mut dyn BsdKernel, a: &[u64; 8]) -> Result<Ret, u32> {
    let (addr, len, prot, flags, fd, offset) = (a[0], a[1], a[2] as u32, a[3] as u32, a[4] as i32, a[5]);
    let page = k.page_size();
    let anon = flags & map::ANON != 0;
    if (len as i64) < 0 {
        return Err(EINVAL);
    }
    if len == 0 {
        // A file mapping of length 0 still checks the descriptor.
        if !anon {
            k.status(fd).map_err(|_| EBADF)?;
        }
        return ok(0);
    }
    if len > 1 << 47 {
        return Err(ENOMEM);
    }
    let fixed = flags & map::FIXED != 0;
    if fixed && addr % page != 0 {
        return Err(EINVAL);
    }
    let shared = flags & map::SHARED != 0;
    let (file, skew) = if anon {
        (None, 0)
    } else {
        if fd < 0 {
            return Err(EBADF);
        }
        let st = k.stat(StatOf::Fd(fd)).map_err(|_| EBADF)?;
        if st.is_dir() {
            return Err(EINVAL);
        }
        let status = k.status(fd)?;
        if prot & 1 != 0 && status.access == file::Access::Write {
            return Err(EACCES);
        }
        if shared && prot & 2 != 0 && status.access == file::Access::Read {
            return Err(EACCES);
        }
        // An unaligned offset: the mapping starts that many bytes into a
        // page (the mmap sheet).
        (Some((fd, offset - offset % page)), offset % page)
    };
    let total = (len + skew).next_multiple_of(page);
    let req = MapRequest { addr: addr - addr % page, len: total, prot: prot & 7, shared, fixed, file };
    k.mmap(&req).and_then(|at| ok(at + skew))
}

fn munmap(k: &mut dyn BsdKernel, addr: u64, len: u64) -> Result<Ret, u32> {
    let page = k.page_size();
    if addr % page != 0 || len == 0 || (len as i64) < 0 {
        return Err(EINVAL);
    }
    // Past the end of the address space: nothing to remove.
    match addr.checked_add(len.next_multiple_of(page)) {
        Some(end) if end <= 1 << 47 => k.munmap(addr, end - addr).and_then(|()| ok(0)),
        _ => ok(0),
    }
}

/// The code-signing flags of an ad hoc signed program (the csops sheet:
/// CS_VALID and the linker-signed ad hoc bits); Poly checks no signature.
pub const CS_STATUS_ADHOC: u32 = 0x2202_0201;

fn csops(k: &mut dyn BsdKernel, pid: i32, op: u32, buf: u64, size: u64) -> Result<Ret, u32> {
    if pid < 0 || (pid != 0 && pid as u32 != k.pid() && !k.process_exists(pid as u32)) {
        return Err(ESRCH);
    }
    match op {
        0 => {
            if size < 4 {
                return Err(EINVAL);
            }
            put(k, buf, &CS_STATUS_ADHOC.to_le_bytes())?;
            ok(0)
        }
        _ => Err(EINVAL),
    }
}

fn getentropy(k: &mut dyn BsdKernel, buf: u64, len: u64) -> Result<Ret, u32> {
    if len > 256 {
        return Err(EINVAL);
    }
    if len == 0 {
        return ok(0);
    }
    let mut b = [0u8; 256];
    k.random(&mut b[..len as usize]);
    put(k, buf, &b[..len as usize])?;
    ok(0)
}

/// Give `value` to `oldp` / `oldlenp` as sysctl does.
fn sysctl_out(k: &mut dyn BsdKernel, value: &[u8], oldp: u64, oldlenp: u64) -> Result<Ret, u32> {
    if oldlenp == 0 {
        return ok(0);
    }
    if oldp == 0 {
        put(k, oldlenp, &(value.len() as u64).to_le_bytes())?;
        return ok(0);
    }
    let room = read_u64(k, oldlenp)?;
    if room < value.len() as u64 {
        put(k, oldlenp, &0u64.to_le_bytes())?;
        return Err(ENOMEM);
    }
    put(k, oldp, value)?;
    put(k, oldlenp, &(value.len() as u64).to_le_bytes())?;
    ok(0)
}

fn sysctl(k: &mut dyn BsdKernel, a: &[u64; 8]) -> Result<Ret, u32> {
    let (name, namelen, oldp, oldlenp, newp, newlen) = (a[0], a[1], a[2], a[3], a[4], a[5]);
    if !(2..=sysctl::CTL_MAXNAME as u64).contains(&namelen) {
        return Err(EINVAL);
    }
    let mut mib = Vec::with_capacity(namelen as usize);
    for i in 0..namelen {
        mib.push(read_u32(k, name + 4 * i)? as i32);
    }
    if sysctl::is_name_to_mib(&mib) {
        if newlen == 0 || newlen > sysctl::MAX_NAME as u64 {
            return Err(EINVAL);
        }
        let mut n = alloc::vec![0u8; newlen as usize];
        k.copy_in(newp, &mut n).map_err(|()| EFAULT)?;
        if let Some(z) = n.iter().position(|&b| b == 0) {
            n.truncate(z);
        }
        let v = sysctl::name_to_mib(&n)?;
        return sysctl_out(k, &v, oldp, oldlenp);
    }
    let v = sysctl::by_mib(&k.system(), &mib)?;
    if newp != 0 {
        return Err(EPERM);
    }
    sysctl_out(k, &v, oldp, oldlenp)
}

fn sysctlbyname(k: &mut dyn BsdKernel, a: &[u64; 8]) -> Result<Ret, u32> {
    let (name, namelen, oldp, oldlenp, newp) = (a[0], a[1], a[2], a[3], a[4]);
    if namelen > sysctl::MAX_NAME as u64 {
        return Err(ENAMETOOLONG);
    }
    let mut n = alloc::vec![0u8; namelen as usize];
    k.copy_in(name, &mut n).map_err(|()| EFAULT)?;
    if let Some(z) = n.iter().position(|&b| b == 0) {
        n.truncate(z);
    }
    let v = sysctl::by_name(&k.system(), &n)?;
    if newp != 0 {
        return Err(EPERM);
    }
    sysctl_out(k, &v, oldp, oldlenp)
}

fn sigaction(k: &mut dyn BsdKernel, sig: u32, new: u64, old: u64) -> Result<Ret, u32> {
    // The kernel's struct __sigaction: handler, trampoline, mask, flags.
    let new = if new == 0 {
        None
    } else {
        let mut b = [0u8; 24];
        k.copy_in(new, &mut b).map_err(|()| EFAULT)?;
        Some(Action {
            handler: u64::from_le_bytes(b[0..8].try_into().unwrap_or_default()),
            trampoline: u64::from_le_bytes(b[8..16].try_into().unwrap_or_default()),
            mask: u32::from_le_bytes(b[16..20].try_into().unwrap_or_default()),
            flags: u32::from_le_bytes(b[20..24].try_into().unwrap_or_default()),
        })
    };
    let mut r = Err(EINVAL);
    k.with_signals(&mut |s| r = s.set_action(sig, new));
    let prev = r?;
    if old != 0 {
        // struct sigaction: handler, mask, flags.
        let mut b = [0u8; 16];
        b[0..8].copy_from_slice(&prev.handler.to_le_bytes());
        b[8..12].copy_from_slice(&prev.mask.to_le_bytes());
        b[12..16].copy_from_slice(&prev.flags.to_le_bytes());
        put(k, old, &b)?;
    }
    ok(0)
}

fn sigprocmask(k: &mut dyn BsdKernel, how: u32, set: u64, old: u64) -> Result<Ret, u32> {
    let set = if set == 0 { None } else { Some(read_u32(k, set)?) };
    let mut r = Err(EINVAL);
    k.with_signals(&mut |s| r = s.set_mask(how, set));
    let prev = r?;
    if old != 0 {
        // An unwritable old pointer was no error on the oracle.
        let _ = k.copy_out(old, &prev.to_le_bytes());
    }
    ok(0)
}

fn kill(k: &mut dyn BsdKernel, pid: i32, sig: i32) -> Result<Ret, u32> {
    if sig < 0 || sig as u32 >= signal::NSIG {
        return Err(EINVAL);
    }
    let sig = sig as u32;
    let me = k.pid();
    if pid == 0 || pid as u32 == me {
        if sig == 0 {
            return ok(0);
        }
        let mut d = signal::Disposition::Discard;
        k.with_signals(&mut |s| d = s.deliver(sig));
        match d {
            signal::Disposition::Terminate => k.exit_signal(sig),
            signal::Disposition::Discard | signal::Disposition::StopOrContinue | signal::Disposition::Pending => {}
        }
        return ok(0);
    }
    if pid < 0 {
        // Process groups come with job control.
        return Err(ESRCH);
    }
    if sig == 0 {
        return if k.process_exists(pid as u32) { ok(0) } else { Err(ESRCH) };
    }
    k.kill(pid as u32, sig).and_then(|()| ok(0))
}

fn posix_spawn(k: &mut dyn BsdKernel, a: &[u64; 8]) -> Result<Ret, u32> {
    let (pid_ptr, path, _attrs, argv, envp) = (a[0], a[1], a[2], a[3], a[4]);
    let path = read_cstr(k, path, PATH_MAX)?;
    if path.is_empty() {
        return Err(ENOENT);
    }
    let mut total = 0;
    let argv = read_strings(k, argv, &mut total)?;
    let envp = read_strings(k, envp, &mut total)?;
    let pid = k.spawn(&path, &argv, &envp)?;
    if pid_ptr != 0 {
        // An unwritable pid pointer is no error (the sheet's probe).
        let _ = k.copy_out(pid_ptr, &(pid as i32).to_le_bytes());
    }
    ok(0)
}

/// `wait4` options (`<sys/wait.h>`).
pub const WNOHANG: u32 = 1;
/// The size of `struct rusage` (`<sys/resource.h>`).
pub const RUSAGE_SIZE: usize = 144;

fn wait4(k: &mut dyn BsdKernel, pid: i32, status: u64, options: u32, rusage: u64) -> Result<Ret, u32> {
    // An unwritable status pointer fails before a child is reaped.
    if status != 0 {
        let v = read_u32(k, status)?;
        put(k, status, &v.to_le_bytes())?;
    }
    let Some((child, st)) = k.wait(pid, options & WNOHANG != 0)? else { return ok(0) };
    if status != 0 {
        put(k, status, &st.word().to_le_bytes())?;
    }
    if rusage != 0 {
        let _ = k.copy_out(rusage, &[0u8; RUSAGE_SIZE]);
    }
    ok(u64::from(child))
}

/// Put `r` into the x86-64 registers.
pub fn encode(regs: &mut SyscallRegs, r: Result<Ret, u32>) {
    match r {
        Ok(Ret { value, second }) => {
            regs.rax = value;
            if let Some(s) = second {
                regs.rdx = s;
            }
            regs.rflags &= !RFLAGS_CF;
        }
        Err(errno) => {
            regs.rax = if errno == 0 { ENOSYS } else { u64::from(errno) };
            regs.rflags |= RFLAGS_CF;
        }
    }
}

/// The x86-64 entry: arguments in `rdi, rsi, rdx, r10, r8, r9`, the
/// seventh and eighth on the user stack. `None` if the call is not a BSD
/// call this layer has (the caller tries the Mach core next).
pub fn dispatch_with(k: &mut dyn BsdKernel, regs: &mut SyscallRegs) -> Option<Outcome> {
    let (class, number) = decode(regs.rax);
    if class != Class::Bsd {
        return None;
    }
    let mut a = [regs.rdi, regs.rsi, regs.rdx, regs.r10, regs.r8, regs.r9, 0, 0];
    for (i, slot) in a[6..].iter_mut().enumerate() {
        if let Ok(v) = read_u64(k, regs.rsp.wrapping_add(8 + 8 * i as u64)) {
            *slot = v;
        }
    }
    let r = call(k, number, &a)?;
    encode(regs, r);
    Some(Outcome::Handled)
}

#[cfg(test)]
mod tests;
