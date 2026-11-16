//! The thread pool API (Learn: "Thread Pools", `threadpoolapiset.h`, `QueueUserWorkItem`): pools, work,
//! timer, wait and I/O objects, callback environments, cleanup groups and the requests a callback
//! makes of its instance. The rules (sizing, the environment's layout, due times) are
//! `poly_kernelbase::threadpool`'s.
//!
//! One lock (an SRW lock of `ntdll`) guards every pool, object and service; callbacks run without
//! it. A pool's threads sleep on a condition variable of the pool. Timers and waits are kept by
//! *service* threads: each waits for its objects' handles (at most 63) and its own control event with
//! the earliest due time as the timeout, and queues the callback of what fired on the object's
//! pool. I/O objects share one completion port and its thread. Objects and pools are never freed
//! while a callback of theirs is queued or running: a close only marks them, and the last
//! callback to finish frees them.

use crate::base::*;
use crate::nt::{self, Handle};
use alloc::boxed::Box;
use alloc::vec::Vec;
use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicI64, AtomicUsize, Ordering};
use poly_kernelbase::error::*;
use poly_kernelbase::threadpool::{self as rules, Next, PoolState};
use poly_rtl::critsec::CriticalSection;

const P: usize = core::mem::size_of::<usize>();
const LAYOUT: rules::EnvLayout = rules::EnvLayout::new(P);
/// "No due time" of a wait object.
const NEVER: i64 = i64::MAX;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Work,
    /// `TrySubmitThreadpoolCallback`'s: one callback, then the object ends.
    Simple,
    /// `QueueUserWorkItem`'s.
    Legacy,
    Timer,
    Wait,
    Io,
}

/// What a pool thread is to run.
#[derive(Clone, Copy)]
enum Job {
    /// The callback of a work, simple, legacy or timer object.
    Run(*mut Object),
    /// A wait object's callback with its `TP_WAIT_RESULT`.
    Wait(*mut Object, u32),
    /// An I/O object's callback: the `OVERLAPPED`, the error, the bytes.
    Io(*mut Object, usize, u32, usize),
}

impl Job {
    fn object(&self) -> *mut Object {
        match *self {
            Job::Run(o) | Job::Wait(o, _) | Job::Io(o, ..) => o,
        }
    }
}

struct Pool {
    state: PoolState<Job>,
    /// Sleeping threads wait here.
    cv: AtomicUsize,
    /// Objects that name this pool.
    objects: u32,
    /// `CloseThreadpool` has been called.
    closed: bool,
    /// The process's default pool: never freed.
    default: bool,
    stack_reserve: usize,
    stack_commit: usize,
}

struct Object {
    kind: Kind,
    pool: *mut Pool,
    group: *mut Group,
    callback: usize,
    context: usize,
    /// The environment's `CleanupGroupCancelCallback` and `FinalizationCallback`.
    cancel_callback: usize,
    finalization: usize,
    /// Callbacks queued or running.
    pending: u32,
    closed: bool,
    /// A timer or wait that is set (a one-shot timer stays set after it fired; a wait does not).
    set: bool,
    /// In a service's list, waiting to fire.
    armed: bool,
    /// Timer: when it fires next; wait: when it times out ([`NEVER`] for none).
    deadline: i64,
    period_ms: u32,
    /// A wait's handle, an I/O object's file.
    handle: Handle,
    service: *mut Service,
    /// `StartThreadpoolIo` calls whose completion has not come yet.
    io_started: u32,
}

struct Group {
    members: Vec<*mut Object>,
}

struct Service {
    control: Handle,
    objects: Vec<*mut Object>,
}

enum Action {
    SetEvent(Handle),
    ReleaseSemaphore(Handle, i32),
    ReleaseMutex(Handle),
    LeaveCriticalSection(*mut CriticalSection),
    FreeLibrary(*mut u8),
}

/// `TP_CALLBACK_INSTANCE`: one running callback.
pub struct Instance {
    object: *mut Object,
    pool: *mut Pool,
    actions: Vec<Action>,
    disassociated: bool,
}

struct Globals {
    default_pool: *mut Pool,
    services: Vec<*mut Service>,
    io_port: Handle,
    io_objects: Vec<*mut Object>,
}

struct Locked<T>(UnsafeCell<T>);
// SAFETY: only used with `LOCK` held.
unsafe impl<T> Sync for Locked<T> {}

static G: Locked<Globals> = Locked(UnsafeCell::new(Globals {
    default_pool: core::ptr::null_mut(),
    services: Vec::new(),
    io_port: 0,
    io_objects: Vec::new(),
}));
static LOCK: AtomicUsize = AtomicUsize::new(0);
/// Waiters for an object to have no callback left (every object's) sleep here.
static IDLE_CV: AtomicUsize = AtomicUsize::new(0);
static FREQUENCY: AtomicI64 = AtomicI64::new(0);

fn lock() {
    // SAFETY: an SRW lock word.
    unsafe { nt::RtlAcquireSRWLockExclusive(LOCK.as_ptr()) };
}

fn unlock() {
    // SAFETY: as above.
    unsafe { nt::RtlReleaseSRWLockExclusive(LOCK.as_ptr()) };
}

/// Sleep on `cv`, giving up the lock meanwhile; `true` when woken.
fn sleep(cv: &AtomicUsize, timeout: Option<i64>) -> bool {
    let tp = timeout.as_ref().map_or(core::ptr::null(), |t| t as *const i64);
    // SAFETY: the condition variable and the lock, which the caller holds.
    unsafe { nt::RtlSleepConditionVariableSRW(cv.as_ptr(), LOCK.as_ptr(), tp, 0) }.0 == 0
}

fn wake_one(cv: &AtomicUsize) {
    // SAFETY: a condition variable word.
    unsafe { nt::RtlWakeConditionVariable(cv.as_ptr()) };
}

fn wake_all(cv: &AtomicUsize) {
    // SAFETY: as above.
    unsafe { nt::RtlWakeAllConditionVariable(cv.as_ptr()) };
}

/// The monotonic clock in 100 ns.
fn now() -> i64 {
    let (mut c, mut f) = (0i64, 0i64);
    let freq = FREQUENCY.load(Ordering::Relaxed);
    // SAFETY: valid pointers.
    unsafe { nt::NtQueryPerformanceCounter(&mut c, if freq == 0 { &mut f } else { core::ptr::null_mut() }) };
    let f = if freq == 0 {
        FREQUENCY.store(f.max(1), Ordering::Relaxed);
        f.max(1)
    } else {
        freq
    };
    (i128::from(c) * 10_000_000 / i128::from(f)) as i64
}

fn system_now() -> i64 {
    let mut t = 0i64;
    // SAFETY: a valid pointer.
    unsafe { nt::NtQuerySystemTime(&mut t) };
    t
}

/// A `FILETIME` the caller passed (two `DWORD`s, low first).
unsafe fn filetime(p: *const u32) -> i64 {
    // SAFETY: the caller's FILETIME.
    unsafe { (i64::from(p.add(1).read_unaligned()) << 32) | i64::from(p.read_unaligned()) }
}

unsafe fn env_ptr(env: *const u8, offset: usize) -> usize {
    if env.is_null() {
        return 0;
    }
    // SAFETY: the caller's `TP_CALLBACK_ENVIRON` (at least version 1's size).
    unsafe { env.add(offset).cast::<usize>().read_unaligned() }
}

// ----- pools ----------------------------------------------------------------------------------------

fn new_pool(default: bool) -> *mut Pool {
    Box::into_raw(Box::new(Pool {
        state: PoolState::default(),
        cv: AtomicUsize::new(0),
        objects: 0,
        closed: false,
        default,
        stack_reserve: 0,
        stack_commit: 0,
    }))
}

/// The pool an environment names, or the process's default pool. The lock is held.
unsafe fn pool_of(env: *const u8) -> *mut Pool {
    // SAFETY: the caller's environment.
    let named = unsafe { env_ptr(env, LAYOUT.pool) } as *mut Pool;
    if !named.is_null() {
        return named;
    }
    // SAFETY: the lock is held.
    let g = unsafe { &mut *G.0.get() };
    if g.default_pool.is_null() {
        g.default_pool = new_pool(true);
    }
    g.default_pool
}

/// Free a closed pool once no thread and no object is left. The lock is held.
unsafe fn maybe_free_pool(pool: *mut Pool) {
    // SAFETY: the lock is held; a pool lives while a thread or an object counts on it.
    unsafe {
        let p = &*pool;
        if p.closed && !p.default && p.state.threads == 0 && p.objects == 0 {
            drop(Box::from_raw(pool));
        }
    }
}

/// Start a thread of `pool` (already counted in its state). The lock is held.
unsafe fn spawn_worker(pool: *mut Pool) {
    // SAFETY: the lock is held; a valid entry point; the thread gets the pool.
    unsafe {
        // A stack size asked for with `SetThreadpoolStackInformation` is the reservation.
        let (stack, flags) = match (*pool).stack_reserve {
            0 => (0, 0),
            n => (n, 0x1_0000),
        };
        let h = crate::sync::CreateThread(
            core::ptr::null(),
            stack,
            worker as *const u8,
            pool.cast(),
            flags,
            core::ptr::null_mut(),
        );
        if h == 0 {
            (*pool).state.spawn_failed();
        } else {
            close(h);
        }
    }
}

/// Put `job` on its object's pool. The lock is held.
unsafe fn queue(job: Job) {
    // SAFETY: the lock is held; the object and its pool are alive while the job is queued.
    unsafe {
        let o = job.object();
        (*o).pending += 1;
        let pool = (*o).pool;
        let d = (*pool).state.push(job);
        if d.wake {
            wake_one(&(*pool).cv);
        }
        if d.spawn {
            spawn_worker(pool);
        }
    }
}

/// The thread of a pool.
unsafe extern "system" fn worker(arg: *mut u8) -> u32 {
    let pool = arg as *mut Pool;
    lock();
    loop {
        // SAFETY: the pool lives while this thread is counted.
        match unsafe { (*pool).state.take_next() } {
            Next::Run(job) => {
                unlock();
                // SAFETY: a job queued on this pool.
                unsafe { run(pool, job) };
                lock();
            }
            Next::Exit => {
                // SAFETY: the lock is held.
                unsafe { maybe_free_pool(pool) };
                unlock();
                return 0;
            }
            Next::Sleep { may_end } => {
                let timeout = may_end.then_some(-rules::IDLE_THREAD_TIMEOUT);
                // SAFETY: as above.
                let woken = sleep(unsafe { &(*pool).cv }, timeout);
                // SAFETY: as above.
                unsafe {
                    (*pool).state.woke();
                    if !woken && (*pool).state.idle_exit() {
                        maybe_free_pool(pool);
                        unlock();
                        return 0;
                    }
                }
            }
        }
    }
}

type SimpleCallback = unsafe extern "system" fn(*mut Instance, *mut u8);
type ObjectCallback = unsafe extern "system" fn(*mut Instance, *mut u8, *mut Object);
type WaitCallback = unsafe extern "system" fn(*mut Instance, *mut u8, *mut Object, u32);
type IoCallback = unsafe extern "system" fn(*mut Instance, *mut u8, *mut u8, u32, usize, *mut Object);
type LegacyCallback = unsafe extern "system" fn(*mut u8) -> u32;
type CancelCallback = unsafe extern "system" fn(*mut u8, *mut u8);

/// Run one callback, then what it asked for when it returns, then account for it.
unsafe fn run(pool: *mut Pool, job: Job) {
    let o = job.object();
    let mut inst = Instance { object: o, pool, actions: Vec::new(), disassociated: false };
    let ip: *mut Instance = &mut inst;
    // SAFETY: the object lives until its last callback finishes.
    unsafe {
        let (cb, ctx, kind, fin) = ((*o).callback, (*o).context as *mut u8, (*o).kind, (*o).finalization);
        match job {
            Job::Run(_) => match kind {
                Kind::Simple => core::mem::transmute::<usize, SimpleCallback>(cb)(ip, ctx),
                Kind::Legacy => {
                    core::mem::transmute::<usize, LegacyCallback>(cb)(ctx);
                }
                _ => core::mem::transmute::<usize, ObjectCallback>(cb)(ip, ctx, o),
            },
            Job::Wait(_, result) => core::mem::transmute::<usize, WaitCallback>(cb)(ip, ctx, o, result),
            Job::Io(_, ov, result, bytes) => {
                core::mem::transmute::<usize, IoCallback>(cb)(ip, ctx, ov as *mut u8, result, bytes, o)
            }
        }
        if fin != 0 {
            core::mem::transmute::<usize, SimpleCallback>(fin)(ip, ctx);
        }
        for a in core::mem::take(&mut inst.actions) {
            match a {
                Action::SetEvent(h) => {
                    nt::NtSetEvent(h, core::ptr::null_mut());
                }
                Action::ReleaseSemaphore(h, n) => {
                    nt::NtReleaseSemaphore(h, n, core::ptr::null_mut());
                }
                Action::ReleaseMutex(h) => {
                    nt::NtReleaseMutant(h, core::ptr::null_mut());
                }
                Action::LeaveCriticalSection(cs) => {
                    nt::RtlLeaveCriticalSection(cs);
                }
                Action::FreeLibrary(m) => {
                    crate::process::FreeLibrary(m);
                }
            }
        }
        lock();
        if !inst.disassociated {
            (*o).pending -= 1;
        }
        if matches!((*o).kind, Kind::Simple | Kind::Legacy) {
            (*o).closed = true;
        }
        wake_all(&IDLE_CV);
        if (*o).closed && (*o).pending == 0 {
            destroy(o);
        }
        unlock();
    }
}

/// Free an object with no callback left. The lock is held.
unsafe fn destroy(o: *mut Object) {
    // SAFETY: the lock is held; the object is closed and idle, so no job or service names it.
    unsafe {
        let ob = &mut *o;
        unarm(o);
        if !ob.group.is_null() {
            (*ob.group).members.retain(|&m| m != o);
        }
        if ob.kind == Kind::Io {
            (*G.0.get()).io_objects.retain(|&m| m != o);
        }
        let pool = ob.pool;
        (*pool).objects -= 1;
        drop(Box::from_raw(o));
        maybe_free_pool(pool);
    }
}

/// Make an object of `kind` for the pool and cleanup group of `env`.
unsafe fn create(kind: Kind, callback: usize, context: usize, env: *const u8) -> *mut Object {
    lock();
    // SAFETY: the lock is held; the caller's environment.
    unsafe {
        let pool = pool_of(env);
        let group = env_ptr(env, LAYOUT.cleanup_group) as *mut Group;
        let o = Box::into_raw(Box::new(Object {
            kind,
            pool,
            group,
            callback,
            context,
            cancel_callback: env_ptr(env, LAYOUT.cancel_callback),
            finalization: env_ptr(env, LAYOUT.finalization),
            pending: 0,
            closed: false,
            set: false,
            armed: false,
            deadline: NEVER,
            period_ms: 0,
            handle: 0,
            service: core::ptr::null_mut(),
            io_started: 0,
        }));
        (*pool).objects += 1;
        if !group.is_null() {
            (*group).members.push(o);
        }
        unlock();
        o
    }
}

/// Wait until the object has no callback queued or running. The lock is held.
unsafe fn wait_idle(o: *mut Object, include_io: bool) {
    // SAFETY: the lock is held; the caller still owns the object.
    unsafe {
        while (*o).pending > 0 || (include_io && (*o).io_started > 0) {
            sleep(&IDLE_CV, None);
        }
    }
}

/// Drop the callbacks of `o` that have not started. The lock is held.
unsafe fn cancel_queued(o: *mut Object) {
    // SAFETY: the lock is held.
    unsafe {
        let pool = &mut *(*o).pool;
        let before = pool.state.queue.len();
        pool.state.queue.retain(|j| j.object() != o);
        let removed = (before - pool.state.queue.len()) as u32;
        (*o).pending -= removed;
        if removed > 0 {
            wake_all(&IDLE_CV);
        }
    }
}

/// Close an object: it ends with its last callback. The lock is held.
unsafe fn close_object(o: *mut Object) {
    // SAFETY: the lock is held.
    unsafe {
        (*o).closed = true;
        unarm(o);
        if (*o).pending == 0 {
            destroy(o);
        }
    }
}

// ----- the API: pools ---------------------------------------------------------------------------------

/// `CreateThreadpool`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn CreateThreadpool(_reserved: *mut u8) -> *mut u8 {
    new_pool(false).cast()
}

/// `CloseThreadpool`: the threads end when their jobs are done.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn CloseThreadpool(pool: *mut u8) {
    if pool.is_null() {
        return;
    }
    let pool = pool as *mut Pool;
    lock();
    // SAFETY: the lock is held; the caller's pool.
    unsafe {
        (*pool).closed = true;
        (*pool).state.closing = true;
        wake_all(&(*pool).cv);
        maybe_free_pool(pool);
    }
    unlock();
}

/// `SetThreadpoolThreadMaximum`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn SetThreadpoolThreadMaximum(pool: *mut u8, max: u32) {
    let pool = pool as *mut Pool;
    lock();
    // SAFETY: the lock is held; the caller's pool.
    unsafe { (*pool).state.set_max(max) };
    unlock();
}

/// `SetThreadpoolThreadMinimum`: the threads are started now and kept.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn SetThreadpoolThreadMinimum(pool: *mut u8, min: u32) -> i32 {
    let pool = pool as *mut Pool;
    lock();
    // SAFETY: the lock is held; the caller's pool.
    unsafe {
        for _ in 0..(*pool).state.set_min(min) {
            spawn_worker(pool);
        }
    }
    unlock();
    TRUE
}

/// `SetThreadpoolStackInformation(pool, TP_POOL_STACK_INFORMATION*)`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn SetThreadpoolStackInformation(pool: *mut u8, info: *const usize) -> i32 {
    if pool.is_null() || info.is_null() {
        return fail(ERROR_INVALID_PARAMETER, FALSE);
    }
    let pool = pool as *mut Pool;
    lock();
    // SAFETY: the lock is held; the caller's pool and structure (two sizes).
    unsafe {
        (*pool).stack_reserve = info.read();
        (*pool).stack_commit = info.add(1).read();
    }
    unlock();
    TRUE
}

/// `QueryThreadpoolStackInformation`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn QueryThreadpoolStackInformation(pool: *mut u8, info: *mut usize) -> i32 {
    if pool.is_null() || info.is_null() {
        return fail(ERROR_INVALID_PARAMETER, FALSE);
    }
    let pool = pool as *mut Pool;
    lock();
    // SAFETY: as above.
    unsafe {
        info.write((*pool).stack_reserve);
        info.add(1).write((*pool).stack_commit);
    }
    unlock();
    TRUE
}

// ----- cleanup groups -------------------------------------------------------------------------------------

/// `CreateThreadpoolCleanupGroup`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn CreateThreadpoolCleanupGroup() -> *mut u8 {
    Box::into_raw(Box::new(Group { members: Vec::new() })).cast()
}

/// `CloseThreadpoolCleanupGroupMembers(group, cancelPending, cleanupContext)`: wait for the members'
/// callbacks (dropping those that have not started if asked), tell each member's cancel callback,
/// and close the members.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn CloseThreadpoolCleanupGroupMembers(
    group: *mut u8,
    cancel: i32,
    cleanup_context: *mut u8,
) {
    if group.is_null() {
        return;
    }
    let group = group as *mut Group;
    if cancel != 0 {
        // Every member's queued callbacks go first: waiting for one must not let another's run.
        lock();
        // SAFETY: the lock is held; the caller's group and its members.
        unsafe {
            for o in (*group).members.clone() {
                cancel_queued(o);
                unarm(o);
            }
        }
        unlock();
    }
    loop {
        lock();
        // SAFETY: the lock is held; the caller's group.
        let Some(o) = (unsafe { (*group).members.first().copied() }) else {
            unlock();
            return;
        };
        // SAFETY: a member lives until it is closed.
        let (context, cancel_callback) = unsafe {
            if cancel != 0 {
                cancel_queued(o);
                unarm(o);
            }
            wait_idle(o, false);
            ((*o).context, (*o).cancel_callback)
        };
        // SAFETY: as above.
        unsafe {
            (*group).members.retain(|&m| m != o);
            (*o).group = core::ptr::null_mut();
        }
        unlock();
        if cancel_callback != 0 {
            // SAFETY: the environment's callback, called with the object's and the caller's contexts.
            unsafe {
                core::mem::transmute::<usize, CancelCallback>(cancel_callback)(context as *mut u8, cleanup_context)
            };
        }
        lock();
        // SAFETY: the lock is held.
        unsafe { close_object(o) };
        unlock();
    }
}

/// `CloseThreadpoolCleanupGroup`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn CloseThreadpoolCleanupGroup(group: *mut u8) {
    if !group.is_null() {
        // SAFETY: made by `CreateThreadpoolCleanupGroup`.
        drop(unsafe { Box::from_raw(group as *mut Group) });
    }
}

// ----- work ----------------------------------------------------------------------------------------------------

/// `CreateThreadpoolWork(callback, context, environment)`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn CreateThreadpoolWork(callback: usize, context: *mut u8, env: *const u8) -> *mut u8 {
    // SAFETY: the caller's environment.
    unsafe { create(Kind::Work, callback, context as usize, env) }.cast()
}

/// `SubmitThreadpoolWork`: one more run of the callback.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn SubmitThreadpoolWork(work: *mut u8) {
    let o = work as *mut Object;
    lock();
    // SAFETY: the lock is held; the caller's object.
    unsafe { queue(Job::Run(o)) };
    unlock();
}

unsafe fn wait_for_callbacks(o: *mut Object, cancel: i32, include_io: bool) {
    lock();
    // SAFETY: the lock is held; the caller's object.
    unsafe {
        if cancel != 0 {
            cancel_queued(o);
        }
        wait_idle(o, include_io && cancel == 0);
    }
    unlock();
}

/// `WaitForThreadpoolWorkCallbacks(work, cancelPending)`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn WaitForThreadpoolWorkCallbacks(work: *mut u8, cancel: i32) {
    // SAFETY: the caller's object.
    unsafe { wait_for_callbacks(work as *mut Object, cancel, false) };
}

unsafe fn close_api(o: *mut u8) {
    if o.is_null() {
        return;
    }
    lock();
    // SAFETY: the lock is held; the caller's object.
    unsafe { close_object(o as *mut Object) };
    unlock();
}

/// `CloseThreadpoolWork`: callbacks queued or running still run.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn CloseThreadpoolWork(work: *mut u8) {
    // SAFETY: as above.
    unsafe { close_api(work) };
}

/// `TrySubmitThreadpoolCallback(callback, context, environment)`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn TrySubmitThreadpoolCallback(callback: usize, context: *mut u8, env: *const u8) -> i32 {
    // SAFETY: the caller's environment.
    let o = unsafe { create(Kind::Simple, callback, context as usize, env) };
    lock();
    // SAFETY: the lock is held; the object was just made.
    unsafe { queue(Job::Run(o)) };
    unlock();
    TRUE
}

/// `QueueUserWorkItem(function, context, flags)`: runs `function(context)` on a pool thread.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn QueueUserWorkItem(function: usize, context: *mut u8, _flags: u32) -> i32 {
    if function == 0 {
        return fail(ERROR_INVALID_PARAMETER, FALSE);
    }
    // SAFETY: no environment: the default pool.
    let o = unsafe { create(Kind::Legacy, function, context as usize, core::ptr::null()) };
    lock();
    // SAFETY: the lock is held.
    unsafe { queue(Job::Run(o)) };
    unlock();
    TRUE
}

// ----- services: timers and waits -----------------------------------------------------------------------------------

/// The service that keeps `o`: one with room (63 waits), or a new one. The lock is held.
unsafe fn service_for(o: *mut Object) -> *mut Service {
    // SAFETY: the lock is held.
    unsafe {
        let g = &mut *G.0.get();
        for &s in &g.services {
            let waits = (*s).objects.iter().filter(|&&x| (*x).kind == Kind::Wait).count();
            if (*o).kind != Kind::Wait || waits < 63 {
                return s;
            }
        }
        let mut control: Handle = 0;
        let st = nt::NtCreateEvent(
            &mut control,
            poly_kernelbase::sync::EVENT_ALL_ACCESS,
            core::ptr::null(),
            poly_kernelbase::sync::SYNCHRONIZATION_EVENT,
            0,
        );
        if !st.is_success() {
            return core::ptr::null_mut();
        }
        let s = Box::into_raw(Box::new(Service { control, objects: Vec::new() }));
        let h =
            crate::sync::CreateThread(core::ptr::null(), 0, service as *const u8, s.cast(), 0, core::ptr::null_mut());
        if h == 0 {
            drop(Box::from_raw(s));
            close(control);
            return core::ptr::null_mut();
        }
        close(h);
        g.services.push(s);
        s
    }
}

/// Put `o` in a service's list. The lock is held.
unsafe fn arm(o: *mut Object) -> bool {
    // SAFETY: the lock is held.
    unsafe {
        if (*o).armed {
            nt::NtSetEvent((*(*o).service).control, core::ptr::null_mut());
            return true;
        }
        let s = service_for(o);
        if s.is_null() {
            return false;
        }
        (*s).objects.push(o);
        (*o).service = s;
        (*o).armed = true;
        nt::NtSetEvent((*s).control, core::ptr::null_mut());
        true
    }
}

/// Take `o` out of its service's list. The lock is held.
unsafe fn unarm(o: *mut Object) {
    // SAFETY: the lock is held.
    unsafe {
        if !(*o).armed {
            return;
        }
        let s = (*o).service;
        (*s).objects.retain(|&m| m != o);
        (*o).armed = false;
        nt::NtSetEvent((*s).control, core::ptr::null_mut());
    }
}

/// The thread of a service.
unsafe extern "system" fn service(arg: *mut u8) -> u32 {
    let svc = arg as *mut Service;
    // SAFETY: services are never freed.
    let control = unsafe { (*svc).control };
    loop {
        lock();
        let t = now();
        let mut handles: Vec<Handle> = alloc::vec![control];
        let mut owners: Vec<(*mut Object, Handle)> = Vec::new();
        let mut next: Option<i64> = None;
        // SAFETY: the lock is held; listed objects are alive.
        unsafe {
            let list = &mut (*svc).objects;
            let mut i = 0;
            while i < list.len() {
                let o = list[i];
                let ob = &mut *o;
                if ob.kind == Kind::Timer {
                    if ob.deadline <= t {
                        queue(Job::Run(o));
                        if ob.period_ms > 0 {
                            let period = i64::from(ob.period_ms) * 10_000;
                            ob.deadline = ob.deadline.saturating_add(period);
                            if ob.deadline <= t {
                                ob.deadline = t + period;
                            }
                        } else {
                            ob.armed = false;
                            list.swap_remove(i);
                            continue;
                        }
                    }
                    next = Some(next.map_or(ob.deadline, |n| n.min(ob.deadline)));
                } else {
                    if ob.deadline != NEVER && ob.deadline <= t {
                        // Out of time: but a handle that is signalled at this moment wins.
                        let zero = 0i64;
                        let st = nt::NtWaitForSingleObject(ob.handle, 0, &zero);
                        queue(Job::Wait(o, if st.0 == 0 { rules::WAIT_OBJECT_0 } else { rules::WAIT_TIMEOUT }));
                        ob.armed = false;
                        ob.set = false;
                        list.swap_remove(i);
                        continue;
                    }
                    handles.push(ob.handle);
                    owners.push((o, ob.handle));
                    if ob.deadline != NEVER {
                        next = Some(next.map_or(ob.deadline, |n| n.min(ob.deadline)));
                    }
                }
                i += 1;
            }
        }
        unlock();
        let timeout = next.map(|d| rules::wait_timeout(d, t));
        let tp = timeout.as_ref().map_or(core::ptr::null(), |t| t as *const i64);
        // SAFETY: valid pointers; wait for any of the handles.
        let st = unsafe { nt::NtWaitForMultipleObjects(handles.len() as u32, handles.as_ptr(), 1, 0, tp) };
        match st.0 {
            0 | 0x102 => {}
            n if (1..64).contains(&n) && (n as usize) <= owners.len() => {
                let (o, h) = owners[n as usize - 1];
                lock();
                // SAFETY: the lock is held; `o` is only used if it is still in this service's list.
                unsafe {
                    if (*svc).objects.contains(&o) && (*o).armed && (*o).handle == h {
                        queue(Job::Wait(o, rules::WAIT_OBJECT_0));
                        (*o).set = false;
                        unarm(o);
                    }
                }
                unlock();
            }
            _ => {
                // A handle that cannot be waited for (closed, wrong type): those waits end.
                lock();
                for &(o, h) in &owners {
                    let zero = 0i64;
                    // SAFETY: the lock is held; `o` is checked against the list before it is used.
                    unsafe {
                        let bad = nt::NtWaitForSingleObject(h, 0, &zero).0 >= 0xC000_0000;
                        if bad && (*svc).objects.contains(&o) && (*o).armed && (*o).handle == h {
                            (*o).set = false;
                            unarm(o);
                        }
                    }
                }
                unlock();
                // SAFETY: a short sleep so a service with nothing valid does not spin.
                unsafe { nt::NtDelayExecution(0, &-10_000i64) };
            }
        }
    }
}

// ----- timers ----------------------------------------------------------------------------------------------------------

/// `CreateThreadpoolTimer(callback, context, environment)`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn CreateThreadpoolTimer(callback: usize, context: *mut u8, env: *const u8) -> *mut u8 {
    // SAFETY: the caller's environment.
    unsafe { create(Kind::Timer, callback, context as usize, env) }.cast()
}

/// `SetThreadpoolTimerEx(timer, dueTime, period, windowLength)`: whether the timer was set before.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn SetThreadpoolTimerEx(timer: *mut u8, due: *const u32, period: u32, _window: u32) -> i32 {
    let o = timer as *mut Object;
    lock();
    // SAFETY: the lock is held; the caller's timer and FILETIME.
    let was = unsafe {
        let was = (*o).set;
        if due.is_null() {
            unarm(o);
            (*o).set = false;
        } else {
            (*o).deadline = rules::deadline(filetime(due), now(), system_now());
            (*o).period_ms = period;
            (*o).set = arm(o);
        }
        was
    };
    unlock();
    i32::from(was)
}

/// `SetThreadpoolTimer`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn SetThreadpoolTimer(timer: *mut u8, due: *const u32, period: u32, window: u32) {
    // SAFETY: as above.
    unsafe { SetThreadpoolTimerEx(timer, due, period, window) };
}

/// `IsThreadpoolTimerSet`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn IsThreadpoolTimerSet(timer: *mut u8) -> i32 {
    lock();
    // SAFETY: the lock is held; the caller's timer.
    let set = unsafe { (*(timer as *mut Object)).set };
    unlock();
    i32::from(set)
}

/// `WaitForThreadpoolTimerCallbacks`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn WaitForThreadpoolTimerCallbacks(timer: *mut u8, cancel: i32) {
    // SAFETY: the caller's object.
    unsafe { wait_for_callbacks(timer as *mut Object, cancel, false) };
}

/// `CloseThreadpoolTimer`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn CloseThreadpoolTimer(timer: *mut u8) {
    // SAFETY: as above.
    unsafe { close_api(timer) };
}

// ----- waits ------------------------------------------------------------------------------------------------------------

/// `CreateThreadpoolWait(callback, context, environment)`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn CreateThreadpoolWait(callback: usize, context: *mut u8, env: *const u8) -> *mut u8 {
    // SAFETY: the caller's environment.
    unsafe { create(Kind::Wait, callback, context as usize, env) }.cast()
}

/// `SetThreadpoolWaitEx(wait, handle, timeout, reserved)`: whether the wait was set before.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn SetThreadpoolWaitEx(
    wait: *mut u8,
    handle: Handle,
    timeout: *const u32,
    _reserved: *mut u8,
) -> i32 {
    let o = wait as *mut Object;
    lock();
    // SAFETY: the lock is held; the caller's wait and FILETIME.
    let was = unsafe {
        let was = (*o).set;
        if handle == 0 {
            unarm(o);
            (*o).set = false;
        } else {
            (*o).handle = handle;
            (*o).deadline =
                if timeout.is_null() { NEVER } else { rules::deadline(filetime(timeout), now(), system_now()) };
            (*o).set = arm(o);
        }
        was
    };
    unlock();
    i32::from(was)
}

/// `SetThreadpoolWait`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn SetThreadpoolWait(wait: *mut u8, handle: Handle, timeout: *const u32) {
    // SAFETY: as above.
    unsafe { SetThreadpoolWaitEx(wait, handle, timeout, core::ptr::null_mut()) };
}

/// `WaitForThreadpoolWaitCallbacks`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn WaitForThreadpoolWaitCallbacks(wait: *mut u8, cancel: i32) {
    // SAFETY: the caller's object.
    unsafe { wait_for_callbacks(wait as *mut Object, cancel, false) };
}

/// `CloseThreadpoolWait`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn CloseThreadpoolWait(wait: *mut u8) {
    // SAFETY: as above.
    unsafe { close_api(wait) };
}

// ----- I/O -------------------------------------------------------------------------------------------------------------------

/// The thread that takes completions off the shared port.
unsafe extern "system" fn io_thread(port: *mut u8) -> u32 {
    let port = port as Handle;
    loop {
        let (mut bytes, mut key, mut ov) = (0u32, 0usize, core::ptr::null_mut());
        // SAFETY: valid out-pointers; wait forever.
        let ok =
            unsafe { crate::iocp::GetQueuedCompletionStatus(port, &mut bytes, &mut key, &mut ov, 0xFFFF_FFFF) } != 0;
        let error = if ok { 0 } else { last_error() };
        if ov.is_null() {
            // SAFETY: a short sleep so a failing port does not spin.
            unsafe { nt::NtDelayExecution(0, &-10_000i64) };
            continue;
        }
        let o = key as *mut Object;
        lock();
        // SAFETY: the lock is held; `o` is only used if the object is still listed.
        unsafe {
            let g = &mut *G.0.get();
            if g.io_objects.contains(&o) {
                if (*o).io_started > 0 {
                    (*o).io_started -= 1;
                }
                if !(*o).closed {
                    queue(Job::Io(o, ov as usize, error, bytes as usize));
                } else {
                    wake_all(&IDLE_CV);
                }
            }
        }
        unlock();
    }
}

/// `CreateThreadpoolIo(file, callback, context, environment)`: `file` is opened for overlapped I/O.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn CreateThreadpoolIo(
    file: Handle,
    callback: usize,
    context: *mut u8,
    env: *const u8,
) -> *mut u8 {
    // SAFETY: the caller's environment.
    let o = unsafe { create(Kind::Io, callback, context as usize, env) };
    lock();
    // SAFETY: the lock is held; the object was just made.
    unsafe {
        let g = &mut *G.0.get();
        if g.io_port == 0 {
            let port = crate::iocp::CreateIoCompletionPort(INVALID_HANDLE_VALUE, 0, 0, 0);
            if port == 0 {
                let e = last_error();
                close_object(o);
                unlock();
                return fail(e, core::ptr::null_mut());
            }
            let h = crate::sync::CreateThread(
                core::ptr::null(),
                0,
                io_thread as *const u8,
                port as *mut u8,
                0,
                core::ptr::null_mut(),
            );
            if h == 0 {
                let e = last_error();
                close(port);
                close_object(o);
                unlock();
                return fail(e, core::ptr::null_mut());
            }
            close(h);
            g.io_port = port;
        }
        (*o).handle = file;
        g.io_objects.push(o);
        if crate::iocp::CreateIoCompletionPort(file, g.io_port, o as usize, 0) == 0 {
            let e = last_error();
            close_object(o);
            unlock();
            return fail(e, core::ptr::null_mut());
        }
    }
    unlock();
    o.cast()
}

/// `StartThreadpoolIo`: an overlapped call is about to be made.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn StartThreadpoolIo(io: *mut u8) {
    lock();
    // SAFETY: the lock is held; the caller's object.
    unsafe { (*(io as *mut Object)).io_started += 1 };
    unlock();
}

/// `CancelThreadpoolIo`: the call did not start (or failed).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn CancelThreadpoolIo(io: *mut u8) {
    lock();
    // SAFETY: as above.
    unsafe {
        let o = io as *mut Object;
        if (*o).io_started > 0 {
            (*o).io_started -= 1;
        }
        wake_all(&IDLE_CV);
    }
    unlock();
}

/// `WaitForThreadpoolIoCallbacks(io, cancelPending)`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn WaitForThreadpoolIoCallbacks(io: *mut u8, cancel: i32) {
    // SAFETY: the caller's object.
    unsafe { wait_for_callbacks(io as *mut Object, cancel, true) };
}

/// `CloseThreadpoolIo`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn CloseThreadpoolIo(io: *mut u8) {
    // SAFETY: as above.
    unsafe { close_api(io) };
}

// ----- what a callback asks of its instance ----------------------------------------------------------------------------------

/// `CallbackMayRunLong(instance)`: TRUE while the pool has a thread to spare.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn CallbackMayRunLong(instance: *mut Instance) -> i32 {
    lock();
    // SAFETY: the lock is held; the running callback's instance and its pool.
    let ok = unsafe {
        let s = &(*(*instance).pool).state;
        s.threads < s.max || s.idle > 0
    };
    unlock();
    i32::from(ok)
}

/// `SetEventWhenCallbackReturns`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn SetEventWhenCallbackReturns(instance: *mut Instance, event: Handle) {
    // SAFETY: the running callback's instance.
    unsafe { (*instance).actions.push(Action::SetEvent(event)) };
}

/// `ReleaseSemaphoreWhenCallbackReturns`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn ReleaseSemaphoreWhenCallbackReturns(
    instance: *mut Instance,
    semaphore: Handle,
    count: u32,
) {
    // SAFETY: as above.
    unsafe { (*instance).actions.push(Action::ReleaseSemaphore(semaphore, count as i32)) };
}

/// `ReleaseMutexWhenCallbackReturns`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn ReleaseMutexWhenCallbackReturns(instance: *mut Instance, mutex: Handle) {
    // SAFETY: as above.
    unsafe { (*instance).actions.push(Action::ReleaseMutex(mutex)) };
}

/// `LeaveCriticalSectionWhenCallbackReturns`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn LeaveCriticalSectionWhenCallbackReturns(
    instance: *mut Instance,
    cs: *mut CriticalSection,
) {
    // SAFETY: as above.
    unsafe { (*instance).actions.push(Action::LeaveCriticalSection(cs)) };
}

/// `FreeLibraryWhenCallbackReturns`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn FreeLibraryWhenCallbackReturns(instance: *mut Instance, module: *mut u8) {
    // SAFETY: as above.
    unsafe { (*instance).actions.push(Action::FreeLibrary(module)) };
}

/// `DisassociateCurrentThreadFromCallback`: waits for the object's callbacks stop covering this one.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn DisassociateCurrentThreadFromCallback(instance: *mut Instance) {
    // SAFETY: the running callback's instance.
    unsafe {
        if (*instance).disassociated {
            return;
        }
        (*instance).disassociated = true;
        lock();
        (*(*instance).object).pending -= 1;
        wake_all(&IDLE_CV);
        unlock();
    }
}
