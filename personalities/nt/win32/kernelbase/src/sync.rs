//! Threads and synchronisation (Learn's `processthreadsapi.h` and
//! `synchapi.h` pages) over the NT dispatcher objects (ADR 0065) and our
//! `ntdll`'s critical sections; the SRW lock is
//! `poly_kernelbase::sync`'s.

use crate::base::*;
use crate::nt::{self, Handle, ObjectAttributes, UnicodeString};
use core::sync::atomic::AtomicUsize;
use poly_kernelbase::error::*;
use poly_kernelbase::sync::{self, *};
use poly_ntobj::NtStatus;
use poly_rtl::critsec::CriticalSection;

// ----- threads -------------------------------------------------------------------------

/// `CREATE_SUSPENDED` (`winbase.h`); `THREAD_CREATE_FLAGS_CREATE_SUSPENDED`.
const CREATE_SUSPENDED: u32 = 0x4;
/// `STACK_SIZE_PARAM_IS_A_RESERVATION` (`winbase.h`).
const STACK_SIZE_PARAM_IS_A_RESERVATION: u32 = 0x1_0000;
/// `THREAD_ALL_ACCESS` (`winnt.h`).
const THREAD_ALL_ACCESS: u32 = 0x1F_FFFF;
/// `ThreadBasicInformation`: `THREAD_BASIC_INFORMATION` is `ExitStatus`
/// (0), `TebBaseAddress` (8), `ClientId` (16: process, 24: thread),
/// `AffinityMask`, `Priority`, `BasePriority` (48 bytes).
const THREAD_BASIC_INFORMATION: u32 = 0;

/// `CreateThread`: `NtCreateThreadEx`; the thread starts in our `ntdll`
/// (`LdrInitializeThunk`, `DLL_THREAD_ATTACH`), which calls
/// `start(parameter)` and ends the thread with its result (ADR 0071).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn CreateThread(
    security: *const u8,
    stack: usize,
    start: *const u8,
    parameter: *mut u8,
    flags: u32,
    thread_id: *mut u32,
) -> Handle {
    let _ = security;
    if start.is_null() {
        return fail(ERROR_INVALID_PARAMETER, 0);
    }
    let suspended = flags & CREATE_SUSPENDED != 0;
    let (commit, reserve) = if flags & STACK_SIZE_PARAM_IS_A_RESERVATION != 0 { (0, stack) } else { (stack, 0) };
    let mut h: Handle = 0;
    // SAFETY: valid pointers; the kernel checks the rest.
    let st = unsafe {
        nt::NtCreateThreadEx(
            &mut h,
            THREAD_ALL_ACCESS,
            core::ptr::null(),
            nt::CURRENT_PROCESS,
            start,
            parameter,
            suspended as u32,
            0,
            commit,
            reserve,
            core::ptr::null(),
        )
    };
    if !st.is_success() {
        return fail_status(st, 0);
    }
    if !thread_id.is_null() {
        let mut info = [0u8; 48];
        // SAFETY: valid pointers.
        let q = unsafe {
            nt::NtQueryInformationThread(h, THREAD_BASIC_INFORMATION, info.as_mut_ptr(), 48, core::ptr::null_mut())
        };
        let id = if q.is_success() { u64::from_le_bytes(info[24..32].try_into().unwrap()) as u32 } else { 0 };
        // SAFETY: the caller's out-pointer.
        unsafe { thread_id.write(id) };
    }
    h
}

/// `ExitThread`: `DLL_THREAD_DETACH`, then the end of the thread.
#[unsafe(no_mangle)]
pub extern "system" fn ExitThread(code: u32) -> ! {
    // SAFETY: never returns.
    unsafe { nt::RtlExitUserThread(code) }
}

/// `GetExitCodeThread`: `STILL_ACTIVE` (259, the pending status) while it
/// runs.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetExitCodeThread(thread: Handle, code: *mut u32) -> i32 {
    let mut info = [0u8; 48];
    // SAFETY: valid pointers.
    let st = unsafe {
        nt::NtQueryInformationThread(thread, THREAD_BASIC_INFORMATION, info.as_mut_ptr(), 48, core::ptr::null_mut())
    };
    if !st.is_success() {
        return fail_status(st, FALSE);
    }
    // SAFETY: the caller's out-pointer.
    unsafe { code.write(u32::from_le_bytes(info[0..4].try_into().unwrap())) };
    TRUE
}

// ----- sleeping and waiting ---------------------------------------------------------------

/// `SleepEx`: Learn: returns 0 when the time elapsed, `WAIT_IO_COMPLETION`
/// when an APC cut an alertable sleep short. `Sleep(INFINITE)` never ends.
#[unsafe(no_mangle)]
pub extern "system" fn SleepEx(ms: u32, alertable: i32) -> u32 {
    let st = match sync::timeout(ms) {
        Some(t) => {
            // SAFETY: a valid pointer.
            unsafe { nt::NtDelayExecution(alertable as u8, &t) }
        }
        None => {
            let forever = i64::MIN;
            // SAFETY: as above; the most negative interval is the longest.
            unsafe { nt::NtDelayExecution(alertable as u8, &forever) }
        }
    };
    if st.0 == 0xC0 || st.0 == 0x101 { WAIT_IO_COMPLETION } else { 0 }
}

/// `Sleep`.
#[unsafe(no_mangle)]
pub extern "system" fn Sleep(ms: u32) {
    SleepEx(ms, FALSE);
}

fn wait_status(st: NtStatus) -> u32 {
    match sync::wait_result(st) {
        Ok(r) => r,
        Err(e) => fail(e, WAIT_FAILED),
    }
}

/// `WaitForSingleObjectEx`.
#[unsafe(no_mangle)]
pub extern "system" fn WaitForSingleObjectEx(handle: Handle, ms: u32, alertable: i32) -> u32 {
    let t = sync::timeout(ms);
    let tp = t.as_ref().map_or(core::ptr::null(), |t| t as *const i64);
    // SAFETY: a valid (or null) timeout pointer.
    wait_status(unsafe { nt::NtWaitForSingleObject(handle, alertable as u8, tp) })
}

/// `WaitForSingleObject`.
#[unsafe(no_mangle)]
pub extern "system" fn WaitForSingleObject(handle: Handle, ms: u32) -> u32 {
    WaitForSingleObjectEx(handle, ms, FALSE)
}

/// `WaitForMultipleObjectsEx`: Learn: 1 to `MAXIMUM_WAIT_OBJECTS` handles,
/// else `ERROR_INVALID_PARAMETER`; `bWaitAll` waits for all
/// (`WaitAllObject`, 0) or any (`WaitAnyObject`, 1).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn WaitForMultipleObjectsEx(
    count: u32,
    handles: *const Handle,
    all: i32,
    ms: u32,
    alertable: i32,
) -> u32 {
    if count == 0 || count > MAXIMUM_WAIT_OBJECTS || handles.is_null() {
        return fail(ERROR_INVALID_PARAMETER, WAIT_FAILED);
    }
    let t = sync::timeout(ms);
    let tp = t.as_ref().map_or(core::ptr::null(), |t| t as *const i64);
    let kind = if all != 0 { 0 } else { 1 };
    // SAFETY: the caller's array of `count` handles.
    wait_status(unsafe { nt::NtWaitForMultipleObjects(count, handles, kind, alertable as u8, tp) })
}

/// `WaitForMultipleObjects`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn WaitForMultipleObjects(count: u32, handles: *const Handle, all: i32, ms: u32) -> u32 {
    // SAFETY: as above.
    unsafe { WaitForMultipleObjectsEx(count, handles, all, ms, FALSE) }
}

// ----- events, mutexes, semaphores ------------------------------------------------------

/// Make a (possibly named) object with `create`, which gets the
/// `OBJECT_ATTRIBUTES` (or null for an unnamed one). Learn: creating a name
/// that exists opens it and sets `ERROR_ALREADY_EXISTS`; otherwise the last
/// error is cleared. A missing name is `ERROR_INVALID_PARAMETER`-free: a
/// null or empty name makes an unnamed object.
unsafe fn create_named(
    name: *const u16,
    security: *const u8,
    create: impl FnOnce(*mut Handle, *const ObjectAttributes) -> NtStatus,
) -> Handle {
    // SAFETY: the caller's string.
    let full = unsafe { wide_arg(name) }.and_then(sync::object_name);
    let string = full.as_ref().map(|f| UnicodeString::new(f));
    let mut attrs = nt::OBJ_OPENIF;
    // SECURITY_ATTRIBUTES.bInheritHandle at 16 (minwinbase.h).
    // SAFETY: the caller's structure when non-null.
    if !security.is_null() && unsafe { (security.add(16) as *const i32).read() } != 0 {
        attrs |= nt::OBJ_INHERIT;
    }
    let oa = ObjectAttributes::new(string.as_ref(), attrs);
    let oa_ptr = if string.is_some() || attrs & nt::OBJ_INHERIT != 0 {
        &oa as *const ObjectAttributes
    } else {
        core::ptr::null()
    };
    let mut h: Handle = 0;
    let st = create(&mut h, oa_ptr);
    if !st.is_success() {
        return fail_status(st, 0);
    }
    set_last_error(if st == nt::STATUS_OBJECT_NAME_EXISTS { ERROR_ALREADY_EXISTS } else { ERROR_SUCCESS });
    h
}

/// `CreateEventW`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn CreateEventW(security: *const u8, manual: i32, initial: i32, name: *const u16) -> Handle {
    let kind = if manual != 0 { NOTIFICATION_EVENT } else { SYNCHRONIZATION_EVENT };
    // SAFETY: valid pointers from `create_named`.
    unsafe {
        create_named(name, security, |h, oa| nt::NtCreateEvent(h, EVENT_ALL_ACCESS, oa, kind, (initial != 0) as u8))
    }
}

/// `SetEvent`.
#[unsafe(no_mangle)]
pub extern "system" fn SetEvent(event: Handle) -> i32 {
    // SAFETY: a handle value.
    let st = unsafe { nt::NtSetEvent(event, core::ptr::null_mut()) };
    if st.is_success() { TRUE } else { fail_status(st, FALSE) }
}

/// `ResetEvent`.
#[unsafe(no_mangle)]
pub extern "system" fn ResetEvent(event: Handle) -> i32 {
    // SAFETY: a handle value.
    let st = unsafe { nt::NtResetEvent(event, core::ptr::null_mut()) };
    if st.is_success() { TRUE } else { fail_status(st, FALSE) }
}

/// `CreateMutexW`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn CreateMutexW(security: *const u8, owner: i32, name: *const u16) -> Handle {
    // SAFETY: valid pointers from `create_named`.
    unsafe { create_named(name, security, |h, oa| nt::NtCreateMutant(h, MUTANT_ALL_ACCESS, oa, (owner != 0) as u8)) }
}

/// `ReleaseMutex`: a mutex the caller does not own is `ERROR_NOT_OWNER`.
#[unsafe(no_mangle)]
pub extern "system" fn ReleaseMutex(mutex: Handle) -> i32 {
    // SAFETY: a handle value.
    let st = unsafe { nt::NtReleaseMutant(mutex, core::ptr::null_mut()) };
    if st.is_success() { TRUE } else { fail_status(st, FALSE) }
}

/// `CreateSemaphoreW`: Learn: `lMaximumCount` > 0 and 0 <= `lInitialCount`
/// <= the maximum, else `ERROR_INVALID_PARAMETER`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn CreateSemaphoreW(
    security: *const u8,
    initial: i32,
    maximum: i32,
    name: *const u16,
) -> Handle {
    if !sync::valid_semaphore(initial, maximum) {
        return fail(ERROR_INVALID_PARAMETER, 0);
    }
    // SAFETY: valid pointers from `create_named`.
    unsafe {
        create_named(name, security, |h, oa| nt::NtCreateSemaphore(h, SEMAPHORE_ALL_ACCESS, oa, initial, maximum))
    }
}

/// `ReleaseSemaphore`: going over the maximum is `ERROR_TOO_MANY_POSTS`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn ReleaseSemaphore(semaphore: Handle, count: i32, previous: *mut i32) -> i32 {
    // SAFETY: a handle value and the caller's (optional) out-pointer.
    let st = unsafe { nt::NtReleaseSemaphore(semaphore, count, previous) };
    if st.is_success() { TRUE } else { fail_status(st, FALSE) }
}

// ----- critical sections (our ntdll's) ------------------------------------------------------

/// `InitializeCriticalSection`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn InitializeCriticalSection(cs: *mut CriticalSection) {
    // SAFETY: the caller's RTL_CRITICAL_SECTION.
    unsafe { nt::RtlInitializeCriticalSection(cs) };
}

/// `InitializeCriticalSectionAndSpinCount`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn InitializeCriticalSectionAndSpinCount(cs: *mut CriticalSection, spin: u32) -> i32 {
    // SAFETY: as above.
    unsafe { nt::RtlInitializeCriticalSectionAndSpinCount(cs, spin) };
    TRUE
}

/// `InitializeCriticalSectionEx`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn InitializeCriticalSectionEx(cs: *mut CriticalSection, spin: u32, flags: u32) -> i32 {
    // SAFETY: as above.
    unsafe { nt::RtlInitializeCriticalSectionEx(cs, spin, flags) };
    TRUE
}

/// `EnterCriticalSection`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn EnterCriticalSection(cs: *mut CriticalSection) {
    // SAFETY: an initialised critical section.
    unsafe { nt::RtlEnterCriticalSection(cs) };
}

/// `LeaveCriticalSection`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn LeaveCriticalSection(cs: *mut CriticalSection) {
    // SAFETY: as above.
    unsafe { nt::RtlLeaveCriticalSection(cs) };
}

/// `TryEnterCriticalSection`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn TryEnterCriticalSection(cs: *mut CriticalSection) -> i32 {
    // SAFETY: as above.
    i32::from(unsafe { nt::RtlTryEnterCriticalSection(cs) })
}

/// `DeleteCriticalSection`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn DeleteCriticalSection(cs: *mut CriticalSection) {
    // SAFETY: as above.
    unsafe { nt::RtlDeleteCriticalSection(cs) };
}

// ----- SRW locks -------------------------------------------------------------------------------

/// How a thread waits for an SRW lock: spin briefly, then yield, then sleep
/// a millisecond at a time (Poly's first SRW lock has no wait queue, ADR
/// 0076).
struct Backoff;

impl Pause for Backoff {
    fn pause(&self, attempt: u32) {
        if attempt < 64 {
            core::hint::spin_loop();
        } else if attempt < 128 {
            // SAFETY: no arguments.
            unsafe { nt::NtYieldExecution() };
        } else {
            let t = -10_000i64;
            // SAFETY: a valid pointer.
            unsafe { nt::NtDelayExecution(0, &t) };
        }
    }
}

fn word<'a>(lock: *mut usize) -> &'a AtomicUsize {
    // SAFETY: an SRWLOCK is one pointer-sized, pointer-aligned word that is
    // only accessed atomically.
    unsafe { AtomicUsize::from_ptr(lock) }
}

/// `InitializeSRWLock`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn InitializeSRWLock(lock: *mut usize) {
    // SAFETY: the caller's SRWLOCK.
    unsafe { lock.write(0) };
}

/// `AcquireSRWLockExclusive`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn AcquireSRWLockExclusive(lock: *mut usize) {
    sync::acquire_exclusive(word(lock), &Backoff);
}

/// `AcquireSRWLockShared`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn AcquireSRWLockShared(lock: *mut usize) {
    sync::acquire_shared(word(lock), &Backoff);
}

/// `ReleaseSRWLockExclusive`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn ReleaseSRWLockExclusive(lock: *mut usize) {
    sync::release_exclusive(word(lock));
}

/// `ReleaseSRWLockShared`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn ReleaseSRWLockShared(lock: *mut usize) {
    sync::release_shared(word(lock));
}

/// `TryAcquireSRWLockExclusive`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn TryAcquireSRWLockExclusive(lock: *mut usize) -> u8 {
    sync::try_acquire_exclusive(word(lock)) as u8
}

/// `TryAcquireSRWLockShared`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn TryAcquireSRWLockShared(lock: *mut usize) -> u8 {
    sync::try_acquire_shared(word(lock)) as u8
}
