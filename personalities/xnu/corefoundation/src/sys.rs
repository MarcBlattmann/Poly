//! What CoreFoundation takes from libSystem, and the few system calls
//! libSystem v0 has no function for yet (made directly, with the Darwin
//! convention: `rax` = 0x2000000 | number, carry set on failure).
//!
//! Constants: `<sys/fcntl.h>`, `<sys/stat.h>`, `<sys/dirent.h>`,
//! `<mach/message.h>`, `<mach/port.h>` of the macOS SDK.

use alloc::string::String;
use alloc::vec::Vec;
use core::ffi::{c_char, c_void};

unsafe extern "C" {
    pub fn malloc(n: usize) -> *mut c_void;
    pub fn realloc(p: *mut c_void, n: usize) -> *mut c_void;
    pub fn free(p: *mut c_void);
    pub fn posix_memalign(out: *mut *mut c_void, align: usize, n: usize) -> i32;
    fn write(fd: i32, buf: *const c_void, n: usize) -> isize;
    fn open(path: *const c_char, flags: i32, ...) -> i32;
    fn close(fd: i32) -> i32;
    fn read(fd: i32, buf: *mut c_void, n: usize) -> isize;
    fn fcntl(fd: i32, cmd: i32, ...) -> i32;
    #[link_name = "stat$INODE64"]
    fn stat(path: *const c_char, buf: *mut u8) -> i32;
    fn getpid() -> i32;
    fn _NSGetExecutablePath(buf: *mut c_char, size: *mut u32) -> i32;
    static mach_task_self_: u32;
    fn mach_port_allocate(task: u32, right: u32, name: *mut u32) -> i32;
    fn mach_msg(msg: *mut c_void, option: i32, send: u32, rcv: u32, name: u32, timeout: u32, notify: u32) -> i32;
}

const O_RDONLY: i32 = 0;
const O_DIRECTORY: i32 = 0x0010_0000;
const F_GETPATH: i32 = 50;
const S_IFMT: u16 = 0o170000;
const S_IFDIR: u16 = 0o040000;
const MAXPATHLEN: usize = 1024;

/// A raw BSD call with up to four arguments; `Err(errno)` on failure.
fn bsd(n: u64, a: u64, b: u64, c: u64, d: u64) -> Result<(u64, u64), u64> {
    let rax: u64;
    let rdx: u64;
    let cf: u8;
    // SAFETY: a Darwin system call: arguments in rdi, rsi, rdx, r10; rcx and
    // r11 are clobbered by `syscall`; the carry flag tells failure.
    unsafe {
        core::arch::asm!(
            "syscall",
            "setc {cf}",
            cf = out(reg_byte) cf,
            inlateout("rax") 0x0200_0000u64 | n => rax,
            in("rdi") a,
            in("rsi") b,
            inlateout("rdx") c => rdx,
            in("r10") d,
            lateout("rcx") _,
            lateout("r11") _,
            options(nostack),
        );
    }
    if cf != 0 { Err(rax) } else { Ok((rax, rdx)) }
}

/// Bytes to standard error.
pub fn stderr(bytes: &[u8]) {
    // SAFETY: a valid buffer.
    unsafe { write(2, bytes.as_ptr() as *const c_void, bytes.len()) };
}

pub fn pid() -> i32 {
    // SAFETY: plain C call.
    unsafe { getpid() }
}

/// The wall clock as seconds since 1970 (`gettimeofday`, BSD call 116:
/// Poly's form writes a `struct timeval` through its first argument).
pub fn unix_time() -> f64 {
    let mut tv = [0u8; 16];
    match bsd(116, tv.as_mut_ptr() as u64, 0, 0, 0) {
        Ok(_) => {
            let sec = i64::from_le_bytes(tv[..8].try_into().unwrap_or([0; 8]));
            let usec = i32::from_le_bytes(tv[8..12].try_into().unwrap_or([0; 4]));
            sec as f64 + f64::from(usec) / 1e6
        }
        Err(_) => 0.0,
    }
}

/// A NUL-terminated copy of `s`.
fn c_path(s: &str) -> Vec<u8> {
    let mut v = Vec::with_capacity(s.len() + 1);
    v.extend_from_slice(s.as_bytes());
    v.push(0);
    v
}

/// Whether `path` exists and is a directory: `None` when it does not exist.
pub fn is_dir(path: &str) -> Option<bool> {
    let p = c_path(path);
    let mut st = [0u8; 144];
    // SAFETY: a NUL-terminated path and a buffer the size of `struct stat`.
    if unsafe { stat(p.as_ptr() as *const c_char, st.as_mut_ptr()) } != 0 {
        return None;
    }
    let mode = u16::from_le_bytes([st[4], st[5]]);
    Some(mode & S_IFMT == S_IFDIR)
}

/// The whole contents of a file.
pub fn read_file(path: &str) -> Option<Vec<u8>> {
    let p = c_path(path);
    // SAFETY: a NUL-terminated path.
    let fd = unsafe { open(p.as_ptr() as *const c_char, O_RDONLY) };
    if fd < 0 {
        return None;
    }
    let mut out = Vec::new();
    let mut buf = [0u8; 4096];
    loop {
        // SAFETY: a valid buffer.
        let n = unsafe { read(fd, buf.as_mut_ptr() as *mut c_void, buf.len()) };
        if n <= 0 {
            break;
        }
        out.extend_from_slice(&buf[..n as usize]);
    }
    // SAFETY: our descriptor.
    unsafe { close(fd) };
    Some(out)
}

/// The names in a directory (without `.` and `..`), from
/// `getdirentries64` (BSD call 344, `struct direntry` of `<sys/dirent.h>`).
pub fn list_dir(path: &str) -> Vec<String> {
    let p = c_path(path);
    let mut names = Vec::new();
    // SAFETY: a NUL-terminated path.
    let fd = unsafe { open(p.as_ptr() as *const c_char, O_RDONLY | O_DIRECTORY) };
    if fd < 0 {
        return names;
    }
    let mut buf = alloc::vec![0u8; 8192];
    let mut base = 0u64;
    while let Ok((n, _)) = bsd(344, fd as u64, buf.as_mut_ptr() as u64, buf.len() as u64, (&raw mut base) as u64) {
        if n == 0 {
            break;
        }
        let mut at = 0usize;
        while at + 21 <= n as usize {
            let reclen = usize::from(u16::from_le_bytes([buf[at + 16], buf[at + 17]]));
            let namlen = usize::from(u16::from_le_bytes([buf[at + 18], buf[at + 19]]));
            if reclen == 0 {
                break;
            }
            let name = &buf[at + 21..(at + 21 + namlen).min(buf.len())];
            if name != b"." && name != b".." {
                names.push(String::from_utf8_lossy(name).into_owned());
            }
            at += reclen;
        }
    }
    // SAFETY: our descriptor.
    unsafe { close(fd) };
    names.sort();
    names
}

/// The current directory (`fcntl(F_GETPATH)` on `.`).
pub fn cwd() -> String {
    // SAFETY: a NUL-terminated path.
    let fd = unsafe { open(c".".as_ptr(), O_RDONLY | O_DIRECTORY) };
    if fd < 0 {
        return String::from("/");
    }
    let mut buf = [0u8; MAXPATHLEN];
    // SAFETY: a buffer of MAXPATHLEN bytes, as F_GETPATH needs.
    let r = unsafe { fcntl(fd, F_GETPATH, buf.as_mut_ptr()) };
    // SAFETY: our descriptor.
    unsafe { close(fd) };
    if r < 0 {
        return String::from("/");
    }
    let n = buf.iter().position(|&b| b == 0).unwrap_or(0);
    let s = String::from_utf8_lossy(&buf[..n]).into_owned();
    if s.is_empty() { String::from("/") } else { s }
}

/// The running program's path, made absolute.
pub fn executable_path() -> String {
    let mut buf = [0u8; MAXPATHLEN];
    let mut size = MAXPATHLEN as u32;
    // SAFETY: a buffer of `size` bytes.
    if unsafe { _NSGetExecutablePath(buf.as_mut_ptr() as *mut c_char, &mut size) } != 0 {
        return String::from("/");
    }
    let n = buf.iter().position(|&b| b == 0).unwrap_or(0);
    let p = String::from_utf8_lossy(&buf[..n]).into_owned();
    poly_cf::bundle::normalize(&cwd(), &p)
}

// ----- Mach ports for the run loop -----

const MACH_PORT_RIGHT_RECEIVE: u32 = 1;
const MACH_RCV_MSG: i32 = 0x2;
const MACH_RCV_TIMEOUT: i32 = 0x100;

/// A new receive right in this task, or `None`.
pub fn new_port() -> Option<u32> {
    let mut name = 0u32;
    // SAFETY: plain Mach call with a valid out pointer.
    let kr = unsafe { mach_port_allocate(mach_task_self_, MACH_PORT_RIGHT_RECEIVE, &mut name) };
    (kr == 0 && name != 0).then_some(name)
}

/// Wait up to `ms` milliseconds for a message on `port` (it is taken and
/// dropped); whether one came.
pub fn wait_port(port: u32, ms: u32) -> bool {
    // A header (24 bytes), a little body and the largest trailer.
    let mut buf = [0u64; 32];
    // SAFETY: a receive into a buffer of the size given.
    let kr = unsafe {
        mach_msg(
            buf.as_mut_ptr() as *mut c_void,
            MACH_RCV_MSG | MACH_RCV_TIMEOUT,
            0,
            core::mem::size_of_val(&buf) as u32,
            port,
            ms,
            0,
        )
    };
    kr == 0
}
