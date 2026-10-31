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
use poly_ntobj::status::{STATUS_INVALID_HANDLE, STATUS_SUCCESS, STATUS_TIMEOUT, STATUS_UNSUCCESSFUL, STATUS_USER_APC};
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
            c.completion = Some(w.status);
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
            if let Some(st) = c.completion.take() {
                c.blocked = None;
                return Next::Done(st);
            }
            if alertable && let Some(apc) = c.apcs.pop_front() {
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
                    if let Some(st) = c.completion.take() {
                        c.blocked = None;
                        return st;
                    }
                    c.blocked = None;
                    s.sys.objects.cancel_wait(id, WaitCancel::Timeout).unwrap_or(STATUS_TIMEOUT)
                });
            }
            Unparked::Interrupted => {
                with(|s| {
                    s.ctl.entry(tid).or_default().blocked = None;
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
/// `PolyResumeApc` to restore (ADR 0065, 0071).
fn deliver_apc(pid: ProcessId, tid: ThreadRef, apc: Apc) {
    let Some(apc_dispatcher) = with(|s| s.processes.get(&pid).map(|p| p.apc_dispatcher)) else { return };
    let regs = user::registers();
    let mut resume = regs;
    resume.rax = u64::from(STATUS_USER_APC.0);
    with(|s| s.ctl.entry(tid).or_default().apc_saved = Some(resume));
    let mut frame = regs;
    frame.rip = apc_dispatcher;
    frame.r11 = apc.routine;
    frame.rcx = apc.arg1;
    frame.rdx = apc.arg2;
    frame.r8 = apc.arg3;
    frame.rsp = resume.rsp & !0xF;
    let _ = user::set_registers(&frame);
}

/// `PolyResumeApc`: restore the saved context, returning the status its wait
/// should see.
pub fn resume_apc(tid: ThreadRef) -> NtStatus {
    let saved = with(|s| s.ctl.get_mut(&tid).and_then(|c| c.apc_saved.take()));
    match saved {
        Some(regs) => {
            let status = NtStatus(regs.rax as u32);
            let _ = user::set_registers(&regs);
            status
        }
        None => STATUS_SUCCESS,
    }
}

/// `NtCreateThreadEx` for the calling process: lay out the thread and start
/// it (or defer it, created suspended).
pub fn create_thread(
    pid: ProcessId,
    caller: ThreadRef,
    memory: &Arc<Memory>,
    req: ThreadCreate,
) -> Result<ThreadCreated, NtStatus> {
    let process = find(pid).ok_or(STATUS_UNSUCCESSFUL)?;
    let tid_u32 = crate::process::alloc_pid();
    let tref = ThreadRef(u64::from(tid_u32));
    let (start, handle) = with(|s| {
        let proc = s.processes.get_mut(&pid).ok_or(STATUS_INVALID_HANDLE)?;
        let mut b = MemBackend { memory, sections: &mut s.sections };
        let start = exec::create_thread(&mut s.sys, proc, &mut b, &req, tref)?;
        let obj = s.sys.objects.thread_object(tref).expect("new thread object");
        let caller = poly_ntobj::Caller::user(pid, caller);
        let handle = s.sys.objects.open_by_pointer(&caller, obj, req.desired_access, 0)?;
        Ok::<_, NtStatus>((start, handle))
    })?;
    let entry = Entry { rip: start.entry, rsp: start.rsp, args: [start.start_routine, start.argument, 0] };
    let created = ThreadCreated { handle, client: ClientId { process: pid, thread: tref } };

    if req.suspended {
        with(|s| {
            let c = s.ctl.entry(tref).or_default();
            c.suspend_count = 1;
            c.deferred_start = Some((entry, start.teb));
        });
        return Ok(created);
    }
    spawn(&process, memory, tref, entry, start.teb)?;
    Ok(created)
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
    match user::spawn_nt_as(Arc::clone(process), Arc::clone(memory), entry, teb, user::DEFAULT_HOOK, tid_u32) {
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
    let process = find(pid).ok_or(STATUS_UNSUCCESSFUL)?;
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

pub fn suspend(pid: ProcessId, thread: ThreadRef) -> Result<u32, NtStatus> {
    let prev = with(|s| {
        let c = s.ctl.entry(thread).or_default();
        let p = c.suspend_count;
        c.suspend_count += 1;
        p
    });
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
            return super::start_first_thread(process, child_memory, &super::Start { entry, teb }).map(|()| prev);
        }
        let process = find(owner_of(pid, thread));
        if let Some((entry, teb)) = deferred {
            if let Some(process) = &process {
                spawn(process, memory, thread, entry, teb)?;
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

pub fn get_context(self_tid: ThreadRef, thread: ThreadRef) -> Result<NtContext, NtStatus> {
    if thread == self_tid {
        return Ok(ctx_from_regs(&user::registers()));
    }
    let mut out = None;
    thread::wait_until(2, || {
        with(|s| {
            if let Some(c) = s.ctl.get(&thread)
                && let Some(ctx) = c.suspended_ctx
            {
                out = Some(ctx);
                return true;
            }
            false
        })
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
