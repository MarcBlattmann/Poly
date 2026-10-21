//! Start-up and termination (Learn: "CRT initialization", `_initterm`,
//! `exit`, `_exit`, `_cexit`, `atexit`, `_onexit`, `abort`, `signal`,
//! `raise`, `terminate`, `_purecall`, `_beginthreadex`; `corecrt_startup.h`
//! for `_onexit_table_t` and `_crt_argv_mode`).
//!
//! The program's arguments and environment are built when its start-up
//! code asks (`_configure_narrow_argv`, `_initialize_narrow_environment`,
//! `__getmainargs`), from `GetCommandLineA/W` and the environment block,
//! with [`poly_crt::args`]. Where they live is a set of cells: the
//! runtime's own variables, or, for `msvcrt.dll`, its exported data
//! (`__argc`, `__argv`, `_environ`, `_fmode` …), bound at its start
//! ([`bind`]).
//!
//! `exit` runs the thread-local destructor callback, then the `atexit`
//! functions last first, then writes every stream, then ends the process.
//! `abort` raises `SIGABRT` and ends with code 3 (Learn: "abort ...
//! terminates ... with exit code 3"); a `SIG_DFL` signal raised ends the
//! process with code 3 as well.

use crate::global::{self, Global, Lock};
use crate::heap;
use crate::stdio;
use crate::sys;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicI32, AtomicUsize, Ordering};
use poly_crt::errno::EINVAL;

/// Where the program-visible variables live.
#[derive(Clone, Copy)]
pub struct Cells {
    pub argc: *mut i32,
    pub argv: *mut *mut *mut u8,
    pub wargv: *mut *mut *mut u16,
    pub environ: *mut *mut *mut u8,
    pub wenviron: *mut *mut *mut u16,
    pub initenv: *mut *mut *mut u8,
    pub winitenv: *mut *mut *mut u16,
    pub acmdln: *mut *mut u8,
    pub wcmdln: *mut *mut u16,
    pub pgmptr: *mut *mut u8,
    pub wpgmptr: *mut *mut u16,
    pub fmode: *mut i32,
    pub commode: *mut i32,
}

/// The runtime's own variables (the UCRT's, reached through `__p_*`).
pub struct Own {
    pub argc: i32,
    pub argv: *mut *mut u8,
    pub wargv: *mut *mut u16,
    pub environ: *mut *mut u8,
    pub wenviron: *mut *mut u16,
    pub initenv: *mut *mut u8,
    pub winitenv: *mut *mut u16,
    pub acmdln: *mut u8,
    pub wcmdln: *mut u16,
    pub pgmptr: *mut u8,
    pub wpgmptr: *mut u16,
    pub fmode: i32,
    pub commode: i32,
}

static OWN: Global<Own> = Global::new(Own {
    argc: 0,
    argv: core::ptr::null_mut(),
    wargv: core::ptr::null_mut(),
    environ: core::ptr::null_mut(),
    wenviron: core::ptr::null_mut(),
    initenv: core::ptr::null_mut(),
    winitenv: core::ptr::null_mut(),
    acmdln: core::ptr::null_mut(),
    wcmdln: core::ptr::null_mut(),
    pgmptr: core::ptr::null_mut(),
    wpgmptr: core::ptr::null_mut(),
    fmode: poly_crt::fmode::O_TEXT,
    commode: 0,
});

static CELLS: Global<Option<Cells>> = Global::new(None);

/// Use `c` for the program-visible variables (`msvcrt.dll`, at attach).
pub fn bind(c: Cells) {
    // SAFETY: process attach, one thread.
    unsafe { *CELLS.get() = Some(c) };
}

/// The cells in use (the runtime's own unless [`bind`] chose others).
pub fn cells() -> Cells {
    // SAFETY: written only at attach.
    if let Some(c) = unsafe { *CELLS.get() } {
        return c;
    }
    let o = OWN.get();
    // SAFETY: field addresses of a static.
    let c = unsafe {
        Cells {
            argc: &raw mut (*o).argc,
            argv: &raw mut (*o).argv,
            wargv: &raw mut (*o).wargv,
            environ: &raw mut (*o).environ,
            wenviron: &raw mut (*o).wenviron,
            initenv: &raw mut (*o).initenv,
            winitenv: &raw mut (*o).winitenv,
            acmdln: &raw mut (*o).acmdln,
            wcmdln: &raw mut (*o).wcmdln,
            pgmptr: &raw mut (*o).pgmptr,
            wpgmptr: &raw mut (*o).wpgmptr,
            fmode: &raw mut (*o).fmode,
            commode: &raw mut (*o).commode,
        }
    };
    // SAFETY: as in `bind`; the first call is at attach.
    unsafe { *CELLS.get() = Some(c) };
    c
}

/// `_fmode`'s value (the default translation of new descriptors).
pub fn fmode() -> i32 {
    // SAFETY: a valid cell.
    unsafe { *cells().fmode }
}

/// Process start-up: the command lines and the program name.
pub fn init() {
    let c = cells();
    // SAFETY: valid cells; the command lines live as long as the process.
    unsafe {
        *c.acmdln = sys::GetCommandLineA() as *mut u8;
        *c.wcmdln = sys::GetCommandLineW() as *mut u16;
        let mut name = [0u16; 1024];
        let n = sys::GetModuleFileNameW(core::ptr::null_mut(), name.as_mut_ptr(), name.len() as u32) as usize;
        *c.wpgmptr = heap::dup(&name[..n]);
        *c.pgmptr = heap::dup(&crate::text::wide_to_ansi(&name[..n]));
    }
    // SAFETY: no preconditions.
    let mut t = 0i64;
    // SAFETY: `t` writable.
    unsafe { sys::QueryPerformanceCounter(&mut t) };
    START.store(t as usize, Ordering::Release);
}

/// The performance counter at start-up (`clock`).
pub static START: AtomicUsize = AtomicUsize::new(0);

/// A NULL-terminated array of heap copies.
fn array<T: Copy + Default>(items: &[&[T]]) -> *mut *mut T {
    let p = heap::malloc((items.len() + 1) * 8) as *mut *mut T;
    if p.is_null() {
        return p;
    }
    for (i, it) in items.iter().enumerate() {
        // SAFETY: the array holds items.len() + 1 pointers.
        unsafe { p.add(i).write(heap::dup(it)) };
    }
    // SAFETY: as above.
    unsafe { p.add(items.len()).write(core::ptr::null_mut()) };
    p
}

/// `_crt_argv_mode` (`corecrt_startup.h`).
const ARGV_NO_ARGUMENTS: i32 = 0;

/// `_configure_narrow_argv`: 0, or `EINVAL` for an unknown mode. Wildcard
/// expansion (`_crt_argv_expanded_arguments`) is not done (Poly passes the
/// patterns through; to come).
pub extern "C" fn _configure_narrow_argv(mode: i32) -> i32 {
    if !(0..=2).contains(&mode) {
        global::invalid(EINVAL);
        return EINVAL;
    }
    let c = cells();
    if mode == ARGV_NO_ARGUMENTS {
        return 0;
    }
    // SAFETY: the command line is a valid C string.
    let line = unsafe { crate::text::cstr(sys::GetCommandLineA()) };
    let args = poly_crt::args::parse(line);
    let refs: Vec<&[u8]> = args.iter().map(Vec::as_slice).collect();
    // SAFETY: valid cells.
    unsafe {
        *c.argv = array(&refs);
        *c.argc = refs.len() as i32;
    }
    0
}

pub extern "C" fn _configure_wide_argv(mode: i32) -> i32 {
    if !(0..=2).contains(&mode) {
        global::invalid(EINVAL);
        return EINVAL;
    }
    let c = cells();
    if mode == ARGV_NO_ARGUMENTS {
        return 0;
    }
    // SAFETY: the command line is a valid C string.
    let line = unsafe { crate::text::wstr(sys::GetCommandLineW()) };
    let args = poly_crt::args::parse(line);
    let refs: Vec<&[u16]> = args.iter().map(Vec::as_slice).collect();
    // SAFETY: valid cells.
    unsafe {
        *c.wargv = array(&refs);
        *c.argc = refs.len() as i32;
    }
    0
}

fn wide_block() -> Vec<u16> {
    // SAFETY: no preconditions.
    let p = unsafe { sys::GetEnvironmentStringsW() };
    if p.is_null() {
        return alloc::vec![0, 0];
    }
    let mut n = 0;
    // SAFETY: the block ends with two NULs.
    while unsafe { *p.add(n) != 0 || *p.add(n + 1) != 0 } {
        n += 1;
    }
    // SAFETY: n + 2 units are readable.
    let v = unsafe { core::slice::from_raw_parts(p, n + 2) }.to_vec();
    // SAFETY: the block from GetEnvironmentStringsW.
    unsafe { sys::FreeEnvironmentStringsW(p) };
    v
}

/// Build `_environ` (and `__initenv`) if not yet made.
pub extern "C" fn _initialize_narrow_environment() -> i32 {
    let c = cells();
    // SAFETY: valid cells.
    if !unsafe { *c.environ }.is_null() {
        return 0;
    }
    let block = wide_block();
    let entries: Vec<Vec<u8>> =
        poly_crt::args::environment(&block).iter().map(|e| crate::text::wide_to_ansi(e)).collect();
    let refs: Vec<&[u8]> = entries.iter().map(Vec::as_slice).collect();
    let env = array(&refs);
    // SAFETY: valid cells.
    unsafe {
        *c.environ = env;
        if (*c.initenv).is_null() {
            *c.initenv = env;
        }
    }
    0
}

pub extern "C" fn _initialize_wide_environment() -> i32 {
    let c = cells();
    // SAFETY: valid cells.
    if !unsafe { *c.wenviron }.is_null() {
        return 0;
    }
    let block = wide_block();
    let refs = poly_crt::args::environment(&block);
    let env = array(&refs);
    // SAFETY: valid cells.
    unsafe {
        *c.wenviron = env;
        if (*c.winitenv).is_null() {
            *c.winitenv = env;
        }
    }
    0
}

pub extern "C" fn _get_initial_narrow_environment() -> *mut *mut u8 {
    // SAFETY: a valid cell.
    unsafe { *cells().initenv }
}

pub extern "C" fn _get_initial_wide_environment() -> *mut *mut u16 {
    // SAFETY: a valid cell.
    unsafe { *cells().winitenv }
}

/// `__getmainargs` / `__wgetmainargs` (`msvcrt.dll`): the arguments and
/// environment as `main` gets them.
///
/// # Safety
/// Each out-pointer is NULL or writable.
pub unsafe fn main_args(argc: *mut i32, argv: *mut *mut *mut u8, env: *mut *mut *mut u8, wide: bool) -> i32 {
    let c = cells();
    if wide {
        _configure_wide_argv(1);
        _initialize_wide_environment();
    } else {
        _configure_narrow_argv(1);
        _initialize_narrow_environment();
    }
    // SAFETY: the caller's out-pointers (checked) and valid cells.
    unsafe {
        if !argc.is_null() {
            *argc = *c.argc;
        }
        if !argv.is_null() {
            *argv = if wide { *c.wargv as *mut *mut u8 } else { *c.argv };
        }
        if !env.is_null() {
            *env = if wide { *c.wenviron as *mut *mut u8 } else { *c.environ };
        }
    }
    0
}

// ----- initialisers -----------------------------------------------------------------

/// `_initterm`: call every non-null function in `[begin, end)`.
///
/// # Safety
/// The range holds function pointers or nulls.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _initterm(begin: *const usize, end: *const usize) {
    let mut p = begin;
    while p < end {
        // SAFETY: inside the caller's range.
        let f = unsafe { *p };
        if f != 0 {
            // SAFETY: a `void (__cdecl *)(void)` of the program.
            let f: unsafe extern "C" fn() = unsafe { core::mem::transmute(f) };
            // SAFETY: as above.
            unsafe { f() };
        }
        // SAFETY: still inside or one past the range.
        p = unsafe { p.add(1) };
    }
}

/// `_initterm_e`: as `_initterm` with `int (*)(void)`; the first non-zero
/// result stops it and is returned.
///
/// # Safety
/// As `_initterm`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _initterm_e(begin: *const usize, end: *const usize) -> i32 {
    let mut p = begin;
    while p < end {
        // SAFETY: inside the caller's range.
        let f = unsafe { *p };
        if f != 0 {
            // SAFETY: an `int (__cdecl *)(void)` of the program.
            let f: unsafe extern "C" fn() -> i32 = unsafe { core::mem::transmute(f) };
            // SAFETY: as above.
            let r = unsafe { f() };
            if r != 0 {
                return r;
            }
        }
        // SAFETY: as in `_initterm`.
        p = unsafe { p.add(1) };
    }
    0
}

// ----- atexit and onexit tables ---------------------------------------------------------

static ATEXIT: Global<Vec<usize>> = Global::new(Vec::new());
static QUICK: Global<Vec<usize>> = Global::new(Vec::new());
static EXIT_LOCK: Lock = Lock::new();
/// `_register_thread_local_exe_atexit_callback`'s callback.
static TLS_CALLBACK: AtomicUsize = AtomicUsize::new(0);

/// Register `f` to run at `exit` (`_crt_atexit`, `atexit`, `_onexit`).
pub fn atexit(f: usize) -> i32 {
    let _g = EXIT_LOCK.lock();
    // SAFETY: under EXIT_LOCK.
    unsafe { (*ATEXIT.get()).push(f) };
    0
}

pub extern "C" fn _crt_atexit(f: usize) -> i32 {
    atexit(f)
}

pub extern "C" fn _crt_at_quick_exit(f: usize) -> i32 {
    let _g = EXIT_LOCK.lock();
    // SAFETY: under EXIT_LOCK.
    unsafe { (*QUICK.get()).push(f) };
    0
}

pub extern "C" fn _register_thread_local_exe_atexit_callback(f: usize) {
    TLS_CALLBACK.store(f, Ordering::Release);
}

/// Run a table's functions, last first; functions registered meanwhile run
/// too.
fn run(table: &Global<Vec<usize>>) {
    loop {
        let f = {
            let _g = EXIT_LOCK.lock();
            // SAFETY: under EXIT_LOCK.
            unsafe { (*table.get()).pop() }
        };
        let Some(f) = f else { break };
        // SAFETY: a `void (__cdecl *)(void)` the program registered.
        let f: unsafe extern "C" fn() = unsafe { core::mem::transmute(f) };
        // SAFETY: as above.
        unsafe { f() };
    }
}

/// `_onexit_table_t` (`corecrt_startup.h`).
#[repr(C)]
pub struct OnExitTable {
    first: *mut usize,
    last: *mut usize,
    end: *mut usize,
}

/// # Safety
/// `t` is NULL or a table.
pub unsafe extern "C" fn _initialize_onexit_table(t: *mut OnExitTable) -> i32 {
    if t.is_null() {
        return -1;
    }
    // SAFETY: the caller's table.
    unsafe {
        if (*t).first.is_null() {
            (*t).last = core::ptr::null_mut();
            (*t).end = core::ptr::null_mut();
        }
    }
    0
}

/// # Safety
/// `t` is a table from `_initialize_onexit_table`.
pub unsafe extern "C" fn _register_onexit_function(t: *mut OnExitTable, f: usize) -> i32 {
    if t.is_null() {
        return -1;
    }
    let _g = EXIT_LOCK.lock();
    // SAFETY: the caller's table; its arrays come from this heap.
    unsafe {
        let t = &mut *t;
        if t.last == t.end {
            let len = if t.first.is_null() { 0 } else { t.last.offset_from(t.first) as usize };
            let cap = (len * 2).max(8);
            let new = heap::realloc(t.first as *mut u8, cap * 8) as *mut usize;
            if new.is_null() {
                return -1;
            }
            t.first = new;
            t.last = new.add(len);
            t.end = new.add(cap);
        }
        t.last.write(f);
        t.last = t.last.add(1);
    }
    0
}

/// # Safety
/// As `_register_onexit_function`.
pub unsafe extern "C" fn _execute_onexit_table(t: *mut OnExitTable) -> i32 {
    if t.is_null() {
        return -1;
    }
    let (first, last) = {
        let _g = EXIT_LOCK.lock();
        // SAFETY: the caller's table, emptied before its functions run.
        unsafe {
            let t = &mut *t;
            let r = (t.first, t.last);
            t.first = core::ptr::null_mut();
            t.last = core::ptr::null_mut();
            t.end = core::ptr::null_mut();
            r
        }
    };
    let mut p = last;
    while !first.is_null() && p > first {
        // SAFETY: inside the table's array.
        unsafe {
            p = p.sub(1);
            let f: unsafe extern "C" fn() = core::mem::transmute(*p);
            f();
        }
    }
    // SAFETY: the array came from this heap.
    unsafe { heap::free(first as *mut u8) };
    0
}

/// `DLL_PROCESS_DETACH` (`winnt.h`).
const DLL_PROCESS_DETACH: u32 = 0;

/// What `exit` and `_cexit` do before the process ends.
fn cleanup() {
    let cb = TLS_CALLBACK.swap(0, Ordering::AcqRel);
    if cb != 0 {
        // SAFETY: a PIMAGE_TLS_CALLBACK the program registered.
        let f: unsafe extern "system" fn(*mut u8, u32, *mut u8) = unsafe { core::mem::transmute(cb) };
        // SAFETY: as above.
        unsafe { f(core::ptr::null_mut(), DLL_PROCESS_DETACH, core::ptr::null_mut()) };
    }
    run(&ATEXIT);
    stdio::flush_all();
}

#[unsafe(no_mangle)]
pub extern "C" fn exit(code: i32) -> ! {
    cleanup();
    // SAFETY: no preconditions.
    unsafe { sys::ExitProcess(code as u32) }
}

#[unsafe(no_mangle)]
pub extern "C" fn _exit(code: i32) -> ! {
    // SAFETY: no preconditions.
    unsafe { sys::ExitProcess(code as u32) }
}

#[allow(non_snake_case)]
pub extern "C" fn _Exit(code: i32) -> ! {
    _exit(code)
}

pub extern "C" fn quick_exit(code: i32) -> ! {
    run(&QUICK);
    _exit(code)
}

#[unsafe(no_mangle)]
pub extern "C" fn _cexit() {
    cleanup();
}

#[unsafe(no_mangle)]
pub extern "C" fn _c_exit() {}

// ----- signals, abort, terminate ----------------------------------------------------------

pub const SIGINT: i32 = 2;
pub const SIGILL: i32 = 4;
pub const SIGABRT_COMPAT: i32 = 6;
pub const SIGFPE: i32 = 8;
pub const SIGSEGV: i32 = 11;
pub const SIGTERM: i32 = 15;
pub const SIGBREAK: i32 = 21;
pub const SIGABRT: i32 = 22;
const SIG_DFL: usize = 0;
const SIG_IGN: usize = 1;
const SIG_ERR: usize = usize::MAX;

static SIGNALS: [AtomicUsize; 23] = [const { AtomicUsize::new(SIG_DFL) }; 23];
static CTRL_HANDLER: AtomicI32 = AtomicI32::new(0);

fn slot(sig: i32) -> Option<&'static AtomicUsize> {
    match sig {
        SIGINT | SIGILL | SIGFPE | SIGSEGV | SIGTERM | SIGBREAK | SIGABRT => Some(&SIGNALS[sig as usize]),
        SIGABRT_COMPAT => Some(&SIGNALS[SIGABRT as usize]),
        _ => None,
    }
}

unsafe extern "system" fn console_ctrl(event: u32) -> i32 {
    let sig = match event {
        0 => SIGINT,
        1 => SIGBREAK,
        _ => return 0,
    };
    let Some(s) = slot(sig) else { return 0 };
    match s.load(Ordering::Acquire) {
        SIG_DFL => 0,
        SIG_IGN => 1,
        h => {
            s.store(SIG_DFL, Ordering::Release);
            // SAFETY: a `void (__cdecl *)(int)` the program installed.
            let f: unsafe extern "C" fn(i32) = unsafe { core::mem::transmute(h) };
            // SAFETY: as above.
            unsafe { f(sig) };
            1
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn signal(sig: i32, handler: usize) -> usize {
    let Some(s) = slot(sig) else {
        global::invalid(EINVAL);
        return SIG_ERR;
    };
    if handler == SIG_ERR || (2..=4).contains(&handler) {
        global::invalid(EINVAL);
        return SIG_ERR;
    }
    if (sig == SIGINT || sig == SIGBREAK) && CTRL_HANDLER.swap(1, Ordering::AcqRel) == 0 {
        // SAFETY: a valid handler routine.
        unsafe { sys::SetConsoleCtrlHandler(Some(console_ctrl), 1) };
    }
    s.swap(handler, Ordering::AcqRel)
}

#[unsafe(no_mangle)]
pub extern "C" fn raise(sig: i32) -> i32 {
    let Some(s) = slot(sig) else {
        global::invalid(EINVAL);
        return -1;
    };
    match s.load(Ordering::Acquire) {
        SIG_IGN => 0,
        SIG_DFL => _exit(3),
        h => {
            if sig != SIGFPE {
                s.store(SIG_DFL, Ordering::Release);
            }
            // SAFETY: a `void (__cdecl *)(int)` the program installed.
            let f: unsafe extern "C" fn(i32) = unsafe { core::mem::transmute(h) };
            // SAFETY: as above.
            unsafe { f(sig) };
            0
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn abort() -> ! {
    if slot(SIGABRT).is_some_and(|s| s.load(Ordering::Acquire) != SIG_DFL) {
        raise(SIGABRT);
    }
    _exit(3)
}

/// `_set_abort_behavior`'s flags (kept, no effect: Poly never reports).
static ABORT_FLAGS: AtomicUsize = AtomicUsize::new(1);

pub extern "C" fn _set_abort_behavior(flags: u32, mask: u32) -> u32 {
    let old = ABORT_FLAGS.load(Ordering::Acquire) as u32;
    ABORT_FLAGS.store(((old & !mask) | (flags & mask)) as usize, Ordering::Release);
    old
}

/// The terminate handler (`set_terminate`).
pub static TERMINATE: AtomicUsize = AtomicUsize::new(0);
/// The pure-call handler (`_set_purecall_handler`).
pub static PURECALL: AtomicUsize = AtomicUsize::new(0);

pub extern "C" fn terminate() -> ! {
    let h = TERMINATE.load(Ordering::Acquire);
    if h != 0 {
        // SAFETY: a `void (__cdecl *)(void)` the program installed.
        let f: unsafe extern "C" fn() = unsafe { core::mem::transmute(h) };
        // SAFETY: as above.
        unsafe { f() };
    }
    abort()
}

#[unsafe(no_mangle)]
pub extern "C" fn _purecall() -> i32 {
    let h = PURECALL.load(Ordering::Acquire);
    if h != 0 {
        // SAFETY: a `void (__cdecl *)(void)` the program installed.
        let f: unsafe extern "C" fn() = unsafe { core::mem::transmute(h) };
        // SAFETY: as above.
        unsafe { f() };
    }
    abort()
}

/// `_seh_filter_exe` / `_XcptFilter`: an exception that maps to a signal
/// with a handler runs it and continues execution (−1); otherwise the
/// search goes on (0). Codes from `ntstatus.h`.
pub fn seh_filter(code: u32) -> i32 {
    let sig = match code {
        0xC000_0005 => SIGSEGV,
        0xC000_001D | 0xC000_0096 => SIGILL,
        0xC000_008D..=0xC000_0093 | 0xC000_02B4 | 0xC000_02B5 => SIGFPE,
        _ => return 0,
    };
    let Some(s) = slot(sig) else { return 0 };
    match s.load(Ordering::Acquire) {
        SIG_DFL => 0,
        SIG_IGN => -1,
        h => {
            if sig != SIGFPE {
                s.store(SIG_DFL, Ordering::Release);
            }
            // SAFETY: a `void (__cdecl *)(int)` the program installed.
            let f: unsafe extern "C" fn(i32) = unsafe { core::mem::transmute(h) };
            // SAFETY: as above.
            unsafe { f(sig) };
            -1
        }
    }
}

// ----- threads -------------------------------------------------------------------------

/// `_beginthreadex`: `CreateThread` (the runtime's per-thread data is made
/// on first use and freed at `DLL_THREAD_DETACH`); 0 on failure with
/// `errno` set.
///
/// # Safety
/// `start` is a thread procedure.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _beginthreadex(
    security: *const u8,
    stack: u32,
    start: Option<unsafe extern "system" fn(*mut u8) -> u32>,
    arg: *mut u8,
    flags: u32,
    id: *mut u32,
) -> usize {
    let Some(start) = start else {
        global::invalid(EINVAL);
        return 0;
    };
    // SAFETY: the caller's arguments.
    let h = unsafe { sys::CreateThread(security, stack as usize, start, arg, flags, id) };
    if h == 0 {
        global::set_errno_from_win32();
        return 0;
    }
    h as usize
}

struct Begin {
    start: unsafe extern "C" fn(*mut u8),
    arg: *mut u8,
}

unsafe extern "system" fn begin_thread(p: *mut u8) -> u32 {
    // SAFETY: the Begin `_beginthread` boxed for this thread.
    let b = unsafe { alloc::boxed::Box::from_raw(p as *mut Begin) };
    // SAFETY: the program's thread function.
    unsafe { (b.start)(b.arg) };
    0
}

/// `_beginthread`: −1 on failure. (The handle is not closed when the thread
/// ends, as Learn says it would be: Poly leaves it to the process's end.)
///
/// # Safety
/// `start` is a thread function.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _beginthread(start: Option<unsafe extern "C" fn(*mut u8)>, stack: u32, arg: *mut u8) -> usize {
    let Some(start) = start else {
        global::invalid(EINVAL);
        return usize::MAX;
    };
    let b = alloc::boxed::Box::into_raw(alloc::boxed::Box::new(Begin { start, arg }));
    let mut id = 0u32;
    // SAFETY: a valid thread procedure and parameter.
    let h = unsafe { sys::CreateThread(core::ptr::null(), stack as usize, begin_thread, b as *mut u8, 0, &mut id) };
    if h == 0 {
        // SAFETY: not handed to a thread.
        drop(unsafe { alloc::boxed::Box::from_raw(b) });
        global::set_errno_from_win32();
        return usize::MAX;
    }
    h as usize
}

#[unsafe(no_mangle)]
pub extern "C" fn _endthreadex(code: u32) -> ! {
    // SAFETY: no preconditions.
    unsafe { sys::ExitThread(code) }
}

#[unsafe(no_mangle)]
pub extern "C" fn _endthread() -> ! {
    _endthreadex(0)
}

#[unsafe(no_mangle)]
pub extern "C" fn __threadid() -> u32 {
    // SAFETY: no preconditions.
    unsafe { sys::GetCurrentThreadId() }
}

#[unsafe(no_mangle)]
pub extern "C" fn _getpid() -> i32 {
    // SAFETY: no preconditions.
    unsafe { sys::GetCurrentProcessId() as i32 }
}

/// `_get_pgmptr(&p)`: the program's path (narrow).
///
/// # Safety
/// `out` writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _get_pgmptr(out: *mut *mut u8) -> i32 {
    if out.is_null() {
        global::invalid(EINVAL);
        return EINVAL;
    }
    // SAFETY: writable; a valid cell.
    unsafe { *out = *cells().pgmptr };
    0
}

/// # Safety
/// `out` writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _get_wpgmptr(out: *mut *mut u16) -> i32 {
    if out.is_null() {
        global::invalid(EINVAL);
        return EINVAL;
    }
    // SAFETY: writable; a valid cell.
    unsafe { *out = *cells().wpgmptr };
    0
}
