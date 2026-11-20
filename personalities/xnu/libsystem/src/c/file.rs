//! libsystem_c's streams on files (ADR 0257 addendum, for SDK-linked
//! command-line programs): `fopen` and its family, `tmpfile` and `mkstemp`,
//! `popen` and `system`, directory streams and `getcwd`.
//!
//! A `fopen`ed stream is a `FILE` and its [`Stream`] in one allocation;
//! open streams are listed so `exit` and `fflush(NULL)` flush them.
//! Constants are those of the SDK's `<sys/fcntl.h>`, `<stdio.h>` and
//! `<sys/dirent.h>`.

use super::*;
use core::ptr::null_mut;

unsafe extern "C" {
    fn open(path: *const u8, flags: i32, mode: i32) -> i32;
    fn close(fd: i32) -> i32;
    fn free(p: *mut u8);
    fn unlink(path: *const u8) -> i32;
    fn rmdir(path: *const u8) -> i32;
    fn access(path: *const u8, mode: i32) -> i32;
    fn fcntl(fd: i32, cmd: i32, arg: usize) -> i32;
    fn dup(fd: i32) -> i32;
    fn dup2(from: i32, to: i32) -> i32;
    fn pipe(fds: *mut i32) -> i32;
    fn posix_spawn(
        pid: *mut i32,
        path: *const u8,
        actions: *const c_void,
        attrs: *const c_void,
        argv: *const *const u8,
        envp: *const *const c_char,
    ) -> i32;
    fn wait4(pid: i32, status: *mut i32, options: i32, rusage: *mut c_void) -> i32;
    fn __getdirentries64(fd: i32, buf: *mut u8, n: usize, base: *mut i64) -> isize;
}

const O_RDONLY: i32 = 0;
const O_WRONLY: i32 = 1;
const O_RDWR: i32 = 2;
const O_APPEND: i32 = 0x8;
const O_CREAT: i32 = 0x200;
const O_TRUNC: i32 = 0x400;
const O_EXCL: i32 = 0x800;
const O_DIRECTORY: i32 = 0x0010_0000;
const O_CLOEXEC: i32 = 0x0100_0000;
const F_GETPATH: i32 = 50;
const PATH_MAX: usize = 1024;
const EEXIST: i32 = 17;
const EISDIR: i32 = 21;
const ERANGE_: i32 = 34;
const EMFILE: i32 = 24;
const X_OK: i32 = 1;

/// A stream `fopen` made.
#[repr(C)]
struct Opened {
    file: File,
    stream: Stream,
    /// `popen`'s child, 0 for a plain stream.
    child: i32,
}

/// The most streams open at once besides the standard three.
const MAX_OPEN: usize = 256;

static OPEN: crate::Global<[*mut Opened; MAX_OPEN]> = crate::Global::new([null_mut(); MAX_OPEN]);

fn open_list() -> &'static mut [*mut Opened; MAX_OPEN] {
    // SAFETY: one use per call (see crate::Global).
    unsafe { OPEN.get() }
}

/// Flush every stream `fopen` made; `EOF` if one failed.
pub(super) fn flush_open() -> i32 {
    let mut r = 0;
    for &o in open_list().iter() {
        // SAFETY: listed streams are live.
        if !o.is_null() && unsafe { flush(&raw mut (*o).file) }.is_err() {
            r = stdio::EOF;
        }
    }
    r
}

/// The open flags and the `FILE` flags of a `fopen` mode.
fn parse_mode(mode: &[u8]) -> Option<(i32, i16)> {
    let (mut flags, mut fflags) = match mode.first()? {
        b'r' => (O_RDONLY, stdio::SRD),
        b'w' => (O_WRONLY | O_CREAT | O_TRUNC, stdio::SWR),
        b'a' => (O_WRONLY | O_CREAT | O_APPEND, stdio::SWR | stdio::SAPP),
        _ => return None,
    };
    for &c in &mode[1..] {
        match c {
            b'+' => {
                flags = (flags & !(O_WRONLY | O_RDONLY)) | O_RDWR;
                fflags = (fflags & !(stdio::SRD | stdio::SWR)) | stdio::SRW;
            }
            b'x' => flags |= O_EXCL,
            b'e' => flags |= O_CLOEXEC,
            _ => {}
        }
    }
    Some((flags, fflags))
}

/// A stream on `fd` with `fflags`, listed as open; null (with `errno`) if
/// there is no memory or no free slot.
fn make(fd: i32, fflags: i16, child: i32) -> *mut File {
    let Some(slot) = open_list().iter().position(|p| p.is_null()) else {
        crate::set_errno(EMFILE);
        return null_mut();
    };
    // SAFETY: a fresh allocation the size of `Opened`.
    unsafe {
        let o = malloc(core::mem::size_of::<Opened>()) as *mut Opened;
        if o.is_null() {
            crate::set_errno(ENOMEM);
            return null_mut();
        }
        o.write(Opened { file: File::new(fd as i16, fflags, null_mut()), stream: Stream::new(fd, Mode::Full), child });
        (*o).file.cookie = &raw mut (*o).stream as *mut c_void;
        open_list()[slot] = o;
        &raw mut (*o).file
    }
}

/// The `Opened` of `f`, if `fopen` made it.
fn opened(f: *mut File) -> Option<(usize, *mut Opened)> {
    open_list().iter().position(|&o| !o.is_null() && core::ptr::eq(o as *mut File, f)).map(|i| (i, open_list()[i]))
}

#[unsafe(no_mangle)]
unsafe extern "C" fn fopen(path: *const u8, mode: *const u8) -> *mut File {
    // SAFETY: the C contract: two strings.
    let Some((flags, fflags)) = parse_mode(unsafe { core::slice::from_raw_parts(mode, cstr::strlen(mode)) }) else {
        crate::set_errno(EINVAL);
        return null_mut();
    };
    // SAFETY: as above.
    let fd = unsafe { open(path, flags, 0o666) };
    if fd < 0 {
        return null_mut();
    }
    let f = make(fd, fflags, 0);
    if f.is_null() {
        // SAFETY: our descriptor.
        unsafe { close(fd) };
    }
    f
}

#[unsafe(no_mangle)]
unsafe extern "C" fn fdopen(fd: i32, mode: *const u8) -> *mut File {
    // SAFETY: the C contract: a string.
    match parse_mode(unsafe { core::slice::from_raw_parts(mode, cstr::strlen(mode)) }) {
        Some((_, fflags)) => make(fd, fflags, 0),
        None => {
            crate::set_errno(EINVAL);
            null_mut()
        }
    }
}

/// Flush `f` and drop its read-ahead, so its descriptor is where the
/// program thinks.
///
/// # Safety
/// `f` is a stream.
unsafe fn settle(f: *mut File) -> i32 {
    // SAFETY: the caller's contract.
    unsafe {
        let mut r = if flush(f).is_ok() { 0 } else { stdio::EOF };
        if let Some(s) = stream(f)
            && s.drop_input(&mut Fd).is_err()
        {
            r = stdio::EOF;
        }
        r
    }
}

#[unsafe(no_mangle)]
unsafe extern "C" fn fclose(f: *mut File) -> i32 {
    // SAFETY: the C contract: a stream.
    unsafe {
        let r = settle(f);
        let fd = i32::from((*f).file);
        let closed = close(fd);
        if let Some((slot, o)) = opened(f) {
            open_list()[slot] = null_mut();
            free(o as *mut u8);
        } else {
            // A standard stream: it stays, closed.
            (*f).flags = 0;
        }
        if r != 0 || closed != 0 { stdio::EOF } else { 0 }
    }
}

#[unsafe(no_mangle)]
unsafe extern "C" fn fread(p: *mut u8, size: usize, n: usize, f: *mut File) -> usize {
    let Some(total) = size.checked_mul(n) else { return 0 };
    if total == 0 {
        return 0;
    }
    // SAFETY: the C contract: `p` holds size × n bytes; `f` is a stream.
    unsafe {
        if f == std_file(0) {
            flush_all();
        }
        let Some(s) = stream(f) else {
            crate::set_errno(EBADF);
            return 0;
        };
        if (*f).flags & (stdio::SRD | stdio::SRW) == 0 {
            (*f).flags |= stdio::SERR;
            crate::set_errno(EBADF);
            return 0;
        }
        if s.pending() > 0 && flush(f).is_err() {
            return 0;
        }
        match s.read(core::slice::from_raw_parts_mut(p, total), &mut Fd) {
            Ok(got) => {
                if got < total {
                    (*f).flags |= stdio::SEOF;
                }
                got / size
            }
            Err(e) => {
                (*f).flags |= stdio::SERR;
                crate::set_errno(e);
                0
            }
        }
    }
}

#[unsafe(no_mangle)]
unsafe extern "C" fn fgets(buf: *mut u8, n: i32, f: *mut File) -> *mut u8 {
    if n <= 0 {
        return null_mut();
    }
    let mut i = 0usize;
    // SAFETY: the C contract: `buf` holds `n` bytes; `f` is a stream.
    unsafe {
        while i + 1 < n as usize {
            let c = get(f);
            if c == stdio::EOF {
                break;
            }
            *buf.add(i) = c as u8;
            i += 1;
            if c == i32::from(b'\n') {
                break;
            }
        }
        if i == 0 {
            return null_mut();
        }
        *buf.add(i) = 0;
    }
    buf
}

#[unsafe(no_mangle)]
unsafe extern "C" fn ungetc(c: i32, f: *mut File) -> i32 {
    if c == stdio::EOF {
        return stdio::EOF;
    }
    // SAFETY: the C contract.
    unsafe {
        let Some(s) = stream(f) else { return stdio::EOF };
        if !s.ungetc(c as u8) {
            return stdio::EOF;
        }
        (*f).flags &= !stdio::SEOF;
    }
    i32::from(c as u8)
}

#[unsafe(no_mangle)]
unsafe extern "C" fn fseeko(f: *mut File, offset: i64, whence: i32) -> i32 {
    // SAFETY: the C contract.
    unsafe {
        let Some(s) = stream(f) else {
            crate::set_errno(EBADF);
            return -1;
        };
        match s.seek(offset, whence, &mut Fd, &mut Fd) {
            Ok(_) => {
                (*f).flags &= !stdio::SEOF;
                0
            }
            Err(e) => {
                crate::set_errno(e);
                -1
            }
        }
    }
}

#[unsafe(no_mangle)]
unsafe extern "C" fn fseek(f: *mut File, offset: i64, whence: i32) -> i32 {
    // SAFETY: the C contract.
    unsafe { fseeko(f, offset, whence) }
}

#[unsafe(no_mangle)]
unsafe extern "C" fn ftello(f: *mut File) -> i64 {
    // SAFETY: the C contract.
    unsafe {
        let Some(s) = stream(f) else {
            crate::set_errno(EBADF);
            return -1;
        };
        s.tell(&mut Fd).unwrap_or_else(|e| {
            crate::set_errno(e);
            -1
        })
    }
}

#[unsafe(no_mangle)]
unsafe extern "C" fn ftell(f: *mut File) -> i64 {
    // SAFETY: the C contract.
    unsafe { ftello(f) }
}

#[unsafe(no_mangle)]
unsafe extern "C" fn rewind(f: *mut File) {
    // SAFETY: the C contract.
    unsafe {
        let _ = fseeko(f, 0, 0);
        (*f).flags &= !stdio::SERR;
    }
}

#[unsafe(no_mangle)]
unsafe extern "C" fn setvbuf(f: *mut File, _buf: *mut u8, mode: i32, _size: usize) -> i32 {
    // SAFETY: the C contract. The stream keeps its own buffer.
    unsafe {
        let Some(s) = stream(f) else { return -1 };
        s.mode = match mode {
            stdio::IOFBF => Mode::Full,
            stdio::IOLBF => Mode::Line,
            stdio::IONBF => Mode::Unbuffered,
            _ => return -1,
        };
    }
    0
}

#[unsafe(no_mangle)]
unsafe extern "C" fn setbuf(f: *mut File, buf: *mut u8) {
    // SAFETY: the C contract.
    unsafe {
        setvbuf(f, buf, if buf.is_null() { stdio::IONBF } else { stdio::IOFBF }, stdio::BUFSIZ);
    }
}

#[unsafe(no_mangle)]
unsafe extern "C" fn freopen(path: *const u8, mode: *const u8, f: *mut File) -> *mut File {
    // SAFETY: the C contract.
    unsafe {
        let Some((flags, fflags)) = parse_mode(core::slice::from_raw_parts(mode, cstr::strlen(mode))) else {
            crate::set_errno(EINVAL);
            return null_mut();
        };
        settle(f);
        let old = i32::from((*f).file);
        if path.is_null() {
            (*f).flags = fflags;
            return f;
        }
        close(old);
        let fd = open(path, flags, 0o666);
        if fd < 0 {
            (*f).flags = 0;
            return null_mut();
        }
        // Keep the descriptor number of a standard stream.
        let fd = if (0..=2).contains(&old) && fd != old {
            dup2(fd, old);
            close(fd);
            old
        } else {
            fd
        };
        (*f).file = fd as i16;
        (*f).flags = fflags;
        if let Some(s) = stream(f) {
            s.fd = fd;
        }
        f
    }
}

#[unsafe(no_mangle)]
extern "C" fn flockfile(_f: *mut File) {}

#[unsafe(no_mangle)]
extern "C" fn funlockfile(_f: *mut File) {}

#[unsafe(no_mangle)]
extern "C" fn ftrylockfile(_f: *mut File) -> i32 {
    0
}

/// Replace the trailing `X`s of `template` with random letters and digits
/// and create the file (`mkstemp(3)`).
#[unsafe(no_mangle)]
unsafe extern "C" fn mkstemp(template: *mut u8) -> i32 {
    const LETTERS: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
    // SAFETY: the C contract: a writable string.
    unsafe {
        let len = cstr::strlen(template);
        let t = core::slice::from_raw_parts_mut(template, len);
        let xs = t.iter().rev().take_while(|&&c| c == b'X').count();
        if xs < 6 {
            crate::set_errno(EINVAL);
            return -1;
        }
        for _ in 0..100 {
            let mut rnd = [0u8; 16];
            getentropy(rnd.as_mut_ptr(), rnd.len());
            for (i, c) in t[len - xs..].iter_mut().enumerate() {
                *c = LETTERS[usize::from(rnd[i % 16].wrapping_add(i as u8)) % LETTERS.len()];
            }
            let fd = open(template, O_RDWR | O_CREAT | O_EXCL, 0o600);
            if fd >= 0 || crate::errno() != EEXIST {
                return fd;
            }
        }
        -1
    }
}

#[unsafe(no_mangle)]
unsafe extern "C" fn tmpfile() -> *mut File {
    let mut name = *b"/tmp/tmp.XXXXXXXX\0";
    // SAFETY: our own buffer.
    unsafe {
        let fd = mkstemp(name.as_mut_ptr());
        if fd < 0 {
            return null_mut();
        }
        unlink(name.as_ptr());
        make(fd, stdio::SRW, 0)
    }
}

#[unsafe(no_mangle)]
unsafe extern "C" fn remove(path: *const u8) -> i32 {
    // SAFETY: the C contract: a string.
    unsafe {
        if unlink(path) == 0 {
            return 0;
        }
        let e = crate::errno();
        if e == EPERM || e == EISDIR { rmdir(path) } else { -1 }
    }
}

/// Start `/bin/sh -c cmd`; its pid, or -1 with `errno`.
///
/// # Safety
/// `cmd` is a string.
unsafe fn spawn_shell(cmd: *const u8) -> i32 {
    let argv: [*const u8; 4] = [c"sh".as_ptr() as *const u8, c"-c".as_ptr() as *const u8, cmd, core::ptr::null()];
    let mut pid = 0;
    // SAFETY: the arguments are strings; `environ` is the environment.
    let r = unsafe {
        posix_spawn(
            &mut pid,
            c"/bin/sh".as_ptr() as *const u8,
            core::ptr::null(),
            core::ptr::null(),
            argv.as_ptr(),
            environ,
        )
    };
    if r != 0 {
        crate::set_errno(r);
        return -1;
    }
    pid
}

#[unsafe(no_mangle)]
unsafe extern "C" fn system(cmd: *const u8) -> i32 {
    // SAFETY: the C contract: a string or null.
    unsafe {
        if cmd.is_null() {
            return i32::from(access(c"/bin/sh".as_ptr() as *const u8, X_OK) == 0);
        }
        flush_all();
        let pid = spawn_shell(cmd);
        if pid < 0 {
            // The shell could not run: as if it exited with 127.
            return 127 << 8;
        }
        let mut status = 0;
        while wait4(pid, &mut status, 0, null_mut()) < 0 {
            if crate::errno() != EINTR {
                return -1;
            }
        }
        status
    }
}

#[unsafe(no_mangle)]
unsafe extern "C" fn popen(cmd: *const u8, mode: *const u8) -> *mut File {
    // SAFETY: the C contract: two strings.
    unsafe {
        let reading = *mode == b'r';
        if !reading && *mode != b'w' {
            crate::set_errno(EINVAL);
            return null_mut();
        }
        let mut fds = [0i32; 2];
        if pipe(fds.as_mut_ptr()) != 0 {
            return null_mut();
        }
        // The child's end goes on its stdin or stdout for the spawn (no
        // file actions yet), then the parent's descriptor comes back.
        let (ours, theirs, target) = if reading { (fds[0], fds[1], 1) } else { (fds[1], fds[0], 0) };
        flush_all();
        let saved = dup(target);
        dup2(theirs, target);
        close(theirs);
        fcntl(ours, 2, 1); // F_SETFD FD_CLOEXEC: the child must not keep our end.
        let pid = spawn_shell(cmd);
        dup2(saved, target);
        close(saved);
        if pid < 0 {
            close(ours);
            return null_mut();
        }
        make(ours, if reading { stdio::SRD } else { stdio::SWR }, pid)
    }
}

#[unsafe(no_mangle)]
unsafe extern "C" fn pclose(f: *mut File) -> i32 {
    let Some((_, o)) = opened(f) else {
        crate::set_errno(EINVAL);
        return -1;
    };
    // SAFETY: a stream popen made.
    unsafe {
        let pid = (*o).child;
        fclose(f);
        let mut status = 0;
        while wait4(pid, &mut status, 0, null_mut()) < 0 {
            if crate::errno() != EINTR {
                return -1;
            }
        }
        status
    }
}

// ------------------------------------------------------------ directories

/// `DIR`: a descriptor and a buffer of 64-bit `struct dirent` records as
/// `__getdirentries64` returns them (`<sys/dirent.h>`). Programs treat it
/// as opaque.
#[repr(C)]
struct Dir {
    fd: i32,
    pos: usize,
    len: usize,
    base: i64,
    buf: [u8; 8192],
}

unsafe fn opendir_impl(path: *const u8) -> *mut Dir {
    // SAFETY: the C contract: a string.
    unsafe {
        let fd = open(path, O_RDONLY | O_DIRECTORY | O_CLOEXEC, 0);
        if fd < 0 {
            return null_mut();
        }
        let d = malloc(core::mem::size_of::<Dir>()) as *mut Dir;
        if d.is_null() {
            close(fd);
            crate::set_errno(ENOMEM);
            return null_mut();
        }
        (*d).fd = fd;
        (*d).pos = 0;
        (*d).len = 0;
        (*d).base = 0;
        d
    }
}

unsafe fn readdir_impl(d: *mut Dir) -> *mut u8 {
    // SAFETY: the C contract: a DIR from opendir.
    unsafe {
        let d = &mut *d;
        if d.pos >= d.len {
            let n = __getdirentries64(d.fd, d.buf.as_mut_ptr(), d.buf.len(), &mut d.base);
            if n <= 0 {
                return null_mut();
            }
            d.len = n as usize;
            d.pos = 0;
        }
        let rec = d.buf.as_mut_ptr().add(d.pos);
        let reclen = usize::from(u16::from_le_bytes([*rec.add(16), *rec.add(17)]));
        if reclen == 0 {
            d.len = 0;
            return null_mut();
        }
        d.pos += reclen;
        rec
    }
}

#[cfg_attr(target_arch = "x86_64", unsafe(export_name = "opendir$INODE64"))]
#[cfg_attr(target_arch = "aarch64", unsafe(no_mangle))]
unsafe extern "C" fn opendir(path: *const u8) -> *mut c_void {
    // SAFETY: the C contract.
    unsafe { opendir_impl(path) as *mut c_void }
}

#[cfg_attr(target_arch = "x86_64", unsafe(export_name = "readdir$INODE64"))]
#[cfg_attr(target_arch = "aarch64", unsafe(no_mangle))]
unsafe extern "C" fn readdir(d: *mut c_void) -> *mut u8 {
    // SAFETY: the C contract.
    unsafe { readdir_impl(d as *mut Dir) }
}

#[unsafe(no_mangle)]
unsafe extern "C" fn closedir(d: *mut c_void) -> i32 {
    // SAFETY: the C contract: a DIR from opendir.
    unsafe {
        let r = close((*(d as *mut Dir)).fd);
        free(d as *mut u8);
        r
    }
}

#[unsafe(no_mangle)]
unsafe extern "C" fn dirfd(d: *mut c_void) -> i32 {
    // SAFETY: the C contract.
    unsafe { (*(d as *mut Dir)).fd }
}

/// `getcwd`: the path of `.` (`fcntl F_GETPATH`).
#[unsafe(no_mangle)]
unsafe extern "C" fn getcwd(buf: *mut u8, size: usize) -> *mut u8 {
    let mut path = [0u8; PATH_MAX];
    // SAFETY: our buffer is PATH_MAX bytes, as F_GETPATH needs.
    unsafe {
        let fd = open(c".".as_ptr() as *const u8, O_RDONLY | O_DIRECTORY | O_CLOEXEC, 0);
        if fd < 0 {
            return null_mut();
        }
        let r = fcntl(fd, F_GETPATH, path.as_mut_ptr() as usize);
        close(fd);
        if r < 0 {
            return null_mut();
        }
        let n = cstr::strlen(path.as_ptr());
        let (out, room) = if buf.is_null() {
            let want = if size == 0 { n + 1 } else { size };
            (malloc(want), want)
        } else {
            (buf, size)
        };
        if out.is_null() {
            crate::set_errno(ENOMEM);
            return null_mut();
        }
        if n + 1 > room {
            crate::set_errno(ERANGE_);
            return null_mut();
        }
        core::ptr::copy_nonoverlapping(path.as_ptr(), out, n + 1);
        out
    }
}
