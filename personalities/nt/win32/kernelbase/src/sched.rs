//! Thread and process scheduling settings and times (Chrome's browser,
//! GPU process and crash handler call them all the time; ADR 0155
//! addendum "S3 hunt"): `Set/GetThreadPriority`,
//! `Set/GetThreadPriorityBoost`, `Set/GetProcessPriorityBoost`,
//! `Set/GetPriorityClass`, `SetThreadInformation`, `GetThreadTimes`,
//! `QueryThreadCycleTime`. The rules are `poly_kernelbase::priority`'s;
//! the settings of this process are kept here (Poly's scheduler has no
//! priorities yet), a thread or process is named by its handle's id.

use crate::base::*;
use crate::nt::{self, Handle};
use poly_kernelbase::priority::{self, Priorities};

/// This process's settings (lock-free: the sampler reads them while their thread is
/// suspended). 512 threads are remembered.
static TABLE: Priorities<512> = Priorities::new();

fn with<R>(f: impl FnOnce(&Priorities<512>) -> R) -> R {
    f(&TABLE)
}

/// The id of the thread `thread` names, or the error of a bad handle
/// (`GetThreadId` sets it).
fn thread_id(thread: Handle) -> Option<u32> {
    match crate::iocp::GetThreadId(thread) {
        0 => None,
        t => Some(t),
    }
}

/// Whether `process` names this process (the last error unchanged), or
/// `None` for a bad handle (the error set).
fn own_process(process: Handle) -> Option<bool> {
    match crate::spawn::GetProcessId(process) {
        0 => None,
        p => Some(p == crate::process::GetCurrentProcessId()),
    }
}

/// `SetThreadPriority` (Learn): one of the `THREAD_PRIORITY_*` levels, or
/// `THREAD_MODE_BACKGROUND_BEGIN/END` for the calling thread.
#[unsafe(no_mangle)]
pub extern "system" fn SetThreadPriority(thread: Handle, value: i32) -> i32 {
    let Some(tid) = thread_id(thread) else { return FALSE };
    let current = tid == crate::process::GetCurrentThreadId();
    match with(|p| p.set_thread(tid, value, current)) {
        Ok(()) => TRUE,
        Err(e) => fail(e, FALSE),
    }
}

/// `GetThreadPriority` (Learn): the level set last, `THREAD_PRIORITY_NORMAL`
/// at first; `THREAD_PRIORITY_ERROR_RETURN` for a bad handle.
#[unsafe(no_mangle)]
pub extern "system" fn GetThreadPriority(thread: Handle) -> i32 {
    let Some(tid) = thread_id(thread) else { return priority::THREAD_PRIORITY_ERROR_RETURN };
    with(|p| p.thread(tid))
}

/// `SetThreadPriorityBoost` (Learn): `disable` TRUE turns dynamic boosting off.
#[unsafe(no_mangle)]
pub extern "system" fn SetThreadPriorityBoost(thread: Handle, disable: i32) -> i32 {
    let Some(tid) = thread_id(thread) else { return FALSE };
    with(|p| p.set_thread_boost(tid, disable != 0));
    TRUE
}

/// `GetThreadPriorityBoost` (Learn): `*disabled` is TRUE when boosting is off.
///
/// # Safety
/// `disabled` is a writable `BOOL` (or null: `ERROR_NOACCESS`).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetThreadPriorityBoost(thread: Handle, disabled: *mut i32) -> i32 {
    let Some(tid) = thread_id(thread) else { return FALSE };
    if disabled.is_null() {
        return fail(ERROR_NOACCESS, FALSE);
    }
    let v = with(|p| p.thread_boost(tid));
    // SAFETY: the caller's BOOL.
    unsafe { disabled.write_unaligned(i32::from(v)) };
    TRUE
}

/// `SetProcessPriorityBoost` (Learn).
#[unsafe(no_mangle)]
pub extern "system" fn SetProcessPriorityBoost(process: Handle, disable: i32) -> i32 {
    match own_process(process) {
        None => FALSE,
        Some(true) => {
            with(|p| p.set_process_boost(disable != 0));
            TRUE
        }
        // Another process keeps its own settings; Poly does not reach them.
        Some(false) => TRUE,
    }
}

/// `GetProcessPriorityBoost` (Learn).
///
/// # Safety
/// `disabled` is a writable `BOOL`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetProcessPriorityBoost(process: Handle, disabled: *mut i32) -> i32 {
    let Some(own) = own_process(process) else { return FALSE };
    if disabled.is_null() {
        return fail(ERROR_NOACCESS, FALSE);
    }
    let v = own && with(|p| p.process_boost());
    // SAFETY: the caller's BOOL.
    unsafe { disabled.write_unaligned(i32::from(v)) };
    TRUE
}

/// `SetPriorityClass` (Learn): a `*_PRIORITY_CLASS`, or
/// `PROCESS_MODE_BACKGROUND_BEGIN/END` for the calling process. Another
/// process's class is accepted and not kept (its settings are its own).
#[unsafe(no_mangle)]
pub extern "system" fn SetPriorityClass(process: Handle, class: u32) -> i32 {
    let Some(own) = own_process(process) else { return FALSE };
    let r = if own { with(|p| p.set_class(class, true)) } else { Priorities::<1>::new().set_class(class, false) };
    match r {
        Ok(()) => TRUE,
        Err(e) => fail(e, FALSE),
    }
}

/// `GetPriorityClass` (Learn): 0 on failure; `NORMAL_PRIORITY_CLASS` for
/// another process.
#[unsafe(no_mangle)]
pub extern "system" fn GetPriorityClass(process: Handle) -> u32 {
    match own_process(process) {
        None => 0,
        Some(true) => with(|p| p.class()),
        Some(false) => priority::NORMAL_PRIORITY_CLASS,
    }
}

/// `SetThreadInformation` (Learn): memory priority, absolute CPU priority,
/// dynamic code policy, power throttling; checked and accepted (Poly has no
/// such policies yet).
///
/// # Safety
/// `info` holds `size` readable bytes.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn SetThreadInformation(thread: Handle, class: u32, info: *const u8, size: u32) -> i32 {
    if thread_id(thread).is_none() {
        return FALSE;
    }
    if info.is_null() {
        return fail(ERROR_NOACCESS, FALSE);
    }
    // SAFETY: the caller's structure, at least one ULONG for every class.
    let first = (size >= 4).then(|| unsafe { (info as *const u32).read_unaligned() });
    match priority::check_thread_information(class, size, first) {
        Ok(()) => TRUE,
        Err(e) => fail(e, FALSE),
    }
}

/// `ThreadTimes` (`ntddk.h`'s `THREADINFOCLASS`): `KERNEL_USER_TIMES`.
const THREAD_TIMES: u32 = 1;

/// The thread's creation, exit, kernel and user times (`KERNEL_USER_TIMES`).
fn thread_times(thread: Handle) -> Option<[u64; 4]> {
    let mut b = [0u8; 32];
    // SAFETY: a valid buffer of the class's size.
    let st = unsafe { nt::NtQueryInformationThread(thread, THREAD_TIMES, b.as_mut_ptr(), 32, core::ptr::null_mut()) };
    if !st.is_success() {
        return fail_status(st, None);
    }
    let q = |i: usize| u64::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3], b[i + 4], b[i + 5], b[i + 6], b[i + 7]]);
    Some([q(0), q(8), q(16), q(24)])
}

/// `GetThreadTimes` (Learn): the four `FILETIME`s of
/// `NtQueryInformationThread(ThreadTimes)`.
///
/// # Safety
/// The four pointers are writable `FILETIME`s.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetThreadTimes(
    thread: Handle,
    creation: *mut u8,
    exit: *mut u8,
    kernel: *mut u8,
    user: *mut u8,
) -> i32 {
    let Some(times) = thread_times(thread) else { return FALSE };
    for (t, out) in times.into_iter().zip([creation, exit, kernel, user]) {
        if out.is_null() {
            return fail(ERROR_NOACCESS, FALSE);
        }
        // SAFETY: the caller's FILETIME (unaligned).
        unsafe { core::ptr::copy_nonoverlapping(t.to_le_bytes().as_ptr(), out, 8) };
    }
    TRUE
}

/// `QueryThreadCycleTime` (Learn: "the number of CPU clock cycles used by
/// the threads"): Poly counts time, not cycles, so the kernel and user time
/// in 100 ns units stands in for it (a count that only grows while the
/// thread runs, which is what callers compare).
///
/// # Safety
/// `cycles` is a writable `ULONG64`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn QueryThreadCycleTime(thread: Handle, cycles: *mut u64) -> i32 {
    let Some(times) = thread_times(thread) else { return FALSE };
    if cycles.is_null() {
        return fail(ERROR_NOACCESS, FALSE);
    }
    // SAFETY: the caller's ULONG64.
    unsafe { cycles.write_unaligned(times[2].saturating_add(times[3])) };
    TRUE
}

/// `ERROR_NOACCESS` (`winerror.h`).
const ERROR_NOACCESS: u32 = 998;

/// `ProcessVmCounters` (`ntddk.h`'s `PROCESSINFOCLASS`).
const PROCESS_VM_COUNTERS: u32 = 3;

/// `K32GetProcessMemoryInfo` (Learn; `GetProcessMemoryInfo` in `psapi.h` maps to it): the
/// process's working set, its peak and its private usage, from
/// `NtQueryInformationProcess(ProcessVmCounters)`; `PROCESS_MEMORY_COUNTERS`, `_EX` or `_EX2` as
/// `cb` asks (`poly_kernelbase::procinfo::process_memory_counters`).
///
/// # Safety
/// `counters` holds `cb` writable bytes.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn K32GetProcessMemoryInfo(process: Handle, counters: *mut u8, cb: u32) -> i32 {
    let mut vm = [0u8; 96];
    // SAFETY: a valid buffer of the size passed.
    let st = unsafe {
        nt::NtQueryInformationProcess(process, PROCESS_VM_COUNTERS, vm.as_mut_ptr(), 96, core::ptr::null_mut())
    };
    if !st.is_success() {
        return fail_status(st, FALSE);
    }
    match poly_kernelbase::procinfo::process_memory_counters(&vm, cb, core::mem::size_of::<usize>()) {
        Ok(b) => {
            if counters.is_null() {
                return fail(ERROR_NOACCESS, FALSE);
            }
            // SAFETY: `b` is at most `cb` bytes, the caller's structure.
            unsafe { core::ptr::copy_nonoverlapping(b.as_ptr(), counters, b.len()) };
            TRUE
        }
        Err(e) => fail(e, FALSE),
    }
}
