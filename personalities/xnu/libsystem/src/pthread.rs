//! libsystem_pthread: threads, mutexes, condition variables, `pthread_once`
//! and thread-specific keys (ADR 0258; threads ADR 0257 addendum).
//!
//! The objects keep their state in the opaque bytes of the SDK's types
//! (`<sys/_pthread/_pthread_types.h>`: a 64-bit signature, then 56 bytes
//! for a mutex and an attribute, 40 for a condition variable, 8 for a
//! once); a statically initialized object keeps the kind its signature
//! names (`<pthread/pthread_impl.h>`). Waiting is done in the kernel with
//! `__ulock_wait` / `__ulock_wake` (the ulock sheets).
//!
//! **Threads.** At start the library registers its `thread_start` with
//! `__bsdthread_register`; `pthread_create` maps a stack and a thread
//! record and calls `__bsdthread_create` with `PTHREAD_START_CUSTOM`. The
//! new thread starts at `thread_start` with Poly's contract (the record,
//! the start function, its argument) and ends with `__bsdthread_terminate`.
//! On arm64 a thread's record is slot 0 of its table (`TPIDRRO_EL0`, which
//! also holds its thread-local blocks; the main thread's is filled at
//! start); on x86-64 every thread is the main record for now (x86-64
//! thread-local storage per thread is a later step).

use core::ffi::c_void;
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};

const EPERM: i32 = 1;
const ESRCH: i32 = 3;
const EAGAIN: i32 = 35;
const EINVAL: i32 = 22;
const EBUSY: i32 = 16;
const EDEADLK: i32 = 11;
const ETIMEDOUT: i32 = 60;

/// `_PTHREAD_MUTEX_SIG_init` and the errorcheck and recursive forms.
const SIG_NORMAL: i64 = 0x32AA_ABA7;
const SIG_ERRORCHECK: i64 = 0x32AA_ABA1;
const SIG_RECURSIVE: i64 = 0x32AA_ABA2;
/// The signature Poly gives an initialized mutex attribute.
const SIG_ATTR: i64 = 0x4D54_5841;
/// `_PTHREAD_COND_SIG_init`.
const SIG_COND: i64 = 0x3CB0_B1BB;
/// The signature Poly gives an initialized thread attribute.
const SIG_THREAD_ATTR: i64 = 0x5448_5241;

const NORMAL: i32 = 0;
const ERRORCHECK: i32 = 1;
const RECURSIVE: i32 = 2;

/// `PTHREAD_CREATE_JOINABLE`, `PTHREAD_CREATE_DETACHED` (`<pthread.h>`).
const JOINABLE: i32 = 1;
const DETACHED: i32 = 2;
/// `PTHREAD_START_CUSTOM`: the caller made the stack and the record.
const START_CUSTOM: u32 = 0x0100_0000;
/// The stack of a new thread unless its attribute says otherwise (512 KiB,
/// what macOS documents for secondary threads).
const DEFAULT_STACK: usize = 512 << 10;
/// `PTHREAD_STACK_MIN` (`<limits.h>`): 16 KiB.
const STACK_MIN: usize = 16 << 10;

/// `UL_COMPARE_AND_WAIT` and `ULF_WAKE_ALL`.
const UL_WAIT: u32 = 1;
const ULF_WAKE_ALL: u32 = 0x100;

unsafe extern "C" {
    fn __ulock_wait(op: u32, addr: *const u32, value: u64, timeout_us: u32) -> i32;
    fn __ulock_wait2(op: u32, addr: *const u32, value: u64, timeout_ns: u64, value2: u64) -> i32;
    fn __ulock_wake(op: u32, addr: *const u32, wake_value: u64) -> i32;
    fn __bsdthread_register(start: usize, wqstart: usize, flags: i32, a: usize, b: usize, c: usize) -> i32;
    fn __bsdthread_create(func: usize, arg: usize, stack: usize, pthread: usize, flags: u32) -> isize;
    fn __bsdthread_terminate(stack: usize, size: usize, port: u32, sem: u32) -> i32;
    fn mmap(addr: *mut c_void, len: usize, prot: i32, flags: i32, fd: i32, off: i64) -> *mut c_void;
}

/// Sleep while `*word == value` (a wake, a signal or a spurious return
/// all come back here; callers loop).
fn wait(word: &AtomicU32, value: u32) {
    // SAFETY: the word lives as long as its object; the kernel only reads it.
    unsafe { __ulock_wait(UL_WAIT, word.as_ptr(), u64::from(value), 0) };
}

fn wake(word: &AtomicU32, all: bool) {
    // SAFETY: as above.
    unsafe { __ulock_wake(UL_WAIT | if all { ULF_WAKE_ALL } else { 0 }, word.as_ptr(), 0) };
}

// ------------------------------------------------------------- threads

/// `PTHREAD_KEYS_MAX` (`<limits.h>`).
const KEYS: usize = 512;

/// A thread's record (`pthread_t` points to it).
#[repr(C, align(16))]
struct Thread {
    func: usize,
    arg: usize,
    result: usize,
    /// 1 once the thread has ended (joiners wait on it).
    done: AtomicU32,
    detached: AtomicU32,
    stack: usize,
    stack_size: usize,
    /// Its thread-specific values.
    values: [usize; KEYS],
}

static MAIN: crate::Global<Thread> = crate::Global::new(Thread {
    func: 0,
    arg: 0,
    result: 0,
    done: AtomicU32::new(0),
    detached: AtomicU32::new(0),
    stack: 0,
    stack_size: 0,
    values: [0; KEYS],
});

fn main_thread() -> *mut Thread {
    MAIN.0.get()
}

/// The calling thread's record.
fn me() -> *mut Thread {
    #[cfg(target_arch = "aarch64")]
    {
        let table: usize;
        // SAFETY: reading TPIDRRO_EL0, which Poly's loader and runtime
        // set to the thread's table.
        unsafe { core::arch::asm!("mrs {t}, tpidrro_el0", t = out(reg) table, options(nomem, nostack)) };
        if table != 0 {
            // SAFETY: slot 0 of the table is the record (0 before start).
            let t = unsafe { *(table as *const usize) };
            if t != 0 {
                return t as *mut Thread;
            }
        }
    }
    main_thread()
}

/// Map `len` bytes of anonymous memory; null on failure.
fn map(len: usize) -> *mut u8 {
    // SAFETY: an anonymous mapping (PROT_READ|WRITE, MAP_PRIVATE|MAP_ANON).
    let p = unsafe { mmap(core::ptr::null_mut(), len, 3, 0x1002, -1, 0) };
    if p as isize == -1 { core::ptr::null_mut() } else { p as *mut u8 }
}

/// The first code of every new thread (Poly's contract: the record, the
/// start function, its argument).
extern "C" fn thread_start(t: *mut Thread, func: extern "C" fn(usize) -> usize, arg: usize) -> ! {
    let r = func(arg);
    // SAFETY: `t` is this thread's record.
    unsafe { end(t, r) }
}

/// End the calling thread `t` with result `r`: the key destructors, the
/// result for a joiner, then the kernel ends the thread. Its stack stays
/// mapped (the thread still runs on it).
///
/// # Safety
/// `t` is the calling thread's record.
unsafe fn end(t: *mut Thread, r: usize) -> ! {
    // SAFETY: the caller's contract.
    unsafe {
        run_destructors(&mut *t);
        (*t).result = r;
        (*t).done.store(1, Ordering::Release);
        wake(&(*t).done, true);
        __bsdthread_terminate(0, 0, 0, 0);
    }
    loop {
        core::hint::spin_loop();
    }
}

extern "C" fn init(_: i32, _: *const *const u8, _: *const *const u8, _: *const *const u8, _: *const u8) {
    #[cfg(target_arch = "aarch64")]
    {
        let table: usize;
        // SAFETY: as in `me`; slot 0 of the main thread's table is ours.
        unsafe {
            core::arch::asm!("mrs {t}, tpidrro_el0", t = out(reg) table, options(nomem, nostack));
            if table != 0 {
                *(table as *mut usize) = main_thread() as usize;
            }
        }
    }
    // SAFETY: a plain call; the entry point is this library's.
    unsafe { __bsdthread_register(thread_start as *const () as usize, 0, 0, 0, 0, 0) };
}

#[used]
#[unsafe(link_section = "__DATA,__mod_init_func,mod_init_funcs")]
static INIT: extern "C" fn(i32, *const *const u8, *const *const u8, *const *const u8, *const u8) = init;

/// `pthread_attr_t`: the signature, then the stack size and the detach
/// state.
#[repr(C)]
struct Attr {
    sig: i64,
    stack_size: usize,
    detach: i32,
    _rest: [u8; 44],
}

#[unsafe(no_mangle)]
unsafe extern "C" fn pthread_attr_init(a: *mut Attr) -> i32 {
    // SAFETY: the C contract: a pthread_attr_t.
    unsafe { a.write(Attr { sig: SIG_THREAD_ATTR, stack_size: DEFAULT_STACK, detach: JOINABLE, _rest: [0; 44] }) };
    0
}

#[unsafe(no_mangle)]
unsafe extern "C" fn pthread_attr_destroy(a: *mut Attr) -> i32 {
    // SAFETY: as above.
    unsafe { (*a).sig = 0 };
    0
}

#[unsafe(no_mangle)]
unsafe extern "C" fn pthread_attr_setstacksize(a: *mut Attr, size: usize) -> i32 {
    if size < STACK_MIN || size % 16384 != 0 {
        return EINVAL;
    }
    // SAFETY: as above.
    unsafe { (*a).stack_size = size };
    0
}

#[unsafe(no_mangle)]
unsafe extern "C" fn pthread_attr_getstacksize(a: *const Attr, size: *mut usize) -> i32 {
    // SAFETY: as above.
    unsafe { *size = (*a).stack_size };
    0
}

#[unsafe(no_mangle)]
unsafe extern "C" fn pthread_attr_setdetachstate(a: *mut Attr, state: i32) -> i32 {
    if state != JOINABLE && state != DETACHED {
        return EINVAL;
    }
    // SAFETY: as above.
    unsafe { (*a).detach = state };
    0
}

#[unsafe(no_mangle)]
unsafe extern "C" fn pthread_attr_getdetachstate(a: *const Attr, state: *mut i32) -> i32 {
    // SAFETY: as above.
    unsafe { *state = (*a).detach };
    0
}

#[unsafe(no_mangle)]
unsafe extern "C" fn pthread_create(out: *mut usize, attr: *const Attr, func: usize, arg: usize) -> i32 {
    // SAFETY: the C contract: `attr` null or initialized.
    let (size, detached) = unsafe {
        if attr.is_null() || (*attr).sig != SIG_THREAD_ATTR {
            (DEFAULT_STACK, false)
        } else {
            ((*attr).stack_size, (*attr).detach == DETACHED)
        }
    };
    let record_len = core::mem::size_of::<Thread>().next_multiple_of(16384);
    let stack = map(size);
    let t = map(record_len) as *mut Thread;
    if stack.is_null() || t.is_null() {
        return EAGAIN;
    }
    // SAFETY: a fresh mapping the size of the record.
    unsafe {
        (*t).func = func;
        (*t).arg = arg;
        (*t).detached.store(u32::from(detached), Ordering::Relaxed);
        (*t).stack = stack as usize;
        (*t).stack_size = size;
    }
    // The stack pointer the thread starts with: 16-byte aligned on arm64;
    // on x86-64 as after a call (a return address slot below the
    // alignment), as a function's first instruction expects.
    let top = stack as usize + size;
    #[cfg(target_arch = "x86_64")]
    let top = top - 8;
    // SAFETY: the kernel starts a thread at `thread_start` with the record.
    let r = unsafe { __bsdthread_create(func, arg, top, t as usize, START_CUSTOM) };
    if r == -1 {
        return EAGAIN;
    }
    // SAFETY: the C contract.
    unsafe { *out = t as usize };
    0
}

#[unsafe(no_mangle)]
unsafe extern "C" fn pthread_join(t: usize, result: *mut usize) -> i32 {
    let t = t as *mut Thread;
    if t.is_null() {
        return ESRCH;
    }
    if t == me() {
        return EDEADLK;
    }
    // SAFETY: the C contract: a thread pthread_create made and nobody joined.
    unsafe {
        if (*t).detached.load(Ordering::Relaxed) != 0 {
            return EINVAL;
        }
        while (*t).done.load(Ordering::Acquire) == 0 {
            wait(&(*t).done, 0);
        }
        if !result.is_null() {
            *result = (*t).result;
        }
    }
    0
}

#[unsafe(no_mangle)]
unsafe extern "C" fn pthread_detach(t: usize) -> i32 {
    // SAFETY: the C contract.
    unsafe { (*(t as *mut Thread)).detached.store(1, Ordering::Relaxed) };
    0
}

#[unsafe(no_mangle)]
unsafe extern "C" fn pthread_exit(r: usize) -> ! {
    let t = me();
    if t == main_thread() {
        // The main thread: the process ends when it does (v0).
        let _ = crate::raw3(1, 0, 0, 0);
    }
    // SAFETY: the calling thread's record.
    unsafe { end(t, r) }
}

#[unsafe(no_mangle)]
extern "C" fn pthread_self() -> *const c_void {
    me() as *const c_void
}

#[unsafe(no_mangle)]
extern "C" fn pthread_main_np() -> i32 {
    i32::from(me() == main_thread())
}

#[unsafe(no_mangle)]
extern "C" fn pthread_equal(a: usize, b: usize) -> i32 {
    i32::from(a == b)
}

// ------------------------------------------------------------- mutexes

/// `pthread_mutex_t` (64 bytes): the signature, then Poly's state: the
/// lock word (0 free, 1 held, 2 held with waiters), the recursion count,
/// the owner.
#[repr(C)]
struct Mutex {
    sig: i64,
    lock: AtomicU32,
    count: u32,
    owner: usize,
    _rest: [u8; 40],
}

fn kind(sig: i64) -> i32 {
    match sig {
        SIG_ERRORCHECK => ERRORCHECK,
        SIG_RECURSIVE => RECURSIVE,
        _ => NORMAL,
    }
}

#[unsafe(no_mangle)]
unsafe extern "C" fn pthread_mutexattr_init(a: *mut i64) -> i32 {
    // SAFETY: `pthread_mutexattr_t` is a signature and 8 bytes.
    unsafe {
        *a = SIG_ATTR;
        *(a.add(1) as *mut i32) = NORMAL;
    }
    0
}

#[unsafe(no_mangle)]
unsafe extern "C" fn pthread_mutexattr_destroy(a: *mut i64) -> i32 {
    // SAFETY: as above.
    unsafe { *a = 0 };
    0
}

#[unsafe(no_mangle)]
unsafe extern "C" fn pthread_mutexattr_settype(a: *mut i64, t: i32) -> i32 {
    // SAFETY: as above.
    unsafe {
        if *a != SIG_ATTR || !(NORMAL..=RECURSIVE).contains(&t) {
            return EINVAL;
        }
        *(a.add(1) as *mut i32) = t;
    }
    0
}

#[unsafe(no_mangle)]
unsafe extern "C" fn pthread_mutexattr_gettype(a: *const i64, t: *mut i32) -> i32 {
    // SAFETY: as above.
    unsafe {
        if *a != SIG_ATTR {
            return EINVAL;
        }
        *t = *(a.add(1) as *const i32);
    }
    0
}

#[unsafe(no_mangle)]
unsafe extern "C" fn pthread_mutex_init(m: *mut Mutex, a: *const i64) -> i32 {
    // SAFETY: the caller passes a mutex and an attribute or null.
    unsafe {
        let t = if a.is_null() { NORMAL } else { *(a.add(1) as *const i32) };
        m.write(Mutex {
            sig: [SIG_NORMAL, SIG_ERRORCHECK, SIG_RECURSIVE][t.clamp(0, 2) as usize],
            lock: AtomicU32::new(0),
            count: 0,
            owner: 0,
            _rest: [0; 40],
        });
    }
    0
}

#[unsafe(no_mangle)]
unsafe extern "C" fn pthread_mutex_destroy(m: *mut Mutex) -> i32 {
    // SAFETY: as above.
    let m = unsafe { &mut *m };
    if m.lock.load(Ordering::Relaxed) != 0 {
        return EBUSY;
    }
    m.sig = 0;
    0
}

/// Take the lock word: 0 → 1, or sleep with it at 2 (held, waiters).
fn lock_word(w: &AtomicU32, wait_for_it: bool) -> bool {
    if w.compare_exchange(0, 1, Ordering::Acquire, Ordering::Relaxed).is_ok() {
        return true;
    }
    if !wait_for_it {
        return false;
    }
    while w.swap(2, Ordering::Acquire) != 0 {
        wait(w, 2);
    }
    true
}

fn unlock_word(w: &AtomicU32) {
    if w.swap(0, Ordering::Release) == 2 {
        wake(w, false);
    }
}

unsafe fn acquire(m: *mut Mutex, wait_for_it: bool) -> i32 {
    // SAFETY: the caller's mutex.
    let m = unsafe { &mut *m };
    let k = kind(m.sig);
    let me = me() as usize;
    if m.lock.load(Ordering::Relaxed) != 0 && m.owner == me {
        match k {
            RECURSIVE => {
                m.count += 1;
                return 0;
            }
            ERRORCHECK => return EDEADLK,
            _ => {}
        }
    }
    if !lock_word(&m.lock, wait_for_it) {
        return EBUSY;
    }
    m.owner = me;
    m.count = 1;
    0
}

#[unsafe(no_mangle)]
unsafe extern "C" fn pthread_mutex_lock(m: *mut Mutex) -> i32 {
    // SAFETY: the caller's mutex.
    unsafe { acquire(m, true) }
}

#[unsafe(no_mangle)]
unsafe extern "C" fn pthread_mutex_trylock(m: *mut Mutex) -> i32 {
    // SAFETY: the caller's mutex.
    unsafe { acquire(m, false) }
}

#[unsafe(no_mangle)]
unsafe extern "C" fn pthread_mutex_unlock(m: *mut Mutex) -> i32 {
    // SAFETY: the caller's mutex.
    let m = unsafe { &mut *m };
    let k = kind(m.sig);
    if k != NORMAL && (m.lock.load(Ordering::Relaxed) == 0 || m.owner != me() as usize) {
        return EPERM;
    }
    if k == RECURSIVE && m.count > 1 {
        m.count -= 1;
        return 0;
    }
    m.owner = 0;
    m.count = 0;
    unlock_word(&m.lock);
    0
}

// ------------------------------------------------- condition variables

/// `pthread_cond_t` (48 bytes): the signature, then a sequence number that
/// every signal and broadcast moves.
#[repr(C)]
struct Cond {
    sig: i64,
    seq: AtomicU32,
    _rest: [u8; 36],
}

#[unsafe(no_mangle)]
unsafe extern "C" fn pthread_cond_init(c: *mut Cond, _attr: *const c_void) -> i32 {
    // SAFETY: the C contract.
    unsafe { c.write(Cond { sig: SIG_COND, seq: AtomicU32::new(0), _rest: [0; 36] }) };
    0
}

#[unsafe(no_mangle)]
unsafe extern "C" fn pthread_cond_destroy(c: *mut Cond) -> i32 {
    // SAFETY: the C contract.
    unsafe { (*c).sig = 0 };
    0
}

#[unsafe(no_mangle)]
unsafe extern "C" fn pthread_cond_signal(c: *mut Cond) -> i32 {
    // SAFETY: the C contract.
    let c = unsafe { &*c };
    c.seq.fetch_add(1, Ordering::Release);
    wake(&c.seq, false);
    0
}

#[unsafe(no_mangle)]
unsafe extern "C" fn pthread_cond_broadcast(c: *mut Cond) -> i32 {
    // SAFETY: the C contract.
    let c = unsafe { &*c };
    c.seq.fetch_add(1, Ordering::Release);
    wake(&c.seq, true);
    0
}

#[unsafe(no_mangle)]
unsafe extern "C" fn pthread_cond_wait(c: *mut Cond, m: *mut Mutex) -> i32 {
    // SAFETY: the C contract: the caller holds `m`.
    unsafe {
        let seq = (*c).seq.load(Ordering::Acquire);
        pthread_mutex_unlock(m);
        wait(&(*c).seq, seq);
        pthread_mutex_lock(m);
    }
    0
}

/// `pthread_cond_timedwait`: as `pthread_cond_wait`, until the absolute
/// time `abstime` (a `struct timespec`), then `ETIMEDOUT`.
#[unsafe(no_mangle)]
unsafe extern "C" fn pthread_cond_timedwait(c: *mut Cond, m: *mut Mutex, abstime: *const [i64; 2]) -> i32 {
    let mut tv = [0i64; 2];
    // gettimeofday (BSD 116) into {seconds, microseconds}.
    let _ = crate::raw3(116, (&raw mut tv) as u64, 0, 0);
    let now_ns = tv[0].saturating_mul(1_000_000_000).saturating_add((tv[1] & 0xffff_ffff) * 1000);
    // SAFETY: the C contract.
    let (s, ns) = unsafe { ((*abstime)[0], (*abstime)[1]) };
    let deadline = s.saturating_mul(1_000_000_000).saturating_add(ns);
    if deadline <= now_ns {
        return ETIMEDOUT;
    }
    // SAFETY: the C contract: the caller holds `m`.
    unsafe {
        let seq = (*c).seq.load(Ordering::Acquire);
        pthread_mutex_unlock(m);
        let r = __ulock_wait2(UL_WAIT, (*c).seq.as_ptr(), u64::from(seq), (deadline - now_ns) as u64, 0);
        let timed_out = r < 0 && crate::errno() == ETIMEDOUT;
        pthread_mutex_lock(m);
        if timed_out && (*c).seq.load(Ordering::Acquire) == seq { ETIMEDOUT } else { 0 }
    }
}

// --------------------------------------------------------------- once

/// `pthread_once_t`: the signature, then the state (0 not run, 1
/// running, 2 done).
#[unsafe(no_mangle)]
unsafe extern "C" fn pthread_once(o: *mut i64, f: extern "C" fn()) -> i32 {
    // SAFETY: `pthread_once_t` has 8 bytes after its signature.
    let state = unsafe { AtomicU32::from_ptr(o.add(1) as *mut u32) };
    loop {
        match state.compare_exchange(0, 1, Ordering::Acquire, Ordering::Acquire) {
            Ok(_) => {
                f();
                state.store(2, Ordering::Release);
                wake(state, true);
                return 0;
            }
            Err(2) => return 0,
            Err(v) => wait(state, v),
        }
    }
}

// --------------------------------------------------------------- keys

/// Which keys are in use and their destructors (a spin lock guards
/// changes; values are per thread, in the records).
static KEY_LOCK: AtomicBool = AtomicBool::new(false);
static KEY_USED: [AtomicBool; KEYS] = [const { AtomicBool::new(false) }; KEYS];
static KEY_DTOR: [AtomicUsize; KEYS] = [const { AtomicUsize::new(0) }; KEYS];

#[unsafe(no_mangle)]
unsafe extern "C" fn pthread_key_create(key: *mut usize, dtor: usize) -> i32 {
    while KEY_LOCK.swap(true, Ordering::Acquire) {
        core::hint::spin_loop();
    }
    let k = KEY_USED.iter().position(|u| !u.load(Ordering::Relaxed));
    if let Some(k) = k {
        KEY_USED[k].store(true, Ordering::Relaxed);
        KEY_DTOR[k].store(dtor, Ordering::Relaxed);
    }
    KEY_LOCK.store(false, Ordering::Release);
    let Some(k) = k else { return EAGAIN };
    // SAFETY: the C contract: `key` points to a `pthread_key_t`; the
    // calling thread's value starts at 0.
    unsafe {
        (*me()).values[k] = 0;
        *key = k;
    }
    0
}

#[unsafe(no_mangle)]
extern "C" fn pthread_key_delete(key: usize) -> i32 {
    if key >= KEYS || !KEY_USED[key].swap(false, Ordering::AcqRel) {
        return EINVAL;
    }
    0
}

#[unsafe(no_mangle)]
extern "C" fn pthread_getspecific(key: usize) -> usize {
    if key >= KEYS {
        return 0;
    }
    // SAFETY: the calling thread's record.
    unsafe { (*me()).values[key] }
}

#[unsafe(no_mangle)]
extern "C" fn pthread_setspecific(key: usize, value: usize) -> i32 {
    if key >= KEYS || !KEY_USED[key].load(Ordering::Relaxed) {
        return EINVAL;
    }
    // SAFETY: the calling thread's record.
    unsafe { (*me()).values[key] = value };
    0
}

/// Call the destructors of the thread's non-null values (POSIX: up to
/// `PTHREAD_DESTRUCTOR_ITERATIONS`, 4, rounds).
fn run_destructors(t: &mut Thread) {
    for _ in 0..4 {
        let mut any = false;
        for k in 0..KEYS {
            let v = t.values[k];
            let d = KEY_DTOR[k].load(Ordering::Relaxed);
            if v != 0 && d != 0 && KEY_USED[k].load(Ordering::Relaxed) {
                t.values[k] = 0;
                // SAFETY: a destructor the program registered for this key.
                let f: extern "C" fn(usize) = unsafe { core::mem::transmute(d) };
                f(v);
                any = true;
            }
        }
        if !any {
            break;
        }
    }
}

/// `pthread_atfork`: accepted; there is no `fork` yet to call the
/// handlers.
#[unsafe(no_mangle)]
extern "C" fn pthread_atfork(_prepare: usize, _parent: usize, _child: usize) -> i32 {
    0
}

/// `sched_yield`: give the processor up for a moment (a 1 µs timed wait on
/// a word nobody wakes, which makes the scheduler run another thread).
#[unsafe(no_mangle)]
extern "C" fn sched_yield() -> i32 {
    let word = AtomicU32::new(0);
    // The wait ends in ETIMEDOUT, which must not reach the caller's errno.
    let saved = crate::errno();
    // SAFETY: our own word; the call returns at the timeout.
    unsafe { __ulock_wait2(UL_WAIT, word.as_ptr(), 0, 1000, 0) };
    crate::set_errno(saved);
    0
}
