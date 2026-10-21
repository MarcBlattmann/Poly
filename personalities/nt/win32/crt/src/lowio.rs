//! File descriptors ("low-level I/O", Learn: `_open`, `_read`, `_write`,
//! `_close`, `_lseek`, `_setmode`, `_get_osfhandle`, `_open_osfhandle`,
//! `_isatty`): a table of handles with their translation mode. Descriptors
//! 0, 1 and 2 are the standard handles at start-up; a missing one is open
//! with the handle −2 (Learn, `_get_osfhandle`: "If stdin, stdout, or
//! stderr aren't associated with a stream ... returns -2"), on which I/O
//! fails with `EBADF`.
//!
//! Text mode (Learn: "Text and binary mode file I/O"): `\n` is written as
//! `\r\n`, `\r\n` is read as `\n`, and Ctrl+Z ends text input. The Unicode
//! modes (`_O_WTEXT`, `_O_U16TEXT`, `_O_U8TEXT`, Learn: `_setmode`) take
//! and give UTF-16 units: written to a console with `WriteConsoleW`, to
//! anything else as UTF-16LE or UTF-8 bytes.

use crate::global::{self, Global, Lock};
use crate::sys::{self, Handle};
use alloc::vec::Vec;
use poly_crt::errno::{EBADF, EINVAL, ESPIPE};
use poly_crt::fmode::{self, Translation};

/// A failed operation; `errno` (and `_doserrno`) say why.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Failed;

/// The handle of a standard descriptor that has none.
pub const NO_HANDLE: Handle = -2;

#[derive(Clone, Copy)]
pub struct Fd {
    pub handle: Handle,
    pub mode: Translation,
    pub append: bool,
    /// `GetFileType`.
    pub kind: u32,
    /// A console handle (its `GetConsoleMode` succeeds).
    pub console: bool,
    /// Ctrl+Z was read in text mode.
    pub eof: bool,
    /// A byte read ahead after a `\r` that was not `\n` (non-seekable
    /// input).
    pub pending: Option<u8>,
    /// A high surrogate waiting for its low half (UTF-8 output).
    pub surrogate: u16,
}

static FDS: Global<Vec<Option<Fd>>> = Global::new(Vec::new());
static LOCK: Lock = Lock::new();

/// The largest number of descriptors (Learn, `_setmaxstdio`: 8,192).
const MAX: usize = 8192;

fn table() -> &'static mut Vec<Option<Fd>> {
    // SAFETY: only used under LOCK.
    unsafe { &mut *FDS.get() }
}

fn describe(handle: Handle, mode: Translation, append: bool) -> Fd {
    let (kind, console) = if handle == NO_HANDLE || handle == 0 || handle == sys::INVALID_HANDLE_VALUE {
        (0, false)
    } else {
        let mut m = 0u32;
        // SAFETY: a handle the program or the system gave.
        let kind = unsafe { sys::GetFileType(handle) };
        // SAFETY: as above, `m` writable.
        let console = kind == sys::FILE_TYPE_CHAR && unsafe { sys::GetConsoleMode(handle, &mut m) } != 0;
        (kind, console)
    };
    Fd { handle, mode, append, kind, console, eof: false, pending: None, surrogate: 0 }
}

/// Make descriptors 0, 1, 2 from the standard handles (start-up).
pub fn init() {
    let _g = LOCK.lock();
    let t = table();
    t.clear();
    for which in sys::STD_HANDLES {
        // SAFETY: no preconditions.
        let mut h = unsafe { sys::GetStdHandle(which) };
        if h == 0 || h == sys::INVALID_HANDLE_VALUE {
            h = NO_HANDLE;
        }
        t.push(Some(describe(h, Translation::Text, false)));
    }
}

/// A copy of a descriptor's state.
pub fn get(fd: i32) -> Option<Fd> {
    let _g = LOCK.lock();
    usize::try_from(fd).ok().and_then(|i| table().get(i).copied().flatten())
}

fn update(fd: i32, f: impl FnOnce(&mut Fd)) {
    let _g = LOCK.lock();
    if let Some(Some(e)) = usize::try_from(fd).ok().and_then(|i| table().get_mut(i)) {
        f(e);
    }
}

/// A descriptor or `EBADF` (with the invalid parameter handler, as Learn
/// documents for every low-level function given a bad descriptor).
pub fn check(fd: i32) -> Result<Fd, Failed> {
    match get(fd) {
        Some(e) => Ok(e),
        None => {
            global::invalid(EBADF);
            Err(Failed)
        }
    }
}

/// The lowest free descriptor for `handle`; −1 and `EMFILE` when full.
pub fn allocate(handle: Handle, mode: Translation, append: bool) -> i32 {
    let fd = describe(handle, mode, append);
    let _g = LOCK.lock();
    let t = table();
    if let Some(i) = t.iter().position(Option::is_none) {
        t[i] = Some(fd);
        return i as i32;
    }
    if t.len() >= MAX {
        global::set_errno(poly_crt::errno::EMFILE);
        return -1;
    }
    t.push(Some(fd));
    t.len() as i32 - 1
}

/// Forget a descriptor (its handle is the caller's business).
pub fn release(fd: i32) -> Option<Fd> {
    let _g = LOCK.lock();
    usize::try_from(fd).ok().and_then(|i| table().get_mut(i)).and_then(Option::take)
}

fn write_all(h: Handle, mut data: &[u8]) -> Result<(), Failed> {
    while !data.is_empty() {
        let mut n = 0u32;
        let chunk = data.len().min(1 << 30) as u32;
        // SAFETY: `data` is readable for `chunk` bytes.
        if unsafe { sys::WriteFile(h, data.as_ptr(), chunk, &mut n, core::ptr::null_mut()) } == 0 {
            global::set_errno_from_win32();
            return Err(Failed);
        }
        if n == 0 {
            global::set_errno(poly_crt::errno::ENOSPC);
            return Err(Failed);
        }
        data = &data[n as usize..];
    }
    Ok(())
}

/// `_write`'s work for a valid descriptor: `data` (UTF-16 units in the
/// Unicode modes) translated as the mode says; the source bytes written.
pub fn write(fd: i32, data: &[u8]) -> Result<usize, Failed> {
    let Some(e) = get(fd) else {
        global::set_errno(EBADF);
        return Err(Failed);
    };
    if e.handle == NO_HANDLE {
        global::set_errno(EBADF);
        return Err(Failed);
    }
    if e.append {
        // SAFETY: a valid handle.
        unsafe { sys::SetFilePointerEx(e.handle, 0, core::ptr::null_mut(), sys::FILE_END) };
    }
    match e.mode {
        Translation::Binary => write_all(e.handle, data)?,
        Translation::Text => write_all(e.handle, &fmode::to_crlf(data))?,
        Translation::Utf16 | Translation::Utf8 => {
            if !data.len().is_multiple_of(2) {
                global::invalid(EINVAL);
                return Err(Failed);
            }
            let mut units: Vec<u16> = Vec::with_capacity(data.len() / 2 + 8);
            for pair in data.chunks_exact(2) {
                let u = u16::from_le_bytes([pair[0], pair[1]]);
                if u == u16::from(b'\n') {
                    units.push(u16::from(b'\r'));
                }
                units.push(u);
            }
            if e.console {
                let mut done = 0u32;
                // SAFETY: `units` is readable.
                if unsafe {
                    sys::WriteConsoleW(e.handle, units.as_ptr(), units.len() as u32, &mut done, core::ptr::null_mut())
                } == 0
                {
                    global::set_errno_from_win32();
                    return Err(Failed);
                }
            } else if e.mode == Translation::Utf16 {
                let bytes: Vec<u8> = units.iter().flat_map(|u| u.to_le_bytes()).collect();
                write_all(e.handle, &bytes)?;
            } else {
                let (bytes, held) = utf8_of(e.surrogate, &units);
                update(fd, |f| f.surrogate = held);
                write_all(e.handle, &bytes)?;
            }
        }
    }
    Ok(data.len())
}

/// UTF-16 → UTF-8, starting with a held high surrogate; the bytes and the
/// high surrogate left at the end, if any (an unpaired one is U+FFFD).
fn utf8_of(held: u16, units: &[u16]) -> (Vec<u8>, u16) {
    let mut out = Vec::with_capacity(units.len() * 3);
    let mut high = held;
    let mut buf = [0u8; 4];
    for &u in units {
        let ch = if (0xD800..0xDC00).contains(&u) {
            if high != 0 {
                out.extend_from_slice('\u{FFFD}'.encode_utf8(&mut buf).as_bytes());
            }
            high = u;
            continue;
        } else if (0xDC00..0xE000).contains(&u) && high != 0 {
            let c = 0x10000 + ((u32::from(high) - 0xD800) << 10) + (u32::from(u) - 0xDC00);
            high = 0;
            char::from_u32(c).unwrap_or('\u{FFFD}')
        } else {
            if high != 0 {
                out.extend_from_slice('\u{FFFD}'.encode_utf8(&mut buf).as_bytes());
                high = 0;
            }
            char::from_u32(u32::from(u)).unwrap_or('\u{FFFD}')
        };
        out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
    }
    (out, high)
}

fn read_raw(e: &Fd, buf: &mut [u8]) -> Result<usize, Failed> {
    let mut n = 0u32;
    // SAFETY: `buf` is writable.
    if unsafe {
        sys::ReadFile(e.handle, buf.as_mut_ptr(), buf.len().min(1 << 30) as u32, &mut n, core::ptr::null_mut())
    } == 0
    {
        // SAFETY: no preconditions.
        if unsafe { sys::GetLastError() } == sys::ERROR_BROKEN_PIPE {
            return Ok(0);
        }
        global::set_errno_from_win32();
        return Err(Failed);
    }
    Ok(n as usize)
}

/// Bytes as they are in the file (the streams translate text themselves).
pub fn read_bytes(fd: i32, buf: &mut [u8]) -> Result<usize, Failed> {
    match get(fd) {
        Some(e) if e.handle != NO_HANDLE => read_raw(&e, buf),
        _ => {
            global::set_errno(EBADF);
            Err(Failed)
        }
    }
}

/// `_read`'s work for a valid descriptor: at most `buf.len()` bytes (or
/// UTF-16 units' bytes in the Unicode modes); 0 at the end.
pub fn read(fd: i32, buf: &mut [u8]) -> Result<usize, Failed> {
    let Some(mut e) = get(fd) else {
        global::set_errno(EBADF);
        return Err(Failed);
    };
    if e.handle == NO_HANDLE {
        global::set_errno(EBADF);
        return Err(Failed);
    }
    if buf.is_empty() {
        return Ok(0);
    }
    match e.mode {
        Translation::Binary => read_raw(&e, buf),
        Translation::Text => {
            if e.eof {
                return Ok(0);
            }
            loop {
                let mut raw = alloc::vec![0u8; buf.len()];
                let mut start = 0;
                if let Some(b) = e.pending.take() {
                    raw[0] = b;
                    start = 1;
                }
                let got = start + read_raw(&e, &mut raw[start..])?;
                if got == 0 {
                    update(fd, |f| f.pending = None);
                    return Ok(0);
                }
                let mut t = fmode::from_crlf(&raw[..got]);
                if t.pending_cr {
                    // Look at the byte after the '\r'.
                    let mut next = [0u8; 1];
                    let n = read_raw(&e, &mut next)?;
                    if n == 1 && next[0] == b'\n' {
                        t.data.push(b'\n');
                    } else {
                        t.data.push(b'\r');
                        if n == 1 {
                            if e.kind == sys::FILE_TYPE_DISK {
                                // SAFETY: a valid handle.
                                unsafe { sys::SetFilePointerEx(e.handle, -1, core::ptr::null_mut(), 1) };
                            } else {
                                e.pending = Some(next[0]);
                            }
                        }
                    }
                }
                let pending = e.pending;
                update(fd, |f| {
                    f.eof = t.eof;
                    f.pending = pending;
                });
                if t.data.is_empty() && !t.eof {
                    continue;
                }
                let n = t.data.len().min(buf.len());
                buf[..n].copy_from_slice(&t.data[..n]);
                return Ok(n);
            }
        }
        Translation::Utf16 | Translation::Utf8 => {
            if buf.len() < 2 {
                global::invalid(EINVAL);
                return Err(Failed);
            }
            let want = buf.len() / 2;
            let mut units: Vec<u16> = alloc::vec![0; want];
            let got_units = if e.console {
                let mut n = 0u32;
                // SAFETY: `units` is writable.
                if unsafe {
                    sys::ReadConsoleW(e.handle, units.as_mut_ptr(), want as u32, &mut n, core::ptr::null_mut())
                } == 0
                {
                    global::set_errno_from_win32();
                    return Err(Failed);
                }
                n as usize
            } else if e.mode == Translation::Utf16 {
                let mut raw = alloc::vec![0u8; want * 2];
                let n = read_raw(&e, &mut raw)? / 2;
                for (k, u) in units.iter_mut().enumerate().take(n) {
                    *u = u16::from_le_bytes([raw[2 * k], raw[2 * k + 1]]);
                }
                n
            } else {
                // UTF-8: read at most `want` bytes, decode what is complete.
                let mut raw = alloc::vec![0u8; want];
                let n = read_raw(&e, &mut raw)?;
                let text = core::str::from_utf8(&raw[..n]).unwrap_or("\u{FFFD}");
                let mut k = 0;
                for u in text.encode_utf16() {
                    if k < want {
                        units[k] = u;
                        k += 1;
                    }
                }
                k
            };
            // \r\n -> \n.
            let mut out: Vec<u16> = Vec::with_capacity(got_units);
            let mut i = 0;
            while i < got_units {
                if units[i] == u16::from(b'\r') && i + 1 < got_units && units[i + 1] == u16::from(b'\n') {
                    i += 1;
                    continue;
                }
                out.push(units[i]);
                i += 1;
            }
            for (k, u) in out.iter().enumerate() {
                buf[2 * k..2 * k + 2].copy_from_slice(&u.to_le_bytes());
            }
            Ok(out.len() * 2)
        }
    }
}

/// `_lseeki64`'s work: the new position, or −1 (`ESPIPE` for a device or
/// pipe, `EINVAL` for a bad origin or a negative result).
pub fn seek(fd: i32, offset: i64, origin: i32) -> i64 {
    let Some(e) = get(fd) else {
        global::set_errno(EBADF);
        return -1;
    };
    if !(0..=2).contains(&origin) {
        global::invalid(EINVAL);
        return -1;
    }
    if e.kind != sys::FILE_TYPE_DISK {
        global::set_errno(ESPIPE);
        return -1;
    }
    let mut new = 0i64;
    // SAFETY: a valid handle; `new` writable.
    if unsafe { sys::SetFilePointerEx(e.handle, offset, &mut new, origin as u32) } == 0 {
        global::set_errno_from_win32();
        return -1;
    }
    update(fd, |f| {
        f.eof = false;
        f.pending = None;
    });
    new
}

/// `_setmode`'s work: the previous mode's flag, or −1.
pub fn set_mode(fd: i32, mode: i32) -> i32 {
    let new = match mode {
        fmode::O_TEXT => Translation::Text,
        fmode::O_BINARY => Translation::Binary,
        fmode::O_WTEXT | fmode::O_U16TEXT => Translation::Utf16,
        fmode::O_U8TEXT => Translation::Utf8,
        _ => {
            global::invalid(EINVAL);
            return -1;
        }
    };
    let Ok(e) = check(fd) else { return -1 };
    update(fd, |f| f.mode = new);
    e.mode.flag()
}

/// Close every descriptor's handle above the standard ones (exit).
pub fn close_all() {
    let _g = LOCK.lock();
    for (i, e) in table().iter_mut().enumerate() {
        if i >= 3
            && let Some(f) = e.take()
        {
            // SAFETY: the descriptor's own handle.
            unsafe { sys::CloseHandle(f.handle) };
        }
    }
}
