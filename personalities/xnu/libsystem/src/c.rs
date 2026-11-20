//! libsystem_c, v0: what a C "hello" and its start-up need (ADR 0258):
//! stdio on the three standard streams (output and byte input), the
//! `printf` family, `exit` with `atexit` and `__cxa_atexit`, `abort`, the
//! stack protector, the `_FORTIFY_SOURCE` checks (`__*_chk`), the string
//! and number functions, `getenv` and the `_NSGet*` accessors.
//!
//! The logic is in `poly_libsystem` (host-tested); this module exports it
//! under the C names. `stdout` is line buffered (whether it is a terminal
//! cannot be asked yet), `stderr` unbuffered, `stdin` reads a byte at a
//! time; every output stream is flushed by `exit` and before `stdin`
//! reads.

use core::ffi::{c_char, c_void};
use poly_libsystem::atexit::{AtExit, Handler};
use poly_libsystem::printf::{self, Sink, VaList, VaListTag};
use poly_libsystem::stdio::{self, File, Mode, Output, Stream};
use poly_libsystem::sys::errno::*;
use poly_libsystem::{cstr, sys};

unsafe extern "C" {
    fn write(fd: i32, buf: *const u8, len: usize) -> isize;
    fn read(fd: i32, buf: *mut u8, len: usize) -> isize;
    fn _exit(status: i32) -> !;
    fn getpid() -> i32;
    fn kill(pid: i32, sig: i32) -> i32;
    fn getentropy(buf: *mut u8, len: usize) -> i32;
    fn malloc(size: usize) -> *mut u8;
    fn _dyld_get_image_header(i: u32) -> *const c_void;
    static mut NXArgc: i32;
    static mut NXArgv: *const *const c_char;
    static mut environ: *const *const c_char;
    static mut __progname: *const c_char;
}

// ---------------------------------------------------------------- stdio

static mut IN: Stream = Stream::new(0, Mode::Unbuffered);
static mut OUT: Stream = Stream::new(1, Mode::Line);
static mut ERR: Stream = Stream::new(2, Mode::Unbuffered);

/// `__sF`: stdin, stdout, stderr (`<_stdio.h>` of older SDKs).
#[unsafe(no_mangle)]
#[allow(non_upper_case_globals)]
static mut __sF: [File; 3] = [
    File::new(0, stdio::SRD, &raw mut IN as *mut c_void),
    File::new(1, stdio::SWR | stdio::SLBF, &raw mut OUT as *mut c_void),
    File::new(2, stdio::SWR | stdio::SNBF, &raw mut ERR as *mut c_void),
];

#[unsafe(no_mangle)]
#[allow(non_upper_case_globals)]
static mut __stdinp: *mut File = &raw mut __sF as *mut File;
#[unsafe(no_mangle)]
#[allow(non_upper_case_globals)]
static mut __stdoutp: *mut File = (&raw mut __sF as *mut File).wrapping_add(1);
#[unsafe(no_mangle)]
#[allow(non_upper_case_globals)]
static mut __stderrp: *mut File = (&raw mut __sF as *mut File).wrapping_add(2);

/// `write(2)`, retried when a signal interrupts it.
struct Fd;

impl Output for Fd {
    fn write(&mut self, fd: i32, bytes: &[u8]) -> Result<usize, i32> {
        loop {
            // SAFETY: a plain write of the caller's bytes.
            let n = unsafe { write(fd, bytes.as_ptr(), bytes.len()) };
            if n >= 0 {
                return Ok(n as usize);
            }
            let e = crate::errno();
            if e != EINTR {
                return Err(e);
            }
        }
    }
}

/// The stream behind a `FILE`, if it is one of ours.
///
/// # Safety
/// `f` is a `FILE` pointer.
unsafe fn stream<'a>(f: *mut File) -> Option<&'a mut Stream> {
    // SAFETY: the caller's contract; our `FILE`s carry their stream in
    // `_cookie`.
    unsafe { ((*f).cookie as *mut Stream).as_mut() }
}

/// Write `bytes` to `f`; on failure the error indicator and `errno` are
/// set.
///
/// # Safety
/// `f` is a `FILE` pointer.
unsafe fn put(f: *mut File, bytes: &[u8]) -> Result<(), ()> {
    // SAFETY: the caller's contract.
    unsafe {
        let Some(s) = stream(f) else {
            crate::set_errno(EBADF);
            return Err(());
        };
        if (*f).flags & stdio::SWR == 0 {
            (*f).flags |= stdio::SERR;
            crate::set_errno(EBADF);
            return Err(());
        }
        s.write(bytes, &mut Fd).map_err(|e| {
            (*f).flags |= stdio::SERR;
            crate::set_errno(e);
        })
    }
}

/// # Safety
/// `f` is a `FILE` pointer.
unsafe fn flush(f: *mut File) -> Result<(), ()> {
    // SAFETY: the caller's contract.
    unsafe {
        let Some(s) = stream(f) else { return Ok(()) };
        s.flush(&mut Fd).map_err(|e| {
            (*f).flags |= stdio::SERR;
            crate::set_errno(e);
        })
    }
}

fn std_file(i: usize) -> *mut File {
    (&raw mut __sF as *mut File).wrapping_add(i)
}

fn flush_all() -> i32 {
    let mut r = 0;
    for i in 1..3 {
        // SAFETY: our own standard streams.
        if unsafe { flush(std_file(i)) }.is_err() {
            r = stdio::EOF;
        }
    }
    r
}

#[unsafe(no_mangle)]
unsafe extern "C" fn fflush(f: *mut File) -> i32 {
    if f.is_null() {
        return flush_all();
    }
    // SAFETY: the C contract: a stream.
    if unsafe { flush(f) }.is_ok() { 0 } else { stdio::EOF }
}

#[unsafe(no_mangle)]
unsafe extern "C" fn fwrite(p: *const u8, size: usize, n: usize, f: *mut File) -> usize {
    let Some(total) = size.checked_mul(n) else { return 0 };
    if total == 0 {
        return 0;
    }
    // SAFETY: the C contract: `p` holds size × n bytes; `f` is a stream.
    match unsafe { put(f, core::slice::from_raw_parts(p, total)) } {
        Ok(()) => n,
        Err(()) => 0,
    }
}

#[unsafe(no_mangle)]
unsafe extern "C" fn fputc(c: i32, f: *mut File) -> i32 {
    // SAFETY: the C contract.
    match unsafe { put(f, &[c as u8]) } {
        Ok(()) => i32::from(c as u8),
        Err(()) => stdio::EOF,
    }
}

#[unsafe(no_mangle)]
unsafe extern "C" fn putc(c: i32, f: *mut File) -> i32 {
    // SAFETY: the C contract.
    unsafe { fputc(c, f) }
}

/// What `putc_unlocked` calls when the buffer it sees is full: Poly's
/// `FILE`s always look full (`_w` is 0), so every byte comes here.
#[unsafe(no_mangle)]
unsafe extern "C" fn __swbuf(c: i32, f: *mut File) -> i32 {
    // SAFETY: the C contract; the macro decremented `_w`, which stays 0.
    unsafe {
        (*f).w = 0;
        fputc(c, f)
    }
}

#[unsafe(no_mangle)]
unsafe extern "C" fn putchar(c: i32) -> i32 {
    // SAFETY: our stdout.
    unsafe { fputc(c, std_file(1)) }
}

#[unsafe(no_mangle)]
unsafe extern "C" fn fputs(s: *const u8, f: *mut File) -> i32 {
    // SAFETY: the C contract: a string and a stream.
    match unsafe { put(f, core::slice::from_raw_parts(s, cstr::strlen(s))) } {
        Ok(()) => 0,
        Err(()) => stdio::EOF,
    }
}

#[unsafe(no_mangle)]
unsafe extern "C" fn puts(s: *const u8) -> i32 {
    let f = std_file(1);
    // SAFETY: the C contract: a string.
    match unsafe { put(f, core::slice::from_raw_parts(s, cstr::strlen(s))).and_then(|()| put(f, b"\n")) } {
        Ok(()) => 0,
        Err(()) => stdio::EOF,
    }
}

/// One byte from `f`; output streams are flushed first.
///
/// # Safety
/// `f` is a `FILE` pointer.
unsafe fn get(f: *mut File) -> i32 {
    flush_all();
    // SAFETY: the C contract.
    unsafe {
        let Some(s) = stream(f) else {
            crate::set_errno(EBADF);
            return stdio::EOF;
        };
        if (*f).flags & stdio::SRD == 0 {
            (*f).flags |= stdio::SERR;
            crate::set_errno(EBADF);
            return stdio::EOF;
        }
        let mut b = 0u8;
        loop {
            match read(s.fd, &mut b, 1) {
                1 => return i32::from(b),
                0 => {
                    (*f).flags |= stdio::SEOF;
                    return stdio::EOF;
                }
                _ if crate::errno() == EINTR => {}
                _ => {
                    (*f).flags |= stdio::SERR;
                    return stdio::EOF;
                }
            }
        }
    }
}

#[unsafe(no_mangle)]
unsafe extern "C" fn fgetc(f: *mut File) -> i32 {
    // SAFETY: the C contract.
    unsafe { get(f) }
}

#[unsafe(no_mangle)]
unsafe extern "C" fn getc(f: *mut File) -> i32 {
    // SAFETY: the C contract.
    unsafe { get(f) }
}

#[unsafe(no_mangle)]
unsafe extern "C" fn getchar() -> i32 {
    // SAFETY: our stdin.
    unsafe { get(std_file(0)) }
}

/// What `getc_unlocked` calls when its buffer is empty (always, `_r` is 0).
#[unsafe(no_mangle)]
unsafe extern "C" fn __srget(f: *mut File) -> i32 {
    // SAFETY: the C contract; the macro decremented `_r`.
    unsafe {
        (*f).r = 0;
        get(f)
    }
}

#[unsafe(no_mangle)]
unsafe extern "C" fn ferror(f: *mut File) -> i32 {
    // SAFETY: the C contract.
    i32::from(unsafe { (*f).flags } & stdio::SERR != 0)
}

#[unsafe(no_mangle)]
unsafe extern "C" fn feof(f: *mut File) -> i32 {
    // SAFETY: the C contract.
    i32::from(unsafe { (*f).flags } & stdio::SEOF != 0)
}

#[unsafe(no_mangle)]
unsafe extern "C" fn clearerr(f: *mut File) {
    // SAFETY: the C contract.
    unsafe { (*f).flags &= !(stdio::SERR | stdio::SEOF) };
}

#[unsafe(no_mangle)]
unsafe extern "C" fn fileno(f: *mut File) -> i32 {
    // SAFETY: the C contract.
    i32::from(unsafe { (*f).file })
}

// --------------------------------------------------------------- printf

// The variadic functions save the argument registers into a System V
// register save area, build a `va_list` over it and call their `v`
// function with it as the argument after the fixed ones.
macro_rules! variadic {
    ($($name:literal => $v:literal, $fixed:literal, $reg:literal;)*) => {
        core::arch::global_asm!(
            $(
                concat!(".globl _", $name),
                ".p2align 4",
                concat!("_", $name, ":"),
                "    push rbp",
                "    mov rbp, rsp",
                "    sub rsp, 208",
                "    mov [rsp], rdi",
                "    mov [rsp + 8], rsi",
                "    mov [rsp + 16], rdx",
                "    mov [rsp + 24], rcx",
                "    mov [rsp + 32], r8",
                "    mov [rsp + 40], r9",
                "    test al, al",
                "    je 2f",
                "    movaps [rsp + 48], xmm0",
                "    movaps [rsp + 64], xmm1",
                "    movaps [rsp + 80], xmm2",
                "    movaps [rsp + 96], xmm3",
                "    movaps [rsp + 112], xmm4",
                "    movaps [rsp + 128], xmm5",
                "    movaps [rsp + 144], xmm6",
                "    movaps [rsp + 160], xmm7",
                "2:",
                "    lea r10, [rsp + 176]",
                concat!("    mov dword ptr [r10], 8 * ", $fixed),
                "    mov dword ptr [r10 + 4], 48",
                "    lea r11, [rbp + 16]",
                "    mov [r10 + 8], r11",
                "    mov [r10 + 16], rsp",
                concat!("    mov ", $reg, ", r10"),
                concat!("    call _", $v),
                "    leave",
                "    ret",
            )*
        );
    };
}

variadic! {
    "printf" => "vprintf", 1, "rsi";
    "fprintf" => "vfprintf", 2, "rdx";
    "sprintf" => "vsprintf", 2, "rdx";
    "dprintf" => "vdprintf", 2, "rdx";
    "asprintf" => "vasprintf", 2, "rdx";
    "snprintf" => "vsnprintf", 3, "rcx";
    "__sprintf_chk" => "__vsprintf_chk", 4, "r8";
    "__snprintf_chk" => "__vsnprintf_chk", 5, "r9";
}

/// The result of a `printf` call: the count, or -1 with `errno`.
fn result(r: Result<usize, printf::Error>) -> i32 {
    match r {
        Ok(n) if n <= i32::MAX as usize => n as i32,
        Ok(_) => {
            crate::set_errno(EOVERFLOW);
            -1
        }
        Err(printf::Error::Invalid) => {
            crate::set_errno(EINVAL);
            -1
        }
        Err(printf::Error::Encoding) => {
            crate::set_errno(EILSEQ);
            -1
        }
    }
}

/// # Safety
/// `fmt` is a string and `ap` the arguments it names.
unsafe fn run<S: Sink>(fmt: *const u8, ap: *mut VaListTag, sink: &mut S) -> Result<usize, printf::Error> {
    // SAFETY: the caller's contract.
    unsafe { printf::format(core::slice::from_raw_parts(fmt, cstr::strlen(fmt)), &mut VaList(ap), sink) }
}

struct ToFile {
    f: *mut File,
    failed: bool,
}

impl Sink for ToFile {
    fn put(&mut self, bytes: &[u8]) {
        // SAFETY: `f` is the caller's stream.
        if !self.failed && unsafe { put(self.f, bytes) }.is_err() {
            self.failed = true;
        }
    }
}

#[unsafe(no_mangle)]
unsafe extern "C" fn vfprintf(f: *mut File, fmt: *const u8, ap: *mut VaListTag) -> i32 {
    let mut sink = ToFile { f, failed: false };
    // SAFETY: the C contract.
    let r = unsafe { run(fmt, ap, &mut sink) };
    if sink.failed { -1 } else { result(r) }
}

#[unsafe(no_mangle)]
unsafe extern "C" fn vprintf(fmt: *const u8, ap: *mut VaListTag) -> i32 {
    // SAFETY: the C contract.
    unsafe { vfprintf(std_file(1), fmt, ap) }
}

#[unsafe(no_mangle)]
unsafe extern "C" fn vdprintf(fd: i32, fmt: *const u8, ap: *mut VaListTag) -> i32 {
    struct ToFd(Stream, bool);
    impl Sink for ToFd {
        fn put(&mut self, bytes: &[u8]) {
            if !self.1 && self.0.write(bytes, &mut Fd).is_err() {
                self.1 = true;
            }
        }
    }
    let mut sink = ToFd(Stream::new(fd, Mode::Full), false);
    // SAFETY: the C contract.
    let r = unsafe { run(fmt, ap, &mut sink) };
    if sink.1 || sink.0.flush(&mut Fd).is_err() {
        return -1;
    }
    result(r)
}

/// Into memory without a bound (`sprintf`).
struct Unbounded(*mut u8, usize);

impl Sink for Unbounded {
    fn put(&mut self, bytes: &[u8]) {
        // SAFETY: the C contract of `sprintf`: the buffer is large enough.
        unsafe { core::ptr::copy_nonoverlapping(bytes.as_ptr(), self.0.add(self.1), bytes.len()) };
        self.1 += bytes.len();
    }
}

#[unsafe(no_mangle)]
unsafe extern "C" fn vsprintf(buf: *mut u8, fmt: *const u8, ap: *mut VaListTag) -> i32 {
    let mut sink = Unbounded(buf, 0);
    // SAFETY: the C contract.
    let r = unsafe { run(fmt, ap, &mut sink) };
    // SAFETY: as above, room for the NUL.
    unsafe { *buf.add(sink.1) = 0 };
    result(r)
}

#[unsafe(no_mangle)]
unsafe extern "C" fn vsnprintf(buf: *mut u8, n: usize, fmt: *const u8, ap: *mut VaListTag) -> i32 {
    if n == 0 {
        // SAFETY: the C contract; nothing is written.
        return result(unsafe { run(fmt, ap, &mut printf::Count(0)) });
    }
    // SAFETY: the C contract: `buf` holds `n` bytes.
    let space = unsafe { core::slice::from_raw_parts_mut(buf, n - 1) };
    let mut sink = printf::Buffer { buf: space, len: 0 };
    // SAFETY: the C contract.
    let r = unsafe { run(fmt, ap, &mut sink) };
    // SAFETY: inside the buffer.
    unsafe { *buf.add(sink.len) = 0 };
    result(r)
}

#[unsafe(no_mangle)]
unsafe extern "C" fn vasprintf(out: *mut *mut u8, fmt: *const u8, ap: *mut VaListTag) -> i32 {
    // Two passes: a copy of the va_list for the count (`va_copy` copies
    // the System V structure).
    // SAFETY: the C contract.
    unsafe {
        *out = core::ptr::null_mut();
        let mut copy = VaListTag {
            gp_offset: (*ap).gp_offset,
            fp_offset: (*ap).fp_offset,
            overflow_arg_area: (*ap).overflow_arg_area,
            reg_save_area: (*ap).reg_save_area,
        };
        let n = result(run(fmt, &mut copy, &mut printf::Count(0)));
        if n < 0 {
            return n;
        }
        let buf = malloc(n as usize + 1);
        if buf.is_null() {
            return -1;
        }
        let mut sink = printf::Buffer { buf: core::slice::from_raw_parts_mut(buf, n as usize), len: 0 };
        let _ = run(fmt, ap, &mut sink);
        *buf.add(n as usize) = 0;
        *out = buf;
        n
    }
}

/// `__vsprintf_chk(buf, flag, size of buf, fmt, ap)`: `vsprintf` that
/// stops the program if the output does not fit (`<secure/_stdio.h>`).
#[unsafe(no_mangle)]
unsafe extern "C" fn __vsprintf_chk(buf: *mut u8, _flag: i32, size: usize, fmt: *const u8, ap: *mut VaListTag) -> i32 {
    if size == 0 {
        chk_fail();
    }
    // SAFETY: the C contract: `buf` holds `size` bytes.
    let n = unsafe { vsnprintf(buf, size, fmt, ap) };
    if n >= 0 && n as usize >= size {
        chk_fail();
    }
    n
}

/// `__vsnprintf_chk(buf, n, flag, size of buf, fmt, ap)`.
#[unsafe(no_mangle)]
unsafe extern "C" fn __vsnprintf_chk(
    buf: *mut u8,
    n: usize,
    _flag: i32,
    size: usize,
    fmt: *const u8,
    ap: *mut VaListTag,
) -> i32 {
    if n > size {
        chk_fail();
    }
    // SAFETY: the C contract.
    unsafe { vsnprintf(buf, n, fmt, ap) }
}

// ----------------------------------------------------------- exit, abort

static AT_EXIT: crate::Global<AtExit> = crate::Global::new(AtExit::new());

fn handlers() -> &'static mut AtExit {
    // SAFETY: single thread (v0); each use takes the table once.
    unsafe { AT_EXIT.get() }
}

fn call(h: Handler) {
    match h {
        Handler::Plain(f) => {
            // SAFETY: registered by `atexit` as `void (*)(void)`.
            let f: extern "C" fn() = unsafe { core::mem::transmute(f) };
            f();
        }
        Handler::WithArg { func, arg, .. } => {
            // SAFETY: registered by `__cxa_atexit` as `void (*)(void *)`.
            let f: extern "C" fn(usize) = unsafe { core::mem::transmute(func) };
            f(arg);
        }
    }
}

#[unsafe(no_mangle)]
extern "C" fn atexit(f: usize) -> i32 {
    if handlers().register(Handler::Plain(f)).is_ok() { 0 } else { -1 }
}

#[unsafe(no_mangle)]
extern "C" fn __cxa_atexit(func: usize, arg: usize, dso: usize) -> i32 {
    if handlers().register(Handler::WithArg { func, arg, dso }).is_ok() { 0 } else { -1 }
}

#[unsafe(no_mangle)]
extern "C" fn __cxa_finalize(dso: usize) {
    while let Some(h) = handlers().pop_for(dso) {
        call(h);
    }
}

/// `exit`: the registered functions (last first), then every stream
/// flushed, then `_exit`.
#[unsafe(no_mangle)]
extern "C" fn exit(status: i32) -> ! {
    while let Some(h) = handlers().pop() {
        call(h);
    }
    flush_all();
    // SAFETY: ends the process.
    unsafe { _exit(status) }
}

/// `abort`: SIGABRT to the process; if that does not end it, status 134.
#[unsafe(no_mangle)]
extern "C" fn abort() -> ! {
    // SAFETY: plain system calls.
    unsafe {
        kill(getpid(), sys::SIGABRT);
        _exit(134)
    }
}

fn chk_fail() -> ! {
    let msg = b"poly libSystem: buffer overflow detected\n";
    // SAFETY: a plain write to stderr.
    unsafe { write(2, msg.as_ptr(), msg.len()) };
    abort()
}

/// The stack protector's canary; set by this library's initializer.
#[unsafe(no_mangle)]
#[allow(non_upper_case_globals)]
static mut __stack_chk_guard: u64 = 0;

#[unsafe(no_mangle)]
extern "C" fn __stack_chk_fail() -> ! {
    let msg = b"poly libSystem: stack buffer overflow detected\n";
    // SAFETY: a plain write to stderr.
    unsafe { write(2, msg.as_ptr(), msg.len()) };
    abort()
}

// ------------------------------------------------------- string checks

#[unsafe(no_mangle)]
unsafe extern "C" fn __memcpy_chk(d: *mut u8, s: *const u8, n: usize, size: usize) -> *mut u8 {
    if n > size {
        chk_fail();
    }
    // SAFETY: the C contract of memcpy.
    unsafe { core::ptr::copy_nonoverlapping(s, d, n) };
    d
}

#[unsafe(no_mangle)]
unsafe extern "C" fn __memmove_chk(d: *mut u8, s: *const u8, n: usize, size: usize) -> *mut u8 {
    if n > size {
        chk_fail();
    }
    // SAFETY: the C contract of memmove.
    unsafe { core::ptr::copy(s, d, n) };
    d
}

#[unsafe(no_mangle)]
unsafe extern "C" fn __memset_chk(d: *mut u8, c: i32, n: usize, size: usize) -> *mut u8 {
    if n > size {
        chk_fail();
    }
    // SAFETY: the C contract of memset.
    unsafe { core::ptr::write_bytes(d, c as u8, n) };
    d
}

#[unsafe(no_mangle)]
unsafe extern "C" fn __strcpy_chk(d: *mut u8, s: *const u8, size: usize) -> *mut u8 {
    // SAFETY: the C contract of strcpy.
    unsafe {
        if cstr::strlen(s) >= size {
            chk_fail();
        }
        cstr::stpcpy(d, s);
    }
    d
}

#[unsafe(no_mangle)]
unsafe extern "C" fn __stpcpy_chk(d: *mut u8, s: *const u8, size: usize) -> *mut u8 {
    // SAFETY: the C contract of stpcpy.
    unsafe {
        if cstr::strlen(s) >= size {
            chk_fail();
        }
        cstr::stpcpy(d, s)
    }
}

#[unsafe(no_mangle)]
unsafe extern "C" fn __strcat_chk(d: *mut u8, s: *const u8, size: usize) -> *mut u8 {
    // SAFETY: the C contract of strcat.
    unsafe {
        let at = cstr::strlen(d);
        if at + cstr::strlen(s) >= size {
            chk_fail();
        }
        cstr::stpcpy(d.add(at), s);
    }
    d
}

#[unsafe(no_mangle)]
unsafe extern "C" fn __strncpy_chk(d: *mut u8, s: *const u8, n: usize, size: usize) -> *mut u8 {
    if n > size {
        chk_fail();
    }
    // SAFETY: the C contract of strncpy.
    unsafe { cstr::strncpy(d, s, n) }
}

#[unsafe(no_mangle)]
unsafe extern "C" fn __strncat_chk(d: *mut u8, s: *const u8, n: usize, size: usize) -> *mut u8 {
    // SAFETY: the C contract of strncat.
    unsafe {
        if cstr::strlen(d) + cstr::strnlen(s, n) >= size {
            chk_fail();
        }
        cstr::strncat(d, s, n)
    }
}

#[unsafe(no_mangle)]
unsafe extern "C" fn __strlcpy_chk(d: *mut u8, s: *const u8, n: usize, size: usize) -> usize {
    if n > size {
        chk_fail();
    }
    // SAFETY: the C contract of strlcpy.
    unsafe { cstr::strlcpy(d, s, n) }
}

#[unsafe(no_mangle)]
unsafe extern "C" fn __strlcat_chk(d: *mut u8, s: *const u8, n: usize, size: usize) -> usize {
    if n > size {
        chk_fail();
    }
    // SAFETY: the C contract of strlcat.
    unsafe { cstr::strlcat(d, s, n) }
}

// -------------------------------------------------------------- strings

#[unsafe(no_mangle)]
unsafe extern "C" fn strnlen(s: *const u8, n: usize) -> usize {
    // SAFETY: the C contract.
    unsafe { cstr::strnlen(s, n) }
}

#[unsafe(no_mangle)]
unsafe extern "C" fn strcmp(a: *const u8, b: *const u8) -> i32 {
    // SAFETY: the C contract.
    unsafe { cstr::strncmp(a, b, usize::MAX) }
}

#[unsafe(no_mangle)]
unsafe extern "C" fn strncmp(a: *const u8, b: *const u8, n: usize) -> i32 {
    // SAFETY: the C contract.
    unsafe { cstr::strncmp(a, b, n) }
}

#[unsafe(no_mangle)]
unsafe extern "C" fn strcpy(d: *mut u8, s: *const u8) -> *mut u8 {
    // SAFETY: the C contract.
    unsafe { cstr::stpcpy(d, s) };
    d
}

#[unsafe(no_mangle)]
unsafe extern "C" fn stpcpy(d: *mut u8, s: *const u8) -> *mut u8 {
    // SAFETY: the C contract.
    unsafe { cstr::stpcpy(d, s) }
}

#[unsafe(no_mangle)]
unsafe extern "C" fn strncpy(d: *mut u8, s: *const u8, n: usize) -> *mut u8 {
    // SAFETY: the C contract.
    unsafe { cstr::strncpy(d, s, n) }
}

#[unsafe(no_mangle)]
unsafe extern "C" fn strlcpy(d: *mut u8, s: *const u8, n: usize) -> usize {
    // SAFETY: the C contract.
    unsafe { cstr::strlcpy(d, s, n) }
}

#[unsafe(no_mangle)]
unsafe extern "C" fn strlcat(d: *mut u8, s: *const u8, n: usize) -> usize {
    // SAFETY: the C contract.
    unsafe { cstr::strlcat(d, s, n) }
}

#[unsafe(no_mangle)]
unsafe extern "C" fn strcat(d: *mut u8, s: *const u8) -> *mut u8 {
    // SAFETY: the C contract.
    unsafe { cstr::stpcpy(d.add(cstr::strlen(d)), s) };
    d
}

#[unsafe(no_mangle)]
unsafe extern "C" fn strncat(d: *mut u8, s: *const u8, n: usize) -> *mut u8 {
    // SAFETY: the C contract.
    unsafe { cstr::strncat(d, s, n) }
}

#[unsafe(no_mangle)]
unsafe extern "C" fn strchr(s: *const u8, c: i32) -> *const u8 {
    // SAFETY: the C contract.
    unsafe { cstr::strchr(s, c) }
}

#[unsafe(no_mangle)]
unsafe extern "C" fn strrchr(s: *const u8, c: i32) -> *const u8 {
    // SAFETY: the C contract.
    unsafe { cstr::strrchr(s, c) }
}

#[unsafe(no_mangle)]
unsafe extern "C" fn strstr(h: *const u8, n: *const u8) -> *const u8 {
    // SAFETY: the C contract.
    unsafe { cstr::strstr(h, n) }
}

#[unsafe(no_mangle)]
unsafe extern "C" fn memchr(s: *const u8, c: i32, n: usize) -> *const u8 {
    // SAFETY: the C contract.
    unsafe { cstr::memchr(s, c, n) }
}

#[unsafe(no_mangle)]
unsafe extern "C" fn bcopy(s: *const u8, d: *mut u8, n: usize) {
    // SAFETY: the C contract (overlap allowed).
    unsafe { core::ptr::copy(s, d, n) }
}

#[unsafe(no_mangle)]
unsafe extern "C" fn strndup(s: *const u8, n: usize) -> *mut u8 {
    // SAFETY: the C contract.
    unsafe {
        let len = cstr::strnlen(s, n);
        let p = malloc(len + 1);
        if !p.is_null() {
            core::ptr::copy_nonoverlapping(s, p, len);
            *p.add(len) = 0;
        }
        p
    }
}

#[unsafe(no_mangle)]
unsafe extern "C" fn strdup(s: *const u8) -> *mut u8 {
    // SAFETY: the C contract.
    unsafe { strndup(s, usize::MAX) }
}

// -------------------------------------------------------------- numbers

/// # Safety
/// `s` is a string; `end` is null or a pointer to a pointer.
unsafe fn strto(s: *const u8, end: *mut *const u8, base: i32, signed: bool, bits: u32) -> u64 {
    // SAFETY: the caller's contract.
    unsafe {
        let r = cstr::strto(s, base, signed, bits);
        if !end.is_null() {
            *end = s.add(r.used);
        }
        if r.bad_base {
            crate::set_errno(EINVAL);
        } else if r.range {
            crate::set_errno(ERANGE);
        }
        r.value
    }
}

#[unsafe(no_mangle)]
unsafe extern "C" fn strtol(s: *const u8, end: *mut *const u8, base: i32) -> i64 {
    // SAFETY: the C contract.
    unsafe { strto(s, end, base, true, 64) as i64 }
}

#[unsafe(no_mangle)]
unsafe extern "C" fn strtoll(s: *const u8, end: *mut *const u8, base: i32) -> i64 {
    // SAFETY: the C contract.
    unsafe { strto(s, end, base, true, 64) as i64 }
}

#[unsafe(no_mangle)]
unsafe extern "C" fn strtoul(s: *const u8, end: *mut *const u8, base: i32) -> u64 {
    // SAFETY: the C contract.
    unsafe { strto(s, end, base, false, 64) }
}

#[unsafe(no_mangle)]
unsafe extern "C" fn strtoull(s: *const u8, end: *mut *const u8, base: i32) -> u64 {
    // SAFETY: the C contract.
    unsafe { strto(s, end, base, false, 64) }
}

#[unsafe(no_mangle)]
unsafe extern "C" fn atoi(s: *const u8) -> i32 {
    // SAFETY: the C contract.
    unsafe { cstr::strto(s, 10, true, 64) }.value as i32
}

#[unsafe(no_mangle)]
unsafe extern "C" fn atol(s: *const u8) -> i64 {
    // SAFETY: the C contract.
    unsafe { cstr::strto(s, 10, true, 64) }.value as i64
}

#[unsafe(no_mangle)]
unsafe extern "C" fn atoll(s: *const u8) -> i64 {
    // SAFETY: the C contract.
    unsafe { atol(s) }
}

// ---------------------------------------------- environment and program

#[unsafe(no_mangle)]
unsafe extern "C" fn getenv(name: *const u8) -> *const u8 {
    // SAFETY: `environ` is libdyld's, set from dyld's envp.
    unsafe { cstr::getenv(environ as *const *const u8, name) }
}

#[unsafe(no_mangle)]
extern "C" fn _NSGetArgc() -> *mut i32 {
    &raw mut NXArgc
}

#[unsafe(no_mangle)]
extern "C" fn _NSGetArgv() -> *mut *const *const c_char {
    &raw mut NXArgv
}

#[unsafe(no_mangle)]
extern "C" fn _NSGetEnviron() -> *mut *const *const c_char {
    &raw mut environ
}

#[unsafe(no_mangle)]
extern "C" fn _NSGetProgname() -> *mut *const c_char {
    &raw mut __progname
}

#[unsafe(no_mangle)]
extern "C" fn _NSGetMachExecuteHeader() -> *const c_void {
    // SAFETY: libdyld's image table; image 0 is the program.
    unsafe { _dyld_get_image_header(0) }
}

#[unsafe(no_mangle)]
extern "C" fn getprogname() -> *const c_char {
    // SAFETY: libdyld's variable.
    unsafe { __progname }
}

#[unsafe(no_mangle)]
unsafe extern "C" fn setprogname(name: *const c_char) {
    // SAFETY: the C contract: a string that lives on.
    unsafe {
        let slash = cstr::strrchr(name as *const u8, i32::from(b'/'));
        __progname = if slash.is_null() { name } else { slash.add(1) as *const c_char };
    }
}

extern "C" fn init(_: i32, _: *const *const u8, _: *const *const u8, _: *const *const u8, _: *const u8) {
    let mut guard = 0u64;
    // SAFETY: 8 bytes of our own; initializers run before any other code
    // of the program.
    unsafe {
        if getentropy((&raw mut guard).cast(), 8) != 0 {
            // No entropy (yet): mix what differs from run to run.
            guard = (&raw const guard as u64).rotate_left(17)
                ^ (core::arch::x86_64::_rdtsc()).wrapping_mul(0x9e37_79b9_7f4a_7c15);
        }
        // A zero byte stops string functions from reading the canary.
        __stack_chk_guard = guard & !0xff;
    }
}

#[used]
#[unsafe(link_section = "__DATA,__mod_init_func,mod_init_funcs")]
static INIT: extern "C" fn(i32, *const *const u8, *const *const u8, *const *const u8, *const u8) = init;
