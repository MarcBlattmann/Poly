//! `ntdll`'s thread synchronisation exports: SRW locks, condition variables, run-once and
//! `RtlWaitOnAddress`, over [`poly_rtl::sync`] (the logic and its host tests) and this process's
//! events. `kernel32` and `kernelbase` call these same exports, so there is one lock
//! implementation in the process (Learn: "Slim Reader/Writer (SRW) Locks", "Condition
//! Variables", "One-Time Initialization").

use crate::NtStatus;
use crate::syscalls::*;
use core::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use poly_rtl::critsec::CriticalSection;
use poly_rtl::sync::{self as rs, Lot, Sys};

/// Every thread of the process parks here.
static LOT: Lot = Lot::new();

/// `EVENT_ALL_ACCESS` (`winnt.h`).
const EVENT_ALL_ACCESS: u32 = 0x1F_0003;
/// `SynchronizationEvent` (`wdm.h`).
const SYNCHRONIZATION_EVENT: u32 = 1;
/// `STATUS_TIMEOUT`.
const STATUS_TIMEOUT: u32 = 0x102;

/// Events handed back by finished waits: a blocking wait takes one instead of making a new one.
static POOL: [AtomicU64; 32] = [const { AtomicU64::new(0) }; 32];

struct Host;

impl Sys for Host {
    fn get_event(&self) -> u64 {
        for slot in &POOL {
            let h = slot.swap(0, Ordering::AcqRel);
            if h != 0 {
                return h;
            }
        }
        let mut h: isize = 0;
        // SAFETY: a valid out-pointer; a synchronisation (auto-reset) event, not signalled.
        let st = unsafe { NtCreateEvent(&mut h, EVENT_ALL_ACCESS, core::ptr::null(), SYNCHRONIZATION_EVENT, 0) };
        if st.is_success() { h as u64 } else { 0 }
    }

    fn put_event(&self, event: u64) {
        for slot in &POOL {
            if slot.compare_exchange(0, event, Ordering::AcqRel, Ordering::Relaxed).is_ok() {
                return;
            }
        }
        // SAFETY: a handle this module made.
        unsafe { NtClose(event as isize) };
    }

    fn wait(&self, event: u64, timeout: Option<i64>) -> bool {
        let t = timeout.unwrap_or(0);
        let p = if timeout.is_some() { &t as *const i64 } else { core::ptr::null() };
        // SAFETY: a handle this module made; `p` is null or valid.
        unsafe { NtWaitForSingleObject(event as isize, 0, p) }.0 != STATUS_TIMEOUT
    }

    fn set(&self, event: u64) {
        // SAFETY: a handle this module made.
        unsafe { NtSetEvent(event as isize, core::ptr::null_mut()) };
    }

    fn yield_now(&self) {
        // SAFETY: no arguments.
        unsafe { NtYieldExecution() };
    }
}

/// An `RTL_SRWLOCK` (or condition variable, run-once object): one pointer-sized word the caller
/// owns, only ever accessed atomically.
unsafe fn word<'a>(p: *mut usize) -> &'a AtomicUsize {
    // SAFETY: pointer-sized and aligned, as `winnt.h` declares them.
    unsafe { AtomicUsize::from_ptr(p) }
}

fn timeout(p: *const i64) -> Option<i64> {
    if p.is_null() {
        None
    } else {
        // SAFETY: the caller's `PLARGE_INTEGER`.
        Some(unsafe { p.read_unaligned() })
    }
}

/// Raise `code` in the calling thread, as `RtlRaiseStatus` does: a non-continuable exception.
fn raise(code: u32) {
    /// `EXCEPTION_RECORD` (`winnt.h`).
    #[repr(C)]
    struct Record {
        code: u32,
        flags: u32,
        record: usize,
        address: usize,
        parameters: u32,
        information: [usize; 15],
    }
    let mut r = Record { code, flags: 1, record: 0, address: 0, parameters: 0, information: [0; 15] };
    #[cfg(target_arch = "x86_64")]
    // SAFETY: a valid record.
    unsafe {
        crate::seh::RtlRaiseException((&mut r as *mut Record).cast());
    }
    #[cfg(target_arch = "x86")]
    {
        unsafe extern "system" {
            fn RtlRaiseException(record: *mut u8);
        }
        // SAFETY: a valid record.
        unsafe { RtlRaiseException((&mut r as *mut Record).cast()) };
    }
}

// ----- SRW locks ---------------------------------------------------------------------------

#[unsafe(no_mangle)]
pub unsafe extern "system" fn RtlInitializeSRWLock(lock: *mut usize) {
    // SAFETY: the caller's lock.
    unsafe { lock.write(0) };
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn RtlAcquireSRWLockExclusive(lock: *mut usize) {
    // SAFETY: the caller's lock.
    LOT.acquire_exclusive(&Host, unsafe { word(lock) });
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn RtlAcquireSRWLockShared(lock: *mut usize) {
    // SAFETY: as above.
    LOT.acquire_shared(&Host, unsafe { word(lock) });
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn RtlReleaseSRWLockExclusive(lock: *mut usize) {
    // SAFETY: as above.
    LOT.release_exclusive(&Host, unsafe { word(lock) });
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn RtlReleaseSRWLockShared(lock: *mut usize) {
    // SAFETY: as above.
    if let Err(status) = LOT.release_shared(&Host, unsafe { word(lock) }) {
        raise(status);
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn RtlTryAcquireSRWLockExclusive(lock: *mut usize) -> u8 {
    // SAFETY: as above.
    LOT.try_acquire_exclusive(unsafe { word(lock) }) as u8
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn RtlTryAcquireSRWLockShared(lock: *mut usize) -> u8 {
    // SAFETY: as above.
    LOT.try_acquire_shared(unsafe { word(lock) }) as u8
}

// ----- condition variables -------------------------------------------------------------------

#[unsafe(no_mangle)]
pub unsafe extern "system" fn RtlInitializeConditionVariable(cv: *mut usize) {
    // SAFETY: the caller's variable.
    unsafe { cv.write(0) };
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn RtlWakeConditionVariable(cv: *mut usize) {
    // SAFETY: the caller's variable.
    LOT.wake_one(&Host, unsafe { word(cv) });
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn RtlWakeAllConditionVariable(cv: *mut usize) {
    // SAFETY: as above.
    LOT.wake_all(&Host, unsafe { word(cv) });
}

/// `RtlSleepConditionVariableCS(cv, cs, timeout)`: leaves the section once, sleeps, enters it
/// once again.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn RtlSleepConditionVariableCS(
    cv: *mut usize,
    cs: *mut CriticalSection,
    time: *const i64,
) -> NtStatus {
    // SAFETY: the caller's variable and critical section, which it owns.
    let woken = LOT.sleep(
        &Host,
        unsafe { word(cv) },
        timeout(time),
        &mut || unsafe {
            crate::exports::RtlLeaveCriticalSection(cs);
        },
        &mut || unsafe {
            crate::exports::RtlEnterCriticalSection(cs);
        },
    );
    NtStatus(if woken { 0 } else { STATUS_TIMEOUT })
}

/// `RtlSleepConditionVariableSRW(cv, lock, timeout, flags)`: `flags` is 0 (exclusive) or
/// `RTL_CONDITION_VARIABLE_LOCKMODE_SHARED`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn RtlSleepConditionVariableSRW(
    cv: *mut usize,
    lock: *mut usize,
    time: *const i64,
    flags: u32,
) -> NtStatus {
    if flags & !rs::CONDITION_VARIABLE_LOCKMODE_SHARED != 0 {
        return NtStatus(rs::STATUS_INVALID_PARAMETER_2);
    }
    let shared = flags & rs::CONDITION_VARIABLE_LOCKMODE_SHARED != 0;
    // SAFETY: the caller's objects.
    let (cv, l) = unsafe { (word(cv), word(lock)) };
    match LOT.sleep_srw(&Host, cv, l, timeout(time), shared) {
        Ok(woken) => NtStatus(if woken { 0 } else { STATUS_TIMEOUT }),
        Err(status) => {
            raise(status);
            NtStatus(status)
        }
    }
}

// ----- run-once ----------------------------------------------------------------------------------

/// `PRTL_RUN_ONCE_INIT_FN`.
type RunOnceFn = unsafe extern "system" fn(*mut usize, *mut u8, *mut *mut u8) -> u32;

#[unsafe(no_mangle)]
pub unsafe extern "system" fn RtlRunOnceInitialize(once: *mut usize) {
    // SAFETY: the caller's object.
    unsafe { once.write(0) };
}

/// `RtlRunOnceBeginInitialize`: `STATUS_PENDING` when the caller must initialise.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn RtlRunOnceBeginInitialize(
    once: *mut usize,
    flags: u32,
    context: *mut *mut u8,
) -> NtStatus {
    // SAFETY: the caller's object.
    match LOT.run_once_begin(&Host, unsafe { word(once) }, flags) {
        Ok(Some(c)) => {
            if !context.is_null() {
                // SAFETY: the caller's out-pointer.
                unsafe { context.write(c as *mut u8) };
            }
            NtStatus(0)
        }
        Ok(None) => NtStatus(rs::STATUS_PENDING),
        Err(status) => NtStatus(status),
    }
}

/// `RtlRunOnceComplete`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn RtlRunOnceComplete(once: *mut usize, flags: u32, context: *mut u8) -> NtStatus {
    // SAFETY: the caller's object.
    match LOT.run_once_complete(&Host, unsafe { word(once) }, flags, context as usize) {
        Ok(()) => NtStatus(0),
        Err(status) => NtStatus(status),
    }
}

/// `RtlRunOnceExecuteOnce(once, function, parameter, context)`: run `function` unless an earlier
/// call completed the object; every caller returns after the initialisation has finished, with
/// its context. A callback that returns 0 gives `STATUS_UNSUCCESSFUL` (and the next caller runs
/// it again); a context with a low bit set raises `STATUS_INVALID_PARAMETER_3`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn RtlRunOnceExecuteOnce(
    once: *mut usize,
    function: Option<RunOnceFn>,
    parameter: *mut u8,
    context: *mut *mut u8,
) -> NtStatus {
    // SAFETY: the caller's object.
    let w = unsafe { word(once) };
    let Some(function) = function else { return NtStatus(0xC000_000D) };
    match LOT.run_once_begin(&Host, w, 0) {
        Ok(Some(c)) => {
            if !context.is_null() {
                // SAFETY: the caller's out-pointer.
                unsafe { context.write(c as *mut u8) };
            }
            NtStatus(0)
        }
        Ok(None) => {
            let mut c: *mut u8 = core::ptr::null_mut();
            // SAFETY: the caller's callback with its documented arguments.
            let ok = unsafe { function(once, parameter, &mut c) } != 0;
            if !context.is_null() {
                // SAFETY: the caller's out-pointer.
                unsafe { context.write(c) };
            }
            if !ok {
                let _ = LOT.run_once_complete(&Host, w, rs::RUN_ONCE_INIT_FAILED, 0);
                return NtStatus(rs::STATUS_UNSUCCESSFUL);
            }
            match LOT.run_once_complete(&Host, w, 0, c as usize) {
                Ok(()) => NtStatus(0),
                Err(status) => {
                    // A bad context: the object goes back to not started, then it is raised.
                    let _ = LOT.run_once_complete(&Host, w, rs::RUN_ONCE_INIT_FAILED, 0);
                    raise(status);
                    NtStatus(status)
                }
            }
        }
        Err(status) => NtStatus(status),
    }
}

// ----- WaitOnAddress ---------------------------------------------------------------------------------

/// `RtlWaitOnAddress(address, compare, size, timeout)`: sleeps while the 1, 2, 4 or 8 bytes at
/// `address` equal those at `compare`; `STATUS_TIMEOUT` when the time ran out.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn RtlWaitOnAddress(
    address: *const u8,
    compare: *const u8,
    size: usize,
    time: *const i64,
) -> NtStatus {
    if !matches!(size, 1 | 2 | 4 | 8) || address.is_null() || compare.is_null() || address as usize & (size - 1) != 0 {
        return NtStatus(0xC000_000D);
    }
    // SAFETY: the caller's pointers, valid for `size` bytes.
    let woken = unsafe { LOT.wait_on_address(&Host, address, compare, size, timeout(time)) };
    NtStatus(if woken { 0 } else { STATUS_TIMEOUT })
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn RtlWakeAddressSingle(address: *const u8) {
    LOT.wake_address(&Host, address, false);
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn RtlWakeAddressAll(address: *const u8) {
    LOT.wake_address(&Host, address, true);
}
