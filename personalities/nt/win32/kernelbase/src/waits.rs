//! Registered waits, thread yielding and thread descriptions (Chrome batch
//! 2, ADR 0155): Learn's `RegisterWaitForSingleObject`, `UnregisterWait`,
//! `UnregisterWaitEx`, `SwitchToThread`, `SetThreadDescription` and
//! `GetThreadDescription` pages. The rules are `poly_kernelbase::waitreg`'s.
//!
//! Poly has no thread pool in its `ntdll` yet, so each registered wait is
//! one thread of `kernelbase` that waits for the object and for a
//! cancellation event, runs the callback, and (unless
//! `WT_EXECUTEONLYONCE`) waits again. Learn only promises that the callback
//! runs on some thread of the pool, never on a particular one.

use crate::base::*;
use crate::nt::{self, Handle};
use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::vec::Vec;
use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicBool, AtomicIsize, AtomicU32, Ordering};
use poly_kernelbase::error::*;
use poly_kernelbase::sync;
use poly_kernelbase::waitreg::{self, Completion};
use poly_rtl::critsec::CriticalSection;

/// `WaitOrTimerCallback` (`winnt.h`): `(context, timer_or_wait_fired)`.
type WaitCallback = unsafe extern "system" fn(*mut u8, u8);

/// `STATUS_WAIT_1`, `STATUS_TIMEOUT` (`ntstatus.h`).
const STATUS_WAIT_1: u32 = 1;
const STATUS_TIMEOUT: u32 = 0x102;

/// One registered wait. Two references: the caller's (until
/// `UnregisterWaitEx`) and the thread's (until it ends).
struct Wait {
    object: Handle,
    cancel: Handle,
    callback: usize,
    context: usize,
    spec: waitreg::WaitSpec,
    thread: AtomicIsize,
    thread_id: AtomicU32,
    running: AtomicU32,
    cancelled: AtomicBool,
    /// A completion event to signal when the running callback ends.
    event: AtomicIsize,
    refs: AtomicU32,
}

impl Wait {
    /// Signal the completion event, if one is waiting to be.
    fn signal_completion(&self) {
        let ev = self.event.swap(0, Ordering::AcqRel);
        if ev != 0 && ev != -1 {
            // SAFETY: the caller's event handle.
            unsafe { nt::NtSetEvent(ev, core::ptr::null_mut()) };
        }
    }
}

/// Drop one reference; the last one frees the wait.
///
/// # Safety
/// `w` came from `Box::into_raw` and the caller owns one reference.
unsafe fn release(w: *const Wait) {
    // SAFETY: a live wait while a reference is held.
    if unsafe { (*w).refs.fetch_sub(1, Ordering::AcqRel) } == 1 {
        // SAFETY: the last reference.
        let b = unsafe { Box::from_raw(w as *mut Wait) };
        close(b.cancel);
        let t = b.thread.load(Ordering::Acquire);
        if t != 0 {
            close(t);
        }
    }
}

/// The thread of a wait: wait for the object, the cancellation or the
/// timeout; run the callback.
unsafe extern "system" fn wait_thread(param: *mut u8) -> u32 {
    let w = param as *const Wait;
    // SAFETY: the thread holds a reference.
    let wr = unsafe { &*w };
    wr.thread_id.store(crate::process::GetCurrentThreadId(), Ordering::Release);
    loop {
        let handles = [wr.object, wr.cancel];
        let t = sync::timeout(wr.spec.timeout_ms);
        let tp = t.as_ref().map_or(core::ptr::null(), |t| t as *const i64);
        // SAFETY: valid pointers; wait for any of the two.
        let st = unsafe { nt::NtWaitForMultipleObjects(2, handles.as_ptr(), 1, 0, tp) };
        if wr.cancelled.load(Ordering::Acquire) || st.0 == STATUS_WAIT_1 {
            break;
        }
        let timed_out = st.0 == STATUS_TIMEOUT;
        if !timed_out && st.0 != 0 {
            // The object is not waitable (closed, wrong type): the wait ends.
            break;
        }
        wr.running.fetch_add(1, Ordering::AcqRel);
        // SAFETY: the callback the program registered.
        unsafe { core::mem::transmute::<usize, WaitCallback>(wr.callback)(wr.context as *mut u8, u8::from(timed_out)) };
        wr.running.fetch_sub(1, Ordering::AcqRel);
        if wr.cancelled.load(Ordering::Acquire) {
            wr.signal_completion();
            break;
        }
        if wr.spec.once {
            // Done; the registration lives until it is unregistered.
            break;
        }
    }
    wr.signal_completion();
    // SAFETY: this thread's reference.
    unsafe { release(w) };
    0
}

/// `RegisterWaitForSingleObject`: the registered wait is the pointer the
/// call returns in `*phNewWaitObject`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn RegisterWaitForSingleObject(
    new_wait: *mut Handle,
    object: Handle,
    callback: Option<WaitCallback>,
    context: *mut u8,
    ms: u32,
    flags: u32,
) -> i32 {
    if new_wait.is_null() {
        return fail(ERROR_INVALID_PARAMETER, FALSE);
    }
    let spec = match waitreg::wait_spec(callback.is_none(), ms, flags) {
        Ok(s) => s,
        Err(e) => return fail(e, FALSE),
    };
    let mut cancel: Handle = 0;
    // SAFETY: valid pointers; a manual-reset event, not signaled.
    let st = unsafe {
        nt::NtCreateEvent(&mut cancel, sync::EVENT_ALL_ACCESS, core::ptr::null(), sync::NOTIFICATION_EVENT, 0)
    };
    if !st.is_success() {
        return fail_status(st, FALSE);
    }
    let w = Box::into_raw(Box::new(Wait {
        object,
        cancel,
        callback: callback.map_or(0, |c| c as usize),
        context: context as usize,
        spec,
        thread: AtomicIsize::new(0),
        thread_id: AtomicU32::new(0),
        running: AtomicU32::new(0),
        cancelled: AtomicBool::new(false),
        event: AtomicIsize::new(0),
        refs: AtomicU32::new(2),
    }));
    // SAFETY: a valid entry point; the thread gets `w`.
    let thread = unsafe {
        crate::sync::CreateThread(
            core::ptr::null(),
            0,
            wait_thread as *const u8,
            w as *mut u8,
            0,
            core::ptr::null_mut(),
        )
    };
    if thread == 0 {
        let e = last_error();
        // SAFETY: both references (the thread never started).
        unsafe {
            release(w);
            release(w);
        }
        return fail(e, FALSE);
    }
    // SAFETY: `w` is live while the caller's reference is held.
    unsafe { (*w).thread.store(thread, Ordering::Release) };
    // SAFETY: the caller's out-pointer.
    unsafe { new_wait.write(w as Handle) };
    TRUE
}

/// `UnregisterWaitEx`: Learn: cancels the wait; see
/// `poly_kernelbase::waitreg::unregister` for what the result is.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn UnregisterWaitEx(wait: Handle, completion_event: Handle) -> i32 {
    if wait == 0 || wait == INVALID_HANDLE_VALUE || wait & 7 != 0 {
        return fail(ERROR_INVALID_HANDLE, FALSE);
    }
    let w = wait as *const Wait;
    // SAFETY: the caller's registration; it holds our first reference.
    let wr = unsafe { &*w };
    let completion = waitreg::completion(completion_event);
    if completion == Completion::Event {
        wr.event.store(completion_event, Ordering::Release);
    }
    wr.cancelled.store(true, Ordering::Release);
    // SAFETY: our event.
    unsafe { nt::NtSetEvent(wr.cancel, core::ptr::null_mut()) };
    let from_callback = wr.thread_id.load(Ordering::Acquire) == crate::process::GetCurrentThreadId();
    let outcome = waitreg::unregister(completion, wr.running.load(Ordering::Acquire) != 0, from_callback);
    if outcome.block {
        // SAFETY: the thread handle stays open until the last release.
        unsafe { nt::NtWaitForSingleObject(wr.thread.load(Ordering::Acquire), 0, core::ptr::null()) };
    }
    if !outcome.signal_later && completion == Completion::Event {
        // Nothing to wait for: the event is not signaled.
        let _ = wr.event.compare_exchange(completion_event, 0, Ordering::AcqRel, Ordering::Acquire);
    }
    // SAFETY: the caller's reference.
    unsafe { release(w) };
    if outcome.ok { TRUE } else { fail(outcome.error, FALSE) }
}

/// `UnregisterWait`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn UnregisterWait(wait: Handle) -> i32 {
    // SAFETY: as above.
    unsafe { UnregisterWaitEx(wait, 0) }
}

/// `SwitchToThread`: Learn: nonzero if the call yielded to another thread.
#[unsafe(no_mangle)]
pub extern "system" fn SwitchToThread() -> i32 {
    // SAFETY: a service without arguments. STATUS_NO_YIELD_PERFORMED
    // (0x40000024) means nothing else was ready.
    let st = unsafe { nt::NtYieldExecution() };
    i32::from(st.0 != 0x4000_0024)
}

// ----- thread descriptions -------------------------------------------------------------------

/// Descriptions by thread ID. Poly's kernel has no thread name; they live
/// here until the thread ends (`thread_detach`).
struct Names {
    lock: CriticalSection,
    map: UnsafeCell<BTreeMap<u32, Vec<u16>>>,
}

// SAFETY: `map` is only touched with `lock` held.
unsafe impl Sync for Names {}

static NAMES: Names = Names { lock: CriticalSection::new(), map: UnsafeCell::new(BTreeMap::new()) };

fn with_names<R>(f: impl FnOnce(&mut BTreeMap<u32, Vec<u16>>) -> R) -> R {
    let cs = &NAMES.lock as *const CriticalSection as *mut CriticalSection;
    // SAFETY: a static critical section.
    unsafe { nt::RtlEnterCriticalSection(cs) };
    // SAFETY: the lock gives exclusive access.
    let r = f(unsafe { &mut *NAMES.map.get() });
    // SAFETY: entered above.
    unsafe { nt::RtlLeaveCriticalSection(cs) };
    r
}

/// The thread ID of a thread handle.
fn thread_id(thread: Handle) -> Result<u32, u32> {
    let mut info = [0u8; 48];
    // SAFETY: valid pointers; ThreadBasicInformation (0) is 48 bytes.
    let st = unsafe { nt::NtQueryInformationThread(thread, 0, info.as_mut_ptr(), 48, core::ptr::null_mut()) };
    if st.is_success() { Ok(u64::from_le_bytes(info[24..32].try_into().unwrap()) as u32) } else { Err(from_status(st)) }
}

/// `HRESULT_FROM_WIN32`.
fn hresult(error: u32) -> i32 {
    if error == 0 { 0 } else { ((error & 0xFFFF) | 0x8007_0000) as i32 }
}

/// `MAX_PATH`-independent limit Learn gives for a description:
/// 32,767 characters.
const MAX_DESCRIPTION: usize = 32767;

/// `SetThreadDescription`: an `HRESULT`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn SetThreadDescription(thread: Handle, description: *const u16) -> i32 {
    // SAFETY: the caller's string.
    let Some(text) = (unsafe { wide_arg(description) }) else { return hresult(ERROR_INVALID_PARAMETER) };
    if text.len() > MAX_DESCRIPTION {
        return hresult(ERROR_INVALID_PARAMETER);
    }
    match thread_id(thread) {
        Ok(id) => {
            with_names(|m| {
                if text.is_empty() {
                    m.remove(&id);
                } else {
                    m.insert(id, text.to_vec());
                }
            });
            0
        }
        Err(e) => hresult(e),
    }
}

/// `GetThreadDescription`: the description in memory the caller frees with
/// `LocalFree` (an empty string if none was set).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetThreadDescription(thread: Handle, out: *mut *mut u16) -> i32 {
    if out.is_null() {
        return hresult(ERROR_INVALID_PARAMETER);
    }
    let id = match thread_id(thread) {
        Ok(i) => i,
        Err(e) => return hresult(e),
    };
    let text = with_names(|m| m.get(&id).cloned().unwrap_or_default());
    let p = crate::memory::LocalAlloc(0, (text.len() + 1) * 2) as *mut u16;
    if p.is_null() {
        return hresult(ERROR_NOT_ENOUGH_MEMORY);
    }
    // SAFETY: room for the text and a NUL; the caller's out-pointer.
    unsafe {
        put_wide(p, &text);
        out.write(p);
    }
    0
}

/// Forget the description of the exiting thread.
pub fn thread_detach() {
    let id = crate::process::GetCurrentThreadId();
    with_names(|m| {
        m.remove(&id);
    });
}
