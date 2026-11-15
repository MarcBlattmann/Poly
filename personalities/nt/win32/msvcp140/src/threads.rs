//! Time, mutexes, condition variables, threads and `std::_Lockit` (`_Query_perf_counter.yaml`,
//! `_Xtime_get_ticks.yaml`, `_Mtx_lock.yaml`, `_Cnd_wait.yaml`, `_Thrd_create.yaml`,
//! `??0_Lockit@std@@QEAA@H@Z.yaml`), on `kernel32`'s performance counter, system time, SRW
//! locks, condition variables and threads (Learn). The decisions are `poly_msvcp::mutex`'s and
//! `poly_msvcp::time`'s.

use alloc::boxed::Box;
use core::alloc::Layout;
use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicI32, Ordering};
use poly_msvcp::mutex::{self, NO_OWNER, Step, ThrdResult};
use poly_msvcp::time;

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn QueryPerformanceCounter(value: *mut i64) -> i32;
    fn QueryPerformanceFrequency(value: *mut i64) -> i32;
    fn GetSystemTimePreciseAsFileTime(time: *mut u64);
    fn GetCurrentThreadId() -> u32;
    fn GetCurrentThread() -> *mut u8;
    fn AcquireSRWLockExclusive(lock: *mut usize);
    fn TryAcquireSRWLockExclusive(lock: *mut usize) -> u8;
    fn ReleaseSRWLockExclusive(lock: *mut usize);
    fn SleepConditionVariableSRW(cv: *mut usize, lock: *mut usize, ms: u32, flags: u32) -> i32;
    fn WakeConditionVariable(cv: *mut usize);
    fn WakeAllConditionVariable(cv: *mut usize);
    fn GetLastError() -> u32;
    fn CreateThread(
        security: *mut u8,
        stack: usize,
        start: unsafe extern "system" fn(*mut u8) -> u32,
        param: *mut u8,
        flags: u32,
        id: *mut u32,
    ) -> *mut u8;
    fn WaitForSingleObject(handle: *mut u8, ms: u32) -> u32;
    fn GetExitCodeThread(handle: *mut u8, code: *mut u32) -> i32;
    fn CloseHandle(handle: *mut u8) -> i32;
    fn SwitchToThread() -> i32;
    fn Sleep(ms: u32);
    fn GetSystemInfo(info: *mut u8);
}

/// `INFINITE` (`winbase.h`).
const INFINITE: u32 = 0xFFFF_FFFF;
/// `WAIT_OBJECT_0` (`winbase.h`).
const WAIT_OBJECT_0: u32 = 0;
/// `ERROR_TIMEOUT` (`winerror.h`).
const ERROR_TIMEOUT: u32 = 1460;
/// `SYSTEM_INFO` (`sysinfoapi.h`, x64): 48 bytes, `dwNumberOfProcessors` at 32.
const SYSTEM_INFO_SIZE: usize = 48;
const SYSTEM_INFO_PROCESSORS: usize = 32;

fn me() -> i32 {
    // SAFETY: takes nothing.
    unsafe { GetCurrentThreadId() as i32 }
}

// ----- time ------------------------------------------------------------------------------------

/// `long long _Query_perf_counter()`: `QueryPerformanceCounter`.
#[unsafe(no_mangle)]
pub extern "C" fn _Query_perf_counter() -> i64 {
    let mut v = 0;
    // SAFETY: a local to write.
    unsafe { QueryPerformanceCounter(&mut v) };
    v
}

/// `long long _Query_perf_frequency()`: `QueryPerformanceFrequency` (never 0).
#[unsafe(no_mangle)]
pub extern "C" fn _Query_perf_frequency() -> i64 {
    let mut v = 0;
    // SAFETY: a local to write.
    unsafe { QueryPerformanceFrequency(&mut v) };
    v.max(1)
}

/// `long long _Xtime_get_ticks()`: the system time in 100 ns since 1970.
#[unsafe(no_mangle)]
pub extern "C" fn _Xtime_get_ticks() -> i64 {
    let mut ft = 0;
    // SAFETY: a local to write.
    unsafe { GetSystemTimePreciseAsFileTime(&mut ft) };
    time::ticks_from_filetime(ft)
}

/// `xtime { __time64_t sec; long nsec; }`.
#[repr(C)]
pub struct Xtime {
    sec: i64,
    nsec: i32,
}

/// The milliseconds until an absolute `xtime` (0 for null or the past).
///
/// # Safety
/// `xt` is null or an `xtime`.
unsafe fn ms_until(xt: *const Xtime) -> u32 {
    if xt.is_null() {
        return 0;
    }
    // SAFETY: the caller's xtime.
    let until = unsafe { time::xtime_ticks((*xt).sec, (*xt).nsec) };
    time::ms_until(until, _Xtime_get_ticks())
}

// ----- mutexes -----------------------------------------------------------------------------

/// `_Mtx_internal_imp_t` (`poly_msvcp::mutex` has the offsets).
#[repr(C)]
pub struct Mtx {
    ty: i32,
    _pad: i32,
    _slot: usize,
    srw: UnsafeCell<usize>,
    _rest: [u8; 48],
    owner: AtomicI32,
    count: UnsafeCell<i32>,
}

const _: () = assert!(core::mem::size_of::<Mtx>() == mutex::SIZE);
const _: () = assert!(core::mem::offset_of!(Mtx, srw) == mutex::OFFSET_SRWLOCK);
const _: () = assert!(core::mem::offset_of!(Mtx, owner) == mutex::OFFSET_THREAD_ID);
const _: () = assert!(core::mem::offset_of!(Mtx, count) == mutex::OFFSET_COUNT);

// SAFETY: the SRW lock serialises the owner's fields; `owner` is atomic.
unsafe impl Sync for Mtx {}

impl Mtx {
    const fn new(ty: i32) -> Mtx {
        Mtx {
            ty,
            _pad: 0,
            _slot: 0,
            srw: UnsafeCell::new(0),
            _rest: [0; 48],
            owner: AtomicI32::new(NO_OWNER),
            count: UnsafeCell::new(0),
        }
    }

    fn srw(&self) -> *mut usize {
        self.srw.get()
    }

    /// The lock was just acquired by the calling thread.
    fn took(&self) -> ThrdResult {
        self.owner.store(me(), Ordering::Release);
        // SAFETY: the caller holds the lock.
        unsafe { *self.count.get() = 1 };
        ThrdResult::Success
    }

    /// Carry out a step of `poly_msvcp::mutex` (a [`Step::Try`] on a taken lock is `_Busy`).
    fn run(&self, step: Step) -> ThrdResult {
        match step {
            Step::Recurse => {
                // SAFETY: the caller owns the lock.
                unsafe { *self.count.get() += 1 };
                ThrdResult::Success
            }
            Step::Busy => ThrdResult::Busy,
            Step::Wait => {
                // SAFETY: the mutex's SRW lock.
                unsafe { AcquireSRWLockExclusive(self.srw()) };
                self.took()
            }
            Step::Try => {
                // SAFETY: the mutex's SRW lock.
                if unsafe { TryAcquireSRWLockExclusive(self.srw()) } != 0 { self.took() } else { ThrdResult::Busy }
            }
        }
    }

    fn lock(&self) -> ThrdResult {
        self.run(mutex::lock_step(self.ty, self.owner.load(Ordering::Acquire), me()))
    }

    fn trylock(&self) -> ThrdResult {
        self.run(mutex::trylock_step(self.ty, self.owner.load(Ordering::Acquire), me()))
    }

    fn unlock(&self) -> ThrdResult {
        // SAFETY: only the owner unlocks.
        let count = unsafe { *self.count.get() };
        let Some((count, release)) = mutex::unlock_step(count) else { return ThrdResult::Error };
        // SAFETY: as above.
        unsafe { *self.count.get() = count };
        if release {
            self.owner.store(NO_OWNER, Ordering::Release);
            // SAFETY: the owner's SRW lock.
            unsafe { ReleaseSRWLockExclusive(self.srw()) };
        }
        ThrdResult::Success
    }

    fn owned(&self) -> bool {
        // SAFETY: the count is only meaningful to the owner, which this checks first.
        self.owner.load(Ordering::Acquire) == me() && unsafe { *self.count.get() } > 0
    }
}

/// The mutex a program passes (`_Mtx_t`).
///
/// # Safety
/// `m` is a `_Mtx_internal_imp_t` made by `_Mtx_init` or `_Mtx_init_in_situ`.
unsafe fn mtx<'a>(m: *mut Mtx) -> &'a Mtx {
    // SAFETY: the caller's mutex.
    unsafe { &*m }
}

fn mtx_layout() -> Layout {
    Layout::new::<Mtx>()
}

/// `_Thrd_result _Mtx_init_in_situ(_Mtx_t, int type)`: set up caller storage.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _Mtx_init_in_situ(m: *mut Mtx, ty: i32) -> i32 {
    // SAFETY: 80 bytes of the caller's.
    unsafe { m.write(Mtx::new(ty)) };
    ThrdResult::Success as i32
}

/// `void _Mtx_destroy_in_situ(_Mtx_t)`: nothing to give back.
#[unsafe(no_mangle)]
pub extern "C" fn _Mtx_destroy_in_situ(_m: *mut Mtx) {}

/// `_Thrd_result _Mtx_init(_Mtx_t*, int type)`: allocate a mutex nobody owns.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _Mtx_init(out: *mut *mut Mtx, ty: i32) -> i32 {
    // SAFETY: a non-zero size.
    let m = unsafe { alloc::alloc::alloc_zeroed(mtx_layout()) }.cast::<Mtx>();
    // SAFETY: the caller's pointer.
    unsafe { out.write(m) };
    if m.is_null() {
        return ThrdResult::Nomem as i32;
    }
    // SAFETY: the block just allocated.
    unsafe { _Mtx_init_in_situ(m, ty) }
}

/// `void _Mtx_destroy(_Mtx_t)`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _Mtx_destroy(m: *mut Mtx) {
    if !m.is_null() {
        // SAFETY: a block of `_Mtx_init`.
        unsafe { alloc::alloc::dealloc(m.cast(), mtx_layout()) };
    }
}

/// `_Thrd_result _Mtx_lock(_Mtx_t)`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _Mtx_lock(m: *mut Mtx) -> i32 {
    // SAFETY: the caller's mutex.
    unsafe { mtx(m) }.lock() as i32
}

/// `_Thrd_result _Mtx_trylock(_Mtx_t)`: blocks for a `_Mtx_plain` mutex without `_Mtx_try`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _Mtx_trylock(m: *mut Mtx) -> i32 {
    // SAFETY: the caller's mutex.
    unsafe { mtx(m) }.trylock() as i32
}

/// `_Thrd_result _Mtx_timedlock(_Mtx_t, const xtime*)`: try until the absolute time passes
/// (not in a sheet; the C11 `mtx_timedlock` contract).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _Mtx_timedlock(m: *mut Mtx, xt: *const Xtime) -> i32 {
    // SAFETY: the caller's mutex.
    let m = unsafe { mtx(m) };
    loop {
        let step = match mutex::trylock_step(m.ty, m.owner.load(Ordering::Acquire), me()) {
            Step::Wait => Step::Try,
            s => s,
        };
        match m.run(step) {
            ThrdResult::Busy if step == Step::Try => {}
            ThrdResult::Busy => return ThrdResult::Timedout as i32,
            r => return r as i32,
        }
        // SAFETY: the caller's xtime.
        if unsafe { ms_until(xt) } == 0 {
            return ThrdResult::Timedout as i32;
        }
        // SAFETY: takes a number.
        unsafe { Sleep(1) };
    }
}

/// `_Thrd_result _Mtx_unlock(_Mtx_t)`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _Mtx_unlock(m: *mut Mtx) -> i32 {
    // SAFETY: the caller's mutex.
    unsafe { mtx(m) }.unlock() as i32
}

/// `int _Mtx_current_owns(_Mtx_t)`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _Mtx_current_owns(m: *mut Mtx) -> i32 {
    // SAFETY: the caller's mutex.
    i32::from(unsafe { mtx(m) }.owned())
}

// ----- condition variables ----------------------------------------------------------------------

/// `_Cnd_internal_imp_t`: a slot, the `CONDITION_VARIABLE` at 8; 72 bytes.
#[repr(C)]
pub struct Cnd {
    _slot: usize,
    cv: UnsafeCell<usize>,
    _rest: [u8; 56],
}

const _: () = assert!(core::mem::size_of::<Cnd>() == mutex::CND_SIZE);
const _: () = assert!(core::mem::offset_of!(Cnd, cv) == mutex::CND_OFFSET_CV);

/// `_Thrd_result _Cnd_init_in_situ(_Cnd_t)`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _Cnd_init_in_situ(c: *mut Cnd) -> i32 {
    // SAFETY: 72 bytes of the caller's.
    unsafe { c.write(Cnd { _slot: 0, cv: UnsafeCell::new(0), _rest: [0; 56] }) };
    ThrdResult::Success as i32
}

/// `void _Cnd_destroy_in_situ(_Cnd_t)`.
#[unsafe(no_mangle)]
pub extern "C" fn _Cnd_destroy_in_situ(_c: *mut Cnd) {}

/// `_Thrd_result _Cnd_init(_Cnd_t*)`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _Cnd_init(out: *mut *mut Cnd) -> i32 {
    // SAFETY: a non-zero size.
    let c = unsafe { alloc::alloc::alloc_zeroed(Layout::new::<Cnd>()) }.cast::<Cnd>();
    // SAFETY: the caller's pointer.
    unsafe { out.write(c) };
    if c.is_null() { ThrdResult::Nomem as i32 } else { ThrdResult::Success as i32 }
}

/// `void _Cnd_destroy(_Cnd_t)`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _Cnd_destroy(c: *mut Cnd) {
    if !c.is_null() {
        // SAFETY: a block of `_Cnd_init`.
        unsafe { alloc::alloc::dealloc(c.cast(), Layout::new::<Cnd>()) };
    }
}

/// Wait on `c`, releasing `m` meanwhile and owning it again (with the same count) after.
///
/// # Safety
/// The caller's condition variable and the mutex it owns.
unsafe fn wait(c: *mut Cnd, m: *mut Mtx, ms: u32) -> ThrdResult {
    // SAFETY: the caller's objects.
    let (cv, m) = unsafe { ((*c).cv.get(), mtx(m)) };
    // SAFETY: the owner's count.
    let count = unsafe { *m.count.get() };
    m.owner.store(NO_OWNER, Ordering::Release);
    // SAFETY: the owner's count, then the SRW lock it holds.
    let ok = unsafe {
        *m.count.get() = 0;
        SleepConditionVariableSRW(cv, m.srw(), ms, 0)
    };
    // SAFETY: as before the wait, the lock held again.
    let error = if ok == 0 { unsafe { GetLastError() } } else { 0 };
    m.owner.store(me(), Ordering::Release);
    // SAFETY: the lock is held again.
    unsafe { *m.count.get() = count };
    match (ok, error) {
        (0, ERROR_TIMEOUT) => ThrdResult::Timedout,
        (0, _) => ThrdResult::Error,
        _ => ThrdResult::Success,
    }
}

/// `_Thrd_result _Cnd_wait(_Cnd_t, _Mtx_t)`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _Cnd_wait(c: *mut Cnd, m: *mut Mtx) -> i32 {
    // SAFETY: the caller's objects.
    unsafe { wait(c, m, INFINITE) as i32 }
}

/// `_Thrd_result _Cnd_timedwait(_Cnd_t, _Mtx_t, const xtime*)`: until the absolute time
/// (not in a sheet; the C11 `cnd_timedwait` contract).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _Cnd_timedwait(c: *mut Cnd, m: *mut Mtx, xt: *const Xtime) -> i32 {
    // SAFETY: the caller's objects.
    unsafe { wait(c, m, ms_until(xt)) as i32 }
}

/// `_Thrd_result _Cnd_signal(_Cnd_t)`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _Cnd_signal(c: *mut Cnd) -> i32 {
    // SAFETY: the caller's condition variable.
    unsafe { WakeConditionVariable((*c).cv.get()) };
    ThrdResult::Success as i32
}

/// `_Thrd_result _Cnd_broadcast(_Cnd_t)`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _Cnd_broadcast(c: *mut Cnd) -> i32 {
    // SAFETY: the caller's condition variable.
    unsafe { WakeAllConditionVariable((*c).cv.get()) };
    ThrdResult::Success as i32
}

// ----- threads -----------------------------------------------------------------------------------

/// `_Thrd_t { void* _Hnd; unsigned _Id; }` (16 bytes; by reference and through a hidden
/// pointer in the x64 convention, as Rust passes it too).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Thrd {
    handle: *mut u8,
    id: u32,
}

type Start = unsafe extern "C" fn(*mut u8) -> i32;

/// The new thread: run the start function; its `int` is the thread's exit code.
unsafe extern "system" fn thread_start(param: *mut u8) -> u32 {
    // SAFETY: the box `_Thrd_create` leaked for this thread.
    let start = unsafe { Box::from_raw(param.cast::<(Start, *mut u8)>()) };
    // SAFETY: the program's start function and argument.
    unsafe { (start.0)(start.1) as u32 }
}

/// `_Thrd_result _Thrd_create(_Thrd_t*, int (*)(void*), void*)`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _Thrd_create(thr: *mut Thrd, func: Option<Start>, arg: *mut u8) -> i32 {
    let Some(func) = func else { return ThrdResult::Error as i32 };
    let param = Box::into_raw(Box::new((func, arg))).cast::<u8>();
    let mut id = 0;
    // SAFETY: a start routine of the right type and its parameter.
    let handle = unsafe { CreateThread(core::ptr::null_mut(), 0, thread_start, param, 0, &mut id) };
    if handle.is_null() {
        // SAFETY: the box made above, which no thread took.
        drop(unsafe { Box::from_raw(param.cast::<(Start, *mut u8)>()) });
        return ThrdResult::Error as i32;
    }
    if !thr.is_null() {
        // SAFETY: the caller's `_Thrd_t`.
        unsafe { thr.write(Thrd { handle, id }) };
    }
    ThrdResult::Success as i32
}

/// `_Thrd_result _Thrd_join(_Thrd_t, int*)`: wait, hand back the start function's result,
/// close the handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _Thrd_join(thr: Thrd, result: *mut i32) -> i32 {
    // SAFETY: the thread's handle.
    if unsafe { WaitForSingleObject(thr.handle, INFINITE) } != WAIT_OBJECT_0 {
        return ThrdResult::Error as i32;
    }
    let mut code = 0;
    // SAFETY: as above, and a local.
    unsafe { GetExitCodeThread(thr.handle, &mut code) };
    if !result.is_null() {
        // SAFETY: the caller's int.
        unsafe { result.write(code as i32) };
    }
    // SAFETY: the handle, closed once.
    unsafe { CloseHandle(thr.handle) };
    ThrdResult::Success as i32
}

/// `_Thrd_result _Thrd_detach(_Thrd_t)`: the thread runs on; its handle is closed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _Thrd_detach(thr: Thrd) -> i32 {
    // SAFETY: the thread's handle.
    if unsafe { CloseHandle(thr.handle) } != 0 { ThrdResult::Success as i32 } else { ThrdResult::Error as i32 }
}

/// `_Thrd_t _Thrd_current()`: this thread's pseudo handle and id.
#[unsafe(no_mangle)]
pub extern "C" fn _Thrd_current() -> Thrd {
    // SAFETY: takes nothing.
    Thrd { handle: unsafe { GetCurrentThread() }, id: me() as u32 }
}

/// `int _Thrd_equal(_Thrd_t, _Thrd_t)`: the same thread id.
#[unsafe(no_mangle)]
pub extern "C" fn _Thrd_equal(a: Thrd, b: Thrd) -> i32 {
    i32::from(a.id == b.id)
}

/// `unsigned _Thrd_id()`.
#[unsafe(no_mangle)]
pub extern "C" fn _Thrd_id() -> u32 {
    me() as u32
}

/// `void _Thrd_yield()`.
#[unsafe(no_mangle)]
pub extern "C" fn _Thrd_yield() {
    // SAFETY: takes nothing.
    unsafe { SwitchToThread() };
}

/// `void _Thrd_sleep(const xtime*)`: until the system clock reaches the absolute time.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _Thrd_sleep(xt: *const Xtime) {
    loop {
        // SAFETY: the caller's xtime.
        let ms = unsafe { ms_until(xt) };
        if ms == 0 {
            return;
        }
        // SAFETY: takes a number.
        unsafe { Sleep(ms) };
    }
}

/// `unsigned _Thrd_hardware_concurrency()`: the processors `GetSystemInfo` reports (at least 1).
#[unsafe(no_mangle)]
pub extern "C" fn _Thrd_hardware_concurrency() -> u32 {
    let mut info = [0u8; SYSTEM_INFO_SIZE];
    // SAFETY: a SYSTEM_INFO-sized buffer.
    unsafe { GetSystemInfo(info.as_mut_ptr()) };
    let n = &info[SYSTEM_INFO_PROCESSORS..SYSTEM_INFO_PROCESSORS + 4];
    u32::from_le_bytes([n[0], n[1], n[2], n[3]]).max(1)
}

// ----- std::_Lockit ---------------------------------------------------------------------------

/// The `_Lockit` locks: recursive, one per lock number (0 is `_LOCK_LOCALE`); a number beyond
/// them shares the lock of its remainder (the sheet does not specify the count).
static LOCKS: [Mtx; 8] = [const { Mtx::new(mutex::MTX_RECURSIVE | mutex::MTX_PLAIN) }; 8];

fn lockit(kind: i32) -> &'static Mtx {
    &LOCKS[kind.rem_euclid(LOCKS.len() as i32) as usize]
}

/// `std::_Lockit::_Lockit(int kind)`: store the number (the object is 4 bytes) and take its lock.
#[unsafe(export_name = "??0_Lockit@std@@QEAA@H@Z")]
pub unsafe extern "C" fn lockit_new(this: *mut i32, kind: i32) -> *mut i32 {
    // SAFETY: the caller's 4 bytes.
    unsafe { this.write(kind) };
    lockit(kind).lock();
    this
}

/// `std::_Lockit::_Lockit()`: lock 0.
#[unsafe(export_name = "??0_Lockit@std@@QEAA@XZ")]
pub unsafe extern "C" fn lockit_default(this: *mut i32) -> *mut i32 {
    // SAFETY: the caller's object.
    unsafe { lockit_new(this, 0) }
}

/// `std::_Lockit::~_Lockit()`: release the lock of the stored number.
#[unsafe(export_name = "??1_Lockit@std@@QEAA@XZ")]
pub unsafe extern "C" fn lockit_drop(this: *mut i32) {
    // SAFETY: an object the constructor made.
    lockit(unsafe { this.read() }).unlock();
}

/// `static void _Lockit::_Lockit_ctor(int)`: take lock `kind`.
#[unsafe(export_name = "?_Lockit_ctor@_Lockit@std@@SAXH@Z")]
pub extern "C" fn lockit_ctor_kind(kind: i32) {
    lockit(kind).lock();
}

/// `static void _Lockit::_Lockit_dtor(int)`: release lock `kind`.
#[unsafe(export_name = "?_Lockit_dtor@_Lockit@std@@SAXH@Z")]
pub extern "C" fn lockit_dtor_kind(kind: i32) {
    lockit(kind).unlock();
}

/// `static void _Lockit::_Lockit_ctor(_Lockit*)`: the default constructor's work.
#[unsafe(export_name = "?_Lockit_ctor@_Lockit@std@@CAXPEAV12@@Z")]
pub unsafe extern "C" fn lockit_ctor_this(this: *mut i32) {
    // SAFETY: the caller's object.
    unsafe { lockit_new(this, 0) };
}

/// `static void _Lockit::_Lockit_ctor(_Lockit*, int)`: the constructor's work.
#[unsafe(export_name = "?_Lockit_ctor@_Lockit@std@@CAXPEAV12@H@Z")]
pub unsafe extern "C" fn lockit_ctor_this_kind(this: *mut i32, kind: i32) {
    // SAFETY: the caller's object.
    unsafe { lockit_new(this, kind) };
}

/// `static void _Lockit::_Lockit_dtor(_Lockit*)`: the destructor's work.
#[unsafe(export_name = "?_Lockit_dtor@_Lockit@std@@CAXPEAV12@@Z")]
pub unsafe extern "C" fn lockit_dtor_this(this: *mut i32) {
    // SAFETY: the caller's object.
    unsafe { lockit_drop(this) };
}
