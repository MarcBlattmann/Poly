//! libsystem_pthread, v0: the main thread only (threads come with
//! `bsdthread_create`, ADR 0246). `pthread_self` is one static thread
//! record; mutexes, `pthread_once` and thread-specific keys keep their
//! state in the opaque bytes of the SDK's types (`<sys/_pthread/
//! _pthread_types.h>`: a 64-bit signature, then 56 bytes for a mutex, 8
//! for a once); a statically initialized mutex keeps the kind its
//! signature names (`<pthread/pthread_impl.h>`).

use core::ffi::c_void;
use core::sync::atomic::{AtomicU32, Ordering};

const EPERM: i32 = 1;
const EAGAIN: i32 = 35;
const EINVAL: i32 = 22;
const EBUSY: i32 = 16;
const EDEADLK: i32 = 11;

/// `_PTHREAD_MUTEX_SIG_init` and the errorcheck and recursive forms.
const SIG_NORMAL: i64 = 0x32AA_ABA7;
const SIG_ERRORCHECK: i64 = 0x32AA_ABA1;
const SIG_RECURSIVE: i64 = 0x32AA_ABA2;
/// The signature Poly gives an initialized mutex attribute.
const SIG_ATTR: i64 = 0x4D54_5841;

const NORMAL: i32 = 0;
const ERRORCHECK: i32 = 1;
const RECURSIVE: i32 = 2;

/// `pthread_mutex_t` (64 bytes): the signature, then Poly's state.
#[repr(C)]
struct Mutex {
    sig: i64,
    lock: AtomicU32,
    count: u32,
    owner: usize,
    _rest: [u8; 40],
}

/// The main thread's record (`pthread_t` points to it).
#[repr(C, align(16))]
struct Thread([u8; 256]);
static MAIN: Thread = Thread([0; 256]);

fn me() -> usize {
    &raw const MAIN as usize
}

#[unsafe(no_mangle)]
extern "C" fn pthread_self() -> *const c_void {
    me() as *const c_void
}

#[unsafe(no_mangle)]
extern "C" fn pthread_main_np() -> i32 {
    1
}

#[unsafe(no_mangle)]
extern "C" fn pthread_equal(a: usize, b: usize) -> i32 {
    i32::from(a == b)
}

/// The kind a mutex's signature names; `None` for one not initialized or
/// destroyed (`EINVAL`).
fn kind(sig: i64) -> Option<i32> {
    match sig {
        SIG_NORMAL => Some(NORMAL),
        SIG_ERRORCHECK => Some(ERRORCHECK),
        SIG_RECURSIVE => Some(RECURSIVE),
        _ => None,
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
    if kind(m.sig).is_none() {
        return EINVAL;
    }
    if m.lock.load(Ordering::Relaxed) != 0 {
        return EBUSY;
    }
    m.sig = 0;
    0
}

unsafe fn acquire(m: *mut Mutex, wait: bool) -> i32 {
    // SAFETY: the caller's mutex.
    let m = unsafe { &mut *m };
    let Some(k) = kind(m.sig) else { return EINVAL };
    if m.lock.load(Ordering::Relaxed) != 0 && m.owner == me() {
        // The pthread_mutex_trylock sheet: trylock of a normal or
        // error-checking mutex the caller holds is EBUSY.
        match k {
            RECURSIVE => {
                m.count += 1;
                return 0;
            }
            ERRORCHECK if wait => return EDEADLK,
            _ => {}
        }
    }
    loop {
        if m.lock.compare_exchange_weak(0, 1, Ordering::Acquire, Ordering::Relaxed).is_ok() {
            m.owner = me();
            m.count = 1;
            return 0;
        }
        if !wait {
            return EBUSY;
        }
        core::hint::spin_loop();
    }
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
    let Some(k) = kind(m.sig) else { return EINVAL };
    if k != NORMAL && (m.lock.load(Ordering::Relaxed) == 0 || m.owner != me()) {
        return EPERM;
    }
    if k == RECURSIVE && m.count > 1 {
        m.count -= 1;
        return 0;
    }
    m.owner = 0;
    m.count = 0;
    m.lock.store(0, Ordering::Release);
    0
}

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
                return 0;
            }
            Err(2) => return 0,
            Err(_) => core::hint::spin_loop(),
        }
    }
}

/// `PTHREAD_KEYS_MAX` (`<limits.h>`) is 512; the pthread_key_create sheet
/// saw the first keys a program makes numbered 258 and 259, and 507 more
/// after those two: keys 258 to 766.
const FIRST_KEY: usize = 258;
const KEYS: usize = 509;

/// The keys: whether each is in use, its destructor, the main thread's
/// value.
struct Keys {
    used: [bool; KEYS],
    dtor: [usize; KEYS],
    value: [usize; KEYS],
}

static KEY_TABLE: crate::Global<Keys> =
    crate::Global::new(Keys { used: [false; KEYS], dtor: [0; KEYS], value: [0; KEYS] });

fn keys() -> &'static mut Keys {
    // SAFETY: single thread (v0); each call takes the table once.
    unsafe { KEY_TABLE.get() }
}

#[unsafe(no_mangle)]
unsafe extern "C" fn pthread_key_create(key: *mut usize, dtor: usize) -> i32 {
    let t = keys();
    let Some(k) = t.used.iter().position(|&u| !u) else { return EAGAIN };
    t.used[k] = true;
    t.dtor[k] = dtor;
    t.value[k] = 0;
    // SAFETY: the C contract: `key` points to a `pthread_key_t`.
    unsafe { *key = FIRST_KEY + k };
    0
}

/// A key's index in the table, if it is one.
fn index(key: usize) -> Option<usize> {
    key.checked_sub(FIRST_KEY).filter(|&k| k < KEYS)
}

/// `pthread_key_delete`: no destructor runs; the value is forgotten.
#[unsafe(no_mangle)]
extern "C" fn pthread_key_delete(key: usize) -> i32 {
    let t = keys();
    let Some(k) = index(key).filter(|&k| t.used[k]) else { return EINVAL };
    t.used[k] = false;
    t.value[k] = 0;
    0
}

#[unsafe(no_mangle)]
extern "C" fn pthread_getspecific(key: usize) -> usize {
    let t = keys();
    index(key).filter(|&k| t.used[k]).map_or(0, |k| t.value[k])
}

#[unsafe(no_mangle)]
extern "C" fn pthread_setspecific(key: usize, value: usize) -> i32 {
    let t = keys();
    let Some(k) = index(key).filter(|&k| t.used[k]) else { return EINVAL };
    t.value[k] = value;
    0
}

/// `pthread_atfork`: accepted; there is no `fork` in v0 to call the
/// handlers.
#[unsafe(no_mangle)]
extern "C" fn pthread_atfork(_prepare: usize, _parent: usize, _child: usize) -> i32 {
    0
}
