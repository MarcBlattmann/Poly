//! The kernel side of NT threads, waits, suspension, APCs and timers: the
//! acts the dispatcher (`poly-ntobj`) and the service layer
//! (`poly_personality_nt`) leave to the kernel (ADR 0065).
//!
//! The decisions — which wait a signal satisfies, mutant ownership, limits,
//! the status each wait returns — are the crate's. Here the kernel makes
//! threads run, blocks and wakes them, suspends them and reads their saved
//! registers, delivers user APCs, and runs the clock for waitable timers.
//! Blocking reuses the generic park/unpark of [`crate::process`] (ADR 0053's
//! SMP-correct wait queues): a thread parks, and a signalling call wakes it
//! after recording its wait's result ([`drain_wakes`]).

use super::{MemBackend, with};
use crate::process::{self, Process, Unparked};
use crate::user::{self, Entry, Memory, UserRegs};
use crate::{thread, time};
use alloc::sync::{Arc, Weak};
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU64, Ordering};
use poly_ntobj::status::{
    STATUS_INVALID_HANDLE, STATUS_PROCESS_IS_TERMINATING, STATUS_SUCCESS, STATUS_TIMEOUT, STATUS_UNSUCCESSFUL,
    STATUS_USER_APC,
};
use poly_ntobj::{NtStatus, ObjectId, ProcessId, ThreadRef, WaitCancel, WaitId};
use poly_personality_nt::context::NtContext;
use poly_personality_nt::kernel::{Apc, NtTimeout, ThreadCreate, ThreadCreated};
use poly_personality_nt::{ClientId, consts, exec};
use poly_vfs::timerfd::{TimerService, TimerTarget};

/// Seconds between 1601-01-01 and 1970-01-01, in 100 ns units: the gap
/// between a `FILETIME` and a Unix time (documented, `FILETIME` on Learn).
const FILETIME_EPOCH_DIFF_100NS: i128 = 11_644_473_600 * 10_000_000;

/// The monotonic-clock deadline (ns) an [`NtTimeout`] names, or `None` for
/// an infinite wait.
pub fn to_deadline(t: NtTimeout) -> Option<u64> {
    match t {
        NtTimeout::Infinite => None,
        NtTimeout::Relative(v) => Some(time::now_ns().saturating_add(v.saturating_mul(100))),
        NtTimeout::Absolute(a) => {
            let target_rt = ((a as i128) - FILETIME_EPOCH_DIFF_100NS).saturating_mul(100);
            let now_rt = time::realtime_ns() as i128;
            let now_mono = time::now_ns();
            if target_rt <= now_rt {
                Some(now_mono)
            } else {
                Some(now_mono.saturating_add((target_rt - now_rt) as u64))
            }
        }
    }
}

fn ctx_from_regs(r: &UserRegs) -> NtContext {
    NtContext {
        rax: r.rax,
        rcx: r.rcx,
        rdx: r.rdx,
        rbx: r.rbx,
        rsp: r.rsp,
        rbp: r.rbp,
        rsi: r.rsi,
        rdi: r.rdi,
        r8: r.r8,
        r9: r.r9,
        r10: r.r10,
        r11: r.r11,
        r12: r.r12,
        r13: r.r13,
        r14: r.r14,
        r15: r.r15,
        rip: r.rip,
        eflags: r.rflags as u32,
        seg_cs: r.cs as u16,
        seg_ss: r.ss as u16,
    }
}

fn apply_ctx(r: &mut UserRegs, c: &NtContext, want: u32) {
    if want & consts::CONTEXT_CONTROL == consts::CONTEXT_CONTROL {
        r.rip = c.rip;
        r.rsp = c.rsp;
        r.rflags = u64::from(c.eflags);
    }
    if want & consts::CONTEXT_INTEGER == consts::CONTEXT_INTEGER {
        r.rax = c.rax;
        r.rcx = c.rcx;
        r.rdx = c.rdx;
        r.rbx = c.rbx;
        r.rbp = c.rbp;
        r.rsi = c.rsi;
        r.rdi = c.rdi;
        r.r8 = c.r8;
        r.r9 = c.r9;
        r.r10 = c.r10;
        r.r11 = c.r11;
        r.r12 = c.r12;
        r.r13 = c.r13;
        r.r14 = c.r14;
        r.r15 = c.r15;
    }
}

fn find(pid: ProcessId) -> Option<Arc<Process>> {
    crate::process::find(pid.0)
}

/// Record the result of every wait a just-finished signalling call
/// satisfied, and wake the waiting threads (their flags were stored when
/// they blocked). Called after each NT system call.
pub fn drain_wakes() {
    let to_wake: Vec<Arc<crate::process::ThreadFlags>> = with(|s| {
        let mut v = Vec::new();
        while let Some(w) = s.sys.objects.pop_wake() {
            let c = s.ctl.entry(w.thread).or_default();
            c.completion = Some((w.wait, w.status));
            if let Some(f) = &c.flags {
                v.push(Arc::clone(f));
            }
        }
        v
    });
    for f in to_wake {
        process::unpark(&f);
    }
}

/// `NtWaitForSingleObject` / `NtWaitForMultipleObjects` once the dispatcher
/// said the wait must block: park until it is satisfied, timed out, or (if
/// alertable) a user APC arrives.
pub fn wait(pid: ProcessId, tid: ThreadRef, id: WaitId, timeout: NtTimeout, alertable: bool) -> NtStatus {
    let deadline = to_deadline(timeout);
    let process = match user::current_process() {
        Some(p) => p,
        None => return STATUS_UNSUCCESSFUL,
    };
    let flags = match user::current_flags() {
        Some(f) => f,
        None => return STATUS_UNSUCCESSFUL,
    };
    with(|s| {
        let c = s.ctl.entry(tid).or_default();
        c.blocked = Some(id);
        c.alertable = alertable;
        c.flags = Some(Arc::clone(&flags));
    });
    loop {
        enum Next {
            Done(NtStatus),
            Apc(Apc),
            Park,
        }
        let next = with(|s| {
            let c = s.ctl.entry(tid).or_default();
            // Only this wait's completion: one for an earlier wait is stale (ADR 0155 step 12).
            match c.completion.take() {
                Some((w, st)) if w == id => {
                    c.blocked = None;
                    return Next::Done(st);
                }
                _ => {}
            }
            if alertable && c.apcs.front().is_some() {
                // Satisfied before the APC could end it: the object is taken, report it (the
                // APC runs at the next alertable wait).
                if let Some(st) = s.sys.objects.take_wake(id) {
                    c.blocked = None;
                    return Next::Done(st);
                }
                let apc = c.apcs.pop_front().expect("checked");
                c.blocked = None;
                let _ = s.sys.objects.cancel_wait(id, WaitCancel::UserApc);
                return Next::Apc(apc);
            }
            Next::Park
        });
        match next {
            Next::Done(st) => return st,
            Next::Apc(apc) => {
                deliver_apc(pid, tid, apc);
                return STATUS_USER_APC;
            }
            Next::Park => {}
        }
        match process::park(&process, &flags, deadline) {
            Unparked::TimedOut => {
                return with(|s| {
                    let c = s.ctl.entry(tid).or_default();
                    c.blocked = None;
                    if let Some((w, st)) = c.completion.take()
                        && w == id
                    {
                        return st;
                    }
                    // Satisfied, its wake not drained yet: the object is taken, so this is
                    // not a timeout.
                    if let Some(st) = s.sys.objects.take_wake(id) {
                        return st;
                    }
                    s.sys.objects.cancel_wait(id, WaitCancel::Timeout).unwrap_or(STATUS_TIMEOUT)
                });
            }
            Unparked::Interrupted => {
                with(|s| {
                    let c = s.ctl.entry(tid).or_default();
                    c.blocked = None;
                    c.completion = None;
                    let _ = s.sys.objects.take_wake(id);
                    let _ = s.sys.objects.cancel_wait(id, WaitCancel::Terminated);
                });
                // The thread is ending (its process is exiting or it was
                // killed); the status is not observed.
                return STATUS_USER_APC;
            }
            Unparked::Woken => {}
        }
    }
}

/// `NtDelayExecution`: sleep until the deadline, or until an APC on an
/// alertable delay.
pub fn delay(pid: ProcessId, tid: ThreadRef, timeout: NtTimeout, alertable: bool) -> NtStatus {
    if timeout.is_zero() {
        thread::yield_now();
        return STATUS_SUCCESS;
    }
    let deadline = to_deadline(timeout);
    let Some(process) = user::current_process() else { return STATUS_SUCCESS };
    let Some(flags) = user::current_flags() else { return STATUS_SUCCESS };
    with(|s| s.ctl.entry(tid).or_default().flags = Some(Arc::clone(&flags)));
    loop {
        if alertable && let Some(apc) = with(|s| s.ctl.entry(tid).or_default().apcs.pop_front()) {
            deliver_apc(pid, tid, apc);
            return STATUS_USER_APC;
        }
        match process::park(&process, &flags, deadline) {
            Unparked::TimedOut => return STATUS_SUCCESS,
            Unparked::Interrupted => return STATUS_SUCCESS,
            Unparked::Woken => {}
        }
    }
}

/// `NtQueueApcThread`: queue the APC and wake the target if it is parked.
pub fn queue_apc(thread: ThreadRef, apc: Apc) -> Result<(), NtStatus> {
    let flags = with(|s| {
        let c = s.ctl.entry(thread).or_default();
        c.apcs.push_back(apc);
        c.flags.clone()
    });
    if let Some(f) = flags {
        process::unpark(&f);
        f.kick();
    }
    Ok(())
}

/// `NtTestAlert`: run one pending user APC, if any.
pub fn test_alert(pid: ProcessId, tid: ThreadRef) -> NtStatus {
    let apc = with(|s| s.ctl.entry(tid).or_default().apcs.pop_front());
    match apc {
        Some(apc) => {
            deliver_apc(pid, tid, apc);
            STATUS_USER_APC
        }
        None => STATUS_SUCCESS,
    }
}

/// Redirect the (current) thread to run `apc` on its way back to user mode,
/// in `ntdll`'s `KiUserApcDispatcher` (R11 = the routine, RCX/RDX/R8 = its
/// arguments, RSP 16-byte aligned), saving the context it interrupts for
/// `PolyResumeApc` to restore (ADR 0065, 0071). A thread of a 32-bit process
/// gets the routine and its arguments on its stack instead
/// (`poly_personality_nt::wow64::apc_frame`, ADR 0186).
fn deliver_apc(pid: ProcessId, tid: ThreadRef, apc: Apc) {
    let Some(apc_dispatcher) = with(|s| s.processes.get(&pid).map(|p| p.apc_dispatcher)) else { return };
    let regs = user::registers();
    let mut resume = regs;
    resume.rax = u64::from(STATUS_USER_APC.0);
    with(|s| s.ctl.entry(tid).or_default().apc_saved = Some(resume));
    start_apc(pid, apc_dispatcher, &resume, &apc);
}

/// Start `apc` in the current thread at `dispatcher`, from the context `from`.
fn start_apc(pid: ProcessId, dispatcher: u64, from: &UserRegs, apc: &Apc) {
    let mut frame = *from;
    frame.rip = dispatcher;
    if super::is_wow64(pid.0) {
        let Some((esp, bytes)) = poly_personality_nt::wow64::apc_frame(from.rsp, apc) else { return };
        let Some(memory) = user::current_memory() else { return };
        if memory.write(esp, &bytes).is_err() {
            return;
        }
        frame.rsp = esp;
    } else {
        frame.r11 = apc.routine;
        frame.rcx = apc.arg1;
        frame.rdx = apc.arg2;
        frame.r8 = apc.arg3;
        frame.rsp = from.rsp & !0xF;
    }
    let _ = user::set_registers(&frame);
}

/// `PolyResumeApc`: restore the saved context, returning the status its wait
/// should see.
pub fn resume_apc(tid: ThreadRef) -> NtStatus {
    let saved = with(|s| s.ctl.get_mut(&tid).and_then(|c| c.apc_saved.take()));
    match saved {
        Some(regs) => {
            let status = NtStatus(regs.rax as u32);
            // A wait that returns `STATUS_USER_APC` runs every user APC that
            // is queued before it returns (Learn: "Asynchronous Procedure
            // Calls"): the next one starts now, with the same context saved.
            if status == STATUS_USER_APC {
                let next = with(|s| {
                    let (pid, dispatcher) = s
                        .processes
                        .iter()
                        .find(|(_, p)| p.thread(tid).is_some())
                        .map(|(pid, p)| (*pid, p.apc_dispatcher))?;
                    let apc = s.ctl.get_mut(&tid)?.apcs.pop_front()?;
                    s.ctl.get_mut(&tid)?.apc_saved = Some(regs);
                    Some((pid, dispatcher, apc))
                });
                if let Some((pid, dispatcher, apc)) = next {
                    start_apc(pid, dispatcher, &regs, &apc);
                    return status;
                }
            }
            let _ = user::set_registers(&regs);
            status
        }
        None => STATUS_SUCCESS,
    }
}

/// `NtCreateThreadEx`: lay out the thread in the calling process or, with `req.process`, in
/// another one (`CreateRemoteThread`), and start it (or defer it, created suspended). The handle
/// goes in the caller's table. A target whose first thread has not started yet (created
/// suspended) takes the thread too; the thread then starts its loader.
pub fn create_thread(
    pid: ProcessId,
    caller: ThreadRef,
    memory: &Arc<Memory>,
    req: ThreadCreate,
) -> Result<ThreadCreated, NtStatus> {
    let target = req.process.unwrap_or(pid);
    let (process, memory) = if target == pid {
        (find(pid).ok_or(STATUS_UNSUCCESSFUL)?, Arc::clone(memory))
    } else {
        let m = super::io::memory_of(target.0).ok_or(STATUS_PROCESS_IS_TERMINATING)?;
        (process_arc(target).ok_or(STATUS_PROCESS_IS_TERMINATING)?, m)
    };
    if target != pid && process.is_exiting() {
        return Err(STATUS_PROCESS_IS_TERMINATING);
    }
    let tid_u32 = crate::process::alloc_nt_id();
    let tref = ThreadRef(u64::from(tid_u32));
    let (start, handle) = with(|s| {
        let proc = s.processes.get_mut(&target).ok_or(STATUS_PROCESS_IS_TERMINATING)?;
        if target != pid && proc.exit_status.is_some() {
            return Err(STATUS_PROCESS_IS_TERMINATING);
        }
        let mut b = MemBackend { memory: &memory, sections: &mut s.sections };
        let start = exec::create_thread(&mut s.sys, proc, &mut b, &req, tref)?;
        let obj = s.sys.objects.thread_object(tref).expect("new thread object");
        let caller = poly_ntobj::Caller::user(pid, caller);
        let attributes = if req.inherit { poly_ntobj::OBJ_INHERIT } else { 0 };
        // ACCESS_SYSTEM_SECURITY needs a privilege the caller may not hold: then the handle
        // gets the rest.
        let handle = match s.sys.objects.open_by_pointer(&caller, obj, req.desired_access, attributes) {
            Err(e) if e == poly_ntobj::status::STATUS_PRIVILEGE_NOT_HELD => s.sys.objects.open_by_pointer(
                &caller,
                obj,
                req.desired_access & !poly_ntobj::access::ACCESS_SYSTEM_SECURITY,
                attributes,
            ),
            r => r,
        };
        let handle = match handle {
            Ok(h) => h,
            Err(e) => {
                let proc = s.processes.get_mut(&target).expect("still there");
                let mut b = MemBackend { memory: &memory, sections: &mut s.sections };
                exec::thread_exit(&mut s.sys, proc, &mut b, tref, STATUS_UNSUCCESSFUL);
                return Err(e);
            }
        };
        // The descriptor the creator asked for applies after its own handle is open.
        if let Some(sd) = req.security.as_deref() {
            let token = s.sys.objects.security.effective(pid, caller.thread);
            s.sys.objects.assign_security(obj, poly_ntobj::ObjectType::Thread, token, Some(sd));
        }
        Ok::<_, NtStatus>((start, handle))
    })?;
    let entry = Entry { rip: start.entry, rsp: start.rsp, args: [start.start_routine, start.argument, start.exe_base] };
    let created = ThreadCreated { handle, client: ClientId { process: target, thread: tref } };

    if req.suspended {
        with(|s| {
            let c = s.ctl.entry(tref).or_default();
            c.suspend_count = 1;
            c.deferred_start = Some((entry, start.teb));
        });
        return Ok(created);
    }
    if let Err(e) = spawn(&process, &memory, tref, entry, start.teb) {
        with(|s| {
            let _ = s.sys.objects.close(&poly_ntobj::Caller::user(pid, caller), handle);
        });
        return Err(e);
    }
    Ok(created)
}

/// NT process `pid`'s kernel process: the running one, or the one whose first thread waits to
/// start (created suspended; it is in the process table only once a thread has run).
fn process_arc(pid: ProcessId) -> Option<Arc<Process>> {
    find(pid).or_else(|| {
        with(|s| {
            let p = s.processes.get(&pid)?;
            p.threads.iter().find_map(|t| s.ctl.get(&t.thread)?.deferred_process.as_ref().map(|(p, _)| Arc::clone(p)))
        })
    })
}

/// NT process `pid`'s memory map, `fallback` if it has none registered.
fn memory_for(pid: ProcessId, fallback: &Arc<Memory>) -> Arc<Memory> {
    super::io::memory_of(pid.0).unwrap_or_else(|| Arc::clone(fallback))
}

/// Actually start a prepared NT thread in ring 3 and register its flags.
fn spawn(
    process: &Arc<Process>,
    memory: &Arc<Memory>,
    tref: ThreadRef,
    entry: Entry,
    teb: u64,
) -> Result<(), NtStatus> {
    let tid_u32 = tref.0 as u32;
    match super::spawn_thread(process, memory, entry, teb, user::DEFAULT_HOOK, Some(tid_u32)) {
        Ok(kid) => {
            let _ = thread::detach(kid);
            if let Some(f) = process.thread_flags(tid_u32) {
                with(|s| s.ctl.entry(tref).or_default().flags = Some(f));
            }
            Ok(())
        }
        Err(_) => {
            // Undo the stack, TEB and object the layout made.
            with(|s| {
                if let Some(proc) = s.processes.get_mut(&ProcessId(process.pid)) {
                    let mut b = MemBackend { memory, sections: &mut s.sections };
                    exec::thread_exit(&mut s.sys, proc, &mut b, tref, STATUS_UNSUCCESSFUL);
                }
            });
            Err(STATUS_UNSUCCESSFUL)
        }
    }
}

/// `NtTerminateThread` of a thread that is not the caller: make it end at
/// its next return to user mode.
pub fn terminate_other(pid: ProcessId, thread: ThreadRef, status: NtStatus) -> Result<(), NtStatus> {
    // The thread may be one of another process (a remote thread).
    let process = find(owner_of(pid, thread)).ok_or(STATUS_UNSUCCESSFUL)?;
    let tid_u32 = thread.0 as u32;
    let Some(flags) = process.thread_flags(tid_u32) else { return Ok(()) };
    with(|s| s.ctl.entry(thread).or_default().exit_status = Some(status));
    flags.killed.store(true, Ordering::SeqCst);
    // A suspended target must stop parking so it can see the kill.
    flags.suspended.store(false, Ordering::SeqCst);
    with(|s| s.ctl.entry(thread).or_default().suspend_count = 0);
    process::unpark(&flags);
    flags.kick();
    Ok(())
}

/// `NtTerminateProcess` of another process (ADR 0169), or the end of a
/// job's members: record the exit status (the first one counts) and make
/// every thread of `target` end at its next return to user mode. A process
/// whose first thread was never started (created suspended) ends at once.
pub fn terminate_process_by_id(target: ProcessId, status: NtStatus) -> Result<(), NtStatus> {
    let (threads, deferred) = with(|s| {
        let Some(p) = s.processes.get_mut(&target) else { return (Vec::new(), None) };
        p.exit_status.get_or_insert(status);
        let threads: Vec<ThreadRef> = p.threads.iter().map(|t| t.thread).collect();
        let mut deferred = None;
        for t in &threads {
            if let Some(c) = s.ctl.get_mut(t)
                && let Some(d) = c.deferred_process.take()
            {
                c.deferred_start = None;
                deferred = Some(d);
            }
        }
        (threads, deferred)
    });
    if let Some((process, memory)) = deferred {
        // Its first thread will not start now: the hold it kept goes. Threads another process
        // made in it end it as they leave; with none left it ends here.
        if process.live_thread_ids().is_empty() {
            let ended = process.drop_holds();
            super::process_exit(target.0, &memory);
            if ended.is_some() {
                process.ended();
            }
            return Ok(());
        }
        let _ = process.drop_holds();
    }
    for t in threads {
        terminate_other(target, t, status)?;
    }
    Ok(())
}

/// `ExitProcess` / `NtTerminateProcess` of the caller's own process (ADR 0155): every other thread
/// of the process ends at its next return to user mode, as Learn says ("all of the threads in the
/// process terminate"). A thread that stays behind (a helper that polls sockets) would keep the
/// process's memory alive for ever.
pub fn terminate_siblings(pid: ProcessId, self_tid: ThreadRef, status: NtStatus) {
    let Some(process) = find(pid) else { return };
    // A first thread that waits to start (the caller is a thread another process made here)
    // never will.
    let unstarted = with(|s| {
        let ts: Vec<ThreadRef> = s.processes.get(&pid).map(|p| p.threads.iter().map(|t| t.thread).collect())?;
        ts.iter().find_map(|t| {
            let c = s.ctl.get_mut(t)?;
            let d = c.deferred_process.take()?;
            c.deferred_start = None;
            Some(d)
        })
    });
    if unstarted.is_some() {
        let _ = process.drop_holds();
    }
    drop(unstarted);
    for tid in process.live_thread_ids() {
        let t = ThreadRef(u64::from(tid));
        if t != self_tid {
            let _ = terminate_other(pid, t, status);
        }
    }
}

/// The NT process `thread` belongs to: the caller's `pid` unless another
/// process has it (a child's first thread, ADR 0089).
fn owner_of(pid: ProcessId, thread: ThreadRef) -> ProcessId {
    with(|s| {
        if s.processes.get(&pid).is_some_and(|p| p.thread(thread).is_some()) {
            return pid;
        }
        s.processes.iter().find(|(_, p)| p.thread(thread).is_some()).map_or(pid, |(&id, _)| id)
    })
}

/// The kernel's thread behind NT thread `thread` of the calling process `pid` (`me` is the
/// calling thread): for its scheduler affinity. A thread that has ended is
/// `STATUS_THREAD_IS_TERMINATING`; one the process does not have `STATUS_INVALID_CID`.
pub fn kernel_thread(pid: ProcessId, me: ThreadRef, thread: ThreadRef) -> Result<thread::ThreadId, NtStatus> {
    if thread == me {
        return Ok(thread::current());
    }
    let flags = find(owner_of(pid, thread))
        .and_then(|p| p.thread_flags(thread.0 as u32))
        .ok_or(poly_ntobj::status::STATUS_THREAD_IS_TERMINATING)?;
    match flags.thread.load(Ordering::SeqCst) {
        0 => Err(poly_ntobj::status::STATUS_THREAD_IS_TERMINATING),
        bits => Ok(thread::ThreadId::from_bits(bits - 1)),
    }
}

/// A thread's suspend count stops here (`MAXIMUM_SUSPEND_COUNT`, `winnt.h`).
const MAXIMUM_SUSPEND_COUNT: u32 = 127;

pub fn suspend(pid: ProcessId, thread: ThreadRef) -> Result<u32, NtStatus> {
    // A thread that has ended cannot be suspended (`SuspendThread`: ERROR_ACCESS_DENIED, which
    // is what STATUS_THREAD_IS_TERMINATING maps to). One that was created suspended and has not
    // started has no live flags yet but is still a thread.
    let unstarted =
        with(|s| s.ctl.get(&thread).is_some_and(|c| c.deferred_start.is_some() || c.deferred_process.is_some()));
    if !unstarted && !find(owner_of(pid, thread)).is_some_and(|p| p.thread_flags(thread.0 as u32).is_some()) {
        return Err(poly_ntobj::status::STATUS_THREAD_IS_TERMINATING);
    }
    let prev = with(|s| {
        let c = s.ctl.entry(thread).or_default();
        let p = c.suspend_count;
        if p < MAXIMUM_SUSPEND_COUNT {
            c.suspend_count += 1;
        }
        p
    });
    if prev >= MAXIMUM_SUSPEND_COUNT {
        return Err(poly_ntobj::status::STATUS_SUSPEND_COUNT_EXCEEDED);
    }
    if prev == 0
        && let Some(process) = find(owner_of(pid, thread))
        && let Some(f) = process.thread_flags(thread.0 as u32)
    {
        f.suspended.store(true, Ordering::SeqCst);
        f.kick();
    }
    Ok(prev)
}

pub fn resume(pid: ProcessId, memory: &Arc<Memory>, thread: ThreadRef) -> Result<u32, NtStatus> {
    let (prev, now, deferred, child) = with(|s| {
        let c = s.ctl.entry(thread).or_default();
        let prev = c.suspend_count;
        c.suspend_count = c.suspend_count.saturating_sub(1);
        let now = c.suspend_count;
        let deferred = if now == 0 { c.deferred_start.take() } else { None };
        let child = if now == 0 { c.deferred_process.take() } else { None };
        (prev, now, deferred, child)
    });
    if prev > 0 && now == 0 {
        // A new process's first thread, created suspended (ADR 0089).
        if let (Some((entry, teb)), Some((process, child_memory))) = (deferred, &child) {
            let started =
                super::start_first_thread(process, child_memory, &super::Start { entry, teb, tid: thread.0 as u32 });
            // The hold `create_process` took while it waited. If it could not start and the
            // threads other processes made in it have gone, the process ends here.
            if process.release().is_some() {
                process.ended();
            }
            return started.map(|()| prev);
        }
        // The thread may be one another process made in its owner (a remote thread).
        let owner = owner_of(pid, thread);
        let process = process_arc(owner);
        if let Some((entry, teb)) = deferred {
            if let Some(process) = &process {
                spawn(process, &memory_for(owner, memory), thread, entry, teb)?;
            }
        } else if let Some(process) = &process
            && let Some(f) = process.thread_flags(thread.0 as u32)
        {
            f.suspended.store(false, Ordering::SeqCst);
            process::unpark(&f);
            f.kick();
        }
    }
    Ok(prev)
}

/// The threads of NT process `target`, or `STATUS_INVALID_PARAMETER` if it is gone.
fn threads_of(target: ProcessId) -> Result<Vec<ThreadRef>, NtStatus> {
    with(|s| s.processes.get(&target).map(|p| p.threads.iter().map(|t| t.thread).collect()))
        .ok_or(poly_ntobj::status::STATUS_INVALID_PARAMETER)
}

/// `NtSuspendProcess` (ADR 0155 step 9): every thread of `target` is suspended once, the way
/// [`suspend`] does one thread. A thread that was created suspended and never started just
/// counts one more; one already at the maximum count stays there and the call still succeeds.
pub fn suspend_process(target: ProcessId) -> Result<(), NtStatus> {
    for t in threads_of(target)? {
        match suspend(target, t) {
            Ok(_) | Err(poly_ntobj::status::STATUS_SUSPEND_COUNT_EXCEEDED) => {}
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

/// `NtResumeProcess`: every thread of `target` is resumed once, exactly as [`resume`] does one
/// thread (the call does not know who suspended: a count that `CREATE_SUSPENDED` left is taken
/// off too, and a thread that reaches 0 starts). A thread at 0 stays at 0. `memory` is the
/// caller's: a thread created suspended and not yet started is always one of the caller's own.
pub fn resume_process(target: ProcessId, memory: &Arc<Memory>) -> Result<(), NtStatus> {
    for t in threads_of(target)? {
        resume(target, memory, t)?;
    }
    Ok(())
}

pub fn get_context(self_tid: ThreadRef, thread: ThreadRef) -> Result<NtContext, NtStatus> {
    if thread == self_tid {
        return Ok(ctx_from_regs(&user::registers()));
    }
    // The kernel thread behind the NT thread (its flags hold its id while it lives).
    let kernel_thread = || {
        let pid = with(|s| s.processes.iter().find(|(_, p)| p.thread(thread).is_some()).map(|(&id, _)| id))?;
        let bits = find(pid)?.thread_flags(thread.0 as u32)?.thread.load(Ordering::SeqCst);
        (bits != 0).then(|| thread::ThreadId::from_bits(bits - 1))
    };
    let mut out = None;
    thread::wait_until(2, || {
        with(|s| {
            let Some(c) = s.ctl.get(&thread) else { return false };
            if let Some(ctx) = c.suspended_ctx {
                out = Some(ctx);
                return true;
            }
            false
        }) || {
            // A suspended thread that is blocked in the kernel (a wait, a system call) parks
            // only when that returns, which may be never: its user context is the one its
            // entry into the kernel saved, read while it does not run (GetThreadContext answers
            // at once for a suspended thread, Learn; Chrome's stack sampler suspends threads
            // that wait in NtRemoveIoCompletion and NtWaitForMultipleObjects).
            let suspended = with(|s| s.ctl.get(&thread).is_some_and(|c| c.suspend_count > 0));
            if let Some(regs) = suspended.then(kernel_thread).flatten().and_then(thread::user_regs_of) {
                out = Some(ctx_from_regs(&regs));
                return true;
            }
            false
        }
    });
    out.ok_or(STATUS_UNSUCCESSFUL)
}

pub fn set_context(self_tid: ThreadRef, thread: ThreadRef, ctx: &NtContext, want: u32) -> Result<(), NtStatus> {
    if thread == self_tid {
        let mut regs = user::registers();
        apply_ctx(&mut regs, ctx, want);
        let _ = user::set_registers(&regs);
        return Ok(());
    }
    with(|s| {
        let c = s.ctl.get_mut(&thread).ok_or(STATUS_INVALID_HANDLE)?;
        if c.suspended_ctx.is_none() {
            return Err(STATUS_UNSUCCESSFUL);
        }
        c.suspended_ctx = Some(*ctx);
        c.set_on_resume = Some(*ctx);
        Ok(())
    })
}

/// Park the (current) suspended thread until it is resumed, with its context
/// saved so another thread may read or change it.
pub fn wait_while_suspended(pid: u32, tid: u32) {
    let tref = ThreadRef(u64::from(tid));
    let Some(process) = user::current_process() else { return };
    let Some(flags) = user::current_flags() else { return };
    let ctx = ctx_from_regs(&user::registers());
    loop {
        let suspended = with(|s| {
            let c = s.ctl.entry(tref).or_default();
            c.flags = Some(Arc::clone(&flags));
            if c.suspend_count == 0 {
                false
            } else {
                c.suspended_ctx = Some(ctx);
                true
            }
        });
        if !suspended || process.is_exiting() || flags.killed.load(Ordering::SeqCst) {
            break;
        }
        process::park(&process, &flags, None);
    }
    let _ = pid;
    let set = with(|s| {
        let c = s.ctl.entry(tref).or_default();
        c.suspended_ctx = None;
        c.set_on_resume.take()
    });
    if let Some(ctx) = set {
        let mut regs = user::registers();
        apply_ctx(&mut regs, &ctx, consts::CONTEXT_CONTROL | consts::CONTEXT_INTEGER);
        let _ = user::set_registers(&regs);
    }
    flags.suspended.store(false, Ordering::SeqCst);
}

// ----- waitable timers

/// An armed NT timer: it signals its object when the clock reaches its due
/// time, re-arms itself for a periodic timer, and queues its APC (if any) to
/// the thread that set it.
pub struct NtTimer {
    object: ObjectId,
    pid: ProcessId,
    setter: ThreadRef,
    period_ms: AtomicU64,
    apc: Option<Apc>,
    service_id: AtomicU64,
}

impl TimerTarget for NtTimer {
    fn expire(&self) {
        let periodic = self.period_ms.load(Ordering::SeqCst) > 0;
        with(|s| {
            let _ = s.sys.objects.timer_expired(self.object, periodic);
            if let Some(apc) = self.apc {
                let c = s.ctl.entry(self.setter).or_default();
                c.apcs.push_back(apc);
            }
        });
        drain_wakes();
        // Wake the setter if it is in an alertable wait, to take the APC.
        if self.apc.is_some()
            && let Some(f) = find(self.pid).and_then(|p| p.thread_flags(self.setter.0 as u32))
        {
            process::unpark(&f);
            f.kick();
        }
        if periodic {
            let period_ns = self.period_ms.load(Ordering::SeqCst).saturating_mul(1_000_000);
            let at = time::now_ns().saturating_add(period_ns);
            if let Some(me) = with(|s| s.timers.get(&self.object).cloned()) {
                let id = crate::fs::poll::timers().arm(at, Arc::downgrade(&me) as Weak<dyn TimerTarget>);
                self.service_id.store(id, Ordering::SeqCst);
            }
        }
    }
}

pub fn arm_timer(
    pid: ProcessId,
    tid: ThreadRef,
    timer: ObjectId,
    due: NtTimeout,
    period_ms: u32,
    apc: Option<Apc>,
) -> Result<bool, NtStatus> {
    let was = with(|s| s.sys.objects.set_timer(timer))?;
    // Replace any previous arming of this timer.
    if let Some(old) = with(|s| s.timers.remove(&timer)) {
        crate::fs::poll::timers().cancel(old.service_id.load(Ordering::SeqCst));
    }
    let nt = Arc::new(NtTimer {
        object: timer,
        pid,
        setter: tid,
        period_ms: AtomicU64::new(u64::from(period_ms)),
        apc,
        service_id: AtomicU64::new(0),
    });
    with(|s| {
        s.timers.insert(timer, Arc::clone(&nt));
    });
    let at = to_deadline(due).unwrap_or_else(time::now_ns);
    let id = crate::fs::poll::timers().arm(at, Arc::downgrade(&nt) as Weak<dyn TimerTarget>);
    nt.service_id.store(id, Ordering::SeqCst);
    Ok(was)
}

pub fn disarm_timer(timer: ObjectId) -> Result<bool, NtStatus> {
    let was = with(|s| s.sys.objects.cancel_timer(timer))?;
    if let Some(old) = with(|s| s.timers.remove(&timer)) {
        crate::fs::poll::timers().cancel(old.service_id.load(Ordering::SeqCst));
    }
    Ok(was)
}
