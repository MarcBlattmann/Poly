//! Process and thread state of the runtime: interior-mutable globals, the
//! locks, the per-thread data (`errno`, `_doserrno`, `strtok`'s context,
//! the `gmtime`/`asctime` buffers, `rand`'s seed), and the invalid
//! parameter handler (Learn: "Parameter validation",
//! `_set_invalid_parameter_handler`).

use crate::sys;
use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use poly_crt::time::Tm;

/// A global the runtime changes behind its own locks (or that only one
/// thread touches, at start-up and exit).
#[repr(transparent)]
pub struct Global<T>(UnsafeCell<T>);

// SAFETY: every access goes through `get`'s raw pointer under the rules the
// owning module documents (a lock, or start-up only).
unsafe impl<T> Sync for Global<T> {}

impl<T> Global<T> {
    pub const fn new(v: T) -> Self {
        Global(UnsafeCell::new(v))
    }
    pub fn get(&self) -> *mut T {
        self.0.get()
    }
}

/// An SRW lock (`kernel32`), for short, non-reentrant sections.
pub struct Lock(UnsafeCell<usize>);

// SAFETY: the kernel32 SRW lock is made for sharing between threads.
unsafe impl Sync for Lock {}

impl Lock {
    pub const fn new() -> Self {
        Lock(UnsafeCell::new(0))
    }
    pub fn lock(&self) -> LockGuard<'_> {
        // SAFETY: a valid SRW lock word (SRWLOCK_INIT is 0).
        unsafe { sys::AcquireSRWLockExclusive(self.0.get()) };
        LockGuard(self)
    }
}

impl Default for Lock {
    fn default() -> Self {
        Self::new()
    }
}

pub struct LockGuard<'a>(&'a Lock);

impl Drop for LockGuard<'_> {
    fn drop(&mut self) {
        // SAFETY: this guard holds the lock.
        unsafe { sys::ReleaseSRWLockExclusive(self.0.0.get()) };
    }
}

/// A `CRITICAL_SECTION` (40 bytes on x64, `winnt.h`), reentrant: what a
/// stream's lock must be (`_lock_file` then `fprintf` on one thread).
#[repr(C, align(8))]
pub struct CritSec(UnsafeCell<[u8; 40]>, AtomicU32);

// SAFETY: a critical section is made for sharing between threads.
unsafe impl Sync for CritSec {}

impl CritSec {
    pub const fn new() -> Self {
        CritSec(UnsafeCell::new([0; 40]), AtomicU32::new(0))
    }

    /// Initialise on first use (0 = not yet, 1 = being made, 2 = ready).
    fn ready(&self) -> *mut u8 {
        let p = self.0.get() as *mut u8;
        if self.1.load(Ordering::Acquire) != 2 {
            if self.1.compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire).is_ok() {
                // SAFETY: 40 writable bytes, initialised once.
                unsafe { sys::InitializeCriticalSection(p) };
                self.1.store(2, Ordering::Release);
            } else {
                while self.1.load(Ordering::Acquire) != 2 {
                    // SAFETY: no preconditions.
                    unsafe { sys::Sleep(0) };
                }
            }
        }
        p
    }

    pub fn enter(&self) {
        // SAFETY: an initialised critical section.
        unsafe { sys::EnterCriticalSection(self.ready()) }
    }

    pub fn leave(&self) {
        // SAFETY: as above; the caller entered it.
        unsafe { sys::LeaveCriticalSection(self.ready()) }
    }
}

impl Default for CritSec {
    fn default() -> Self {
        Self::new()
    }
}

// ----- per-thread data --------------------------------------------------------------

/// What each thread keeps.
pub struct Thread {
    pub errno: i32,
    pub doserrno: u32,
    pub strtok: *mut u8,
    pub wcstok: *mut u16,
    pub tm: Tm,
    pub asctime: [u8; 26],
    pub wasctime: [u16; 26],
    pub rand: u32,
    pub invalid_parameter: usize,
}

impl Thread {
    const fn new() -> Self {
        Thread {
            errno: 0,
            doserrno: 0,
            strtok: core::ptr::null_mut(),
            wcstok: core::ptr::null_mut(),
            tm: Tm { sec: 0, min: 0, hour: 0, mday: 0, mon: 0, year: 0, wday: 0, yday: 0, isdst: 0 },
            asctime: [0; 26],
            wasctime: [0; 26],
            rand: 1,
            invalid_parameter: 0,
        }
    }
}

/// `TLS_OUT_OF_INDEXES`.
const NO_INDEX: u32 = u32::MAX;
static TLS: AtomicU32 = AtomicU32::new(NO_INDEX);
/// The data of threads when no TLS index could be had (shared; better than
/// failing).
static FALLBACK: Global<Thread> = Global::new(Thread::new());

/// Called at `DLL_PROCESS_ATTACH`.
pub fn init_tls() {
    // SAFETY: no preconditions.
    let i = unsafe { sys::TlsAlloc() };
    TLS.store(i, Ordering::Release);
}

/// The calling thread's data, made on first use. The last error is kept:
/// `TlsGetValue` clears it, and a program may read it after a runtime call.
pub fn thread() -> &'static mut Thread {
    let index = TLS.load(Ordering::Acquire);
    if index == NO_INDEX {
        // SAFETY: the fallback (see FALLBACK).
        return unsafe { &mut *FALLBACK.get() };
    }
    // SAFETY: no preconditions.
    let saved = unsafe { sys::GetLastError() };
    // SAFETY: a valid index.
    let mut p = unsafe { sys::TlsGetValue(index) } as *mut Thread;
    if p.is_null() {
        p = alloc::boxed::Box::into_raw(alloc::boxed::Box::new(Thread::new()));
        // SAFETY: a valid index.
        unsafe { sys::TlsSetValue(index, p as *mut u8) };
    }
    // SAFETY: as above.
    unsafe { sys::SetLastError(saved) };
    // SAFETY: this thread's own allocation, freed only at its detach.
    unsafe { &mut *p }
}

/// `DLL_THREAD_DETACH`: free the thread's data.
pub fn thread_detach() {
    let index = TLS.load(Ordering::Acquire);
    if index == NO_INDEX {
        return;
    }
    // SAFETY: a valid index.
    let p = unsafe { sys::TlsGetValue(index) } as *mut Thread;
    if !p.is_null() {
        // SAFETY: made by `thread` with Box; nothing uses it after detach.
        drop(unsafe { alloc::boxed::Box::from_raw(p) });
        // SAFETY: a valid index.
        unsafe { sys::TlsSetValue(index, core::ptr::null_mut()) };
    }
}

/// Set `errno`.
pub fn set_errno(e: i32) {
    thread().errno = e;
}

/// Set `errno` and `_doserrno` from a failed Win32 call's last error.
pub fn set_errno_from_win32() {
    // SAFETY: no preconditions.
    let e = unsafe { sys::GetLastError() };
    let t = thread();
    t.doserrno = e;
    t.errno = poly_crt::errno::from_win32(e);
}

// ----- the invalid parameter handler ------------------------------------------------

/// `_invalid_parameter_handler`: `(expression, function, file, line,
/// reserved)`.
pub type InvalidParameterHandler = unsafe extern "C" fn(*const u16, *const u16, *const u16, u32, usize);

pub static INVALID_PARAMETER: AtomicUsize = AtomicUsize::new(0);

/// `FAST_FAIL_INVALID_ARG` (`winnt.h`).
const FAST_FAIL_INVALID_ARG: u32 = 5;

/// Report an invalid parameter: the thread's handler, else the process's,
/// else the default, which ends the process (Learn: "the default invalid
/// parameter handler ... terminates the application"). If a handler
/// returns, the function goes on to fail with its documented `errno`.
pub fn invalid_parameter() {
    let t = thread().invalid_parameter;
    let h = if t != 0 { t } else { INVALID_PARAMETER.load(Ordering::Acquire) };
    if h == 0 {
        poly_win32_rt::fail_fast(FAST_FAIL_INVALID_ARG);
    }
    // SAFETY: a handler the program registered, with the documented
    // signature.
    let f: InvalidParameterHandler = unsafe { core::mem::transmute(h) };
    // SAFETY: as above; the release runtime passes nulls and 0.
    unsafe { f(core::ptr::null(), core::ptr::null(), core::ptr::null(), 0, 0) };
}

/// Report an invalid parameter and set `errno` to `e`.
pub fn invalid(e: i32) {
    invalid_parameter();
    set_errno(e);
}
