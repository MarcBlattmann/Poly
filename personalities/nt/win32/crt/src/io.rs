//! The low-level I/O and file-system functions (Learn: each function's
//! page): descriptors over [`crate::lowio`], and names given to `kernel32`
//! (narrow names through the ANSI code page).

use crate::global::{self, Global};
use crate::lowio;
use crate::sys::{self, Handle};
use crate::text;
use alloc::vec::Vec;
use poly_crt::errno::{EACCES, EBADF, EINVAL, ENOENT, ERANGE};
use poly_crt::fmode::{self, Translation};

/// Open `name` (wide, with its NUL); a descriptor or −1.
fn open_wide(name: &[u16], flags: i32, share: i32, pmode: i32) -> i32 {
    let Some(args) = fmode::create_file_args(flags, share, pmode) else {
        global::invalid(EINVAL);
        return -1;
    };
    // SAFETY: a NUL-terminated name.
    let h = unsafe {
        sys::CreateFileW(
            name.as_ptr(),
            args.access,
            args.share,
            core::ptr::null(),
            args.disposition,
            args.attributes,
            0,
        )
    };
    if h == sys::INVALID_HANDLE_VALUE {
        global::set_errno_from_win32();
        return -1;
    }
    let mode = Translation::of(flags, crate::startup::fmode());
    let fd = lowio::allocate_with(h, mode, flags & fmode::O_APPEND != 0, flags & fmode::O_NOINHERIT != 0);
    if fd < 0 {
        // SAFETY: the handle just opened.
        unsafe { sys::CloseHandle(h) };
    }
    fd
}

/// `_open(name, flags, pmode)`: `pmode` is read only with `_O_CREAT`.
///
/// # Safety
/// `name` is a valid C string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _open(name: *const u8, flags: i32, pmode: i32) -> i32 {
    if name.is_null() {
        global::invalid(EINVAL);
        return -1;
    }
    // SAFETY: the caller's string.
    open_wide(&unsafe { text::wide_path(name) }, flags, fmode::SH_DENYNO, pmode)
}

/// # Safety
/// As `_open`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _wopen(name: *const u16, flags: i32, pmode: i32) -> i32 {
    if name.is_null() {
        global::invalid(EINVAL);
        return -1;
    }
    // SAFETY: the caller's string.
    open_wide(&unsafe { text::wide_path_w(name) }, flags, fmode::SH_DENYNO, pmode)
}

/// # Safety
/// As `_open`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _sopen(name: *const u8, flags: i32, share: i32, pmode: i32) -> i32 {
    if name.is_null() {
        global::invalid(EINVAL);
        return -1;
    }
    // SAFETY: the caller's string.
    open_wide(&unsafe { text::wide_path(name) }, flags, share, pmode)
}

/// # Safety
/// As `_open`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _wsopen(name: *const u16, flags: i32, share: i32, pmode: i32) -> i32 {
    if name.is_null() {
        global::invalid(EINVAL);
        return -1;
    }
    // SAFETY: the caller's string.
    open_wide(&unsafe { text::wide_path_w(name) }, flags, share, pmode)
}

/// `_sopen_s(&fd, …)`: 0 or the `errno`.
///
/// # Safety
/// `fd` is writable; `name` a valid C string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _sopen_s(fd: *mut i32, name: *const u8, flags: i32, share: i32, pmode: i32) -> i32 {
    if fd.is_null() || name.is_null() {
        global::invalid(EINVAL);
        return EINVAL;
    }
    // SAFETY: the caller's string.
    let r = open_wide(&unsafe { text::wide_path(name) }, flags, share, pmode);
    // SAFETY: writable.
    unsafe { *fd = r };
    if r < 0 { global::thread().errno } else { 0 }
}

/// # Safety
/// As `_sopen_s`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _wsopen_s(fd: *mut i32, name: *const u16, flags: i32, share: i32, pmode: i32) -> i32 {
    if fd.is_null() || name.is_null() {
        global::invalid(EINVAL);
        return EINVAL;
    }
    // SAFETY: the caller's string.
    let r = open_wide(&unsafe { text::wide_path_w(name) }, flags, share, pmode);
    // SAFETY: writable.
    unsafe { *fd = r };
    if r < 0 { global::thread().errno } else { 0 }
}

/// `_creat`: `_open(name, _O_CREAT | _O_TRUNC | _O_RDWR, pmode)`.
///
/// # Safety
/// As `_open`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _creat(name: *const u8, pmode: i32) -> i32 {
    // SAFETY: the caller's string.
    unsafe { _open(name, fmode::O_CREAT | fmode::O_TRUNC | fmode::O_RDWR, pmode) }
}

/// # Safety
/// As `_open`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _wcreat(name: *const u16, pmode: i32) -> i32 {
    // SAFETY: the caller's string.
    unsafe { _wopen(name, fmode::O_CREAT | fmode::O_TRUNC | fmode::O_RDWR, pmode) }
}

#[unsafe(no_mangle)]
pub extern "C" fn _close(fd: i32) -> i32 {
    if lowio::check(fd).is_err() {
        return -1;
    }
    let Some(e) = lowio::release(fd) else { return -1 };
    if e.handle != lowio::NO_HANDLE {
        // SAFETY: the descriptor's handle.
        if unsafe { sys::CloseHandle(e.handle) } == 0 {
            global::set_errno_from_win32();
            return -1;
        }
    }
    0
}

/// # Safety
/// `buf` holds `count` writable bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _read(fd: i32, buf: *mut u8, count: u32) -> i32 {
    if lowio::check(fd).is_err() {
        return -1;
    }
    if buf.is_null() || count > i32::MAX as u32 {
        global::invalid(EINVAL);
        return -1;
    }
    // SAFETY: the caller's buffer.
    let b = unsafe { core::slice::from_raw_parts_mut(buf, count as usize) };
    lowio::read(fd, b).map_or(-1, |n| n as i32)
}

/// # Safety
/// `buf` holds `count` readable bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _write(fd: i32, buf: *const u8, count: u32) -> i32 {
    if lowio::check(fd).is_err() {
        return -1;
    }
    if count == 0 {
        return 0;
    }
    if buf.is_null() {
        global::invalid(EINVAL);
        return -1;
    }
    // SAFETY: the caller's buffer.
    let b = unsafe { core::slice::from_raw_parts(buf, count as usize) };
    lowio::write(fd, b).map_or(-1, |n| n as i32)
}

#[unsafe(no_mangle)]
pub extern "C" fn _lseeki64(fd: i32, offset: i64, origin: i32) -> i64 {
    if lowio::check(fd).is_err() {
        return -1;
    }
    lowio::seek(fd, offset, origin)
}

/// `_lseek`: a position beyond 2 GiB cannot be returned (−1, `EINVAL`).
#[unsafe(no_mangle)]
pub extern "C" fn _lseek(fd: i32, offset: i32, origin: i32) -> i32 {
    let r = _lseeki64(fd, i64::from(offset), origin);
    if r > i64::from(i32::MAX) {
        global::set_errno(EINVAL);
        return -1;
    }
    r as i32
}

#[unsafe(no_mangle)]
pub extern "C" fn _telli64(fd: i32) -> i64 {
    _lseeki64(fd, 0, 1)
}

#[unsafe(no_mangle)]
pub extern "C" fn _tell(fd: i32) -> i32 {
    _lseek(fd, 0, 1)
}

fn file_size(h: Handle) -> Option<i64> {
    let mut size = 0i64;
    // SAFETY: `size` writable.
    (unsafe { sys::GetFileSizeEx(h, &mut size) } != 0).then_some(size)
}

#[unsafe(no_mangle)]
pub extern "C" fn _filelengthi64(fd: i32) -> i64 {
    let Ok(e) = lowio::check(fd) else { return -1 };
    file_size(e.handle).unwrap_or_else(|| {
        global::set_errno_from_win32();
        -1
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn _filelength(fd: i32) -> i32 {
    let n = _filelengthi64(fd);
    if n > i64::from(i32::MAX) {
        global::set_errno(EINVAL);
        return -1;
    }
    n as i32
}

/// `_eof`: 1 at the end of the file, 0 before it, −1 on error.
#[unsafe(no_mangle)]
pub extern "C" fn _eof(fd: i32) -> i32 {
    let Ok(e) = lowio::check(fd) else { return -1 };
    let pos = lowio::seek(fd, 0, 1);
    match (pos, file_size(e.handle)) {
        (p, Some(size)) if p >= 0 => i32::from(p >= size),
        _ => -1,
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn _commit(fd: i32) -> i32 {
    let Ok(e) = lowio::check(fd) else { return -1 };
    // SAFETY: the descriptor's handle.
    if unsafe { sys::FlushFileBuffers(e.handle) } == 0 {
        global::set_errno_from_win32();
        return -1;
    }
    0
}

/// `_chsize_s`: set the size (the file pointer unchanged); 0 or `errno`.
#[unsafe(no_mangle)]
pub extern "C" fn _chsize_s(fd: i32, size: i64) -> i32 {
    let Ok(e) = lowio::check(fd) else { return EBADF };
    if size < 0 {
        global::invalid(EINVAL);
        return EINVAL;
    }
    let here = lowio::seek(fd, 0, 1);
    // SAFETY: the descriptor's handle.
    let ok = unsafe {
        sys::SetFilePointerEx(e.handle, size, core::ptr::null_mut(), 0) != 0 && sys::SetEndOfFile(e.handle) != 0
    };
    if here >= 0 {
        // SAFETY: as above.
        unsafe { sys::SetFilePointerEx(e.handle, here, core::ptr::null_mut(), 0) };
    }
    if !ok {
        global::set_errno(EACCES);
        return EACCES;
    }
    0
}

#[unsafe(no_mangle)]
pub extern "C" fn _chsize(fd: i32, size: i32) -> i32 {
    if _chsize_s(fd, i64::from(size)) == 0 { 0 } else { -1 }
}

/// `_get_osfhandle`: the handle, −2 for a standard descriptor without one,
/// −1 with `EBADF` for a bad descriptor.
#[unsafe(no_mangle)]
pub extern "C" fn _get_osfhandle(fd: i32) -> isize {
    match lowio::check(fd) {
        Ok(e) => e.handle,
        Err(_) => -1,
    }
}

/// `_open_osfhandle(handle, flags)`: `_O_APPEND`, `_O_RDONLY`, `_O_TEXT`,
/// `_O_WTEXT`, `_O_U16TEXT`, `_O_U8TEXT`, `_O_NOINHERIT` (no flag: the
/// default `_fmode`'s translation).
#[unsafe(no_mangle)]
pub extern "C" fn _open_osfhandle(handle: isize, flags: i32) -> i32 {
    if handle == sys::INVALID_HANDLE_VALUE {
        global::set_errno(EBADF);
        return -1;
    }
    let mode = Translation::of(flags, crate::startup::fmode());
    lowio::allocate_with(handle, mode, flags & fmode::O_APPEND != 0, flags & fmode::O_NOINHERIT != 0)
}

// ----- _dup, _dup2, _pipe (ADR 0093) ------------------------------------------------

/// A second handle to `h`'s object in this process (`inherit`: for a child).
pub(crate) fn duplicate(h: Handle, inherit: bool) -> Option<Handle> {
    let mut out: Handle = 0;
    // SAFETY: the pseudo handle of this process and a handle of it; `out`
    // writable.
    let ok = unsafe {
        let me = sys::GetCurrentProcess();
        sys::DuplicateHandle(me, h, me, &mut out, 0, i32::from(inherit), sys::DUPLICATE_SAME_ACCESS)
    };
    (ok != 0).then_some(out)
}

/// `_dup(fd)`: the lowest free descriptor for a new handle to `fd`'s file,
/// in `fd`'s mode (Learn: "the new file descriptor ... refers to the same
/// file"); −1 with `EBADF` or `EMFILE`.
#[unsafe(no_mangle)]
pub extern "C" fn _dup(fd: i32) -> i32 {
    let Ok(e) = lowio::check(fd) else { return -1 };
    if e.handle == lowio::NO_HANDLE {
        global::set_errno(EBADF);
        return -1;
    }
    let Some(h) = duplicate(e.handle, false) else {
        global::set_errno_from_win32();
        return -1;
    };
    let new = lowio::allocate_with(h, e.mode, e.append, e.noinherit);
    if new < 0 {
        // SAFETY: the handle just made.
        unsafe { sys::CloseHandle(h) };
    }
    new
}

/// `_dup2(fd1, fd2)`: `fd2` refers to `fd1`'s file, closing what `fd2` had
/// (Learn); 0, or −1 with `EBADF`.
#[unsafe(no_mangle)]
pub extern "C" fn _dup2(fd1: i32, fd2: i32) -> i32 {
    let Ok(e) = lowio::check(fd1) else { return -1 };
    if fd1 == fd2 {
        return 0;
    }
    if e.handle == lowio::NO_HANDLE {
        global::set_errno(EBADF);
        return -1;
    }
    let Some(h) = duplicate(e.handle, false) else {
        global::set_errno_from_win32();
        return -1;
    };
    let copy = lowio::Fd { handle: h, eof: false, pending: None, surrogate: 0, ..e };
    match lowio::place(fd2, copy) {
        Ok(old) => {
            if let Some(old) = old.filter(|o| o.handle != lowio::NO_HANDLE) {
                // SAFETY: the replaced descriptor's own handle.
                unsafe { sys::CloseHandle(old.handle) };
            }
            0
        }
        Err(lowio::Failed) => {
            // SAFETY: the handle just made.
            unsafe { sys::CloseHandle(h) };
            -1
        }
    }
}

/// `_pipe(fds, size, textmode)`: an anonymous pipe (`CreatePipe`), its read
/// end in `fds[0]` and its write end in `fds[1]`, in `textmode`'s
/// translation (`_O_BINARY`, `_O_TEXT`, a Unicode mode, or `_fmode`'s with
/// none); `_O_NOINHERIT` keeps both from a child. 0, or −1 with `EINVAL` or
/// `EMFILE` (Learn).
///
/// # Safety
/// `fds` holds two writable `int`s.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _pipe(fds: *mut i32, size: u32, textmode: i32) -> i32 {
    const KNOWN: i32 =
        fmode::O_BINARY | fmode::O_TEXT | fmode::O_WTEXT | fmode::O_U16TEXT | fmode::O_U8TEXT | fmode::O_NOINHERIT;
    if fds.is_null() || textmode & !KNOWN != 0 {
        global::invalid(EINVAL);
        return -1;
    }
    let (mut r, mut w): (Handle, Handle) = (0, 0);
    // SAFETY: `r` and `w` writable; no security attributes.
    if unsafe { sys::CreatePipe(&mut r, &mut w, core::ptr::null(), size) } == 0 {
        global::set_errno_from_win32();
        return -1;
    }
    let mode = Translation::of(textmode, crate::startup::fmode());
    let noinherit = textmode & fmode::O_NOINHERIT != 0;
    let rfd = lowio::allocate_with(r, mode, false, noinherit);
    let wfd = if rfd < 0 { -1 } else { lowio::allocate_with(w, mode, false, noinherit) };
    if wfd < 0 {
        if rfd >= 0 {
            lowio::release(rfd);
        }
        // SAFETY: the handles just made.
        unsafe {
            sys::CloseHandle(r);
            sys::CloseHandle(w);
        }
        return -1;
    }
    // SAFETY: the caller's two ints.
    unsafe {
        fds.write(rfd);
        fds.add(1).write(wfd);
    }
    0
}

/// `_isatty`: non-zero for a character device.
#[unsafe(no_mangle)]
pub extern "C" fn _isatty(fd: i32) -> i32 {
    match lowio::check(fd) {
        Ok(e) => i32::from(e.kind == sys::FILE_TYPE_CHAR) * 0x40,
        Err(lowio::Failed) => 0,
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn _setmode(fd: i32, mode: i32) -> i32 {
    lowio::set_mode(fd, mode)
}

static UMASK: Global<i32> = Global::new(0);

/// `_umask`: the previous mask (Poly honours only `_S_IWRITE` on create
/// through `pmode`; the mask is kept for `_umask` itself).
#[unsafe(no_mangle)]
pub extern "C" fn _umask(mask: i32) -> i32 {
    // SAFETY: a plain word; races are the program's.
    unsafe { core::mem::replace(&mut *UMASK.get(), mask & (fmode::S_IREAD | fmode::S_IWRITE)) }
}

// ----- names --------------------------------------------------------------------

/// `_access_s`'s check of a wide name (with NUL): 0 or the `errno`.
fn access_wide(name: &[u16], mode: i32) -> i32 {
    if mode & !6 != 0 {
        global::invalid(EINVAL);
        return EINVAL;
    }
    // SAFETY: a NUL-terminated name.
    let attrs = unsafe { sys::GetFileAttributesW(name.as_ptr()) };
    if attrs == sys::INVALID_FILE_ATTRIBUTES {
        global::set_errno_from_win32();
        return global::thread().errno;
    }
    if mode & 2 != 0 && attrs & sys::FILE_ATTRIBUTE_READONLY != 0 && attrs & sys::FILE_ATTRIBUTE_DIRECTORY == 0 {
        global::set_errno(EACCES);
        return EACCES;
    }
    0
}

/// # Safety
/// `name` is a valid C string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _access(name: *const u8, mode: i32) -> i32 {
    if name.is_null() {
        global::invalid(EINVAL);
        return -1;
    }
    // SAFETY: the caller's string.
    if access_wide(&unsafe { text::wide_path(name) }, mode) == 0 { 0 } else { -1 }
}

/// # Safety
/// As `_access`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _waccess(name: *const u16, mode: i32) -> i32 {
    if name.is_null() {
        global::invalid(EINVAL);
        return -1;
    }
    // SAFETY: the caller's string.
    if access_wide(&unsafe { text::wide_path_w(name) }, mode) == 0 { 0 } else { -1 }
}

fn result(ok: i32) -> i32 {
    if ok != 0 {
        0
    } else {
        global::set_errno_from_win32();
        -1
    }
}

/// # Safety
/// `name` is a valid C string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn remove(name: *const u8) -> i32 {
    // SAFETY: the caller's string; DeleteFileW takes a NUL-terminated name.
    result(unsafe { sys::DeleteFileW(text::wide_path(name).as_ptr()) })
}

/// # Safety
/// As `remove`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _wremove(name: *const u16) -> i32 {
    // SAFETY: the caller's string.
    result(unsafe { sys::DeleteFileW(name) })
}

/// # Safety
/// As `remove`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _unlink(name: *const u8) -> i32 {
    // SAFETY: the caller's string.
    unsafe { remove(name) }
}

/// # Safety
/// As `remove`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _wunlink(name: *const u16) -> i32 {
    // SAFETY: the caller's string.
    unsafe { _wremove(name) }
}

/// `rename`: an existing target is an error (`EACCES`, Learn: "The new name
/// must not be the name of an existing file").
///
/// # Safety
/// Both are valid C strings.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rename(from: *const u8, to: *const u8) -> i32 {
    // SAFETY: the caller's strings.
    let (f, t) = unsafe { (text::wide_path(from), text::wide_path(to)) };
    // SAFETY: NUL-terminated names.
    result(unsafe { sys::MoveFileExW(f.as_ptr(), t.as_ptr(), sys::MOVEFILE_COPY_ALLOWED) })
}

/// # Safety
/// As `rename`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _wrename(from: *const u16, to: *const u16) -> i32 {
    // SAFETY: the caller's strings.
    result(unsafe { sys::MoveFileExW(from, to, sys::MOVEFILE_COPY_ALLOWED) })
}

/// # Safety
/// `name` is a valid C string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _mkdir(name: *const u8) -> i32 {
    // SAFETY: the caller's string.
    result(unsafe { sys::CreateDirectoryW(text::wide_path(name).as_ptr(), core::ptr::null()) })
}

/// # Safety
/// As `_mkdir`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _wmkdir(name: *const u16) -> i32 {
    // SAFETY: the caller's string.
    result(unsafe { sys::CreateDirectoryW(name, core::ptr::null()) })
}

/// # Safety
/// As `_mkdir`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _rmdir(name: *const u8) -> i32 {
    // SAFETY: the caller's string.
    result(unsafe { sys::RemoveDirectoryW(text::wide_path(name).as_ptr()) })
}

/// # Safety
/// As `_mkdir`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _wrmdir(name: *const u16) -> i32 {
    // SAFETY: the caller's string.
    result(unsafe { sys::RemoveDirectoryW(name) })
}

/// # Safety
/// As `_mkdir`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _chdir(name: *const u8) -> i32 {
    // SAFETY: the caller's string.
    result(unsafe { sys::SetCurrentDirectoryW(text::wide_path(name).as_ptr()) })
}

/// # Safety
/// As `_mkdir`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _wchdir(name: *const u16) -> i32 {
    // SAFETY: the caller's string.
    result(unsafe { sys::SetCurrentDirectoryW(name) })
}

fn current_dir() -> Vec<u16> {
    // SAFETY: a size query.
    let n = unsafe { sys::GetCurrentDirectoryW(0, core::ptr::null_mut()) };
    let mut buf = alloc::vec![0u16; n as usize + 1];
    // SAFETY: `buf` holds n + 1 units.
    let got = unsafe { sys::GetCurrentDirectoryW(buf.len() as u32, buf.as_mut_ptr()) };
    buf.truncate(got as usize);
    buf
}

/// Copy `s` and a NUL to a caller's buffer of `size` units, or to a new
/// block of at least `size` units when `buf` is NULL (Learn: `_getcwd`).
fn give<T: Copy + Default>(s: &[T], buf: *mut T, size: usize) -> *mut T {
    if buf.is_null() {
        let n = (s.len() + 1).max(size);
        let p = crate::heap::malloc(n * core::mem::size_of::<T>()) as *mut T;
        if !p.is_null() {
            // SAFETY: n >= s.len() + 1 units.
            unsafe {
                core::ptr::copy_nonoverlapping(s.as_ptr(), p, s.len());
                p.add(s.len()).write(T::default());
            }
        }
        return p;
    }
    if size == 0 {
        global::invalid(EINVAL);
        return core::ptr::null_mut();
    }
    if s.len() + 1 > size {
        global::set_errno(ERANGE);
        return core::ptr::null_mut();
    }
    // SAFETY: the caller's buffer holds `size` units.
    unsafe {
        core::ptr::copy_nonoverlapping(s.as_ptr(), buf, s.len());
        buf.add(s.len()).write(T::default());
    }
    buf
}

/// # Safety
/// `buf` is NULL or holds `size` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _getcwd(buf: *mut u8, size: i32) -> *mut u8 {
    give(&text::wide_to_ansi(&current_dir()), buf, size.max(0) as usize)
}

/// # Safety
/// `buf` is NULL or holds `size` units.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _wgetcwd(buf: *mut u16, size: i32) -> *mut u16 {
    give(&current_dir(), buf, size.max(0) as usize)
}

pub(crate) fn full_path(name: &[u16]) -> Option<Vec<u16>> {
    // SAFETY: a size query; `name` has its NUL.
    let n = unsafe { sys::GetFullPathNameW(name.as_ptr(), 0, core::ptr::null_mut(), core::ptr::null_mut()) };
    if n == 0 {
        global::set_errno_from_win32();
        return None;
    }
    let mut buf = alloc::vec![0u16; n as usize];
    // SAFETY: `buf` holds n units.
    let got = unsafe { sys::GetFullPathNameW(name.as_ptr(), n, buf.as_mut_ptr(), core::ptr::null_mut()) };
    buf.truncate(got as usize);
    Some(buf)
}

/// # Safety
/// `name` is a valid C string; `buf` NULL or `size` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _fullpath(buf: *mut u8, name: *const u8, size: usize) -> *mut u8 {
    if name.is_null() {
        // SAFETY: no name is the current directory.
        return unsafe { _getcwd(buf, size as i32) };
    }
    // SAFETY: the caller's string.
    match full_path(&unsafe { text::wide_path(name) }) {
        Some(p) => give(&text::wide_to_ansi(&p), buf, size),
        None => core::ptr::null_mut(),
    }
}

/// # Safety
/// As `_fullpath`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _wfullpath(buf: *mut u16, name: *const u16, size: usize) -> *mut u16 {
    if name.is_null() {
        // SAFETY: as above.
        return unsafe { _wgetcwd(buf, size as i32) };
    }
    // SAFETY: the caller's string.
    match full_path(&unsafe { text::wide_path_w(name) }) {
        Some(p) => give(&p, buf, size),
        None => core::ptr::null_mut(),
    }
}

// ----- _stat ----------------------------------------------------------------------

/// `sys/stat.h`'s mode bits.
const S_IFDIR: u16 = 0x4000;
const S_IFREG: u16 = 0x8000;
const S_IFCHR: u16 = 0x2000;
const S_IFIFO: u16 = 0x1000;

/// What `_stat` reports, before it is laid out.
struct Stat {
    mode: u16,
    drive: u32,
    size: i64,
    atime: i64,
    mtime: i64,
    ctime: i64,
}

fn filetime(ft: u64) -> i64 {
    if ft == 0 { 0 } else { poly_crt::time::from_filetime(ft) }
}

/// The permission bits, replicated for group and others (Learn, `_stat`:
/// "user read/write bits ... the same for all").
fn perm(readonly: bool, exec: bool) -> u16 {
    let mut p = fmode::S_IREAD as u16;
    if !readonly {
        p |= fmode::S_IWRITE as u16;
    }
    if exec {
        p |= 0x40;
    }
    p | (p >> 3) | (p >> 6)
}

fn stat_wide(name: &[u16]) -> Result<Stat, lowio::Failed> {
    // WIN32_FIND_DATAW (minwinbase.h): 592 bytes.
    let mut data = [0u8; 592];
    let path = &name[..name.len() - 1];
    if path.iter().any(|&c| c == u16::from(b'*') || c == u16::from(b'?')) || path.is_empty() {
        global::set_errno(ENOENT);
        return Err(lowio::Failed);
    }
    let full = full_path(name).unwrap_or_default();
    let drive = full.first().map_or(0, |&c| (c as u8).to_ascii_uppercase().wrapping_sub(b'A')) as u32;
    // SAFETY: `data` holds a WIN32_FIND_DATAW.
    let h = unsafe { sys::FindFirstFileW(name.as_ptr(), data.as_mut_ptr()) };
    if h == sys::INVALID_HANDLE_VALUE {
        // A root has no directory entry.
        // SAFETY: a NUL-terminated name.
        let attrs = unsafe { sys::GetFileAttributesW(name.as_ptr()) };
        if attrs == sys::INVALID_FILE_ATTRIBUTES || attrs & sys::FILE_ATTRIBUTE_DIRECTORY == 0 {
            global::set_errno(ENOENT);
            return Err(lowio::Failed);
        }
        return Ok(Stat { mode: S_IFDIR | perm(false, true), drive, size: 0, atime: 0, mtime: 0, ctime: 0 });
    }
    // SAFETY: the find handle.
    unsafe { sys::FindClose(h) };
    let u32_at = |o: usize| u32::from_le_bytes(data[o..o + 4].try_into().unwrap_or_default());
    let u64_at = |o: usize| u64::from(u32_at(o)) | (u64::from(u32_at(o + 4)) << 32);
    let attrs = u32_at(0);
    let dir = attrs & sys::FILE_ATTRIBUTE_DIRECTORY != 0;
    let lower: Vec<u8> = path.iter().rev().take(4).rev().map(|&c| (c as u8).to_ascii_lowercase()).collect();
    let exec = dir || [&b".exe"[..], b".com", b".bat", b".cmd"].contains(&lower.as_slice());
    let mode = if dir { S_IFDIR } else { S_IFREG } | perm(attrs & sys::FILE_ATTRIBUTE_READONLY != 0, exec);
    let size = ((u64::from(u32_at(28)) << 32) | u64::from(u32_at(32))) as i64;
    Ok(Stat {
        mode,
        drive,
        size: if dir { 0 } else { size },
        ctime: filetime(u64_at(4)),
        atime: filetime(u64_at(12)),
        mtime: filetime(u64_at(20)),
    })
}

fn stat_fd(fd: i32) -> Result<Stat, lowio::Failed> {
    let e = lowio::check(fd)?;
    let mode = match e.kind {
        sys::FILE_TYPE_CHAR => S_IFCHR,
        sys::FILE_TYPE_PIPE => S_IFIFO,
        _ => S_IFREG,
    };
    let size = if e.kind == sys::FILE_TYPE_DISK { file_size(e.handle).unwrap_or(0) } else { 0 };
    let mode = mode | perm(false, false);
    Ok(Stat { mode, drive: 0, size, atime: 0, mtime: 0, ctime: 0 })
}

/// Lay out `struct _stat64` (56 bytes) or `_stat64i32` (48 bytes,
/// `sys/stat.h`): dev, ino (u16), mode, nlink, uid, gid, rdev, size, times.
///
/// # Safety
/// `out` holds the structure.
unsafe fn store(st: &Stat, out: *mut u8, wide_size: bool) {
    let mut b = [0u8; 56];
    b[0..4].copy_from_slice(&st.drive.to_le_bytes());
    b[6..8].copy_from_slice(&st.mode.to_le_bytes());
    b[8..10].copy_from_slice(&1i16.to_le_bytes());
    b[16..20].copy_from_slice(&st.drive.to_le_bytes());
    let len = if wide_size {
        b[24..32].copy_from_slice(&st.size.to_le_bytes());
        b[32..40].copy_from_slice(&st.atime.to_le_bytes());
        b[40..48].copy_from_slice(&st.mtime.to_le_bytes());
        b[48..56].copy_from_slice(&st.ctime.to_le_bytes());
        56
    } else {
        b[20..24].copy_from_slice(&(st.size.min(i64::from(i32::MAX)) as i32).to_le_bytes());
        b[24..32].copy_from_slice(&st.atime.to_le_bytes());
        b[32..40].copy_from_slice(&st.mtime.to_le_bytes());
        b[40..48].copy_from_slice(&st.ctime.to_le_bytes());
        48
    };
    // SAFETY: the caller's structure.
    unsafe { core::ptr::copy_nonoverlapping(b.as_ptr(), out, len) };
}

fn stat_out(r: Result<Stat, lowio::Failed>, out: *mut u8, wide: bool) -> i32 {
    if out.is_null() {
        global::invalid(EINVAL);
        return -1;
    }
    match r {
        // SAFETY: the caller's structure.
        Ok(st) => unsafe {
            store(&st, out, wide);
            0
        },
        Err(lowio::Failed) => -1,
    }
}

/// # Safety
/// `name` is a valid C string; `out` a `struct _stat64`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _stat64(name: *const u8, out: *mut u8) -> i32 {
    // SAFETY: the caller's string.
    stat_out(stat_wide(&unsafe { text::wide_path(name) }), out, true)
}

/// # Safety
/// As `_stat64`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _wstat64(name: *const u16, out: *mut u8) -> i32 {
    // SAFETY: the caller's string.
    stat_out(stat_wide(&unsafe { text::wide_path_w(name) }), out, true)
}

/// # Safety
/// `out` is a `struct _stat64`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _fstat64(fd: i32, out: *mut u8) -> i32 {
    stat_out(stat_fd(fd), out, true)
}

/// # Safety
/// As `_stat64`, with a `struct _stat64i32`.
pub unsafe extern "C" fn _stat64i32(name: *const u8, out: *mut u8) -> i32 {
    // SAFETY: the caller's string.
    stat_out(stat_wide(&unsafe { text::wide_path(name) }), out, false)
}

/// # Safety
/// As `_stat64i32`.
pub unsafe extern "C" fn _wstat64i32(name: *const u16, out: *mut u8) -> i32 {
    // SAFETY: the caller's string.
    stat_out(stat_wide(&unsafe { text::wide_path_w(name) }), out, false)
}

/// # Safety
/// `out` is a `struct _stat64i32`.
pub unsafe extern "C" fn _fstat64i32(fd: i32, out: *mut u8) -> i32 {
    stat_out(stat_fd(fd), out, false)
}
