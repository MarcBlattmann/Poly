//! Streams (C11 7.21; Learn: `fopen`, `fread`, `fwrite`, `fflush`, `fseek`,
//! `ftell`, `setvbuf`, `_setmode`, "Stream I/O").
//!
//! A `FILE` is the classic 48-byte `_iobuf` of the public `stdio.h`
//! (`_ptr`, `_cnt`, `_base`, `_flag`, `_file`, `_charbuf`, `_bufsiz`,
//! `_tmpfname`), so `msvcrt.dll`'s `_iob` array has the stride old programs
//! compiled against it expect; the UCRT's `FILE` is opaque. Poly keeps only
//! `_file` (the descriptor) current; the stream's state is its own, beside
//! the `FILE`: for `stdin`, `stdout`, `stderr` in a table parallel to the
//! `_iob` array, for opened files in the same allocation after the `FILE`.
//!
//! Buffering (Poly's choices, to be confirmed by the spec agent): a stream
//! on a file or pipe is fully buffered (4,096 bytes); `stdout` on a console
//! is written at the end of each call, as is `stderr` always (unbuffered);
//! `_IOLBF` is `_IOFBF` (Learn, `setvbuf`: "_IOLBF ... the same as
//! _IOFBF"). Text-mode streams turn `\r\n` into `\n` and stop at Ctrl+Z when
//! reading; writing leaves the translation to the descriptor. The wide
//! functions write UTF-16 units in the Unicode and binary modes and the
//! "C" locale's single bytes in text mode (`EILSEQ` for other characters);
//! a narrow function on a Unicode-mode stream is an invalid parameter.

use crate::global::{self, CritSec, Global, Lock};
use crate::io;
use crate::lowio;
use crate::sys;
use alloc::boxed::Box;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicPtr, Ordering};
use poly_crt::errno::{EBADF, EILSEQ, EINVAL, ESPIPE};
use poly_crt::fmode::{self, Translation};
use poly_crt::printf::{self, Args, Sink};
use poly_crt::unit::{self, Unit};

pub const EOF: i32 = -1;
pub const WEOF: u16 = 0xFFFF;
/// `setvbuf` modes (`stdio.h`).
pub const IOFBF: i32 = 0x0000;
pub const IOLBF: i32 = 0x0040;
pub const IONBF: i32 = 0x0004;
/// `BUFSIZ` (`stdio.h`) and the buffer Poly gives files.
pub const BUFSIZ: usize = 512;
const FILE_BUFFER: usize = 4096;

/// The public `stdio.h`'s `struct _iobuf`.
#[repr(C)]
pub struct File {
    pub ptr: *mut u8,
    pub cnt: i32,
    pub base: *mut u8,
    pub flag: i32,
    pub file: i32,
    pub charbuf: i32,
    pub bufsiz: i32,
    pub tmpfname: *mut u8,
}

impl File {
    pub const fn new(fd: i32) -> Self {
        File {
            ptr: core::ptr::null_mut(),
            cnt: 0,
            base: core::ptr::null_mut(),
            flag: 0,
            file: fd,
            charbuf: 0,
            bufsiz: 0,
            tmpfname: core::ptr::null_mut(),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Last {
    None,
    Read,
    Write,
}

/// A stream's state.
pub struct Stream {
    cs: CritSec,
    open: bool,
    fd: i32,
    read: bool,
    write: bool,
    eof: bool,
    error: bool,
    /// Pending output, or input read ahead (raw bytes).
    buf: Vec<u8>,
    rpos: usize,
    last: Last,
    unget: Vec<u8>,
    wunget: Vec<u16>,
    buffered: bool,
    bufsize: usize,
    /// Write at the end of every call (a console `stdout`).
    per_call: bool,
    commit: bool,
}

impl Stream {
    const fn closed() -> Self {
        Stream {
            cs: CritSec::new(),
            open: false,
            fd: -1,
            read: false,
            write: false,
            eof: false,
            error: false,
            buf: Vec::new(),
            rpos: 0,
            last: Last::None,
            unget: Vec::new(),
            wunget: Vec::new(),
            buffered: true,
            bufsize: FILE_BUFFER,
            per_call: false,
            commit: false,
        }
    }

    fn reset(&mut self, fd: i32, read: bool, write: bool, commit: bool) {
        self.open = true;
        self.fd = fd;
        self.read = read;
        self.write = write;
        self.eof = false;
        self.error = false;
        self.buf.clear();
        self.rpos = 0;
        self.last = Last::None;
        self.unget.clear();
        self.wunget.clear();
        self.commit = commit;
        let kind = lowio::get(fd).map_or(0, |e| e.kind);
        self.buffered = true;
        self.bufsize = FILE_BUFFER;
        self.per_call = kind == sys::FILE_TYPE_CHAR;
    }

    fn mode(&self) -> Translation {
        lowio::get(self.fd).map_or(Translation::Binary, |e| e.mode)
    }
}

/// An opened file's `FILE` and state in one allocation.
#[repr(C)]
struct OpenFile {
    file: File,
    stream: Stream,
}

/// The standard streams' state, parallel to the `_iob` array.
static STD: Global<[Stream; 3]> = Global::new([Stream::closed(), Stream::closed(), Stream::closed()]);
/// The runtime's own `_iob` (the UCRT's `__acrt_iob_func`); `msvcrt.dll`
/// binds its exported array instead ([`bind_iob`]).
static IOB_OWN: Global<[File; 3]> = Global::new([File::new(0), File::new(1), File::new(2)]);
static IOB: AtomicPtr<File> = AtomicPtr::new(core::ptr::null_mut());
/// Opened streams, for `_flushall`, `_fcloseall` and exit.
static OPEN: Global<Vec<usize>> = Global::new(Vec::new());
static OPEN_LOCK: Lock = Lock::new();

/// Use `iob` (at least three `FILE`s) as the standard streams' `FILE`s
/// (before [`init`]).
pub fn bind_iob(iob: *mut File) {
    IOB.store(iob, Ordering::Release);
}

/// The `FILE` of `stdin` (0), `stdout` (1), `stderr` (2).
pub fn iob(i: usize) -> *mut File {
    let base = IOB.load(Ordering::Acquire);
    let base = if base.is_null() { IOB_OWN.get() as *mut File } else { base };
    // SAFETY: i < 3, inside the array.
    unsafe { base.add(i.min(2)) }
}

/// Set up the standard streams (start-up, after the descriptors).
pub fn init() {
    // SAFETY: start-up, one thread.
    let std = unsafe { &mut *STD.get() };
    for (i, s) in std.iter_mut().enumerate() {
        s.reset(i as i32, i == 0, i != 0, false);
        if i == 2 {
            s.buffered = false;
        }
        // SAFETY: the standard FILEs.
        unsafe { (*iob(i)).file = i as i32 };
    }
}

/// The state of a `FILE`.
fn state(f: *mut File) -> Option<&'static mut Stream> {
    if f.is_null() {
        return None;
    }
    let first = iob(0) as usize;
    let at = f as usize;
    if at >= first
        && at < first + 3 * core::mem::size_of::<File>()
        && (at - first).is_multiple_of(core::mem::size_of::<File>())
    {
        // SAFETY: one of the three standard streams' states.
        return Some(unsafe { &mut (*STD.get())[(at - first) / core::mem::size_of::<File>()] });
    }
    // SAFETY: any other FILE the runtime gave out is an OpenFile.
    Some(unsafe { &mut (*(f as *mut OpenFile)).stream })
}

/// Run `op` on an open stream under its lock, then write what a per-call
/// stream holds. An invalid stream is an invalid parameter (`EINVAL`).
fn with<R>(f: *mut File, fail: R, op: impl FnOnce(&mut Stream) -> R) -> R {
    let Some(s) = state(f) else {
        global::invalid(EINVAL);
        return fail;
    };
    s.cs.enter();
    let r = if s.open { op(s) } else { fail };
    if s.open && (!s.buffered || s.per_call) && s.last == Last::Write {
        let _ = flush(s);
    }
    s.cs.leave();
    r
}

/// Write the pending output; drop read-ahead input, moving the descriptor
/// back over what was not consumed.
fn flush(s: &mut Stream) -> Result<(), ()> {
    match s.last {
        Last::Write => {
            if !s.buf.is_empty() {
                let r = lowio::write(s.fd, &s.buf);
                s.buf.clear();
                if r.is_err() {
                    s.error = true;
                    return Err(());
                }
                if s.commit {
                    io::_commit(s.fd);
                }
            }
        }
        Last::Read => {
            let unread = (s.buf.len() - s.rpos + s.unget.len()) as i64;
            if unread > 0 && lowio::get(s.fd).is_some_and(|e| e.kind == sys::FILE_TYPE_DISK) {
                lowio::seek(s.fd, -unread, 1);
            }
            s.buf.clear();
            s.rpos = 0;
            s.unget.clear();
            s.wunget.clear();
        }
        Last::None => {}
    }
    s.last = Last::None;
    Ok(())
}

fn begin_write(s: &mut Stream) -> bool {
    if !s.write {
        s.error = true;
        global::set_errno(EBADF);
        return false;
    }
    if s.last == Last::Read {
        let _ = flush(s);
    }
    s.last = Last::Write;
    true
}

fn put_bytes(s: &mut Stream, data: &[u8]) -> bool {
    if !begin_write(s) {
        return false;
    }
    s.buf.extend_from_slice(data);
    if s.buf.len() >= s.bufsize { flush(s).is_ok() } else { true }
}

fn narrow_ok(s: &mut Stream) -> bool {
    if matches!(s.mode(), Translation::Utf16 | Translation::Utf8) {
        global::invalid(EINVAL);
        return false;
    }
    true
}

/// Write one wide character as the stream's mode says.
fn put_wide(s: &mut Stream, c: u16) -> bool {
    match s.mode() {
        Translation::Text => match unit::c_locale_to_narrow(c) {
            Some(b) => put_bytes(s, &[b]),
            None => {
                s.error = true;
                global::set_errno(EILSEQ);
                false
            }
        },
        _ => put_bytes(s, &c.to_le_bytes()),
    }
}

fn begin_read(s: &mut Stream) -> bool {
    if !s.read {
        s.error = true;
        global::set_errno(EBADF);
        return false;
    }
    if s.last == Last::Write && flush(s).is_err() {
        return false;
    }
    s.last = Last::Read;
    true
}

/// The next raw byte (`peek` leaves it).
fn raw(s: &mut Stream, peek: bool) -> Option<u8> {
    if let Some(&b) = s.unget.last() {
        if !peek {
            s.unget.pop();
        }
        return Some(b);
    }
    if s.rpos >= s.buf.len() {
        if s.eof {
            return None;
        }
        s.buf.resize(s.bufsize.max(1), 0);
        s.rpos = 0;
        let n = match lowio::read_bytes(s.fd, &mut s.buf) {
            Ok(n) => n,
            Err(_) => {
                s.buf.clear();
                s.error = true;
                return None;
            }
        };
        s.buf.truncate(n);
        if n == 0 {
            s.eof = true;
            return None;
        }
    }
    let b = s.buf[s.rpos];
    if !peek {
        s.rpos += 1;
    }
    Some(b)
}

/// `getc`'s work: the next character after text translation.
fn get_byte(s: &mut Stream) -> Option<u8> {
    let text = s.mode() == Translation::Text;
    let had_unget = !s.unget.is_empty();
    let b = raw(s, false)?;
    if text && !had_unget {
        if b == 0x1A {
            s.eof = true;
            s.unget.push(b);
            return None;
        }
        if b == b'\r' && raw(s, true) == Some(b'\n') {
            raw(s, false);
            return Some(b'\n');
        }
    }
    Some(b)
}

/// `fgetwc`'s work.
fn get_wide(s: &mut Stream) -> Option<u16> {
    if let Some(c) = s.wunget.pop() {
        return Some(c);
    }
    match s.mode() {
        Translation::Text => get_byte(s).map(unit::c_locale_to_wide),
        Translation::Binary => {
            let lo = raw(s, false)?;
            let hi = raw(s, false)?;
            Some(u16::from_le_bytes([lo, hi]))
        }
        Translation::Utf16 | Translation::Utf8 => {
            let mut two = [0u8; 2];
            match lowio::read(s.fd, &mut two) {
                Ok(2) => Some(u16::from_le_bytes(two)),
                Ok(_) => {
                    s.eof = true;
                    None
                }
                Err(_) => {
                    s.error = true;
                    None
                }
            }
        }
    }
}

// ----- opening and closing ----------------------------------------------------------

fn register(f: *mut OpenFile) {
    let _g = OPEN_LOCK.lock();
    // SAFETY: under OPEN_LOCK.
    unsafe { (*OPEN.get()).push(f as usize) };
}

fn unregister(f: *mut OpenFile) {
    let _g = OPEN_LOCK.lock();
    // SAFETY: under OPEN_LOCK.
    unsafe { (*OPEN.get()).retain(|&p| p != f as usize) };
}

/// A stream on descriptor `fd` (`_fdopen`'s work).
fn stream_on(fd: i32, mode: &poly_crt::fmode::Mode) -> *mut File {
    let mut s = Stream::closed();
    s.reset(fd, mode.read, mode.write, mode.commit);
    let f = Box::into_raw(Box::new(OpenFile { file: File::new(fd), stream: s }));
    register(f);
    f as *mut File
}

/// `fopen` / `_wfsopen`'s work with a wide name (and NUL).
pub fn open(name: &[u16], mode: &[u8], share: i32) -> *mut File {
    let Some(m) = fmode::parse_mode(mode) else {
        global::invalid(EINVAL);
        return core::ptr::null_mut();
    };
    let fd = open_fd(name, &m, share);
    if fd < 0 {
        return core::ptr::null_mut();
    }
    stream_on(fd, &m)
}

fn open_fd(name: &[u16], m: &poly_crt::fmode::Mode, share: i32) -> i32 {
    let pmode = fmode::S_IREAD | fmode::S_IWRITE;
    // SAFETY: a NUL-terminated name.
    unsafe { io::_wsopen(name.as_ptr(), m.flags, share, pmode) }
}

/// `freopen`'s work: close `f`'s descriptor and open `name` in its place.
///
/// # Safety
/// `f` is a stream of this runtime.
pub unsafe fn reopen(name: &[u16], mode: &[u8], f: *mut File) -> *mut File {
    let Some(m) = fmode::parse_mode(mode) else {
        global::invalid(EINVAL);
        return core::ptr::null_mut();
    };
    let ok = with(f, false, |s| {
        let _ = flush(s);
        io::_close(s.fd);
        s.open = false;
        let fd = open_fd(name, &m, fmode::SH_DENYNO);
        if fd < 0 {
            return false;
        }
        s.reset(fd, m.read, m.write, m.commit);
        true
    });
    if !ok {
        return core::ptr::null_mut();
    }
    // SAFETY: a valid FILE (checked by `with`).
    unsafe { (*f).file = state(f).map_or(-1, |s| s.fd) };
    f
}

/// `_fdopen`.
///
/// # Safety
/// `mode` is a valid C string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _fdopen(fd: i32, mode: *const u8) -> *mut File {
    if mode.is_null() || lowio::check(fd).is_err() {
        return core::ptr::null_mut();
    }
    // SAFETY: the caller's string.
    match fmode::parse_mode(unsafe { unit::slice(mode) }) {
        Some(m) => stream_on(fd, &m),
        None => {
            global::invalid(EINVAL);
            core::ptr::null_mut()
        }
    }
}

/// `fclose`: 0, or `EOF` if writing the pending output or closing failed.
#[unsafe(no_mangle)]
pub extern "C" fn fclose(f: *mut File) -> i32 {
    let r = with(f, EOF, |s| {
        let flushed = flush(s);
        let closed = io::_close(s.fd);
        s.open = false;
        s.buf = Vec::new();
        if flushed.is_err() || closed != 0 { EOF } else { 0 }
    });
    let first = iob(0) as usize;
    let at = f as usize;
    if !f.is_null() && !(at >= first && at < first + 3 * core::mem::size_of::<File>()) {
        unregister(f as *mut OpenFile);
        // SAFETY: an OpenFile from `stream_on`, now closed and unlisted.
        drop(unsafe { Box::from_raw(f as *mut OpenFile) });
    }
    r
}

/// `_fcloseall`: close every stream but the standard ones; how many.
#[unsafe(no_mangle)]
pub extern "C" fn _fcloseall() -> i32 {
    let all: Vec<usize> = {
        let _g = OPEN_LOCK.lock();
        // SAFETY: under OPEN_LOCK.
        unsafe { (*OPEN.get()).clone() }
    };
    for &f in &all {
        fclose(f as *mut File);
    }
    all.len() as i32
}

/// Write every stream's pending output; how many streams are open.
pub fn flush_all() -> i32 {
    let all: Vec<usize> = {
        let _g = OPEN_LOCK.lock();
        // SAFETY: under OPEN_LOCK.
        unsafe { (*OPEN.get()).clone() }
    };
    let mut n = 0;
    for i in 0..3 {
        with(iob(i), (), |s| {
            if s.last == Last::Write {
                let _ = flush(s);
            }
            n += 1;
        });
    }
    for &f in &all {
        with(f as *mut File, (), |s| {
            if s.last == Last::Write {
                let _ = flush(s);
            }
            n += 1;
        });
    }
    n
}

#[unsafe(no_mangle)]
pub extern "C" fn _flushall() -> i32 {
    flush_all()
}

/// `fflush`: NULL writes every stream.
#[unsafe(no_mangle)]
pub extern "C" fn fflush(f: *mut File) -> i32 {
    if f.is_null() {
        flush_all();
        return 0;
    }
    with(f, EOF, |s| if flush(s).is_ok() { 0 } else { EOF })
}

// ----- characters and strings ---------------------------------------------------------

#[unsafe(no_mangle)]
pub extern "C" fn fputc(c: i32, f: *mut File) -> i32 {
    with(f, EOF, |s| if narrow_ok(s) && put_bytes(s, &[c as u8]) { i32::from(c as u8) } else { EOF })
}

#[unsafe(no_mangle)]
pub extern "C" fn putc(c: i32, f: *mut File) -> i32 {
    fputc(c, f)
}

#[unsafe(no_mangle)]
pub extern "C" fn putchar(c: i32) -> i32 {
    fputc(c, iob(1))
}

#[unsafe(no_mangle)]
pub extern "C" fn _fputchar(c: i32) -> i32 {
    putchar(c)
}

/// # Safety
/// `s` is a valid C string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fputs(text: *const u8, f: *mut File) -> i32 {
    if text.is_null() {
        global::invalid(EINVAL);
        return EOF;
    }
    // SAFETY: the caller's string.
    let t = unsafe { unit::slice(text) };
    with(f, EOF, |s| if narrow_ok(s) && put_bytes(s, t) { 0 } else { EOF })
}

/// # Safety
/// As `fputs`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn puts(text: *const u8) -> i32 {
    if text.is_null() {
        global::invalid(EINVAL);
        return EOF;
    }
    // SAFETY: the caller's string.
    let t = unsafe { unit::slice(text) };
    with(iob(1), EOF, |s| if narrow_ok(s) && put_bytes(s, t) && put_bytes(s, b"\n") { 0 } else { EOF })
}

#[unsafe(no_mangle)]
pub extern "C" fn fgetc(f: *mut File) -> i32 {
    with(f, EOF, |s| if narrow_ok(s) && begin_read(s) { get_byte(s).map_or(EOF, i32::from) } else { EOF })
}

#[unsafe(no_mangle)]
pub extern "C" fn getc(f: *mut File) -> i32 {
    fgetc(f)
}

#[unsafe(no_mangle)]
pub extern "C" fn getchar() -> i32 {
    fgetc(iob(0))
}

#[unsafe(no_mangle)]
pub extern "C" fn _fgetchar() -> i32 {
    getchar()
}

/// `ungetc`: one or more characters pushed back; `EOF` is refused.
#[unsafe(no_mangle)]
pub extern "C" fn ungetc(c: i32, f: *mut File) -> i32 {
    if c == EOF {
        return EOF;
    }
    with(f, EOF, |s| {
        if !s.read {
            return EOF;
        }
        if s.last == Last::Write {
            let _ = flush(s);
        }
        s.last = Last::Read;
        s.unget.push(c as u8);
        s.eof = false;
        i32::from(c as u8)
    })
}

/// `fgets`: up to `n − 1` characters, through a `\n`; NULL at the end with
/// nothing read, or on an error.
///
/// # Safety
/// `buf` holds `n` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fgets(buf: *mut u8, n: i32, f: *mut File) -> *mut u8 {
    if buf.is_null() || n <= 0 {
        global::invalid(EINVAL);
        return core::ptr::null_mut();
    }
    with(f, core::ptr::null_mut(), |s| {
        if !narrow_ok(s) || !begin_read(s) {
            return core::ptr::null_mut();
        }
        let mut i = 0usize;
        while i + 1 < n as usize {
            let Some(b) = get_byte(s) else { break };
            // SAFETY: i < n - 1.
            unsafe { buf.add(i).write(b) };
            i += 1;
            if b == b'\n' {
                break;
            }
        }
        if i == 0 && n > 1 || s.error {
            return core::ptr::null_mut();
        }
        // SAFETY: i < n.
        unsafe { buf.add(i).write(0) };
        buf
    })
}

/// `fread`: whole items read.
///
/// # Safety
/// `buf` holds `size × count` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fread(buf: *mut u8, size: usize, count: usize, f: *mut File) -> usize {
    let Some(total) = size.checked_mul(count) else {
        global::invalid(EINVAL);
        return 0;
    };
    if total == 0 {
        return 0;
    }
    if buf.is_null() {
        global::invalid(EINVAL);
        return 0;
    }
    with(f, 0, |s| {
        if !begin_read(s) {
            return 0;
        }
        let text = s.mode() == Translation::Text;
        let mut got = 0usize;
        while got < total {
            if !text && s.unget.is_empty() && s.rpos < s.buf.len() {
                let n = (s.buf.len() - s.rpos).min(total - got);
                // SAFETY: n bytes of the buffer, n bytes left in `buf`.
                unsafe { core::ptr::copy_nonoverlapping(s.buf.as_ptr().add(s.rpos), buf.add(got), n) };
                s.rpos += n;
                got += n;
                continue;
            }
            let Some(b) = get_byte(s) else { break };
            // SAFETY: got < total.
            unsafe { buf.add(got).write(b) };
            got += 1;
        }
        got / size
    })
}

/// `fwrite`: whole items written.
///
/// # Safety
/// `buf` holds `size × count` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fwrite(buf: *const u8, size: usize, count: usize, f: *mut File) -> usize {
    let Some(total) = size.checked_mul(count) else {
        global::invalid(EINVAL);
        return 0;
    };
    if total == 0 {
        return 0;
    }
    if buf.is_null() {
        global::invalid(EINVAL);
        return 0;
    }
    // SAFETY: the caller's buffer.
    let data = unsafe { core::slice::from_raw_parts(buf, total) };
    with(f, 0, |s| if put_bytes(s, data) { count } else { 0 })
}

// ----- wide characters ------------------------------------------------------------------

#[unsafe(no_mangle)]
pub extern "C" fn fputwc(c: u16, f: *mut File) -> u16 {
    with(f, WEOF, |s| if put_wide(s, c) { c } else { WEOF })
}

#[unsafe(no_mangle)]
pub extern "C" fn putwc(c: u16, f: *mut File) -> u16 {
    fputwc(c, f)
}

#[unsafe(no_mangle)]
pub extern "C" fn putwchar(c: u16) -> u16 {
    fputwc(c, iob(1))
}

#[unsafe(no_mangle)]
pub extern "C" fn _fputwchar(c: u16) -> u16 {
    putwchar(c)
}

/// # Safety
/// `text` is a valid wide C string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fputws(text: *const u16, f: *mut File) -> i32 {
    if text.is_null() {
        global::invalid(EINVAL);
        return EOF;
    }
    // SAFETY: the caller's string.
    let t = unsafe { unit::slice(text) };
    with(f, EOF, |s| if t.iter().all(|&c| put_wide(s, c)) { 0 } else { EOF })
}

/// # Safety
/// As `fputws`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _putws(text: *const u16) -> i32 {
    if text.is_null() {
        global::invalid(EINVAL);
        return EOF;
    }
    // SAFETY: the caller's string.
    let t = unsafe { unit::slice(text) };
    with(iob(1), EOF, |s| if t.iter().all(|&c| put_wide(s, c)) && put_wide(s, u16::from(b'\n')) { 0 } else { EOF })
}

#[unsafe(no_mangle)]
pub extern "C" fn fgetwc(f: *mut File) -> u16 {
    with(f, WEOF, |s| if begin_read(s) { get_wide(s).unwrap_or(WEOF) } else { WEOF })
}

#[unsafe(no_mangle)]
pub extern "C" fn getwc(f: *mut File) -> u16 {
    fgetwc(f)
}

#[unsafe(no_mangle)]
pub extern "C" fn getwchar() -> u16 {
    fgetwc(iob(0))
}

#[unsafe(no_mangle)]
pub extern "C" fn _fgetwchar() -> u16 {
    getwchar()
}

#[unsafe(no_mangle)]
pub extern "C" fn ungetwc(c: u16, f: *mut File) -> u16 {
    if c == WEOF {
        return WEOF;
    }
    with(f, WEOF, |s| {
        s.last = Last::Read;
        s.wunget.push(c);
        s.eof = false;
        c
    })
}

/// # Safety
/// `buf` holds `n` units.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fgetws(buf: *mut u16, n: i32, f: *mut File) -> *mut u16 {
    if buf.is_null() || n <= 0 {
        global::invalid(EINVAL);
        return core::ptr::null_mut();
    }
    with(f, core::ptr::null_mut(), |s| {
        if !begin_read(s) {
            return core::ptr::null_mut();
        }
        let mut i = 0usize;
        while i + 1 < n as usize {
            let Some(c) = get_wide(s) else { break };
            // SAFETY: i < n - 1.
            unsafe { buf.add(i).write(c) };
            i += 1;
            if c == u16::from(b'\n') {
                break;
            }
        }
        if i == 0 && n > 1 || s.error {
            return core::ptr::null_mut();
        }
        // SAFETY: i < n.
        unsafe { buf.add(i).write(0) };
        buf
    })
}

// ----- position and state ------------------------------------------------------------------

/// `_ftelli64`'s work on a locked stream.
fn tell(s: &mut Stream) -> i64 {
    if !lowio::get(s.fd).is_some_and(|e| e.kind == sys::FILE_TYPE_DISK) {
        global::set_errno(ESPIPE);
        return -1;
    }
    let pos = lowio::seek(s.fd, 0, 1);
    if pos < 0 {
        return -1;
    }
    match s.last {
        Last::Read => pos - (s.buf.len() - s.rpos) as i64 - s.unget.len() as i64,
        Last::Write => {
            let extra = if s.mode() == Translation::Text { s.buf.iter().filter(|&&b| b == b'\n').count() } else { 0 };
            pos + (s.buf.len() + extra) as i64
        }
        Last::None => pos,
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn _ftelli64(f: *mut File) -> i64 {
    with(f, -1, tell)
}

#[unsafe(no_mangle)]
pub extern "C" fn ftell(f: *mut File) -> i32 {
    let p = _ftelli64(f);
    if p > i64::from(i32::MAX) {
        global::set_errno(EINVAL);
        return -1;
    }
    p as i32
}

#[unsafe(no_mangle)]
pub extern "C" fn _fseeki64(f: *mut File, offset: i64, origin: i32) -> i32 {
    if !(0..=2).contains(&origin) {
        global::invalid(EINVAL);
        return -1;
    }
    with(f, -1, |s| {
        let (offset, origin) = if origin == 1 {
            let here = tell(s);
            if here < 0 {
                return -1;
            }
            (here + offset, 0)
        } else {
            (offset, origin)
        };
        if flush(s).is_err() {
            return -1;
        }
        s.eof = false;
        if lowio::seek(s.fd, offset, origin) < 0 { -1 } else { 0 }
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn fseek(f: *mut File, offset: i32, origin: i32) -> i32 {
    _fseeki64(f, i64::from(offset), origin)
}

#[unsafe(no_mangle)]
pub extern "C" fn rewind(f: *mut File) {
    _fseeki64(f, 0, 0);
    clearerr(f);
}

/// # Safety
/// `pos` is writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fgetpos(f: *mut File, pos: *mut i64) -> i32 {
    if pos.is_null() {
        global::invalid(EINVAL);
        return -1;
    }
    let p = _ftelli64(f);
    if p < 0 {
        return -1;
    }
    // SAFETY: writable.
    unsafe { *pos = p };
    0
}

/// # Safety
/// `pos` is readable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fsetpos(f: *mut File, pos: *const i64) -> i32 {
    if pos.is_null() {
        global::invalid(EINVAL);
        return -1;
    }
    // SAFETY: readable.
    _fseeki64(f, unsafe { *pos }, 0)
}

#[unsafe(no_mangle)]
pub extern "C" fn feof(f: *mut File) -> i32 {
    with(f, 0, |s| i32::from(s.eof && s.unget.is_empty() && s.rpos >= s.buf.len()) * 0x10)
}

#[unsafe(no_mangle)]
pub extern "C" fn ferror(f: *mut File) -> i32 {
    with(f, 0, |s| i32::from(s.error) * 0x20)
}

#[unsafe(no_mangle)]
pub extern "C" fn clearerr(f: *mut File) {
    with(f, (), |s| {
        s.error = false;
        s.eof = false;
        if lowio::get(s.fd).is_some() {
            // A Ctrl+Z read in text mode stays the end until a seek.
        }
    });
}

#[unsafe(no_mangle)]
pub extern "C" fn clearerr_s(f: *mut File) -> i32 {
    if f.is_null() {
        global::invalid(EINVAL);
        return EINVAL;
    }
    clearerr(f);
    0
}

#[unsafe(no_mangle)]
pub extern "C" fn _fileno(f: *mut File) -> i32 {
    with(f, -1, |s| s.fd)
}

/// `setvbuf`: the mode and size are honoured with Poly's own buffer (the
/// caller's buffer is not used).
#[unsafe(no_mangle)]
pub extern "C" fn setvbuf(f: *mut File, _buf: *mut u8, mode: i32, size: usize) -> i32 {
    if !matches!(mode, IOFBF | IOLBF | IONBF) || (mode != IONBF && !(2..=i32::MAX as usize).contains(&size)) {
        global::invalid(EINVAL);
        return -1;
    }
    with(f, -1, |s| {
        let _ = flush(s);
        s.buffered = mode != IONBF;
        s.per_call = false;
        if s.buffered {
            s.bufsize = size;
        }
        0
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn setbuf(f: *mut File, buf: *mut u8) {
    if buf.is_null() {
        setvbuf(f, buf, IONBF, 0);
    } else {
        setvbuf(f, buf, IOFBF, BUFSIZ);
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn _lock_file(f: *mut File) {
    if let Some(s) = state(f) {
        s.cs.enter();
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn _unlock_file(f: *mut File) {
    if let Some(s) = state(f) {
        s.cs.leave();
    }
}

// ----- names ----------------------------------------------------------------------------

/// # Safety
/// `name` and `mode` are valid C strings.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fopen(name: *const u8, mode: *const u8) -> *mut File {
    if name.is_null() || mode.is_null() {
        global::invalid(EINVAL);
        return core::ptr::null_mut();
    }
    // SAFETY: the caller's strings.
    unsafe { open(&crate::text::wide_path(name), unit::slice(mode), fmode::SH_DENYNO) }
}

/// # Safety
/// As `fopen`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _wfopen(name: *const u16, mode: *const u16) -> *mut File {
    if name.is_null() || mode.is_null() {
        global::invalid(EINVAL);
        return core::ptr::null_mut();
    }
    // SAFETY: the caller's strings.
    let m: Vec<u8> = unsafe { unit::slice(mode) }.iter().map(|&c| if c < 0x80 { c as u8 } else { 0xFF }).collect();
    // SAFETY: as above.
    open(&unsafe { crate::text::wide_path_w(name) }, &m, fmode::SH_DENYNO)
}

/// `_fsopen(name, mode, shflag)`.
///
/// # Safety
/// As `fopen`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _fsopen(name: *const u8, mode: *const u8, share: i32) -> *mut File {
    if name.is_null() || mode.is_null() {
        global::invalid(EINVAL);
        return core::ptr::null_mut();
    }
    // SAFETY: the caller's strings.
    unsafe { open(&crate::text::wide_path(name), unit::slice(mode), share) }
}

/// # Safety
/// As `fopen`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _wfsopen(name: *const u16, mode: *const u16, share: i32) -> *mut File {
    if name.is_null() || mode.is_null() {
        global::invalid(EINVAL);
        return core::ptr::null_mut();
    }
    // SAFETY: the caller's strings.
    let m: Vec<u8> = unsafe { unit::slice(mode) }.iter().map(|&c| if c < 0x80 { c as u8 } else { 0xFF }).collect();
    // SAFETY: as above.
    open(&unsafe { crate::text::wide_path_w(name) }, &m, share)
}

/// `fopen_s(&f, name, mode)`: 0 or the `errno`.
///
/// # Safety
/// `out` is writable; the strings valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fopen_s(out: *mut *mut File, name: *const u8, mode: *const u8) -> i32 {
    if out.is_null() {
        global::invalid(EINVAL);
        return EINVAL;
    }
    // SAFETY: the caller's arguments.
    let f = unsafe { fopen(name, mode) };
    // SAFETY: writable.
    unsafe { *out = f };
    if f.is_null() { global::thread().errno } else { 0 }
}

/// # Safety
/// As `fopen_s`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _wfopen_s(out: *mut *mut File, name: *const u16, mode: *const u16) -> i32 {
    if out.is_null() {
        global::invalid(EINVAL);
        return EINVAL;
    }
    // SAFETY: the caller's arguments.
    let f = unsafe { _wfopen(name, mode) };
    // SAFETY: writable.
    unsafe { *out = f };
    if f.is_null() { global::thread().errno } else { 0 }
}

/// # Safety
/// As `fopen`; `f` a stream.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn freopen(name: *const u8, mode: *const u8, f: *mut File) -> *mut File {
    if name.is_null() || mode.is_null() {
        global::invalid(EINVAL);
        return core::ptr::null_mut();
    }
    // SAFETY: the caller's strings.
    unsafe { reopen(&crate::text::wide_path(name), unit::slice(mode), f) }
}

/// # Safety
/// As `freopen`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _wfreopen(name: *const u16, mode: *const u16, f: *mut File) -> *mut File {
    if name.is_null() || mode.is_null() {
        global::invalid(EINVAL);
        return core::ptr::null_mut();
    }
    // SAFETY: the caller's strings.
    let m: Vec<u8> = unsafe { unit::slice(mode) }.iter().map(|&c| if c < 0x80 { c as u8 } else { 0xFF }).collect();
    // SAFETY: as above.
    unsafe { reopen(&crate::text::wide_path_w(name), &m, f) }
}

/// # Safety
/// As `freopen`; `out` writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn freopen_s(out: *mut *mut File, name: *const u8, mode: *const u8, f: *mut File) -> i32 {
    if out.is_null() {
        global::invalid(EINVAL);
        return EINVAL;
    }
    // SAFETY: the caller's arguments.
    let r = unsafe { freopen(name, mode, f) };
    // SAFETY: writable.
    unsafe { *out = r };
    if r.is_null() { global::thread().errno } else { 0 }
}

// ----- formatted I/O ------------------------------------------------------------------

/// The unit types a stream writes.
pub trait StreamUnit: Unit {
    fn put(s: &mut Stream, c: Self) -> bool;
}

impl StreamUnit for u8 {
    fn put(s: &mut Stream, c: u8) -> bool {
        put_bytes(s, &[c])
    }
}

impl StreamUnit for u16 {
    fn put(s: &mut Stream, c: u16) -> bool {
        put_wide(s, c)
    }
}

/// A sink into a locked stream; remembers a failure.
struct StreamSink<'a> {
    s: &'a mut Stream,
    failed: bool,
}

impl<C: StreamUnit> Sink<C> for StreamSink<'_> {
    fn put(&mut self, c: C) {
        if !self.failed && !C::put(self.s, c) {
            self.failed = true;
        }
    }
}

/// Map a format error to `errno` (and the handler for an invalid format).
fn format_error(e: printf::Error) {
    match e {
        printf::Error::Invalid => global::invalid(EINVAL),
        printf::Error::Encoding => global::set_errno(EILSEQ),
    }
}

/// `vfprintf` / `vfwprintf` with the UCRT's option bits: the units
/// written, or -1.
///
/// # Safety
/// `fmt` is a valid C string and `args` its arguments.
pub unsafe fn vfprintf<C: StreamUnit>(f: *mut File, fmt: *const C, args: &mut impl Args, options: u64) -> i32 {
    if fmt.is_null() {
        global::invalid(EINVAL);
        return -1;
    }
    // SAFETY: the caller's string.
    let fmt = unsafe { unit::slice(fmt) };
    let count_output = crate::misc::count_output();
    with(f, -1, |s| {
        if !C::WIDE && !narrow_ok(s) {
            return -1;
        }
        let mut sink = StreamSink { s, failed: false };
        // SAFETY: the caller's arguments.
        match unsafe { printf::format(fmt, args, &mut sink, options, count_output) } {
            Ok(n) if !sink.failed => n as i32,
            Ok(_) => -1,
            Err(e) => {
                format_error(e);
                -1
            }
        }
    })
}

/// Which buffer rule a `sprintf`-style call follows.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum BufferRule {
    /// `__stdio_common_vsprintf`: by the option bits.
    Options,
    /// `__stdio_common_vsprintf_s`.
    Secure,
    /// `__stdio_common_vsnprintf_s` with this count.
    SecureCount(usize),
}

/// `vsprintf`-style formatting into `buf` of `capacity` units.
///
/// # Safety
/// `buf` holds `capacity` units (or is NULL with 0); `fmt` and `args` as
/// [`vfprintf`].
pub unsafe fn vsprintf<C: Unit>(
    buf: *mut C,
    capacity: usize,
    fmt: *const C,
    args: &mut impl Args,
    options: u64,
    rule: BufferRule,
) -> i32 {
    if fmt.is_null() || (buf.is_null() && capacity != 0) {
        global::invalid(EINVAL);
        return -1;
    }
    if buf.is_null() && rule != BufferRule::Options {
        global::invalid(EINVAL);
        return -1;
    }
    // SAFETY: the caller's string.
    let f = unsafe { unit::slice(fmt) };
    let keep = match rule {
        BufferRule::SecureCount(count) if count != printf::TRUNCATE => count.min(capacity),
        _ => capacity,
    };
    let mut sink = printf::Buffer { ptr: buf, capacity: keep, kept: 0 };
    // SAFETY: the caller's arguments and buffer.
    let total = match unsafe { printf::format(f, args, &mut sink, options, crate::misc::count_output()) } {
        Ok(n) => n,
        Err(e) => {
            format_error(e);
            if !buf.is_null() && capacity > 0 {
                // SAFETY: at least one unit.
                unsafe { buf.write(C::default()) };
            }
            return -1;
        }
    };
    let fin = match rule {
        BufferRule::Options => printf::finish_sprintf(options, capacity, total),
        BufferRule::Secure => printf::finish_sprintf_s(capacity, total),
        BufferRule::SecureCount(count) => printf::finish_snprintf_s(capacity, count, total),
    };
    if let Some(at) = fin.nul_at {
        // SAFETY: `at` < capacity.
        unsafe { buf.add(at).write(C::default()) };
    }
    if fin.invalid {
        global::invalid(poly_crt::errno::ERANGE);
    }
    fin.result
}

/// A stream as `scanf` input: characters after text translation; the one
/// looked at but not taken is pushed back at the end.
struct StreamInput<'a> {
    s: &'a mut Stream,
    wide: bool,
    held: Option<u32>,
}

impl poly_crt::scanf::Input for StreamInput<'_> {
    fn peek(&mut self) -> Option<u32> {
        if self.held.is_none() {
            self.held = if self.wide { get_wide(self.s).map(u32::from) } else { get_byte(self.s).map(u32::from) };
        }
        self.held
    }
    fn bump(&mut self) {
        self.held = None;
    }
}

/// `vfscanf` / `vfwscanf`.
///
/// # Safety
/// `fmt` is a valid C string and `args` its pointers.
pub unsafe fn vfscanf<C: Unit>(f: *mut File, fmt: *const C, args: &mut impl Args, options: u64) -> i32 {
    if fmt.is_null() {
        global::invalid(EINVAL);
        return EOF;
    }
    // SAFETY: the caller's string.
    let fmt = unsafe { unit::slice(fmt) };
    with(f, EOF, |s| {
        if !begin_read(s) {
            return EOF;
        }
        let mut input = StreamInput { s, wide: C::WIDE, held: None };
        // SAFETY: the caller's arguments.
        let r = unsafe { poly_crt::scanf::scan(fmt, &mut input, args, options) };
        if let Some(c) = input.held {
            if C::WIDE {
                input.s.wunget.push(c as u16);
            } else {
                input.s.unget.push(c as u8);
            }
        }
        r.unwrap_or_else(|_| {
            global::invalid(EINVAL);
            EOF
        })
    })
}

/// `vsscanf` / `vswscanf` (`count`: the units of `buf` to read, or
/// `usize::MAX` for up to its NUL).
///
/// # Safety
/// `buf` and `fmt` are valid strings and `args` the pointers.
pub unsafe fn vsscanf<C: Unit>(buf: *const C, count: usize, fmt: *const C, args: &mut impl Args, options: u64) -> i32 {
    if buf.is_null() || fmt.is_null() {
        global::invalid(EINVAL);
        return EOF;
    }
    // SAFETY: the caller's strings.
    let (s, f) = unsafe { (unit::slice_max(buf, count), unit::slice(fmt)) };
    let mut input = poly_crt::scanf::Str { s, at: 0 };
    // SAFETY: the caller's arguments.
    unsafe { poly_crt::scanf::scan(f, &mut input, args, options) }.unwrap_or_else(|_| {
        global::invalid(EINVAL);
        EOF
    })
}

/// `perror`'s text: `"<s>: <message>\n"` (or the message alone) on
/// `stderr`.
pub fn perror_text(prefix: &[u8]) {
    let e = global::thread().errno;
    let mut out: Vec<u8> = Vec::new();
    if !prefix.is_empty() {
        out.extend_from_slice(prefix);
        out.extend_from_slice(b": ");
    }
    out.extend_from_slice(poly_crt::errno::message(e).as_bytes());
    out.push(b'\n');
    with(iob(2), (), |s| {
        put_bytes(s, &out);
    });
}

/// # Safety
/// `s` is NULL or a valid C string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn perror(s: *const u8) {
    // SAFETY: the caller's string.
    perror_text(if s.is_null() { &[] } else { unsafe { unit::slice(s) } });
}

/// # Safety
/// `s` is NULL or a valid wide C string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _wperror(s: *const u16) {
    // SAFETY: the caller's string.
    let p = if s.is_null() { Vec::new() } else { crate::text::wide_to_ansi(unsafe { unit::slice(s) }) };
    perror_text(&p);
}
