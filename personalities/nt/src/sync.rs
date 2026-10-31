//! The dispatcher services: mutants, semaphores, waitable timers, the waits
//! and user APCs. Arguments copied in, `poly-ntobj`'s dispatcher asked for
//! the decision, and the blocking left to the kernel ([`NtKernel::wait`],
//! [`NtKernel::delay`], [`NtKernel::arm_timer`]).
//!
//! The object manager ([`poly_ntobj`], ADR 0030) already decides which wait a
//! signal satisfies, mutant ownership and abandonment, semaphore limits and
//! the status each wait returns; these services only decode the ABI and,
//! when a wait cannot be satisfied at once, hand the blocking to the kernel.

use crate::abi;
use crate::consts::*;
use crate::kernel::{Apc, NtKernel, NtTimeout};
use crate::objects::{Result, return_handle};
use alloc::vec::Vec;
use poly_abi::SyscallRegs;
use poly_ntobj::access::{
    MUTANT_QUERY_STATE, SEMAPHORE_QUERY_STATE, SYNCHRONIZE, THREAD_SET_CONTEXT, TIMER_MODIFY_STATE, TIMER_QUERY_STATE,
};
use poly_ntobj::status::*;
use poly_ntobj::{EventType, Handle, NtStatus, ObjectType, WaitOutcome, WaitType};

/// `(handle*, access, attributes*)`: the three arguments every create and
/// open service starts with (as in [`crate::objects`]).
#[inline(never)]
fn open_args<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Result<(u64, u32, crate::abi::Attributes)> {
    let [out, access, attrs] = abi::args::<K, 3>(k, regs)?;
    let attrs = abi::read_attributes(k, attrs)?;
    Ok((out, access as u32, attrs))
}

/// Read a `LARGE_INTEGER*` timeout (null: forever; negative: relative;
/// non-negative: absolute 100 ns units since 1601), as the wait and delay
/// services take it (Learn: "NtWaitForSingleObject").
pub fn read_timeout<K: NtKernel>(k: &mut K, ptr: u64) -> Result<NtTimeout> {
    if ptr == 0 {
        return Ok(NtTimeout::Infinite);
    }
    let raw = abi::read_u64(k, ptr)? as i64;
    Ok(if raw < 0 { NtTimeout::Relative((-(raw as i128)) as u64) } else { NtTimeout::Absolute(raw) })
}

// ----- events (the query/pulse/clear that join the create/open/set/reset of
// `crate::objects`)

pub fn pulse_event<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Result<NtStatus> {
    let [h, prev] = abi::args::<K, 2>(k, regs)?;
    let caller = k.caller();
    let was = k.with_state(|sys, _, _| sys.objects.nt_pulse_event(&caller, Handle(h)))?;
    abi::write_opt_u32(k, prev, was as u32)?;
    Ok(STATUS_SUCCESS)
}

pub fn clear_event<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Result<NtStatus> {
    let h = Handle(abi::arg(k, regs, 0)?);
    let caller = k.caller();
    k.with_state(|sys, _, _| sys.objects.nt_clear_event(&caller, h))?;
    Ok(STATUS_SUCCESS)
}

/// `NtQueryEvent(EventBasicInformation)`: `EVENT_TYPE` (0 notification, 1
/// synchronization) then the signal state, two `ULONG`s.
pub fn query_event<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Result<NtStatus> {
    let [h, class, buf, length, ret] = abi::args::<K, 5>(k, regs)?;
    if class as u32 != 0 {
        return Err(STATUS_INVALID_INFO_CLASS);
    }
    let caller = k.caller();
    let info = k.with_state(|sys, _, _| sys.objects.nt_query_event(&caller, Handle(h)))?;
    let mut b = [0u8; 8];
    let kind = if info.kind == EventType::Notification { 0u32 } else { 1 };
    b[0..4].copy_from_slice(&kind.to_le_bytes());
    b[4..8].copy_from_slice(&(info.signaled as u32).to_le_bytes());
    write_info(k, buf, length, ret, &b)
}

// ----- mutants

pub fn create_mutant<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Result<NtStatus> {
    let (out, access, attrs) = open_args(k, regs)?;
    let initial = abi::arg(k, regs, 3)? as u8 != 0;
    let caller = k.caller();
    let c = k.with_state(|sys, _, _| {
        sys.objects.nt_create_mutant(&caller, &attrs.as_object_attributes(), access, initial)
    })?;
    return_handle(k, out, c.handle)?;
    Ok(c.status)
}

pub fn open_mutant<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Result<NtStatus> {
    let (out, access, attrs) = open_args(k, regs)?;
    let caller = k.caller();
    let h = k.with_state(|sys, _, _| sys.objects.nt_open_mutant(&caller, &attrs.as_object_attributes(), access))?;
    return_handle(k, out, h)?;
    Ok(STATUS_SUCCESS)
}

pub fn release_mutant<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Result<NtStatus> {
    let [h, prev] = abi::args::<K, 2>(k, regs)?;
    let caller = k.caller();
    let was = k.with_state(|sys, _, _| sys.objects.nt_release_mutant(&caller, Handle(h)))?;
    abi::write_opt_u32(k, prev, was)?;
    Ok(STATUS_SUCCESS)
}

pub fn query_mutant<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Result<NtStatus> {
    let [h, class, buf, length, ret] = abi::args::<K, 5>(k, regs)?;
    if class as u32 != 0 {
        return Err(STATUS_INVALID_INFO_CLASS);
    }
    let caller = k.caller();
    let info = k.with_state(|sys, _, _| {
        sys.objects.object_by_handle(&caller, Handle(h), MUTANT_QUERY_STATE, Some(ObjectType::Mutant))?;
        sys.objects.nt_query_mutant(&caller, Handle(h))
    })?;
    // `MUTANT_BASIC_INFORMATION`: LONG CurrentCount (1 minus the recursion
    // count), BOOLEAN OwnedByCaller, BOOLEAN AbandonedState.
    let current: i32 = 1 - info.recursion as i32;
    let owned = info.owner == Some(caller.thread);
    let mut b = [0u8; 12];
    b[0..4].copy_from_slice(&current.to_le_bytes());
    b[4] = owned as u8;
    b[8] = info.abandoned as u8;
    write_info(k, buf, length, ret, &b)
}

// ----- semaphores

pub fn create_semaphore<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Result<NtStatus> {
    let (out, access, attrs) = open_args(k, regs)?;
    let [_, _, _, initial, maximum] = abi::args::<K, 5>(k, regs)?;
    let caller = k.caller();
    let c = k.with_state(|sys, _, _| {
        sys.objects.nt_create_semaphore(&caller, &attrs.as_object_attributes(), access, initial as i32, maximum as i32)
    })?;
    return_handle(k, out, c.handle)?;
    Ok(c.status)
}

pub fn open_semaphore<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Result<NtStatus> {
    let (out, access, attrs) = open_args(k, regs)?;
    let caller = k.caller();
    let h = k.with_state(|sys, _, _| sys.objects.nt_open_semaphore(&caller, &attrs.as_object_attributes(), access))?;
    return_handle(k, out, h)?;
    Ok(STATUS_SUCCESS)
}

pub fn release_semaphore<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Result<NtStatus> {
    let [h, count, prev] = abi::args::<K, 3>(k, regs)?;
    let caller = k.caller();
    let was = k.with_state(|sys, _, _| sys.objects.nt_release_semaphore(&caller, Handle(h), count as i32))?;
    abi::write_opt_u32(k, prev, was as u32)?;
    Ok(STATUS_SUCCESS)
}

pub fn query_semaphore<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Result<NtStatus> {
    let [h, class, buf, length, ret] = abi::args::<K, 5>(k, regs)?;
    if class as u32 != 0 {
        return Err(STATUS_INVALID_INFO_CLASS);
    }
    let caller = k.caller();
    let info = k.with_state(|sys, _, _| {
        sys.objects.object_by_handle(&caller, Handle(h), SEMAPHORE_QUERY_STATE, Some(ObjectType::Semaphore))?;
        sys.objects.nt_query_semaphore(&caller, Handle(h))
    })?;
    // `SEMAPHORE_BASIC_INFORMATION`: LONG CurrentCount, LONG MaximumCount.
    let mut b = [0u8; 8];
    b[0..4].copy_from_slice(&info.count.to_le_bytes());
    b[4..8].copy_from_slice(&info.maximum.to_le_bytes());
    write_info(k, buf, length, ret, &b)
}

// ----- timers

pub fn create_timer<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Result<NtStatus> {
    let (out, access, attrs) = open_args(k, regs)?;
    let kind = match abi::arg(k, regs, 3)? as u32 {
        NOTIFICATION_EVENT => EventType::Notification,
        SYNCHRONIZATION_EVENT => EventType::Synchronization,
        _ => return Err(STATUS_INVALID_PARAMETER),
    };
    let caller = k.caller();
    let c =
        k.with_state(|sys, _, _| sys.objects.nt_create_timer(&caller, &attrs.as_object_attributes(), access, kind))?;
    return_handle(k, out, c.handle)?;
    Ok(c.status)
}

pub fn open_timer<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Result<NtStatus> {
    let (out, access, attrs) = open_args(k, regs)?;
    let caller = k.caller();
    let h = k.with_state(|sys, _, _| sys.objects.nt_open_timer(&caller, &attrs.as_object_attributes(), access))?;
    return_handle(k, out, h)?;
    Ok(STATUS_SUCCESS)
}

/// `NtSetTimer(TimerHandle, DueTime*, ApcRoutine, ApcContext, Resume,
/// Period, PreviousState*)`. The due time and the period (milliseconds) and
/// an optional timer APC are the kernel's to run; `ApcContext` is the APC's
/// first and only argument (Learn: "NtSetTimer", "TimerApcRoutine").
pub fn set_timer<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Result<NtStatus> {
    let [h, due_ptr, apc_routine, apc_context, _resume, period, prev] = abi::args::<K, 7>(k, regs)?;
    let due = read_timeout(k, due_ptr)?;
    let apc = (apc_routine != 0).then_some(Apc { routine: apc_routine, arg1: apc_context, arg2: 0, arg3: 0 });
    let caller = k.caller();
    let id = k.with_state(|sys, _, _| {
        sys.objects.object_by_handle(&caller, Handle(h), TIMER_MODIFY_STATE, Some(ObjectType::Timer))
    })?;
    let was = k.arm_timer(id, due, period as u32, apc)?;
    abi::write_opt_u32(k, prev, was as u32)?;
    Ok(STATUS_SUCCESS)
}

pub fn cancel_timer<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Result<NtStatus> {
    let [h, prev] = abi::args::<K, 2>(k, regs)?;
    let caller = k.caller();
    let id = k.with_state(|sys, _, _| {
        sys.objects.object_by_handle(&caller, Handle(h), TIMER_MODIFY_STATE, Some(ObjectType::Timer))
    })?;
    let was = k.disarm_timer(id)?;
    abi::write_opt_u32(k, prev, was as u32)?;
    Ok(STATUS_SUCCESS)
}

pub fn query_timer<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Result<NtStatus> {
    let [h, class, buf, length, ret] = abi::args::<K, 5>(k, regs)?;
    if class as u32 != 0 {
        return Err(STATUS_INVALID_INFO_CLASS);
    }
    let caller = k.caller();
    let (info, _armed) = k.with_state(|sys, _, _| {
        let id = sys.objects.object_by_handle(&caller, Handle(h), TIMER_QUERY_STATE, Some(ObjectType::Timer))?;
        sys.objects.timer_info(id)
    })?;
    // `TIMER_BASIC_INFORMATION`: LARGE_INTEGER RemainingTime, BOOLEAN
    // TimerState. RemainingTime is left zero for now (to be confirmed by the
    // spec agent).
    let mut b = [0u8; 12];
    b[8] = info.signaled as u8;
    write_info(k, buf, length, ret, &b)
}

// ----- waits

pub fn wait_single<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Result<NtStatus> {
    let [h, alertable, timeout_ptr] = abi::args::<K, 3>(k, regs)?;
    let timeout = read_timeout(k, timeout_ptr)?;
    wait(k, &[Handle(h)], WaitType::Any, alertable as u8 != 0, timeout)
}

pub fn wait_multiple<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Result<NtStatus> {
    let [count, handles_ptr, wait_type, alertable, timeout_ptr] = abi::args::<K, 5>(k, regs)?;
    let count = count as usize;
    if count == 0 || count > poly_ntobj::MAXIMUM_WAIT_OBJECTS {
        return Err(STATUS_INVALID_PARAMETER);
    }
    let wait_type = match wait_type as u32 {
        0 => WaitType::All, // WaitAll
        1 => WaitType::Any, // WaitAny
        _ => return Err(STATUS_INVALID_PARAMETER),
    };
    let mut handles = Vec::with_capacity(count);
    for i in 0..count {
        handles.push(Handle(abi::read_u64(k, handles_ptr + 8 * i as u64)?));
    }
    let timeout = read_timeout(k, timeout_ptr)?;
    wait(k, &handles, wait_type, alertable as u8 != 0, timeout)
}

/// The heart of both waits: ask the dispatcher, then block in the kernel if
/// it said so.
fn wait<K: NtKernel>(
    k: &mut K,
    handles: &[Handle],
    wait_type: WaitType,
    alertable: bool,
    timeout: NtTimeout,
) -> Result<NtStatus> {
    let caller = k.caller();
    let dispatch = timeout.dispatch();
    let handles = handles.to_vec();
    let outcome = k.with_state(|sys, _, _| {
        sys.objects.nt_wait_for_multiple_objects(&caller, &handles, wait_type, alertable, dispatch)
    })?;
    match outcome {
        WaitOutcome::Done(status) => Ok(status),
        WaitOutcome::Blocked(id) => Ok(k.wait(id, timeout, alertable)),
    }
}

/// `NtSignalAndWaitForSingleObject`: signal `signal` (an event, mutant or
/// semaphore) and wait on `wait`, as one call (Learn:
/// "SignalObjectAndWait"). The signal is not atomic with the start of the
/// wait here (to be confirmed by the spec agent); the two are done in order.
pub fn signal_and_wait<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Result<NtStatus> {
    let [signal, wait_h, alertable, timeout_ptr] = abi::args::<K, 4>(k, regs)?;
    let timeout = read_timeout(k, timeout_ptr)?;
    let caller = k.caller();
    k.with_state(|sys, _, _| signal_object(sys, &caller, Handle(signal)))?;
    wait(k, &[Handle(wait_h)], WaitType::Any, alertable as u8 != 0, timeout)
}

/// Signal an event (set), release a mutant, or release one count of a
/// semaphore, by the object's type.
fn signal_object(sys: &mut crate::system::NtSystem, caller: &poly_ntobj::Caller, h: Handle) -> Result<()> {
    let id = sys.objects.object_by_handle(caller, h, SYNCHRONIZE, None)?;
    match sys.objects.object_type(id) {
        Some(ObjectType::Event) => sys.objects.nt_set_event(caller, h).map(drop),
        Some(ObjectType::Mutant) => sys.objects.nt_release_mutant(caller, h).map(drop),
        Some(ObjectType::Semaphore) => sys.objects.nt_release_semaphore(caller, h, 1).map(drop),
        _ => Err(STATUS_OBJECT_TYPE_MISMATCH),
    }
}

// ----- APCs, delay, yield

pub fn queue_apc_thread<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Result<NtStatus> {
    let [h, routine, arg1, arg2, arg3] = abi::args::<K, 5>(k, regs)?;
    let caller = k.caller();
    let thread = k.with_state(|sys, _, _| {
        let id = sys.objects.object_by_handle(&caller, Handle(h), THREAD_SET_CONTEXT, Some(ObjectType::Thread))?;
        Ok::<_, NtStatus>(poly_ntobj::ThreadRef(sys.objects.cookie(id).expect("live thread object")))
    })?;
    k.queue_apc(thread, Apc { routine, arg1, arg2, arg3 })?;
    Ok(STATUS_SUCCESS)
}

pub fn test_alert<K: NtKernel>(k: &mut K, _regs: &SyscallRegs) -> Result<NtStatus> {
    Ok(k.test_alert())
}

pub fn delay_execution<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Result<NtStatus> {
    let [alertable, interval_ptr] = abi::args::<K, 2>(k, regs)?;
    let timeout = read_timeout(k, interval_ptr)?;
    Ok(k.delay(timeout, alertable as u8 != 0))
}

pub fn yield_execution<K: NtKernel>(k: &mut K, _regs: &SyscallRegs) -> Result<NtStatus> {
    Ok(k.yield_execution())
}

/// Write `info` (as much as `length` allows) and the return length, as the
/// `NtQuery*` dispatcher services report their basic information
/// (`STATUS_INFO_LENGTH_MISMATCH` if the buffer is too small).
fn write_info<K: NtKernel>(k: &mut K, buf: u64, length: u64, ret: u64, info: &[u8]) -> Result<NtStatus> {
    abi::write_opt_u32(k, ret, info.len() as u32)?;
    if (length as usize) < info.len() {
        return Ok(STATUS_INFO_LENGTH_MISMATCH);
    }
    abi::write(k, buf, info)?;
    Ok(STATUS_SUCCESS)
}
