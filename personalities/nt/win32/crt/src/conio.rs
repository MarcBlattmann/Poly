//! Console I/O (Learn: "Console and port I/O", `_getch`, `_getche`,
//! `_kbhit`, `_ungetch`, `_putch`, `_cputs`, `_cgets`, `_cprintf` and their
//! wide forms; ADR 0093): straight to the console (`CONIN$` / `CONOUT$`,
//! the console host of ADR 0080), whatever the standard handles are.
//!
//! - `_getch` reads one key press from the input buffer without echo, in a
//!   mode with only `ENABLE_PROCESSED_INPUT` (so Ctrl+C stays a signal:
//!   Learn, "None of these functions can be used to read CTRL+C"),
//!   restoring the mode after; which presses give what is
//!   `poly_crt::conio::key`. A narrow character is the console input code
//!   page's bytes of the key's character, a function or cursor key two
//!   calls (prefix, code). `_getwch` gives UTF-16 units.
//! - `_getche` echoes a character it read with `_putch` (not a function
//!   key's prefix and code: Poly's choice).
//! - `_kbhit` is non-zero when a call of `_getch` would not wait; it drops
//!   the records at the front of the buffer that `_getch` would skip
//!   (releases, mouse and focus events: Poly's choice, so they cannot hide
//!   a later key press from the next call).
//! - `_ungetch` / `_ungetwch` keep one character for the next read of
//!   their width (a second one fails with `EOF`).
//! - `_putch`, `_cputs`, `_cprintf` write with `WriteConsoleA` (the output
//!   code page) and the wide forms with `WriteConsoleW`; no `\n` is
//!   translated (Learn).
//! - `_cgets` reads a cooked line (`ENABLE_LINE_INPUT | ENABLE_ECHO_INPUT |
//!   ENABLE_PROCESSED_INPUT` for the call): `buffer[0]` is the most
//!   characters, `buffer[1]` gets the count, the text from `buffer[2]`
//!   (Learn). A longer line leaves its rest in the console for the next
//!   read.
//!
//! The `_nolock` forms (`ucrtbase` only) are the same without taking the
//! console lock.

use crate::global::{self, Global, Lock};
use crate::sys::{self, Handle};
use alloc::collections::VecDeque;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicIsize, Ordering};
use poly_crt::conio::{self as c, Key, RECORD_SIZE};
use poly_crt::errno::EINVAL;
use poly_crt::printf::{self, Args, VaList};
use poly_crt::unit;

/// `EOF` / `WEOF`.
const EOF: i32 = -1;
const WEOF: u32 = 0xFFFF;

static CONIN: AtomicIsize = AtomicIsize::new(0);
static CONOUT: AtomicIsize = AtomicIsize::new(0);
static LOCK: Lock = Lock::new();

/// What a narrow and a wide read have waiting: a pushed-back character,
/// then the rest of a key (a code after a prefix, more bytes of a
/// character).
struct Pending {
    back: Option<u32>,
    /// Units, each with whether it is (part of) a character.
    queue: VecDeque<(u32, bool)>,
}

static NARROW: Global<Pending> = Global::new(Pending { back: None, queue: VecDeque::new() });
static WIDE: Global<Pending> = Global::new(Pending { back: None, queue: VecDeque::new() });

fn pending(wide: bool) -> &'static mut Pending {
    // SAFETY: used under LOCK (or by a `_nolock` caller that holds the
    // program's own exclusion, as Learn requires of those).
    unsafe { &mut *(if wide { WIDE.get() } else { NARROW.get() }) }
}

/// The console's input or output buffer, opened on first use.
fn console(output: bool) -> Option<Handle> {
    let cell = if output { &CONOUT } else { &CONIN };
    let h = cell.load(Ordering::Acquire);
    if h != 0 {
        return Some(h);
    }
    let name: &[u16] = if output {
        &[b'C' as u16, b'O' as u16, b'N' as u16, b'O' as u16, b'U' as u16, b'T' as u16, b'$' as u16, 0]
    } else {
        &[b'C' as u16, b'O' as u16, b'N' as u16, b'I' as u16, b'N' as u16, b'$' as u16, 0]
    };
    // SAFETY: a NUL-terminated name.
    let h = unsafe {
        sys::CreateFileW(
            name.as_ptr(),
            sys::GENERIC_READ | sys::GENERIC_WRITE,
            sys::FILE_SHARE_READ_WRITE,
            core::ptr::null(),
            sys::OPEN_EXISTING,
            0,
            0,
        )
    };
    if h == sys::INVALID_HANDLE_VALUE {
        return None;
    }
    match cell.compare_exchange(0, h, Ordering::AcqRel, Ordering::Acquire) {
        Ok(_) => Some(h),
        Err(other) => {
            // Another thread opened it first.
            // SAFETY: the handle just opened.
            unsafe { sys::CloseHandle(h) };
            Some(other)
        }
    }
}

/// Run `f` with the input buffer in `mode`, restoring its mode after.
fn in_mode<R>(h: Handle, mode: impl FnOnce(u32) -> u32, f: impl FnOnce() -> R) -> R {
    let mut old = 0u32;
    // SAFETY: a console handle; `old` writable.
    let had = unsafe { sys::GetConsoleMode(h, &mut old) } != 0;
    if had {
        // SAFETY: as above.
        unsafe { sys::SetConsoleMode(h, mode(old)) };
    }
    let r = f();
    if had {
        // SAFETY: as above.
        unsafe { sys::SetConsoleMode(h, old) };
    }
    r
}

/// The units a key's character is in the console input code page.
fn narrow_bytes(ch: u16) -> Vec<u32> {
    let mut buf = [0u8; 8];
    // SAFETY: one unit in, `buf` out.
    let n = unsafe {
        sys::WideCharToMultiByte(
            sys::GetConsoleCP(),
            0,
            &ch,
            1,
            buf.as_mut_ptr(),
            buf.len() as i32,
            core::ptr::null(),
            core::ptr::null_mut(),
        )
    };
    if n <= 0 {
        return alloc::vec![u32::from(b'?')];
    }
    buf[..n as usize].iter().map(|&b| u32::from(b)).collect()
}

/// Queue what a key gives a read of this width.
fn queue_key(p: &mut Pending, key: Key, repeat: u16, wide: bool) {
    for _ in 0..repeat.max(1) {
        match key {
            Key::Char(ch) if wide => p.queue.push_back((u32::from(ch), true)),
            Key::Char(ch) => p.queue.extend(narrow_bytes(ch).into_iter().map(|b| (b, true))),
            Key::Extended(prefix, code) => {
                p.queue.push_back((u32::from(prefix), false));
                p.queue.push_back((u32::from(code), false));
            }
        }
    }
}

/// One unit for `_getch` / `_getwch`, and whether it is (part of) a
/// character rather than a function key's prefix or code; `None` without
/// a console or when reading fails.
fn read_unit(wide: bool) -> Option<(u32, bool)> {
    let p = pending(wide);
    if let Some(b) = p.back.take() {
        return Some((b, true));
    }
    if let Some(u) = p.queue.pop_front() {
        return Some(u);
    }
    let h = console(false)?;
    let key = in_mode(
        h,
        |m| m & c::ENABLE_PROCESSED_INPUT,
        || loop {
            let mut rec = [0u8; RECORD_SIZE];
            let mut n = 0u32;
            // SAFETY: a console input handle; one record's room.
            if unsafe { sys::ReadConsoleInputW(h, rec.as_mut_ptr(), 1, &mut n) } == 0 || n == 0 {
                return None;
            }
            if let Some(k) = c::key_event(&rec)
                && let Some(key) = c::key(&k)
            {
                return Some((key, k.repeat));
            }
        },
    )?;
    let (key, repeat) = key;
    queue_key(p, key, repeat, wide);
    p.queue.pop_front()
}

fn getch(wide: bool, echo: bool) -> u32 {
    match read_unit(wide) {
        Some((u, is_char)) => {
            if echo && is_char {
                if wide {
                    putwch(u as u16);
                } else {
                    putch(u as i32);
                }
            }
            u
        }
        None => {
            if wide {
                WEOF
            } else {
                EOF as u32
            }
        }
    }
}

fn locked<R>(f: impl FnOnce() -> R) -> R {
    let _g = LOCK.lock();
    f()
}

#[unsafe(no_mangle)]
pub extern "C" fn _getch() -> i32 {
    locked(|| getch(false, false) as i32)
}

#[unsafe(no_mangle)]
pub extern "C" fn _getche() -> i32 {
    locked(|| getch(false, true) as i32)
}

#[unsafe(no_mangle)]
pub extern "C" fn _getwch() -> u16 {
    locked(|| getch(true, false) as u16)
}

#[unsafe(no_mangle)]
pub extern "C" fn _getwche() -> u16 {
    locked(|| getch(true, true) as u16)
}

/// `_getch_nolock` … (`ucrtbase` only).
pub fn getch_nolock(wide: bool, echo: bool) -> u32 {
    getch(wide, echo)
}

/// `_kbhit`'s work.
fn kbhit() -> i32 {
    let (n, w) = (pending(false), pending(true));
    if n.back.is_some() || !n.queue.is_empty() || w.back.is_some() || !w.queue.is_empty() {
        return 1;
    }
    let Some(h) = console(false) else { return 0 };
    loop {
        let mut count = 0u32;
        // SAFETY: a console input handle; `count` writable.
        if unsafe { sys::GetNumberOfConsoleInputEvents(h, &mut count) } == 0 || count == 0 {
            return 0;
        }
        let want = count.min(64) as usize;
        let mut recs = alloc::vec![0u8; want * RECORD_SIZE];
        let mut got = 0u32;
        // SAFETY: room for `want` records.
        if unsafe { sys::PeekConsoleInputW(h, recs.as_mut_ptr(), want as u32, &mut got) } == 0 || got == 0 {
            return 0;
        }
        let gives = |i: usize| {
            let rec: &[u8; RECORD_SIZE] = recs[i * RECORD_SIZE..(i + 1) * RECORD_SIZE].try_into().expect("a record");
            c::key_event(rec).is_some_and(|k| c::key(&k).is_some())
        };
        match (0..got as usize).position(gives) {
            Some(0) => return 1,
            skip => {
                // Drop what `_getch` would skip before the key (or all).
                let drop = skip.unwrap_or(got as usize);
                let mut done = 0u32;
                // SAFETY: room for `drop` records.
                unsafe { sys::ReadConsoleInputW(h, recs.as_mut_ptr(), drop as u32, &mut done) };
                if skip.is_some() {
                    return 1;
                }
            }
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn _kbhit() -> i32 {
    locked(kbhit)
}

fn ungetch(c: u32, wide: bool) -> u32 {
    let fail = if wide { WEOF } else { EOF as u32 };
    let p = pending(wide);
    if c == fail || p.back.is_some() {
        return fail;
    }
    p.back = Some(if wide { c & 0xFFFF } else { c & 0xFF });
    c
}

#[unsafe(no_mangle)]
pub extern "C" fn _ungetch(c: i32) -> i32 {
    locked(|| ungetch(c as u32, false) as i32)
}

#[unsafe(no_mangle)]
pub extern "C" fn _ungetwch(c: u16) -> u16 {
    locked(|| ungetch(u32::from(c), true) as u16)
}

/// `_ungetch_nolock` … (`ucrtbase` only).
pub fn ungetch_nolock(c: u32, wide: bool) -> u32 {
    ungetch(c, wide)
}

// ----- output ---------------------------------------------------------------

/// Write narrow bytes to the console: whether all were written.
fn write_narrow(s: &[u8]) -> bool {
    let Some(h) = console(true) else { return false };
    let mut rest = s;
    while !rest.is_empty() {
        let mut n = 0u32;
        let chunk = rest.len().min(1 << 16) as u32;
        // SAFETY: `rest` readable for `chunk` bytes.
        if unsafe { sys::WriteConsoleA(h, rest.as_ptr(), chunk, &mut n, core::ptr::null_mut()) } == 0 || n == 0 {
            return false;
        }
        rest = &rest[(n as usize).min(rest.len())..];
    }
    true
}

/// Write UTF-16 units to the console.
fn write_wide(s: &[u16]) -> bool {
    let Some(h) = console(true) else { return false };
    let mut rest = s;
    while !rest.is_empty() {
        let mut n = 0u32;
        let chunk = rest.len().min(1 << 15) as u32;
        // SAFETY: `rest` readable for `chunk` units.
        if unsafe { sys::WriteConsoleW(h, rest.as_ptr(), chunk, &mut n, core::ptr::null_mut()) } == 0 || n == 0 {
            return false;
        }
        rest = &rest[(n as usize).min(rest.len())..];
    }
    true
}

fn putch(c: i32) -> i32 {
    if write_narrow(&[c as u8]) { c & 0xFF } else { EOF }
}

fn putwch(c: u16) -> u16 {
    if write_wide(&[c]) { c } else { WEOF as u16 }
}

#[unsafe(no_mangle)]
pub extern "C" fn _putch(c: i32) -> i32 {
    locked(|| putch(c))
}

#[unsafe(no_mangle)]
pub extern "C" fn _putwch(c: u16) -> u16 {
    locked(|| putwch(c))
}

/// `_putch_nolock` (`ucrtbase` only).
pub fn putch_nolock(c: i32) -> i32 {
    putch(c)
}

/// `_putwch_nolock` (`ucrtbase` only).
pub fn putwch_nolock(c: u16) -> u16 {
    putwch(c)
}

/// `_cputs(s)`: 0, or −1 (Learn: non-zero on failure).
///
/// # Safety
/// `s` is NULL or a C string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _cputs(s: *const u8) -> i32 {
    if s.is_null() {
        global::invalid(EINVAL);
        return -1;
    }
    // SAFETY: the caller's string.
    let s = unsafe { unit::slice(s) };
    locked(|| if write_narrow(s) { 0 } else { -1 })
}

/// # Safety
/// As `_cputs`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _cputws(s: *const u16) -> i32 {
    if s.is_null() {
        global::invalid(EINVAL);
        return -1;
    }
    // SAFETY: the caller's string.
    let s = unsafe { unit::slice(s) };
    locked(|| if write_wide(s) { 0 } else { -1 })
}

/// `_cprintf` / `_cwprintf` and the `__conio_common_vc*printf` forms: the
/// units written, or −1.
///
/// # Safety
/// `fmt` is NULL or a C string; `args` its arguments.
pub unsafe fn vcprintf<C: unit::Unit>(fmt: *const C, args: &mut impl Args, options: u64) -> i32 {
    if fmt.is_null() {
        global::invalid(EINVAL);
        return -1;
    }
    // SAFETY: the caller's string.
    let fmt = unsafe { unit::slice(fmt) };
    let mut out: Vec<C> = Vec::new();
    // SAFETY: the caller's arguments.
    match unsafe { printf::format(fmt, args, &mut out, options, crate::misc::count_output()) } {
        Ok(n) => {
            let ok = locked(|| {
                if C::WIDE {
                    write_wide(&out.iter().map(|c| c.to_u32() as u16).collect::<Vec<u16>>())
                } else {
                    write_narrow(&out.iter().map(|c| c.to_u32() as u8).collect::<Vec<u8>>())
                }
            });
            if ok { n as i32 } else { -1 }
        }
        Err(printf::Error::Invalid) => {
            global::invalid(EINVAL);
            -1
        }
        Err(printf::Error::Encoding) => {
            global::set_errno(poly_crt::errno::EILSEQ);
            -1
        }
    }
}

/// [`vcprintf`] over a caller's `va_list`.
///
/// # Safety
/// As [`vcprintf`].
pub unsafe fn vcprintf_va<C: unit::Unit>(fmt: *const C, va: *const u64, options: u64) -> i32 {
    // SAFETY: as documented.
    unsafe { vcprintf(fmt, &mut VaList(va), options) }
}

// ----- input lines ------------------------------------------------------------

/// A cooked line from the console (at most `max` units, without its line
/// end); `None` without a console.
fn read_line(max: usize) -> Option<Vec<u16>> {
    let h = console(false)?;
    let mut buf = alloc::vec![0u16; max + 2];
    let mut n = 0u32;
    let ok = in_mode(
        h,
        |_| c::ENABLE_LINE_INPUT | c::ENABLE_ECHO_INPUT | c::ENABLE_PROCESSED_INPUT,
        // SAFETY: room for `buf.len()` units.
        || unsafe { sys::ReadConsoleW(h, buf.as_mut_ptr(), buf.len() as u32, &mut n, core::ptr::null_mut()) },
    );
    if ok == 0 {
        return None;
    }
    Some(c::line(&buf[..n as usize], max).to_vec())
}

/// The console input code page's bytes of a line, cut to `max` bytes at a
/// character's end.
fn line_bytes(line: &[u16], max: usize) -> Vec<u8> {
    let mut out = Vec::new();
    for &u in line {
        let b = narrow_bytes(u);
        if out.len() + b.len() > max {
            break;
        }
        out.extend(b.iter().map(|&x| x as u8));
    }
    out
}

/// `_cgets(buffer)`: `buffer + 2`, or NULL.
///
/// # Safety
/// `buffer` holds `buffer[0] + 3` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _cgets(buffer: *mut u8) -> *mut u8 {
    if buffer.is_null() {
        global::invalid(EINVAL);
        return core::ptr::null_mut();
    }
    // SAFETY: the caller's buffer.
    let max = usize::from(unsafe { *buffer });
    let Some(line) = locked(|| read_line(max)) else { return core::ptr::null_mut() };
    let bytes = line_bytes(&line, max);
    // SAFETY: `buffer` holds max + 3 bytes and `bytes.len()` <= max.
    unsafe {
        core::ptr::copy_nonoverlapping(bytes.as_ptr(), buffer.add(2), bytes.len());
        *buffer.add(2 + bytes.len()) = 0;
        *buffer.add(1) = bytes.len() as u8;
        buffer.add(2)
    }
}

/// `_cgetws(buffer)` (`ucrtbase` only): `buffer[0]` the most units,
/// `buffer[1]` the count, the text from `buffer + 2`.
///
/// # Safety
/// `buffer` holds `buffer[0] + 3` units.
pub unsafe fn cgetws(buffer: *mut u16) -> *mut u16 {
    if buffer.is_null() {
        global::invalid(EINVAL);
        return core::ptr::null_mut();
    }
    // SAFETY: the caller's buffer.
    let max = usize::from(unsafe { *buffer });
    let Some(line) = locked(|| read_line(max)) else { return core::ptr::null_mut() };
    // SAFETY: `buffer` holds max + 3 units and `line.len()` <= max.
    unsafe {
        core::ptr::copy_nonoverlapping(line.as_ptr(), buffer.add(2), line.len());
        *buffer.add(2 + line.len()) = 0;
        *buffer.add(1) = line.len() as u16;
        buffer.add(2)
    }
}

/// `_cgets_s(buffer, count, &read)` / `_cgetws_s` (`ucrtbase` only): at most
/// `count − 1` units and a NUL; 0 or the `errno`.
///
/// # Safety
/// `buffer` holds `count` units of its width; `read` is writable.
pub unsafe fn cgets_s<C: unit::Unit>(buffer: *mut C, count: usize, read: *mut usize) -> i32 {
    if buffer.is_null() || count == 0 || read.is_null() {
        global::invalid(EINVAL);
        return EINVAL;
    }
    let Some(line) = locked(|| read_line(count - 1)) else {
        // SAFETY: the caller's buffer and count.
        unsafe {
            *buffer = C::default();
            *read = 0;
        }
        global::set_errno_from_win32();
        return global::thread().errno;
    };
    let units: Vec<C> = if C::WIDE {
        line.iter().map(|&u| C::from_u32(u32::from(u))).collect()
    } else {
        line_bytes(&line, count - 1).into_iter().map(|b| C::from_u32(u32::from(b))).collect()
    };
    // SAFETY: `units.len()` <= count - 1.
    unsafe {
        core::ptr::copy_nonoverlapping(units.as_ptr(), buffer, units.len());
        *buffer.add(units.len()) = C::default();
        *read = units.len();
    }
    0
}
