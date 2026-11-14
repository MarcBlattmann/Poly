//! (The variant of `polytrace` the Chrome bring-up kit uses, ADR 0155: it also writes the caller's
//! return address and every record when its call returns. `polytrace` itself stays as the 7-Zip kit's
//! committed reference needs it.)
//! `polytrace.dll`: the logger of the Windows API tracer (ADR 0090).
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

mod dump;

use core::cell::UnsafeCell;
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
    fn GetEnvironmentVariableW(name: *const u16, buf: *mut u16, size: u32) -> u32;
    fn InitializeCriticalSection(cs: *mut CriticalSection);
    fn EnterCriticalSection(cs: *mut CriticalSection);
    fn LeaveCriticalSection(cs: *mut CriticalSection);
    fn GetCommandLineW() -> *const u16;
    fn Sleep(ms: u32);
    fn CreateThread(
        attributes: *mut u8,
        stack: usize,
        start: unsafe extern "system" fn(usize) -> u32,
        parameter: usize,
        flags: u32,
        id: *mut u32,
    ) -> Handle;
    fn AddVectoredExceptionHandler(first: u32, handler: unsafe extern "system" fn(*mut *mut u32) -> i32) -> usize;
}

// winnt.h / fileapi.h
const GENERIC_WRITE: u32 = 0x4000_0000;
const FILE_SHARE_READ: u32 = 1;
const CREATE_NEW: u32 = 1;
const CREATE_ALWAYS: u32 = 2;
const FILE_ATTRIBUTE_NORMAL: u32 = 0x80;
/// `ERROR_SHARING_VIOLATION` (`winerror.h`).
const ERROR_SHARING_VIOLATION: u32 = 32;
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
const MAX_STRING: usize = 200;
/// Where [`pt_enter`] keeps the call's first [`NRAW`] argument values in the scratch area, for
/// [`pt_leave`]'s out-buffer dump (`dump.rs`): the end of the area.
const RAW_AT: usize = SCRATCH - 8 * NRAW - 32;
/// Argument values kept for the dump.
const NRAW: usize = 12;
/// The dump record's size (a call's whole record with its `"out"` entries).
const DUMPBUF: usize = 16 * 1024;

/// A cell shared by threads under [`State::lock`] (or written once at
/// `DLL_PROCESS_ATTACH`, before any other thread can call in).
struct Shared<T>(UnsafeCell<T>);
// SAFETY: every access goes through the critical section or happens
// during DLL_PROCESS_ATTACH (see the uses).
unsafe impl<T> Sync for Shared<T> {}

const BUFFER: usize = 1 << 18;

static TLS: AtomicU32 = AtomicU32::new(TLS_OUT_OF_INDEXES);
static FILE: AtomicIsize = AtomicIsize::new(INVALID_HANDLE_VALUE);
static SEQ: AtomicU64 = AtomicU64::new(0);
static THREADS: AtomicU32 = AtomicU32::new(0);
static LOCK: Shared<CriticalSection> = Shared(UnsafeCell::new(CriticalSection([0; 5])));
static BUF: Shared<[u8; BUFFER]> = Shared(UnsafeCell::new([0; BUFFER]));
static LEN: Shared<usize> = Shared(UnsafeCell::new(0));
static DUMP_BUF: Shared<[u8; DUMPBUF]> = Shared(UnsafeCell::new([0; DUMPBUF]));

/// Take the logger's lock (recursive).
pub(crate) fn lock() {
    // SAFETY: `LOCK` is initialised at DLL_PROCESS_ATTACH, before any call is traced.
    unsafe { EnterCriticalSection(LOCK.0.get()) }
}

/// Release the logger's lock.
pub(crate) fn unlock() {
    // SAFETY: as in [`lock`].
    unsafe { LeaveCriticalSection(LOCK.0.get()) }
}

/// A NUL-terminated string as a byte slice.
unsafe fn cname<'a>(p: *const u8) -> &'a [u8] {
    let mut n = 0;
    // SAFETY: the generator's strings are NUL-terminated.
    unsafe {
        while *p.add(n) != 0 {
            n += 1;
        }
        core::slice::from_raw_parts(p, n)
    }
}

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

/// Write the buffer to the file (under the lock).
unsafe fn write_out(buf: *mut u8, len: &mut usize) {
    let file = FILE.load(Ordering::Relaxed);
    let mut at = 0;
    while file != INVALID_HANDLE_VALUE && at < *len {
        let mut wrote = 0u32;
        // SAFETY: `buf[at..len]` is initialised; `wrote` is writable.
        let ok = unsafe { WriteFile(file, buf.add(at), (*len - at) as u32, &mut wrote, core::ptr::null_mut()) };
        if ok == 0 || wrote == 0 {
            break;
        }
        at += wrote as usize;
    }
    *len = 0;
}

/// Append one record (and write the buffer out if `flush`).
unsafe fn append(rec: &[u8], flush: bool) {
    // SAFETY: the critical section serialises BUF and LEN.
    unsafe {
        EnterCriticalSection(LOCK.0.get());
        let buf = BUF.0.get() as *mut u8;
        let len = &mut *LEN.0.get();
        if *len + rec.len() > BUFFER {
            write_out(buf, len);
        }
        if rec.len() <= BUFFER {
            core::ptr::copy_nonoverlapping(rec.as_ptr(), buf.add(*len), rec.len());
            *len += rec.len();
        }
        if flush {
            write_out(buf, len);
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
        let mut w = W { p: scratch, len: 0, cap: RAW_AT - SUFFIX, full: false };
        begin(&mut w, f, t, seq, regs, stack, true);
        if w.full {
            // Strings too long for the record: pointers instead.
            w = W { p: scratch, len: 0, cap: RAW_AT - SUFFIX, full: false };
            begin(&mut w, f, t, seq, regs, stack, false);
        }
        if SYNC.load(Ordering::Relaxed) || RING.load(Ordering::Relaxed) != 0 || FIRST.load(Ordering::Relaxed) {
            // The caller's return address (`pt_common`'s frame: the return address is 0x888
            // bytes above the saved argument registers `regs` points at): which code made the
            // call, for reading a program's behaviour on Poly (ADR 0155).
            let ra = *regs.cast::<u8>().add(0x888).cast::<u64>();
            w.bytes(b",\"ra\":");
            w.hex(ra);
            if DUMP.load(Ordering::Relaxed) {
                // ... and the module it lies in with the offset (`"rm"`, `"ro"`): the key a run
                // on Windows and a run on Poly are compared by; and the class of each pointer
                // and handle argument (`"ac"`).
                dump::put_caller(&mut w, ra);
                let (_, kinds, n) = signature(f.sig);
                dump::put_arg_classes(&mut w, &kinds[..n], regs, stack);
            }
        }
        // The first arguments, kept for `pt_leave`'s dump (arguments 1-4 are in `regs`, the
        // rest in the thunk's copy `stack`).
        let raw = scratch.add(RAW_AT).cast::<u64>();
        for i in 0..NRAW {
            let v = if i < 4 { *regs.add(i) } else { *stack.add(i - 4) };
            raw.add(i).write_unaligned(v);
        }
        let mut cookie = (w.len as u64) | ((err as u64) << 32);
        if f.flags & (NORETURN | TAIL) != 0 {
            w.cap = SCRATCH;
            w.bytes(b",\"ret\":null}\n");
            record(f, 0, core::slice::from_raw_parts(scratch, w.len), f.flags & NORETURN != 0);
            cookie = 0;
        } else if f.flags & FLUSH != 0 && RING.load(Ordering::Relaxed) == 0 && !FIRST.load(Ordering::Relaxed) {
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
        let len = cookie as u32 as usize;
        if len == 0 {
            return;
        }
        let Some((tls, t)) = enter_logger() else {
            SetLastError(err);
            return;
        };
        let before = (cookie >> 32) as u32;
        let (rk, _, _) = signature(f.sig);
        let mut w = W { p: scratch, len, cap: RAW_AT, full: false };
        w.bytes(b",\"ret\":");
        w.value(rk, ret, false);
        if DUMP.load(Ordering::Relaxed) && (rk == b'p' || rk == b'h') {
            // The class of a returned pointer or handle (`"rc"`), see `dump.rs`.
            w.bytes(b",\"rc\":");
            if rk == b'p' {
                dump::pclass(&mut w, ret);
            } else {
                dump::hclass(&mut w, ret);
            }
        }
        if err != before {
            w.bytes(b",\"err\":");
            w.dec(err as u64);
        }
        let sync = SYNC.load(Ordering::Relaxed);
        let name = cname(f.name);
        if sync && DUMP.load(Ordering::Relaxed) && dump::has_rule(name) {
            // What the call left in memory (`dump.rs`), appended as `"out":[...]` to the record.
            let mut raw = [0u64; NRAW];
            let rawp = scratch.add(RAW_AT).cast::<u64>();
            for (i, r) in raw.iter_mut().enumerate() {
                *r = rawp.add(i).read_unaligned();
            }
            lock();
            let mut d = W { p: DUMP_BUF.0.get().cast::<u8>(), len: 0, cap: DUMPBUF - 8, full: false };
            d.bytes(core::slice::from_raw_parts(scratch, w.len));
            d.bytes(b",\"out\":[");
            dump::dump(&mut dump::D::new(&mut d), name, &raw, ret);
            d.bytes(b"]}\n");
            let done = !d.full;
            if done {
                append(core::slice::from_raw_parts(d.p, d.len), true);
            }
            unlock();
            if done {
                leave_logger(tls, t);
                SetLastError(err);
                return;
            }
        }
        w.bytes(b"}\n");
        // The result's class, on the width the signature gives it (a `void` function has none).
        let value = match rk {
            kind::INT => ret & 0xFFFF_FFFF,
            kind::SHORT => ret & 0xFFFF,
            kind::BYTE => ret & 0xFF,
            _ => ret,
        };
        let class = if rk == kind::VOID {
            0
        } else if value == 0 {
            1
        } else {
            2
        };
        let outcome = if err != before { class | (u64::from(err & 0xFFFF) << 2) } else { class };
        record(f, outcome, core::slice::from_raw_parts(scratch, w.len), sync);
        leave_logger(tls, t);
        SetLastError(err);
    }
}

/// `POLYTRACE_FIRST=1`: write a record only the first time a function returns with a given
/// outcome (its result is zero or not, and the last error it set, if any): which functions a
/// process reaches and how they answer, in the order it first reached them, in a few hundred
/// records however long it runs. Compared with the Windows reference's per-function rows (ADR
/// 0155 step 11). Lock-free, like ring mode.
static FIRST: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);
const SEEN_SLOTS: usize = 1 << 14;
#[allow(clippy::declare_interior_mutable_const)]
const SEEN_FREE: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(0);
static SEEN: [core::sync::atomic::AtomicUsize; SEEN_SLOTS] = [SEEN_FREE; SEEN_SLOTS];
/// Set while a first-mode record is being written.
static FIRST_WRITING: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

/// Whether `key` (never 0) is new; it is remembered. A full table says no.
fn first_time(key: usize) -> bool {
    let mut i = (key.wrapping_mul(0x9E37_79B9_7F4A_7C15) >> 40) % SEEN_SLOTS;
    for _ in 0..SEEN_SLOTS {
        match SEEN[i].compare_exchange(0, key, Ordering::AcqRel, Ordering::Acquire) {
            Ok(_) => return true,
            Err(v) if v == key => return false,
            Err(_) => i = (i + 1) % SEEN_SLOTS,
        }
    }
    false
}

/// Keep or write one finished record of a call to `f` that ended with `outcome` (see
/// [`FIRST`]; 0 for a call that does not return): into the ring in ring mode, to the file the
/// first time in first mode, else into the buffer.
unsafe fn record(f: &Func, outcome: u64, rec: &[u8], flush: bool) {
    if FIRST.load(Ordering::Relaxed) {
        // A function's table entry is below 2^48; the outcome takes the bits above.
        let key = (f as *const Func as usize) ^ ((outcome as usize) << 48);
        if first_time(key) {
            // One writer at a time (two writes at the same file position overlap on Poly), but
            // never wait long: a thread suspended while it writes must not stop the others, so
            // after a while the record is written anyway.
            let mut spins = 0u32;
            while FIRST_WRITING.swap(true, Ordering::Acquire) && spins < 1 << 20 {
                core::hint::spin_loop();
                spins += 1;
            }
            let mut wrote = 0u32;
            // SAFETY: the record and the trace file.
            unsafe {
                WriteFile(
                    FILE.load(Ordering::Relaxed),
                    rec.as_ptr(),
                    rec.len() as u32,
                    &mut wrote,
                    core::ptr::null_mut(),
                )
            };
            FIRST_WRITING.store(false, Ordering::Release);
        }
        return;
    }
    let n = RING.load(Ordering::Relaxed);
    if n == 0 {
        // SAFETY: the caller's record.
        unsafe { append(rec, flush) };
        return;
    }
    let i = (RING_NEXT.fetch_add(1, Ordering::Relaxed) % u64::from(n)) as usize;
    let len = rec.len().min(RING_SLOT - 2);
    // SAFETY: slot `i` is inside RING_BUF (i < n <= RING_SLOTS). Two threads write the same slot
    // only when the ring wraps around within one record's copy: a torn record, which a debugging
    // trail can afford; no lock is taken, so a thread suspended here blocks nobody.
    unsafe {
        let slot = (RING_BUF.0.get() as *mut u8).add(i * RING_SLOT);
        core::ptr::copy_nonoverlapping(rec.as_ptr(), slot.add(2), len);
        if len < rec.len() {
            // Cut: the line still ends.
            *slot.add(2 + len - 1) = b'\n';
        }
        slot.cast::<u16>().write_unaligned(len as u16);
    }
}

/// `POLYTRACE_RING=N`: keep only the last N records (at most [`RING_SLOTS`]) and write them when the
/// process meets a breakpoint or a fatal exception ([`on_exception`]): the calls before a failed
/// check, without writing the millions of records that come before it (ADR 0155, step 11). No
/// lock is taken on this path (a stack sampler that suspends a thread inside the logger cannot
/// block the others), and the records carry `ra` but no buffer dumps.
static RING: AtomicU32 = AtomicU32::new(0);
static RING_NEXT: AtomicU64 = AtomicU64::new(0);
const RING_SLOTS: usize = 4096;
/// Bytes per kept record (a 2-byte length, then the record; longer records are cut).
const RING_SLOT: usize = 640;
static RING_BUF: Shared<[u8; RING_SLOTS * RING_SLOT]> = Shared(UnsafeCell::new([0; RING_SLOTS * RING_SLOT]));
/// Fatal exceptions written so far (breakpoints are written every time, other codes once).
static RING_FATAL: AtomicU32 = AtomicU32::new(0);

/// `EXCEPTION_BREAKPOINT` (`winnt.h`).
const EXCEPTION_BREAKPOINT: u32 = 0x8000_0003;
/// `EXCEPTION_CONTINUE_SEARCH` (`excpt.h`).
const EXCEPTION_CONTINUE_SEARCH: i32 = 0;

/// The vectored handler of ring mode: `pointers` is `EXCEPTION_POINTERS`, whose first field points
/// at the `EXCEPTION_RECORD` (whose first field is the code). Writes the ring and lets the
/// exception go on to the program's own handlers.
unsafe extern "system" fn on_exception(pointers: *mut *mut u32) -> i32 {
    // SAFETY: the dispatcher passes a valid EXCEPTION_POINTERS.
    let code = unsafe { **pointers };
    let fatal = code >= 0xC000_0000;
    if code != EXCEPTION_BREAKPOINT && !(fatal && RING_FATAL.fetch_add(1, Ordering::Relaxed) == 0) {
        return EXCEPTION_CONTINUE_SEARCH;
    }
    // SAFETY: no arguments.
    let err = unsafe { GetLastError() };
    write_ring(u64::from(code));
    // SAFETY: no arguments.
    unsafe { SetLastError(err) };
    EXCEPTION_CONTINUE_SEARCH
}

/// `POLYTRACE_RING_AT=<seconds>`: a thread writes the ring once, that many seconds after the
/// logger was loaded (header `"ring":"at"`): what a process that went quiet did last.
unsafe extern "system" fn ring_at(seconds: usize) -> u32 {
    // SAFETY: no arguments but the time.
    unsafe { Sleep((seconds as u32).saturating_mul(1000)) };
    write_ring(u64::MAX);
    0
}

/// Write the ring to the trace file, oldest record first, between a header naming `why` (an
/// exception code, or `u64::MAX` for [`ring_at`]) and `{"ring":"end"}`.
fn write_ring(why: u64) {
    let file = FILE.load(Ordering::Relaxed);
    let n = u64::from(RING.load(Ordering::Relaxed));
    let put = |bytes: &[u8]| {
        let mut wrote = 0u32;
        // SAFETY: a byte slice and the trace file.
        unsafe { WriteFile(file, bytes.as_ptr(), bytes.len() as u32, &mut wrote, core::ptr::null_mut()) };
    };
    let mut head = [0u8; 96];
    let mut w = W { p: head.as_mut_ptr(), len: 0, cap: head.len(), full: false };
    let next = RING_NEXT.load(Ordering::Relaxed);
    // `hex` writes a quoted string.
    w.bytes(b"{\"ring\":");
    if why == u64::MAX {
        w.bytes(b"\"at\"");
    } else {
        w.hex(why);
    }
    w.bytes(b",\"calls\":");
    w.dec(next);
    w.bytes(b"}\n");
    put(&head[..w.len]);
    for k in next.saturating_sub(n)..next {
        let i = (k % n) as usize;
        // SAFETY: slot `i` of RING_BUF; its length was written after its record.
        unsafe {
            let slot = (RING_BUF.0.get() as *const u8).add(i * RING_SLOT);
            let len = (slot.cast::<u16>().read_unaligned() as usize).min(RING_SLOT - 2);
            put(core::slice::from_raw_parts(slot.add(2), len));
        }
    }
    put(b"{\"ring\":\"end\"}\n");
}

/// `POLYTRACE_SYNC` is set: every record is written when its call returns, so a process that
/// dies on an unhandled exception leaves its trace up to the last call (bring-up of a program on
/// Poly, ADR 0155). Off by default: the buffering is what makes a trace cheap.
static SYNC: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

/// Out-buffer dumps and the caller's module and offset (`dump.rs`): on with `POLYTRACE_SYNC`,
/// unless `POLYTRACE_DUMP=0` (spec-chrome-win-3).
static DUMP: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

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

/// `POLYTRACE_ONLY=<text>`: trace only a process whose command line contains `text` (Chrome's
/// `--type=renderer`); in the others the logger opens no file and every thunk calls straight
/// through.
unsafe fn selected() -> bool {
    let mut want = [0u16; 128];
    // SAFETY: the name is NUL-terminated; `want` holds 128 units.
    let n = unsafe { GetEnvironmentVariableW(wide!("POLYTRACE_ONLY").as_ptr(), want.as_mut_ptr(), 128) } as usize;
    if n == 0 || n >= 128 {
        return true;
    }
    // SAFETY: the process's command line, NUL-terminated.
    let line = unsafe {
        let p = GetCommandLineW();
        if p.is_null() {
            return false;
        }
        let mut len = 0;
        while *p.add(len) != 0 {
            len += 1;
        }
        core::slice::from_raw_parts(p, len)
    };
    line.windows(n).any(|w| w == &want[..n])
}

/// Open the trace file (`POLYTRACE_OUT` or `polytrace.jsonl`).
///
/// A child process inherits the environment and the current directory, so it names the file its
/// parent still holds open; that open fails (the file is not shared for writing). Such a process
/// writes `<name>.1`, `<name>.2`, ... instead: the first of them that does not exist yet, so the
/// children of one run are numbered in the order they loaded the logger (spec-chrome-win-4).
unsafe fn open() {
    const TAIL: usize = 4; // ".99" and the NUL
    let mut path = [0u16; 520 + TAIL];
    // SAFETY: the name is NUL-terminated; `path` holds more than 520 units.
    let n = unsafe { GetEnvironmentVariableW(wide!("POLYTRACE_OUT").as_ptr(), path.as_mut_ptr(), 520) } as usize;
    let len = if n > 0 && n < 520 {
        n
    } else {
        let default = wide!("polytrace.jsonl");
        path[..default.len()].copy_from_slice(default);
        default.len() - 1
    };
    let create = |name: *const u16, disposition: u32| {
        // SAFETY: `name` is NUL-terminated.
        unsafe {
            CreateFileW(
                name,
                GENERIC_WRITE,
                FILE_SHARE_READ,
                core::ptr::null_mut(),
                disposition,
                FILE_ATTRIBUTE_NORMAL,
                0,
            )
        }
    };
    let mut file = create(path.as_ptr(), CREATE_ALWAYS);
    // SAFETY: no arguments.
    if file == INVALID_HANDLE_VALUE && unsafe { GetLastError() } == ERROR_SHARING_VIOLATION {
        for i in 1..100u16 {
            let mut at = len;
            path[at] = b'.' as u16;
            at += 1;
            if i >= 10 {
                path[at] = b'0' as u16 + i / 10;
                at += 1;
            }
            path[at] = b'0' as u16 + i % 10;
            path[at + 1] = 0;
            file = create(path.as_ptr(), CREATE_NEW);
            if file != INVALID_HANDLE_VALUE {
                break;
            }
        }
    }
    FILE.store(file, Ordering::Relaxed);
}

/// The entry point (`DllMain`).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn polytrace_main(_module: usize, reason: u32, _reserved: usize) -> i32 {
    // SAFETY: (whole function) the loader calls this once per reason,
    // under its lock; at attach no other thread runs our code yet.
    unsafe {
        match reason {
            DLL_PROCESS_ATTACH => {
                let err = GetLastError();
                InitializeCriticalSection(LOCK.0.get());
                if !selected() {
                    SetLastError(err);
                    return 1;
                }
                open();
                let mut probe = [0u16; 4];
                SYNC.store(
                    GetEnvironmentVariableW(wide!("POLYTRACE_SYNC").as_ptr(), probe.as_mut_ptr(), probe.len() as u32)
                        > 0,
                    Ordering::Relaxed,
                );
                let mut off = [0u16; 4];
                let n = GetEnvironmentVariableW(wide!("POLYTRACE_DUMP").as_ptr(), off.as_mut_ptr(), off.len() as u32);
                DUMP.store(!(n > 0 && off[0] == b'0' as u16) && SYNC.load(Ordering::Relaxed), Ordering::Relaxed);
                let mut first = [0u16; 4];
                if GetEnvironmentVariableW(wide!("POLYTRACE_FIRST").as_ptr(), first.as_mut_ptr(), 4) > 0 {
                    FIRST.store(true, Ordering::Relaxed);
                    DUMP.store(false, Ordering::Relaxed);
                }
                let mut ring = [0u16; 8];
                let n = GetEnvironmentVariableW(wide!("POLYTRACE_RING").as_ptr(), ring.as_mut_ptr(), ring.len() as u32);
                let slots = ring[..(n as usize).min(ring.len())]
                    .iter()
                    .take_while(|c| (b'0' as u16..=b'9' as u16).contains(c))
                    .fold(0u32, |v, &c| v.saturating_mul(10).saturating_add(u32::from(c - b'0' as u16)));
                if slots > 0 && FILE.load(Ordering::Relaxed) != INVALID_HANDLE_VALUE {
                    RING.store(slots.min(RING_SLOTS as u32), Ordering::Relaxed);
                    DUMP.store(false, Ordering::Relaxed);
                    AddVectoredExceptionHandler(1, on_exception);
                    let mut at = [0u16; 8];
                    let n = GetEnvironmentVariableW(wide!("POLYTRACE_RING_AT").as_ptr(), at.as_mut_ptr(), 8);
                    let seconds = at[..(n as usize).min(at.len())]
                        .iter()
                        .take_while(|c| (b'0' as u16..=b'9' as u16).contains(c))
                        .fold(0usize, |v, &c| v.saturating_mul(10).saturating_add(usize::from(c - b'0' as u16)));
                    if seconds > 0 {
                        let mut id = 0u32;
                        // The thread starts once the loader lock is released.
                        let h = CreateThread(core::ptr::null_mut(), 0, ring_at, seconds, 0, &mut id);
                        if h != 0 && h != INVALID_HANDLE_VALUE {
                            CloseHandle(h);
                        }
                    }
                }
                if FILE.load(Ordering::Relaxed) != INVALID_HANDLE_VALUE {
                    append(b"{\"format\":\"polytrace/1\"}\n", false);
                    TLS.store(TlsAlloc(), Ordering::Release);
                }
                SetLastError(err);
            }
            DLL_PROCESS_DETACH => {
                let err = GetLastError();
                TLS.store(TLS_OUT_OF_INDEXES, Ordering::Release);
                append(&[], true);
                let file = FILE.swap(INVALID_HANDLE_VALUE, Ordering::Relaxed);
                if file != INVALID_HANDLE_VALUE {
                    CloseHandle(file);
                }
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
