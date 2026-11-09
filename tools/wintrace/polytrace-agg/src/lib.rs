//! `polytrace.dll` in its counting form (`polytrace-agg`): the logger of the
//! Windows API tracer (ADR 0090) for a program that makes millions of calls
//! and starts other processes (Chrome). A copy of `../polytrace` (the 7-Zip
//! kit's logger, whose bytes are in the committed reference and must not
//! change) with the additions of the [`agg`] module; the proxies and the
//! thunk are the same.
//!
//! The proxy DLLs `cargo xtask win trace-kit` generates (`pt_kernel32.dll`
//! and so on) export the functions of the DLL they stand for; each export
//! is an assembly thunk (`tools/wintrace/proxy/common.rs`) that calls
//! [`pt_enter`] before the real function and [`pt_leave`] after it. This
//! DLL keeps the one trace of the process: `polytrace.jsonl` in the
//! current directory at load (`POLYTRACE_OUT` names another file), one
//! JSON object per line:
//!
//! ```text
//! {"format":"polytrace/1"}
//! {"seq":7,"t":0,"dll":"kernel32.dll","fn":"CreateFileW","args":[{"w":"hello.txt"},2147483648,1,"0x0",3,128,"0x0"],"ret":"0x9c","err":0}
//! ```
//!
//! - `seq`: the order in which calls began, over the whole process;
//!   a record is written when its call returns, so nested calls (a
//!   callback that calls an import) come before the call around them.
//! - `t`: a small per-process thread number, in the order threads first
//!   called a traced function.
//! - `args`: as many as the function's signature has (from the
//!   generator's table, which follows Microsoft Learn's prototypes; four
//!   when it is not known): numbers for integer kinds, `"0x…"` strings for
//!   pointers and handles, `{"w":…}` / `{"a":…}` for the wide and ANSI
//!   strings of well-known functions (read when the call begins).
//! - `ret`: the same, or `null` for `void` and for a call that does not
//!   return (`ExitProcess`, `_CxxThrowException`: written when it begins).
//! - `err`: `GetLastError()` after the call, only when the call changed it.
//!
//! Records are buffered and written when the buffer is full, when a
//! function that ends the process begins, and at `DLL_PROCESS_DETACH`.
//! The logger calls only the real `kernel32` (its own imports), never a
//! proxy; a per-thread flag in a TLS slot still makes a thunk reached from
//! inside the logger call straight through. Everything it calls keeps the
//! caller's last error.
//!
//! Written for Poly from Microsoft Learn (the functions' reference pages,
//! "Dynamic-Link Library Entry-Point Function", "Thread Local Storage");
//! constants from the public SDK headers, cited where used.

#![no_std]
#![allow(clippy::missing_safety_doc)]

use core::cell::UnsafeCell;
use core::sync::atomic::AtomicUsize;
use core::sync::atomic::{AtomicIsize, AtomicU32, AtomicU64, Ordering};

type Handle = isize;

/// `RTL_CRITICAL_SECTION` (`winnt.h`): 40 bytes on x64, opaque here.
#[repr(C, align(8))]
struct CriticalSection([u64; 5]);

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn TlsAlloc() -> u32;
    fn TlsGetValue(index: u32) -> usize;
    fn TlsSetValue(index: u32, value: usize) -> i32;
    fn GetLastError() -> u32;
    fn SetLastError(code: u32);
    fn CreateFileW(
        name: *const u16,
        access: u32,
        share: u32,
        sa: *mut u8,
        disp: u32,
        flags: u32,
        tmpl: Handle,
    ) -> Handle;
    fn WriteFile(file: Handle, buf: *const u8, len: u32, written: *mut u32, overlapped: *mut u8) -> i32;
    fn CloseHandle(h: Handle) -> i32;
    fn ReadFile(file: Handle, buf: *mut u8, len: u32, read: *mut u32, overlapped: *mut u8) -> i32;
    fn LoadLibraryW(name: *const u16) -> Handle;
    fn GetModuleHandleW(name: *const u16) -> Handle;
    fn GetProcAddress(module: Handle, name: *const u8) -> *const u8;
    fn GetCurrentThreadId() -> u32;
    fn GetModuleFileNameW(module: Handle, buf: *mut u16, size: u32) -> u32;
    fn CreateFileMappingW(file: Handle, sa: *mut u8, protect: u32, high: u32, low: u32, name: *const u16) -> Handle;
    fn MapViewOfFile(map: Handle, access: u32, high: u32, low: u32, bytes: usize) -> *mut u8;
    fn GetEnvironmentVariableW(name: *const u16, buf: *mut u16, size: u32) -> u32;
    fn InitializeCriticalSection(cs: *mut CriticalSection);
    fn EnterCriticalSection(cs: *mut CriticalSection);
    fn LeaveCriticalSection(cs: *mut CriticalSection);
}

// winnt.h / fileapi.h
const GENERIC_WRITE: u32 = 0x4000_0000;
const FILE_SHARE_READ: u32 = 1;
const CREATE_ALWAYS: u32 = 2;
const FILE_ATTRIBUTE_NORMAL: u32 = 0x80;
const INVALID_HANDLE_VALUE: Handle = -1;
/// `TLS_OUT_OF_INDEXES` (`winbase.h`).
const TLS_OUT_OF_INDEXES: u32 = 0xFFFF_FFFF;
// DllMain's reasons (`winnt.h`).
const DLL_PROCESS_DETACH: u32 = 0;
const DLL_PROCESS_ATTACH: u32 = 1;

/// One traced function, as the proxy's table describes it (the layout the
/// generator writes in assembly: five pointers' worth).
#[repr(C)]
pub struct Func {
    /// The real function's import address table slot (the thunk's).
    slot: *const usize,
    /// NUL-terminated: the DLL the program imported, the function (or
    /// `#ordinal`), and its signature (`ret=args`, see [`kind`]).
    dll: *const u8,
    name: *const u8,
    sig: *const u8,
    flags: u32,
    /// Stack slots the thunk copies for the real function (arguments
    /// beyond the fourth; 16 when the signature is not known).
    stack: u32,
}

/// [`Func::flags`]: the call does not return (write the record when it
/// begins, then write the buffer out).
pub const NORETURN: u32 = 1;
/// The thunk jumps to the function instead of calling it (functions that
/// capture or unwind their caller's frame); implies a record at the start.
pub const TAIL: u32 = 2;
/// Write the buffer out before the call (it may end the process).
pub const FLUSH: u32 = 4;

/// The kinds of a signature string `ret=args`, one letter each:
/// `i` a 32-bit integer (printed unsigned), `q` a 64-bit integer, `s` a
/// 16-bit and `c` an 8-bit one (`WORD`, `BOOLEAN`), `p` a pointer, `h` a
/// handle, `w`/`a` a wide/ANSI string (a pointer that is
/// read; a value below 0x10000 is printed as a number: an ordinal, an
/// atom, a character), `v` nothing (`void`, results only), `?` unknown
/// (printed as a pointer).
mod kind {
    pub const INT: u8 = b'i';
    pub const QUAD: u8 = b'q';
    pub const SHORT: u8 = b's';
    pub const BYTE: u8 = b'c';
    pub const WIDE: u8 = b'w';
    pub const ANSI: u8 = b'a';
    pub const VOID: u8 = b'v';
}

/// Arguments the thunk passes: four registers and sixteen stack slots.
const MAX_ARGS: usize = 20;
/// The thunk's scratch area for one record (`common.rs`: 0x800 bytes).
const SCRATCH: usize = 0x800;
/// Room [`pt_leave`] needs after what [`pt_enter`] wrote.
const SUFFIX: usize = 64;
/// Longest string read from an argument, in characters.
const MAX_STRING: usize = 400;

/// A cell shared by threads under [`State::lock`] (or written once at
/// `DLL_PROCESS_ATTACH`, before any other thread can call in).
struct Shared<T>(UnsafeCell<T>);
// SAFETY: every access goes through the critical section or happens
// during DLL_PROCESS_ATTACH (see the uses).
unsafe impl<T> Sync for Shared<T> {}

const BUFFER: usize = 1 << 18;

/// `pt_enter`'s cookie for a call that is only counted (the counting logger).
const COUNT_ONLY: u64 = 0x8000_0000;

static TLS: AtomicU32 = AtomicU32::new(TLS_OUT_OF_INDEXES);
static FILE: AtomicIsize = AtomicIsize::new(INVALID_HANDLE_VALUE);
static SEQ: AtomicU64 = AtomicU64::new(0);
static THREADS: AtomicU32 = AtomicU32::new(0);
static LOCK: Shared<CriticalSection> = Shared(UnsafeCell::new(CriticalSection([0; 5])));
static BUF: Shared<[u8; BUFFER]> = Shared(UnsafeCell::new([0; BUFFER]));
static LEN: Shared<usize> = Shared(UnsafeCell::new(0));

/// A bounded writer into a byte buffer; what does not fit is dropped and
/// [`W::full`] says so.
struct W {
    p: *mut u8,
    len: usize,
    cap: usize,
    full: bool,
}

impl W {
    fn byte(&mut self, b: u8) {
        if self.len < self.cap {
            // SAFETY: `p` points to at least `cap` writable bytes.
            unsafe { *self.p.add(self.len) = b };
            self.len += 1;
        } else {
            self.full = true;
        }
    }
    fn bytes(&mut self, s: &[u8]) {
        for &b in s {
            self.byte(b);
        }
    }
    fn dec(&mut self, mut v: u64) {
        let mut d = [0u8; 20];
        let mut i = d.len();
        loop {
            i -= 1;
            d[i] = b'0' + (v % 10) as u8;
            v /= 10;
            if v == 0 {
                break;
            }
        }
        self.bytes(&d[i..]);
    }
    fn hex(&mut self, v: u64) {
        self.bytes(b"\"0x");
        let mut started = false;
        for shift in (0..16).rev() {
            let n = ((v >> (shift * 4)) & 0xF) as u8;
            if n != 0 || started || shift == 0 {
                started = true;
                self.byte(if n < 10 { b'0' + n } else { b'a' + n - 10 });
            }
        }
        self.byte(b'"');
    }
    /// A NUL-terminated string the generator wrote (plain ASCII).
    fn cstr(&mut self, mut s: *const u8) {
        // SAFETY: the generator's strings are NUL-terminated.
        unsafe {
            while *s != 0 {
                self.json_char(*s as u32);
                s = s.add(1);
            }
        }
    }
    /// One character inside a JSON string.
    fn json_char(&mut self, c: u32) {
        match c {
            0x22 => self.bytes(b"\\\""),
            0x5C => self.bytes(b"\\\\"),
            0x0A => self.bytes(b"\\n"),
            0x0D => self.bytes(b"\\r"),
            0x09 => self.bytes(b"\\t"),
            c if c < 0x20 => {
                self.bytes(b"\\u00");
                let hexd = |n: u32| if n < 10 { b'0' + n as u8 } else { b'a' + (n - 10) as u8 };
                self.byte(hexd(c >> 4));
                self.byte(hexd(c & 0xF));
            }
            c if c < 0x80 => self.byte(c as u8),
            c if c < 0x800 => {
                self.byte(0xC0 | (c >> 6) as u8);
                self.byte(0x80 | (c & 0x3F) as u8);
            }
            c if c < 0x10000 => {
                self.byte(0xE0 | (c >> 12) as u8);
                self.byte(0x80 | ((c >> 6) & 0x3F) as u8);
                self.byte(0x80 | (c & 0x3F) as u8);
            }
            c => {
                self.byte(0xF0 | (c >> 18) as u8);
                self.byte(0x80 | ((c >> 12) & 0x3F) as u8);
                self.byte(0x80 | ((c >> 6) & 0x3F) as u8);
                self.byte(0x80 | (c & 0x3F) as u8);
            }
        }
    }
    /// `{"w":"…"}`: a UTF-16 string, at most [`MAX_STRING`] units (an
    /// unpaired surrogate becomes U+FFFD; a longer string ends in `…`).
    unsafe fn wide(&mut self, mut s: *const u16) {
        self.bytes(b"{\"w\":\"");
        let mut n = 0;
        // SAFETY (whole loop): the caller passed a string pointer to a
        // function whose documented argument is a NUL-terminated string.
        unsafe {
            while *s != 0 {
                if n == MAX_STRING {
                    self.json_char(0x2026);
                    break;
                }
                let u = *s as u32;
                let c = if (0xD800..0xDC00).contains(&u) {
                    let lo = *s.add(1) as u32;
                    if (0xDC00..0xE000).contains(&lo) {
                        s = s.add(1);
                        n += 1;
                        0x10000 + ((u - 0xD800) << 10) + (lo - 0xDC00)
                    } else {
                        0xFFFD
                    }
                } else if (0xDC00..0xE000).contains(&u) {
                    0xFFFD
                } else {
                    u
                };
                self.json_char(c);
                s = s.add(1);
                n += 1;
            }
        }
        self.bytes(b"\"}");
    }
    /// `{"a":"…"}`: an ANSI string; bytes above 0x7F are shown as the
    /// code points U+0080..U+00FF (the code page is not applied).
    unsafe fn ansi(&mut self, mut s: *const u8) {
        self.bytes(b"{\"a\":\"");
        let mut n = 0;
        // SAFETY: as in `wide`.
        unsafe {
            while *s != 0 {
                if n == MAX_STRING {
                    self.json_char(0x2026);
                    break;
                }
                self.json_char(*s as u32);
                s = s.add(1);
                n += 1;
            }
        }
        self.bytes(b"\"}");
    }
    /// One value of kind `k`; `strings`: read string arguments.
    unsafe fn value(&mut self, k: u8, v: u64, strings: bool) {
        match k {
            kind::INT => self.dec(v as u32 as u64),
            kind::QUAD => self.dec(v),
            kind::SHORT => self.dec(v as u16 as u64),
            kind::BYTE => self.dec(v as u8 as u64),
            kind::VOID => self.bytes(b"null"),
            kind::WIDE | kind::ANSI if v != 0 && v < 0x10000 => self.dec(v),
            // SAFETY: the signature says this argument is a string.
            kind::WIDE if v != 0 && strings => unsafe { self.wide(v as *const u16) },
            // SAFETY: as above.
            kind::ANSI if v != 0 && strings => unsafe { self.ansi(v as *const u8) },
            _ => self.hex(v),
        }
    }
}

/// The signature's result kind and argument kinds (`ret=args`); an empty
/// or malformed one is "unknown, four arguments".
unsafe fn signature(sig: *const u8) -> (u8, [u8; MAX_ARGS], usize) {
    let mut args = [b'?'; MAX_ARGS];
    // SAFETY: the generator's strings are NUL-terminated.
    unsafe {
        if sig.is_null() || *sig == 0 || *sig.add(1) != b'=' {
            return (b'?', args, 4);
        }
        let ret = *sig;
        let mut n = 0;
        let mut p = sig.add(2);
        while *p != 0 && n < MAX_ARGS {
            args[n] = *p;
            n += 1;
            p = p.add(1);
        }
        (ret, args, n)
    }
}

/// This thread's number, and whether it is inside the logger; marks it
/// inside. `None` when tracing is off or the thread is already inside.
unsafe fn enter_logger() -> Option<(u32, u32)> {
    let tls = TLS.load(Ordering::Acquire);
    if tls == TLS_OUT_OF_INDEXES {
        return None;
    }
    // SAFETY: `tls` is our slot.
    let v = unsafe { TlsGetValue(tls) };
    if v & 1 != 0 {
        return None;
    }
    let t = if v == 0 { THREADS.fetch_add(1, Ordering::Relaxed) } else { ((v >> 1) - 1) as u32 };
    // SAFETY: as above.
    unsafe { TlsSetValue(tls, (((t as usize) + 1) << 1) | 1) };
    Some((tls, t))
}

unsafe fn leave_logger(tls: u32, t: u32) {
    // SAFETY: `tls` is our slot.
    unsafe { TlsSetValue(tls, ((t as usize) + 1) << 1) };
}

/// The trace lives in a view of its file (the counting logger holds no
/// file handle: Chrome's sandbox closes the handles of a process it
/// confines, and the pages of a mapped file reach the disk when the
/// process dies, however it dies). A NUL byte ends the text.
const VIEW_SIZE: usize = 16 << 20;
static VIEW: AtomicUsize = AtomicUsize::new(0);
static CURSOR: Shared<usize> = Shared(UnsafeCell::new(0));

/// Append one record to the view (under the lock); `flush` is not needed.
unsafe fn append(rec: &[u8], _flush: bool) {
    // SAFETY: the critical section serialises CURSOR; the view is
    // VIEW_SIZE bytes of writable memory.
    unsafe {
        EnterCriticalSection(LOCK.0.get());
        let view = VIEW.load(Ordering::Acquire) as *mut u8;
        let at = &mut *CURSOR.0.get();
        if !view.is_null() && *at + rec.len() < VIEW_SIZE {
            core::ptr::copy_nonoverlapping(rec.as_ptr(), view.add(*at), rec.len());
            *at += rec.len();
        }
        LeaveCriticalSection(LOCK.0.get());
    }
}

/// The record's beginning: `{"seq":…,"t":…,"dll":…,"fn":…,"args":[…]`.
unsafe fn begin(w: &mut W, f: &Func, t: u32, seq: u64, regs: *const u64, stack: *const u64, strings: bool) {
    // SAFETY: `f`'s strings are the generator's; `regs` has four values,
    // `stack` sixteen (the thunk's copy).
    unsafe {
        let (_, kinds, n) = signature(f.sig);
        w.bytes(b"{\"seq\":");
        w.dec(seq);
        w.bytes(b",\"t\":");
        w.dec(t as u64);
        w.bytes(b",\"dll\":\"");
        w.cstr(f.dll);
        w.bytes(b"\",\"fn\":\"");
        w.cstr(f.name);
        w.bytes(b"\",\"args\":[");
        for (i, &k) in kinds.iter().enumerate().take(n) {
            if i > 0 {
                w.byte(b',');
            }
            let v = if i < 4 { *regs.add(i) } else { *stack.add(i - 4) };
            w.value(k, v, strings);
        }
        w.byte(b']');
    }
}

/// Called by a thunk before the real function: `regs` are its four
/// register arguments, `stack` the copy of its stack arguments, `scratch`
/// [`SCRATCH`] bytes in the thunk's frame that stay until [`pt_leave`].
/// Returns the cookie `pt_leave` gets: 0 when this call is not recorded
/// at its end, else the length written to `scratch` and (high half) the
/// last error before the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pt_enter(f: &Func, regs: *const u64, stack: *const u64, scratch: *mut u8) -> u64 {
    // SAFETY: (whole function) the thunk passes what the doc says.
    unsafe {
        let err = GetLastError();
        let Some((tls, t)) = enter_logger() else {
            SetLastError(err);
            return 0;
        };
        let seq = SEQ.fetch_add(1, Ordering::Relaxed);
        {
            let full = agg::on_enter(f, regs, stack, seq);
            if agg::due(seq + 1) || f.flags & (NORETURN | FLUSH) != 0 {
                agg::snapshot();
            }
            if !full && f.flags & (NORETURN | TAIL) == 0 {
                // Counted only; `pt_leave` counts the result.
                leave_logger(tls, t);
                SetLastError(err);
                return COUNT_ONLY | ((err as u64) << 32);
            }
            if !full {
                leave_logger(tls, t);
                SetLastError(err);
                return 0;
            }
        }
        let mut w = W { p: scratch, len: 0, cap: SCRATCH - SUFFIX, full: false };
        begin(&mut w, f, t, seq, regs, stack, true);
        if w.full {
            // Strings too long for the record: pointers instead.
            w = W { p: scratch, len: 0, cap: SCRATCH - SUFFIX, full: false };
            begin(&mut w, f, t, seq, regs, stack, false);
        }
        let mut cookie = (w.len as u64) | ((err as u64) << 32);
        if f.flags & (NORETURN | TAIL) != 0 {
            w.cap = SCRATCH;
            w.bytes(b",\"ret\":null}\n");
            append(core::slice::from_raw_parts(scratch, w.len), f.flags & NORETURN != 0);
            cookie = 0;
        } else if f.flags & FLUSH != 0 {
            append(&[], true);
        }
        leave_logger(tls, t);
        SetLastError(err);
        cookie
    }
}

/// Called by a thunk after the real function returned `ret`, with
/// [`pt_enter`]'s cookie and the same scratch area.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pt_leave(f: &Func, cookie: u64, ret: u64, scratch: *mut u8) {
    // SAFETY: (whole function) the thunk passes what the doc says.
    unsafe {
        let err = GetLastError();
        if cookie as u32 == COUNT_ONLY as u32 {
            agg::on_leave(f, ret, (cookie >> 32) as u32, err);
            return;
        }
        let len = cookie as u32 as usize;
        if len == 0 {
            return;
        }
        agg::on_leave(f, ret, (cookie >> 32) as u32, err);
        let Some((tls, t)) = enter_logger() else {
            SetLastError(err);
            return;
        };
        let before = (cookie >> 32) as u32;
        let (rk, _, _) = signature(f.sig);
        let mut w = W { p: scratch, len, cap: SCRATCH, full: false };
        w.bytes(b",\"ret\":");
        w.value(rk, ret, false);
        if err != before {
            w.bytes(b",\"err\":");
            w.dec(err as u64);
        }
        w.bytes(b"}\n");
        append(core::slice::from_raw_parts(scratch, w.len), false);
        leave_logger(tls, t);
        SetLastError(err);
    }
}

/// The counting form (the Chrome kit, `cargo xtask win trace-kit --agg`):
/// a trace small enough for a program that makes millions of calls.
///
/// * every function has counters (calls, first `seq`, results zero / not
///   zero / all ones, the first distinct last errors and result values);
/// * a call is written in full only for the first [`agg::SAMPLE`] calls of
///   a function and, for a function with a string argument, whenever that
///   string was not seen before by that function (paths, key names,
///   DLL names, named objects), so a path is never lost and a hot
///   function costs a counter;
/// * the counters are written as `{"stat":1,...}` lines at call 4096, 8192,
///   16384 ... and every [`agg::SNAPSHOT_EVERY`] calls ([`agg::due`]), before a call that ends the process and
///   at `DLL_PROCESS_DETACH`; a reader keeps the last line per function;
/// * the file name is `POLYTRACE_OUT` with `%p` replaced by the process id
///   (default `polytrace-%p.jsonl`), so every process of a program that
///   starts others has its own file.
mod agg {
    use super::*;

    /// Calls of a function always written in full.
    pub const SAMPLE: u64 = 4;
    /// Calls between two snapshots of the counters.
    pub const SNAPSHOT_EVERY: u64 = 1 << 19;
    const TABLE: usize = 1 << 13;
    const DEDUP: usize = 1 << 16;

    /// Up to four distinct values with their counts; the rest is `other`.
    pub struct Hist4 {
        v: [AtomicU64; 4],
        c: [AtomicU64; 4],
        other: AtomicU64,
    }

    #[allow(clippy::declare_interior_mutable_const)]
    const Z: AtomicU64 = AtomicU64::new(0);

    impl Hist4 {
        const fn new() -> Hist4 {
            Hist4 { v: [Z; 4], c: [Z; 4], other: Z }
        }
        fn add(&self, value: u64) {
            for i in 0..4 {
                if self.c[i].load(Ordering::Acquire) != 0 && self.v[i].load(Ordering::Relaxed) == value {
                    self.c[i].fetch_add(1, Ordering::Relaxed);
                    return;
                }
            }
            // SAFETY: the critical section is initialised at attach.
            unsafe { EnterCriticalSection(LOCK.0.get()) };
            let mut done = false;
            for i in 0..4 {
                if self.c[i].load(Ordering::Acquire) != 0 {
                    if self.v[i].load(Ordering::Relaxed) == value {
                        self.c[i].fetch_add(1, Ordering::Relaxed);
                        done = true;
                        break;
                    }
                } else {
                    self.v[i].store(value, Ordering::Relaxed);
                    self.c[i].store(1, Ordering::Release);
                    done = true;
                    break;
                }
            }
            // SAFETY: as above.
            unsafe { LeaveCriticalSection(LOCK.0.get()) };
            if !done {
                self.other.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    pub struct Stat {
        f: AtomicUsize,
        n: AtomicU64,
        first: AtomicU64,
        zero: AtomicU64,
        nonzero: AtomicU64,
        m1: AtomicU64,
        errs: Hist4,
        rets: Hist4,
    }

    #[allow(clippy::declare_interior_mutable_const)]
    const STAT: Stat = Stat {
        f: AtomicUsize::new(0),
        n: Z,
        first: Z,
        zero: Z,
        nonzero: Z,
        m1: Z,
        errs: Hist4::new(),
        rets: Hist4::new(),
    };
    static STATS: [Stat; TABLE] = [STAT; TABLE];
    static SEEN: [AtomicU64; DEDUP] = [Z; DEDUP];

    /// The counters of `f` (claimed on first use).
    fn stat(f: &Func) -> Option<&'static Stat> {
        let key = f as *const Func as usize;
        let mut i = (key >> 3).wrapping_mul(0x9E37_79B9_7F4A_7C15) >> (usize::BITS as usize - 13);
        for _ in 0..TABLE {
            let s = &STATS[i % TABLE];
            let cur = s.f.load(Ordering::Acquire);
            if cur == key {
                return Some(s);
            }
            if cur == 0 {
                match s.f.compare_exchange(0, key, Ordering::AcqRel, Ordering::Acquire) {
                    Ok(_) => return Some(s),
                    Err(now) if now == key => return Some(s),
                    Err(_) => {}
                }
            }
            i += 1;
        }
        None
    }

    /// FNV-1a of the first string argument, mixed with the function;
    /// `Some(0)` when that argument is an atom or NULL.
    unsafe fn string_hash(f: &Func, regs: *const u64, stack: *const u64) -> Option<u64> {
        // SAFETY: as `begin`: the signature says which arguments are strings.
        unsafe {
            let (_, kinds, n) = signature(f.sig);
            for (i, &k) in kinds.iter().enumerate().take(n) {
                if k != kind::WIDE && k != kind::ANSI {
                    continue;
                }
                let v = if i < 4 { *regs.add(i) } else { *stack.add(i - 4) };
                if v < 0x10000 {
                    return Some(0);
                }
                let mut h: u64 = 0xcbf2_9ce4_8422_2325 ^ (f as *const Func as u64);
                let mut c = 0;
                if k == kind::WIDE {
                    let mut s = v as *const u16;
                    while *s != 0 && c < MAX_STRING {
                        h = (h ^ *s as u64).wrapping_mul(0x0000_0100_0000_01b3);
                        s = s.add(1);
                        c += 1;
                    }
                } else {
                    let mut s = v as *const u8;
                    while *s != 0 && c < MAX_STRING {
                        h = (h ^ *s as u64).wrapping_mul(0x0000_0100_0000_01b3);
                        s = s.add(1);
                        c += 1;
                    }
                }
                return Some(h | 2);
            }
            None
        }
    }

    /// Count the call; true when it is to be written in full.
    pub unsafe fn on_enter(f: &Func, regs: *const u64, stack: *const u64, seq: u64) -> bool {
        let Some(s) = stat(f) else { return true };
        let n = s.n.fetch_add(1, Ordering::Relaxed);
        if n == 0 {
            s.first.store(seq + 1, Ordering::Relaxed);
            // SAFETY: as `string_hash`.
            unsafe { note_function(f) };
        }
        let key = f as *const Func as usize;
        if key == CWE_W.load(Ordering::Relaxed) || key == CWE_A.load(Ordering::Relaxed) {
            // SAFETY: documented calls.
            unsafe { hook_thread() };
        }
        if n < SAMPLE {
            return true;
        }
        // SAFETY: as `string_hash`.
        match unsafe { string_hash(f, regs, stack) } {
            Some(h) if h > 1 => {
                let mut i = (h as usize) & (DEDUP - 1);
                for _ in 0..8 {
                    let cur = SEEN[i].load(Ordering::Relaxed);
                    if cur == h {
                        return false;
                    }
                    if cur == 0 {
                        match SEEN[i].compare_exchange(0, h, Ordering::Relaxed, Ordering::Relaxed) {
                            Ok(_) => return true,
                            Err(now) if now == h => return false,
                            Err(_) => {}
                        }
                    }
                    i = (i + 1) & (DEDUP - 1);
                }
                false
            }
            _ => false,
        }
    }

    /// Count the result of a call that returned (`before`: the last error
    /// before it, `err`: after).
    pub unsafe fn on_leave(f: &Func, ret: u64, before: u32, err: u32) {
        let Some(s) = stat(f) else { return };
        // SAFETY: the generator's signature string.
        let (rk, _, _) = unsafe { signature(f.sig) };
        let (zero, m1, value) = match rk {
            kind::VOID => return,
            kind::INT => (ret as u32 == 0, ret as u32 == u32::MAX, ret as u32 as u64),
            kind::SHORT => (ret as u16 == 0, ret as u16 == u16::MAX, ret as u16 as u64),
            kind::BYTE => (ret as u8 == 0, ret as u8 == u8::MAX, ret as u8 as u64),
            _ => (ret == 0, ret == u64::MAX, ret),
        };
        if zero {
            s.zero.fetch_add(1, Ordering::Relaxed);
        } else if m1 {
            s.m1.fetch_add(1, Ordering::Relaxed);
        } else {
            s.nonzero.fetch_add(1, Ordering::Relaxed);
        }
        // Handles and pointers are classes; numbers keep their values.
        if !matches!(rk, b'h' | b'p') {
            s.rets.add(value);
        }
        if err != before {
            s.errs.add(err as u64);
        }
    }

    fn hist(w: &mut W, h: &Hist4) {
        w.byte(b'[');
        let mut first = true;
        for i in 0..4 {
            let c = h.c[i].load(Ordering::Acquire);
            if c != 0 {
                if !first {
                    w.byte(b',');
                }
                first = false;
                w.byte(b'[');
                w.dec(h.v[i].load(Ordering::Relaxed));
                w.byte(b',');
                w.dec(c);
                w.byte(b']');
            }
        }
        w.byte(b']');
    }

    /// Write every function's counters (and the buffer out).
    pub unsafe fn snapshot() {
        let mut line = [0u8; 1024];
        for s in STATS.iter() {
            let key = s.f.load(Ordering::Acquire);
            if key == 0 {
                continue;
            }
            // SAFETY: `key` is the address of a `Func` record of a proxy
            // that is loaded (the table is only filled by `stat`).
            let f = unsafe { &*(key as *const Func) };
            let mut w = W { p: line.as_mut_ptr(), len: 0, cap: line.len() - 1, full: false };
            w.bytes(b"{\"stat\":1,\"dll\":\"");
            w.cstr(f.dll);
            w.bytes(b"\",\"fn\":\"");
            w.cstr(f.name);
            w.bytes(b"\",\"n\":");
            w.dec(s.n.load(Ordering::Relaxed));
            w.bytes(b",\"first\":");
            w.dec(s.first.load(Ordering::Relaxed).wrapping_sub(1));
            w.bytes(b",\"z\":");
            w.dec(s.zero.load(Ordering::Relaxed));
            w.bytes(b",\"nz\":");
            w.dec(s.nonzero.load(Ordering::Relaxed));
            w.bytes(b",\"m1\":");
            w.dec(s.m1.load(Ordering::Relaxed));
            w.bytes(b",\"errs\":");
            hist(&mut w, &s.errs);
            w.bytes(b",\"rets\":");
            hist(&mut w, &s.rets);
            w.bytes(b",\"other\":");
            w.dec(s.errs.other.load(Ordering::Relaxed) + s.rets.other.load(Ordering::Relaxed));
            w.bytes(b"}\n");
            // SAFETY: the critical section is initialised.
            unsafe { append(core::slice::from_raw_parts(line.as_ptr(), w.len), false) };
        }
        // SAFETY: as above.
        unsafe { snapshot_messages() };
        let mut w = W { p: line.as_mut_ptr(), len: 0, cap: line.len() - 1, full: false };
        w.bytes(b"{\"snapshot\":1,\"calls\":");
        w.dec(SEQ.load(Ordering::Relaxed));
        w.bytes(b",\"threads\":");
        w.dec(THREADS.load(Ordering::Relaxed) as u64);
        w.bytes(
            b"}
",
        );
        // SAFETY: as above.
        unsafe { append(core::slice::from_raw_parts(line.as_ptr(), w.len), true) };
    }

    // ---- window messages -------------------------------------------------------------
    //
    // A thread that creates a window gets two hooks of its own (`WH_CALLWNDPROC`: messages
    // sent to its windows; `WH_GETMESSAGE`: messages it retrieves from its queue), installed
    // when it calls `CreateWindowExW` (before the first message of that window, WM_GETMINMAXINFO,
    // WM_NCCREATE, WM_CREATE are sent). Both only count the message number, `sent` or `posted`,
    // and the order of first sight; they pass every message on. `SetWindowsHookExW` is the
    // documented per-thread hook of Learn's "Hooks Overview"; nothing of user32 is looked into.

    const USER32: [u16; 11] = [
        b'u' as u16,
        b's' as u16,
        b'e' as u16,
        b'r' as u16,
        b'3' as u16,
        b'2' as u16,
        b'.' as u16,
        b'd' as u16,
        b'l' as u16,
        b'l' as u16,
        0,
    ];
    const MSGS: usize = 1 << 12;
    struct Msg {
        id: AtomicU64,
        sent: AtomicU64,
        posted: AtomicU64,
        first: AtomicU64,
    }
    #[allow(clippy::declare_interior_mutable_const)]
    const MSG0: Msg = Msg { id: Z, sent: Z, posted: Z, first: Z };
    static MSG_TABLE: [Msg; MSGS] = [MSG0; MSGS];
    static MSG_SEQ: AtomicU64 = AtomicU64::new(0);
    pub static CWE_W: AtomicUsize = AtomicUsize::new(0);
    pub static CWE_A: AtomicUsize = AtomicUsize::new(0);
    static HOOKED: [AtomicU64; 64] = [Z; 64];
    static SET_HOOK: AtomicUsize = AtomicUsize::new(0);
    static NEXT_HOOK: AtomicUsize = AtomicUsize::new(0);

    fn count_msg(id: u32, posted: bool) {
        let key = id as u64 + 1;
        let mut i = (key as usize).wrapping_mul(0x9E37_79B9) >> 7;
        for _ in 0..MSGS {
            let m = &MSG_TABLE[i % MSGS];
            let cur = m.id.load(Ordering::Acquire);
            if cur == key {
                if posted {
                    m.posted.fetch_add(1, Ordering::Relaxed);
                } else {
                    m.sent.fetch_add(1, Ordering::Relaxed);
                }
                return;
            }
            if cur == 0 {
                match m.id.compare_exchange(0, key, Ordering::AcqRel, Ordering::Acquire) {
                    Ok(_) => {
                        m.first.store(MSG_SEQ.fetch_add(1, Ordering::Relaxed) + 1, Ordering::Relaxed);
                        continue;
                    }
                    Err(_) => continue,
                }
            }
            i += 1;
        }
    }

    /// `WH_CALLWNDPROC`'s procedure (`CWPSTRUCT`: lParam, wParam, message, hwnd; `winuser.h`).
    unsafe extern "system" fn call_wnd(code: i32, wparam: usize, lparam: isize) -> isize {
        // SAFETY: user32 passes a CWPSTRUCT for code >= 0.
        unsafe {
            if code >= 0 && lparam != 0 {
                count_msg(*((lparam as usize + 16) as *const u32), false);
            }
            let next = NEXT_HOOK.load(Ordering::Relaxed);
            if next != 0 {
                let f: unsafe extern "system" fn(isize, i32, usize, isize) -> isize = core::mem::transmute(next);
                return f(0, code, wparam, lparam);
            }
            0
        }
    }

    /// `WH_GETMESSAGE`'s procedure (lParam: the `MSG` retrieved).
    unsafe extern "system" fn get_msg(code: i32, wparam: usize, lparam: isize) -> isize {
        // SAFETY: user32 passes the MSG for code >= 0.
        unsafe {
            if code >= 0 && lparam != 0 {
                count_msg(*((lparam as usize + 8) as *const u32), true);
            }
            let next = NEXT_HOOK.load(Ordering::Relaxed);
            if next != 0 {
                let f: unsafe extern "system" fn(isize, i32, usize, isize) -> isize = core::mem::transmute(next);
                return f(0, code, wparam, lparam);
            }
            0
        }
    }

    /// Remember which `Func` records are `CreateWindowExW` / `CreateWindowExA`.
    pub unsafe fn note_function(f: &Func) {
        // SAFETY: the generator's NUL-terminated strings.
        unsafe {
            let name = core::slice::from_raw_parts(f.name, 15);
            if name == b"CreateWindowExW" && *f.name.add(15) == 0 {
                CWE_W.store(f as *const Func as usize, Ordering::Relaxed);
            } else if name == b"CreateWindowExA" && *f.name.add(15) == 0 {
                CWE_A.store(f as *const Func as usize, Ordering::Relaxed);
            }
        }
    }

    /// Install the two hooks on the calling thread (once per thread).
    pub unsafe fn hook_thread() {
        // SAFETY: documented calls; the procedures above match the hook prototypes.
        unsafe {
            let tid = GetCurrentThreadId() as u64 + 1;
            for h in HOOKED.iter() {
                let cur = h.load(Ordering::Relaxed);
                if cur == tid {
                    return;
                }
                if cur == 0 && h.compare_exchange(0, tid, Ordering::AcqRel, Ordering::Relaxed).is_ok() {
                    let mut set = SET_HOOK.load(Ordering::Relaxed);
                    if set == 0 {
                        let user32 = GetModuleHandleW(USER32.as_ptr());
                        if user32 == 0 {
                            return;
                        }
                        set = GetProcAddress(user32, b"SetWindowsHookExW\0".as_ptr()) as usize;
                        let next = GetProcAddress(user32, b"CallNextHookEx\0".as_ptr()) as usize;
                        SET_HOOK.store(set, Ordering::Relaxed);
                        NEXT_HOOK.store(next, Ordering::Relaxed);
                    }
                    if set != 0 {
                        let f: unsafe extern "system" fn(i32, usize, Handle, u32) -> Handle = core::mem::transmute(set);
                        f(4, call_wnd as usize, 0, tid as u32 - 1);
                        f(3, get_msg as usize, 0, tid as u32 - 1);
                    }
                    return;
                }
            }
        }
    }

    /// The message lines of a snapshot.
    pub unsafe fn snapshot_messages() {
        let mut line = [0u8; 160];
        for m in MSG_TABLE.iter() {
            let key = m.id.load(Ordering::Acquire);
            if key == 0 {
                continue;
            }
            let mut w = W { p: line.as_mut_ptr(), len: 0, cap: line.len() - 1, full: false };
            w.bytes(b"{\"msg\":1,\"id\":");
            w.dec(key - 1);
            w.bytes(b",\"sent\":");
            w.dec(m.sent.load(Ordering::Relaxed));
            w.bytes(b",\"posted\":");
            w.dec(m.posted.load(Ordering::Relaxed));
            w.bytes(b",\"first\":");
            w.dec(m.first.load(Ordering::Relaxed));
            w.bytes(b"}\n");
            // SAFETY: the critical section is initialised.
            unsafe { append(core::slice::from_raw_parts(line.as_ptr(), w.len), false) };
        }
    }

    // Buffers of `preload` (static: the DLL has no stack probe, so no large locals).
    static PL_DIR: Shared<[u16; 520]> = Shared(UnsafeCell::new([0; 520]));
    static PL_TMP: Shared<[u16; 520]> = Shared(UnsafeCell::new([0; 520]));
    static PL_TEXT: Shared<[u8; 4096]> = Shared(UnsafeCell::new([0; 4096]));

    /// A child process of the program (its command line has `--type=`) is
    /// confined by Chrome's sandbox, which narrows the DLL search path to
    /// System32 before the program's delay-loaded imports are first used:
    /// the proxies would then not be found. So the proxies listed in
    /// `polytrace-preload.txt` (next to this DLL) are loaded here, while the
    /// process still starts up; a later `LoadLibrary("pt_x.dll")` finds the
    /// loaded module by its name.
    #[inline(never)]
    pub unsafe fn preload(module: Handle) {
        // SAFETY: (whole function) buffers are local and bounded; the
        // command line and the file list are NUL-terminated / sized.
        unsafe {
            let cmd = command_line();
            let needle: [u16; 7] =
                [b'-' as u16, b'-' as u16, b't' as u16, b'y' as u16, b'p' as u16, b'e' as u16, b'=' as u16];
            let mut found = false;
            let mut i = 0;
            while *cmd.add(i) != 0 {
                if (0..7).all(|k| *cmd.add(i + k) == needle[k]) {
                    found = true;
                    break;
                }
                i += 1;
            }
            if !found {
                return;
            }
            let dir = &mut *PL_DIR.0.get();
            let n = GetModuleFileNameW(module, dir.as_mut_ptr(), 400) as usize;
            let mut cut = n;
            while cut > 0 && dir[cut - 1] != b'\\' as u16 {
                cut -= 1;
            }
            if cut == 0 {
                return;
            }
            let list: [u16; 21] = {
                let mut a = [0u16; 21];
                for (k, c) in b"polytrace-preload.txt".iter().enumerate() {
                    a[k] = *c as u16;
                }
                a
            };
            let path = &mut *PL_TMP.0.get();
            *path = *dir;
            path[cut..cut + 21].copy_from_slice(&list);
            path[cut + 21] = 0;
            let f = CreateFileW(path.as_ptr(), 0x8000_0000, 1, core::ptr::null_mut(), 3, FILE_ATTRIBUTE_NORMAL, 0);
            if f == INVALID_HANDLE_VALUE {
                return;
            }
            let text = &mut *PL_TEXT.0.get();
            let mut got = 0u32;
            ReadFile(f, text.as_mut_ptr(), text.len() as u32, &mut got, core::ptr::null_mut());
            CloseHandle(f);
            let mut start = 0usize;
            let got = got as usize;
            for at in 0..=got {
                if at == got || text[at] == b'\n' || text[at] == b'\r' {
                    let len = at - start;
                    if len > 0 && cut + len < 510 {
                        let name = &mut *PL_TMP.0.get();
                        *name = *dir;
                        for k in 0..len {
                            name[cut + k] = text[start + k] as u16;
                        }
                        name[cut + len] = 0;
                        LoadLibraryW(name.as_ptr());
                    }
                    start = at + 1;
                }
            }
        }
    }

    /// Snapshot at call number `n` (1-based): every power of two from
    /// 4096 on and every [`SNAPSHOT_EVERY`] calls, so a process that is
    /// killed leaves counters at most twice too low.
    pub fn due(n: u64) -> bool {
        (n >= 4096 && n.is_power_of_two()) || n % SNAPSHOT_EVERY == 0
    }

    #[link(name = "kernel32", kind = "raw-dylib")]
    unsafe extern "system" {
        fn GetCurrentProcessId() -> u32;
        fn GetCommandLineW() -> *const u16;
    }

    /// The process's command line (for the header).
    pub fn command_line() -> *const u16 {
        // SAFETY: no arguments.
        unsafe { GetCommandLineW() }
    }

    /// The process id, for the trace file name and header.
    pub fn pid() -> u32 {
        // SAFETY: no arguments.
        unsafe { GetCurrentProcessId() }
    }
}

/// UTF-16 of an ASCII string literal, NUL-terminated, at compile time.
macro_rules! wide {
    ($s:literal) => {{
        const S: &str = $s;
        const N: usize = S.len() + 1;
        const W: [u16; N] = {
            let b = S.as_bytes();
            let mut w = [0u16; N];
            let mut i = 0;
            while i < b.len() {
                w[i] = b[i] as u16;
                i += 1;
            }
            w
        };
        &W
    }};
}

/// Open the trace file (`POLYTRACE_OUT` or `polytrace.jsonl`).
#[inline(never)]
unsafe fn open() {
    let mut path = [0u16; 520];
    // SAFETY: the name is NUL-terminated; `path` holds 520 units.
    let n = unsafe { GetEnvironmentVariableW(wide!("POLYTRACE_OUT").as_ptr(), path.as_mut_ptr(), path.len() as u32) };
    let name = if n > 0 && (n as usize) < path.len() { path.as_ptr() } else { wide!("polytrace.jsonl").as_ptr() };
    // `agg`: the process id for `%p` (default name `polytrace-%p.jsonl`).
    let mut expanded = [0u16; 560];
    let name = {
        let src: &[u16] =
            if n > 0 && (n as usize) < path.len() { &path[..n as usize] } else { &wide!("polytrace-%p.jsonl")[..18] };
        let pid = agg::pid();
        let mut o = 0;
        let mut i = 0;
        while i < src.len() && o < 540 {
            if src[i] == b'%' as u16 && i + 1 < src.len() && src[i + 1] == b'p' as u16 {
                let mut d = [0u16; 10];
                let mut k = d.len();
                let mut v = pid;
                loop {
                    k -= 1;
                    d[k] = b'0' as u16 + (v % 10) as u16;
                    v /= 10;
                    if v == 0 {
                        break;
                    }
                }
                for &c in &d[k..] {
                    expanded[o] = c;
                    o += 1;
                }
                i += 2;
            } else {
                expanded[o] = src[i];
                o += 1;
                i += 1;
            }
        }
        expanded.as_ptr()
    };
    // SAFETY: `name` is NUL-terminated.
    let file = unsafe {
        CreateFileW(
            name,
            GENERIC_WRITE | 0x8000_0000,
            FILE_SHARE_READ,
            core::ptr::null_mut(),
            CREATE_ALWAYS,
            FILE_ATTRIBUTE_NORMAL,
            0,
        )
    };
    if file != INVALID_HANDLE_VALUE {
        // PAGE_READWRITE = 4, FILE_MAP_WRITE = 2 (`memoryapi.h`); the file
        // takes the size of the view.
        let map = unsafe { CreateFileMappingW(file, core::ptr::null_mut(), 4, 0, VIEW_SIZE as u32, core::ptr::null()) };
        if map != 0 {
            let view = unsafe { MapViewOfFile(map, 2, 0, 0, VIEW_SIZE) };
            VIEW.store(view as usize, Ordering::Release);
            // SAFETY: the view keeps the file's section alive.
            unsafe { CloseHandle(map) };
        }
        // SAFETY: a handle this function opened.
        unsafe { CloseHandle(file) };
        if VIEW.load(Ordering::Relaxed) != 0 {
            FILE.store(1, Ordering::Relaxed);
        }
    }
}

/// The entry point (`DllMain`).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn polytrace_main(module: usize, reason: u32, _reserved: usize) -> i32 {
    // SAFETY: (whole function) the loader calls this once per reason,
    // under its lock; at attach no other thread runs our code yet.
    unsafe {
        match reason {
            DLL_PROCESS_ATTACH => {
                let err = GetLastError();
                InitializeCriticalSection(LOCK.0.get());
                open();
                if FILE.load(Ordering::Relaxed) != INVALID_HANDLE_VALUE {
                    {
                        let mut line = [0u8; 1700];
                        let mut w = W { p: line.as_mut_ptr(), len: 0, cap: line.len(), full: false };
                        w.bytes(b"{\"format\":\"polytrace/1\",\"mode\":\"agg\",\"pid\":");
                        w.dec(agg::pid() as u64);
                        w.bytes(b",\"cmd\":");
                        w.wide(agg::command_line());
                        w.bytes(b"}\n");
                        append(core::slice::from_raw_parts(line.as_ptr(), w.len), true);
                    }
                    TLS.store(TlsAlloc(), Ordering::Release);
                    agg::preload(module as Handle);
                }
                SetLastError(err);
            }
            DLL_PROCESS_DETACH => {
                let err = GetLastError();
                TLS.store(TLS_OUT_OF_INDEXES, Ordering::Release);
                agg::snapshot();
                FILE.store(INVALID_HANDLE_VALUE, Ordering::Relaxed);
                SetLastError(err);
            }
            _ => {}
        }
        1
    }
}

/// `_fltused`: the MSVC toolchain's marker that floating point is used,
/// which `core` references; no C runtime defines it here.
#[unsafe(no_mangle)]
#[used]
pub static _fltused: i32 = 0;

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    // SAFETY: `ud2` raises an illegal-instruction exception; nothing here
    // panics on purpose.
    unsafe { core::arch::asm!("ud2", options(noreturn)) }
}
