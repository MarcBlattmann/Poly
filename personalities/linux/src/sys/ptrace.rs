//! ptrace(2) (ADR 0134): the tracer's requests, and the stops a traced
//! thread makes itself on its way through the kernel.
//!
//! The state is in [`crate::task::ptrace`] and lives in the process tree.
//! A traced thread stops at:
//!
//! - a signal it is about to act on (signal-delivery-stop,
//!   [`signal_stop`], from [`crate::signal::deliver`]);
//! - `PTRACE_INTERRUPT` and its first stop as an auto-attached child
//!   (`PTRACE_EVENT_STOP`, [`event_stops`]);
//! - system call entry and exit after `PTRACE_SYSCALL` ([`syscall_entry`],
//!   [`syscall_exit`], from [`crate::Linux::dispatch_with`]);
//! - the events its options ask for: fork, vfork, clone ([`clone_event`],
//!   called by the kernel before a vfork parent waits), vfork done
//!   ([`vfork_done`]), exec ([`exec_event`]; without `PTRACE_O_TRACEEXEC` a
//!   `SIGTRAP`), exit ([`exit_event`]).
//!
//! At a stop it saves its registers, FPU area and signal mask, tells its
//! tracer (`SIGCHLD`, `CLD_TRAPPED`; its wait(2) reports the stop) and
//! waits on the tree until the tracer restarts it, detaches, or it gets
//! `SIGKILL`. It then loads whatever the tracer changed.

use crate::abi::SigInfo;
use crate::abi::USER_ADDRESS_LIMIT;
use crate::abi::signal::{SI_USER, SIGCHLD, SIGKILL, SIGSTOP, SIGTRAP};
use crate::args::Args;
use crate::cred::{self, cap};
use crate::errno::{Errno, SysResult};
use crate::kernel::{LinuxKernel, SigSet, Target, UserPtr};
use crate::signal::deliver::{with_own, with_target};
use crate::sys::procaccess;
use crate::table::{Handler, nr};
use crate::task::ptrace::{
    self as state, FPREGS_SIZE, How, NREGS, NT_PRFPREG, NT_PRSTATUS, NT_X86_XSTATE, REGS_SIZE, Resume, Stop, SyscallOp,
    Tracee, UserArea, event, opt, reg, req,
};
use crate::task::tree::{ChildEvent, ProcessTree, Waited};
use crate::task::{signal_process, wait, with_tree};
use alloc::boxed::Box;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, Ordering};

pub const CALLS: &[(u64, Handler)] = &[(nr::PTRACE, ptrace)];

/// `_NSIG` (`asm/signal.h`): a restart's signal must not be above it.
const NSIG: u64 = 64;
/// `rflags` trap flag (single-step).
const TF: u64 = 1 << 8;

/// Some process may be traced: the hooks look at the tree only then. Set
/// and cleared with the tree locked, so a link made meanwhile is never
/// missed.
static TRACING: AtomicBool = AtomicBool::new(false);

fn tracing() -> bool {
    // Unit tests run many fake kernels at once, each with its own tree.
    cfg!(test) || TRACING.load(Ordering::Relaxed)
}

/// Link `tid` in `t` (the tree locked).
fn link(t: &mut ProcessTree, tid: i32, l: Tracee) {
    t.attach(tid, l);
    TRACING.store(true, Ordering::Relaxed);
}

/// Run `f` on the calling thread's link, if it is traced.
fn with_link<R>(k: &mut dyn LinuxKernel, f: impl FnOnce(&mut Tracee) -> R) -> Option<R> {
    if !tracing() {
        return None;
    }
    let tid = k.tid();
    with_tree(k, |t| {
        if !t.any_traced() {
            TRACING.store(false, Ordering::Relaxed);
        }
        t.tracee_mut(tid).filter(|l| !l.detaching).map(f)
    })
    .ok()
    .flatten()
}

/// Whether the calling thread is traced.
pub fn is_traced(k: &mut dyn LinuxKernel) -> bool {
    with_link(k, |_| ()).is_some()
}

/// Set or clear the `ptraced` mark of process `pid`'s signal state.
fn mark(k: &mut dyn LinuxKernel, pid: i32, on: bool) {
    let _ = with_target(k, Target::Process(pid), |p, _| p.ptraced = on);
}

/// What a stop shows, besides the registers.
struct StopKind {
    signal: u32,
    event: u32,
    delivery: bool,
    info: SigInfo,
    message: u64,
    op: SyscallOp,
    /// `rax` as the tracer sees it, if not the thread's.
    rax: Option<u64>,
}

impl StopKind {
    fn event(e: u32, message: u64) -> StopKind {
        StopKind {
            signal: SIGTRAP,
            event: e,
            delivery: false,
            info: state::trap_info(SIGTRAP, e),
            message,
            op: SyscallOp::None,
            rax: None,
        }
    }
}

/// What the tracer left when it restarted a stopped thread.
struct Restarted {
    resume: Resume,
    /// The registers, if the tracer changed them (already loaded).
    regs: Option<[u64; NREGS]>,
    info: SigInfo,
}

/// Stop the calling thread for its tracer and wait. `None` if it is not
/// traced (any more): it goes on as if untraced.
fn stop(k: &mut dyn LinuxKernel, kind: StopKind) -> Option<Restarted> {
    let tid = k.tid();
    let mut ctx = k.user_context();
    if let Some(rax) = kind.rax {
        ctx.rax = rax;
    }
    let (fs_base, gs_base) = (k.fs_base(), k.gs_base());
    let fpu = k.fpu_state().image.to_vec();
    let mask = with_own(k, |p| p.thread(tid).mask.0);
    let code = (kind.signal & 0xff) | (kind.event << 8);
    let tracer = with_link(k, |l| {
        let orig_rax = l.in_syscall.unwrap_or(u64::MAX);
        let mut regs = state::regs_of(&ctx, orig_rax, fs_base, gs_base);
        // A single step ends at the next stop.
        let stepped = core::mem::take(&mut l.stepping) && regs[reg::EFLAGS] & TF != 0;
        if stepped {
            regs[reg::EFLAGS] &= !TF;
        }
        l.stop = Some(Box::new(Stop {
            signal: kind.signal,
            event: kind.event,
            delivery: kind.delivery,
            info: kind.info,
            message: kind.message,
            regs,
            regs_changed: stepped,
            fpu,
            fpu_changed: false,
            mask,
            mask_changed: false,
            op: kind.op,
            reported: false,
        }));
        l.resume = None;
        l.tracer
    })?;
    let _ = with_tree(k, |t| t.touch());
    // The tracer hears of it: SIGCHLD with CLD_TRAPPED, then its wait.
    let me = Waited { pid: k.pid(), uid: k.credentials().uid, event: ChildEvent::Trapped(code) };
    signal_process(k, tracer, wait::child_info(SIGCHLD, &me));
    loop {
        k.clear_signal_notification();
        let (generation, woke) = with_tree(k, |t| {
            let generation = t.generation();
            let woke = match t.tracee_mut(tid) {
                None => Woke::Gone,
                Some(l) => match l.resume.take() {
                    Some(resume) => {
                        let stop = l.stop.take();
                        l.syscall_trace = resume.how == How::Syscall;
                        l.stepping = resume.how == How::Step;
                        if resume.how == How::Detach {
                            t.untrace(tid);
                        }
                        Woke::Resumed(resume, stop)
                    }
                    None if l.stop.is_none() => Woke::Gone,
                    None => Woke::Waiting,
                },
            };
            (generation, woke)
        })
        .ok()?;
        match woke {
            Woke::Gone => {
                let pid = k.pid();
                mark(k, pid, false);
                return None;
            }
            Woke::Resumed(resume, stop) => {
                if resume.how == How::Detach {
                    let pid = k.pid();
                    mark(k, pid, false);
                }
                return Some(load(k, resume, stop));
            }
            Woke::Waiting => {
                if killed(k) {
                    // SIGKILL ends any stop; the tracer sees the death.
                    let _ = with_tree(k, |t| {
                        if let Some(l) = t.tracee_mut(tid) {
                            l.stop = None;
                        }
                        t.touch();
                    });
                    return Some(Restarted { resume: Resume { how: How::Cont, signal: 0 }, regs: None, info: kind.info });
                }
                k.wait_tree(generation);
            }
        }
    }
}

enum Woke {
    Gone,
    Waiting,
    Resumed(Resume, Option<Box<Stop>>),
}

/// The calling thread has `SIGKILL` pending or its process is ending.
fn killed(k: &mut dyn LinuxKernel) -> bool {
    let tid = k.tid();
    k.exiting() || with_own(k, |p| p.pending(tid).contains(SIGKILL))
}

/// Load what the tracer changed at a stop into the calling thread.
fn load(k: &mut dyn LinuxKernel, resume: Resume, stop: Option<Box<Stop>>) -> Restarted {
    let Some(stop) = stop else { return Restarted { resume, regs: None, info: SigInfo::new(0, 0) } };
    let tid = k.tid();
    let mut regs = None;
    if stop.regs_changed || resume.how == How::Step {
        let mut r = stop.regs;
        if resume.how == How::Step {
            r[reg::EFLAGS] |= TF;
        }
        let mut ctx = k.user_context();
        state::apply_regs(&r, &mut ctx);
        k.set_user_context(&ctx);
        if r[reg::FS_BASE] != k.fs_base() {
            k.set_fs_base(r[reg::FS_BASE]);
        }
        if r[reg::GS_BASE] != k.gs_base() {
            k.set_gs_base(r[reg::GS_BASE]);
        }
        regs = Some(r);
    }
    if stop.fpu_changed {
        let _ = k.set_fpu_state(&stop.fpu);
    }
    if stop.mask_changed {
        with_own(k, |p| p.thread_mut(tid).mask = SigSet(stop.mask).without(SigSet::UNBLOCKABLE));
    }
    Restarted { resume, regs, info: stop.info }
}

/// A traced thread is about to act on signal `info`: its
/// signal-delivery-stop. Returns the signal to deliver: the tracer's
/// (`None`: suppressed).
pub fn signal_stop(k: &mut dyn LinuxKernel, info: SigInfo) -> Option<SigInfo> {
    let signal = info.signal();
    let kind = StopKind {
        signal,
        event: 0,
        delivery: true,
        info,
        message: 0,
        op: SyscallOp::None,
        rax: None,
    };
    let tracer = with_link(k, |l| l.tracer).unwrap_or(0);
    let Some(r) = stop(k, kind) else { return Some(info) };
    match r.resume.signal {
        0 => None,
        s if s == signal => Some(r.info),
        // Another signal: as if the tracer had sent it.
        s => Some(SigInfo::from_process(s, SI_USER, tracer, 0)),
    }
}

/// Before a traced thread looks at its signals: a `PTRACE_INTERRUPT` or
/// its first stop as an auto-attached child (`PTRACE_EVENT_STOP`; a child
/// of a tracer that used `PTRACE_ATTACH` gets `SIGSTOP` instead).
pub fn event_stops(k: &mut dyn LinuxKernel) {
    let Some((interrupt, start, seized)) = with_link(k, |l| {
        let r = (l.interrupt, l.start_stop, l.seized);
        l.interrupt = false;
        l.start_stop = false;
        r
    }) else {
        return;
    };
    if start && !seized {
        let pid = k.pid();
        signal_process(k, pid, SigInfo::new(SIGSTOP, SI_USER));
    } else if interrupt || start {
        stop(k, StopKind::event(event::STOP, 0));
    }
}

/// The calling thread leaves the kernel (to user mode): it is no longer
/// in a system call.
pub fn left_kernel(k: &mut dyn LinuxKernel) {
    with_link(k, |l| l.in_syscall = None);
}

/// What [`syscall_entry`] decided.
pub struct Entry {
    /// The call is skipped (the tracer set `orig_rax` to -1); `rax` holds
    /// its result.
    pub skip: bool,
    /// The registers were loaded from the tracer's copy, so the result
    /// must be written to them too ([`syscall_exit`]).
    pub replaced: bool,
    /// An exit stop follows.
    pub exit_stop: bool,
}

/// A traced thread enters system call `regs.rax`: a first or interrupt
/// stop that is due, then the entry stop after `PTRACE_SYSCALL` (at which
/// `orig_rax` is the number and `rax` `-ENOSYS`). The tracer may change
/// the number and arguments, or skip the call with `orig_rax` -1. `None`
/// if the thread is not traced.
pub fn syscall_entry(k: &mut dyn LinuxKernel, regs: &mut poly_abi::SyscallRegs) -> Option<Entry> {
    let number = regs.rax;
    let (pending, trace, sysgood) = with_link(k, |l| {
        l.in_syscall = Some(number);
        (l.interrupt || l.start_stop, l.syscall_trace, l.options & opt::TRACESYSGOOD != 0)
    })?;
    let mut entry = Entry { skip: false, replaced: false, exit_stop: false };
    if pending {
        event_stops(k);
    }
    let trace = if pending { with_link(k, |l| l.syscall_trace).unwrap_or(false) } else { trace };
    if !trace {
        return Some(entry);
    }
    let signal = if sysgood { SIGTRAP | 0x80 } else { SIGTRAP };
    let args = [regs.rdi, regs.rsi, regs.rdx, regs.r10, regs.r8, regs.r9];
    let kind = StopKind {
        signal,
        event: 0,
        delivery: false,
        info: SigInfo::new(SIGTRAP, signal as i32),
        message: 0,
        op: SyscallOp::Entry { nr: number, args },
        rax: Some(Errno::ENOSYS.to_rax()),
    };
    let Some(r) = stop(k, kind) else { return Some(entry) };
    entry.exit_stop = r.resume.how == How::Syscall;
    if let Some(new) = r.regs {
        entry.replaced = true;
        let nr = new[reg::ORIG_RAX];
        with_link(k, |l| l.in_syscall = Some(nr));
        if nr == u64::MAX {
            entry.skip = true;
            regs.rax = new[reg::RAX];
        } else {
            regs.rax = nr;
            regs.rdi = new[reg::RDI];
            regs.rsi = new[reg::RSI];
            regs.rdx = new[reg::RDX];
            regs.r10 = new[reg::R10];
            regs.r8 = new[reg::R8];
            regs.r9 = new[reg::R9];
        }
    }
    Some(entry)
}

/// A traced thread leaves system call `number` with `regs.rax`: the exit
/// stop after `PTRACE_SYSCALL` (the tracer may change the result).
pub fn syscall_exit(k: &mut dyn LinuxKernel, regs: &mut poly_abi::SyscallRegs, number: u64, entry: &Entry) {
    // exit and exit_group do not come back.
    let mut replaced = entry.replaced;
    if entry.exit_stop && number != nr::EXIT && number != nr::EXIT_GROUP {
        let sysgood = with_link(k, |l| l.options & opt::TRACESYSGOOD != 0).unwrap_or(false);
        let signal = if sysgood { SIGTRAP | 0x80 } else { SIGTRAP };
        let rval = regs.rax as i64;
        let kind = StopKind {
            signal,
            event: 0,
            delivery: false,
            info: SigInfo::new(SIGTRAP, signal as i32),
            message: 0,
            op: SyscallOp::Exit { rval, is_error: (-4095..0).contains(&rval) },
            rax: Some(regs.rax),
        };
        if let Some(r) = stop(k, kind)
            && let Some(new) = r.regs
        {
            regs.rax = new[reg::RAX];
            replaced = true;
        }
    }
    if replaced {
        // The return to user mode restores every register from the
        // thread's context: it must hold the result.
        let mut ctx = k.user_context();
        ctx.rax = regs.rax;
        k.set_user_context(&ctx);
    }
}

/// The options of the calling thread's tracer, if it is traced.
fn options(k: &mut dyn LinuxKernel) -> Option<u32> {
    with_link(k, |l| l.options)
}

/// A traced thread's execve(2) succeeded (`former` was its thread id):
/// `PTRACE_EVENT_EXEC` with `PTRACE_O_TRACEEXEC`, else a `SIGTRAP`.
pub fn exec_event(k: &mut dyn LinuxKernel, former: i32) {
    let Some(options) = options(k) else { return };
    if options & opt::TRACEEXEC != 0 {
        stop(k, StopKind::event(event::EXEC, former as u64));
    } else {
        let pid = k.pid();
        signal_process(k, pid, SigInfo::new(SIGTRAP, SI_USER));
    }
}

/// A traced thread is about to exit with wait status `status`:
/// `PTRACE_EVENT_EXIT` with `PTRACE_O_TRACEEXIT`.
pub fn exit_event(k: &mut dyn LinuxKernel, status: u64) {
    if options(k).is_some_and(|o| o & opt::TRACEEXIT != 0) {
        stop(k, StopKind::event(event::EXIT, status));
    }
}

/// Whether a child made by clone(2) with `flags` and `exit_signal` is
/// traced by its creator's tracer, and with which event.
fn inherits(l: &Tracee, flags: u64, exit_signal: u32) -> Option<u32> {
    use crate::task::clone::{CLONE_PTRACE, CLONE_THREAD, CLONE_UNTRACED};
    if flags & (CLONE_UNTRACED | CLONE_THREAD) != 0 {
        return None;
    }
    let e = state::clone_event(flags, exit_signal);
    (l.options & state::option_for(e) != 0 || flags & CLONE_PTRACE != 0).then_some(e)
}

/// The kernel, with the tree locked, has just put `child` in it as a
/// child that thread `creator` made with clone(2) `flags`: if `creator`'s
/// tracer follows such children, `child` is its tracee from the start
/// (its first stop is due). Returns whether it is (the kernel marks its
/// signal state `ptraced`).
pub fn trace_new_child(t: &mut ProcessTree, creator: i32, child: i32, flags: u64, exit_signal: u32) -> bool {
    let Some(l) = t.tracee(creator).filter(|l| !l.detaching) else { return false };
    if inherits(l, flags, exit_signal).is_none() {
        return false;
    }
    let mut new = Tracee::new(l.tracer, l.seized, l.options);
    new.start_stop = true;
    link(t, child, new);
    true
}

/// A traced thread made `child` with clone(2) `flags` (called by the
/// kernel once the child runs, before a vfork parent waits for it): the
/// fork, vfork or clone event stop its options ask for, with the child's
/// id as message.
pub fn clone_event(k: &mut dyn LinuxKernel, child: i32, flags: u64, exit_signal: u32) {
    let Some(e) = with_link(k, |l| {
        inherits(l, flags, exit_signal).filter(|&e| l.options & state::option_for(e) != 0)
    })
    .flatten() else {
        return;
    };
    stop(k, StopKind::event(e, child as u64));
}

/// A traced vfork(2) parent was released by `child`:
/// `PTRACE_EVENT_VFORK_DONE` with `PTRACE_O_TRACEVFORKDONE`.
pub fn vfork_done(k: &mut dyn LinuxKernel, child: i32, flags: u64) {
    use crate::task::clone::{CLONE_UNTRACED, CLONE_VFORK};
    if flags & CLONE_VFORK == 0 || flags & CLONE_UNTRACED != 0 {
        return;
    }
    if options(k).is_some_and(|o| o & opt::TRACEVFORKDONE != 0) {
        stop(k, StopKind::event(event::VFORK_DONE, child as u64));
    }
}

/// Process `pid` (the calling thread's) ended: its tracees are let go
/// (stopped ones run on; `PTRACE_O_EXITKILL` kills them), and a tracer
/// other than its parent hears of its end.
pub fn process_exited(k: &mut dyn LinuxKernel, pid: i32, how: crate::task::tree::Termination) {
    if !tracing() {
        return;
    }
    let Ok((freed, kill, tracer)) = with_tree(k, |t| {
        let mut freed = Vec::new();
        let mut kill = Vec::new();
        for tracee in t.tracees_of(pid) {
            let Some(l) = t.tracee_mut(tracee) else { continue };
            if l.options & opt::EXITKILL != 0 {
                kill.push(tracee);
            }
            if l.stop.is_some() && l.resume.is_none() {
                l.detaching = true;
                l.resume = Some(Resume { how: How::Detach, signal: 0 });
            } else if l.stop.is_none() {
                t.untrace(tracee);
                freed.push(tracee);
            }
        }
        t.touch();
        let tracer = t.tracee(pid).filter(|l| !l.detaching && Some(l.tracer) != t.parent(pid)).map(|l| l.tracer);
        (freed, kill, tracer)
    }) else {
        return;
    };
    for tracee in freed {
        mark(k, tracee, false);
    }
    for tracee in kill {
        signal_process(k, tracee, SigInfo::new(SIGKILL, SI_USER));
    }
    if let Some(tracer) = tracer {
        let me = Waited { pid, uid: k.credentials().uid, event: ChildEvent::Ended(how) };
        signal_process(k, tracer, wait::child_info(SIGCHLD, &me));
    }
}

/// ptrace(request, pid, addr, data).
pub fn ptrace(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let request = a.ulong(0);
    let pid = a.int(1);
    let addr = a.ulong(2);
    let data = a.ulong(3);
    match request {
        req::TRACEME => traceme(k),
        req::ATTACH | req::SEIZE => attach(k, request == req::SEIZE, pid, addr, data),
        _ => request_of(k, request, pid, addr, data),
    }
}

/// `PTRACE_TRACEME`: the parent becomes the tracer. `EPERM` when already
/// traced.
fn traceme(k: &mut dyn LinuxKernel) -> SysResult {
    let me = k.pid();
    let tid = k.tid();
    let outer = with_own(k, |p| p.traced || p.ptraced);
    let made = with_tree(k, |t| {
        let parent = t.parent(me).unwrap_or(0);
        if outer || t.tracee(tid).is_some() || parent <= 0 {
            return false;
        }
        link(t, tid, Tracee::new(parent, false, 0));
        true
    })?;
    if !made {
        return Err(Errno::EPERM);
    }
    mark(k, me, true);
    Ok(0)
}

/// `PTRACE_ATTACH` / `PTRACE_SEIZE`: checks in the spec sheet's order.
fn attach(k: &mut dyn LinuxKernel, seize: bool, pid: i32, addr: u64, data: u64) -> SysResult {
    let target = procaccess::find(k, pid, true)?;
    if seize && (addr != 0 || data & !u64::from(opt::MASK) != 0) {
        return Err(Errno::EIO);
    }
    let me = k.pid();
    if target.pid == me || target.info.zombie {
        return Err(Errno::EPERM);
    }
    let c = cred::current(k);
    let privileged = c.capable(cap::SYS_PTRACE);
    if !procaccess::may_access(me, c.uid, c.gid, privileged, &target) {
        return Err(Errno::EPERM);
    }
    let outer = with_target(k, Target::Process(target.pid), |p, _| p.traced || p.ptraced)?;
    let uid = c.uid;
    let made = with_tree(k, |t| {
        // Yama (ptrace_scope 1, the oracle's): an unprivileged tracer
        // attaches only to its descendants.
        if outer || t.tracee(target.pid).is_some() || (!privileged && !t.is_ancestor(me, target.pid)) {
            return false;
        }
        let options = if seize { data as u32 } else { 0 };
        link(t, target.pid, Tracee::new(me, seize, options));
        true
    })?;
    if !made {
        return Err(Errno::EPERM);
    }
    mark(k, target.pid, true);
    if !seize {
        signal_process(k, target.pid, SigInfo::from_process(SIGSTOP, SI_USER, me, uid));
    }
    Ok(0)
}

/// What a request needs of the tracee's link.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Needs {
    /// Any tracee of the caller.
    Tracee,
    /// One in a ptrace-stop.
    Stopped,
}

/// The requests other than attaching: the caller must trace `pid`
/// (`ESRCH`), and most need it stopped (`ESRCH`); then unknown requests
/// are `EIO`.
fn request_of(k: &mut dyn LinuxKernel, request: u64, pid: i32, addr: u64, data: u64) -> SysResult {
    let target = if pid > 0 { crate::ns::pid::find(k, pid)? } else { return Err(Errno::ESRCH) };
    let me = k.pid();
    let needs = match request {
        req::KILL | req::INTERRUPT | req::LISTEN => Needs::Tracee,
        _ => Needs::Stopped,
    };
    let ok = with_tree(k, |t| {
        t.tracee(target).filter(|l| l.traced_by(me)).is_some_and(|l| needs == Needs::Tracee || l.stopped().is_some())
    })?;
    if !ok {
        return Err(Errno::ESRCH);
    }
    let out = UserPtr(data);
    match request {
        req::PEEKTEXT | req::PEEKDATA => {
            let mut word = [0u8; 8];
            k.ptrace_peek(target, addr, &mut word)?;
            k.copy_to_user(out, &word)?;
            Ok(0)
        }
        req::POKETEXT | req::POKEDATA => {
            k.ptrace_poke(target, addr, &data.to_le_bytes())?;
            Ok(0)
        }
        req::PEEKUSR => {
            let area = state::user_area(addr).ok_or(Errno::EIO)?;
            let value = with_stop(k, target, |s| match area {
                UserArea::Reg(i) => s.regs[i],
                _ => 0,
            })?;
            k.copy_to_user(out, &value.to_le_bytes())?;
            Ok(0)
        }
        req::POKEUSR => {
            let area = state::user_area(addr).ok_or(Errno::EIO)?;
            match area {
                UserArea::Reg(i) => {
                    check_reg(i, data)?;
                    with_stop(k, target, |s| {
                        s.regs[i] = data;
                        s.regs_changed = true;
                    })
                }
                // No hardware breakpoints: only clearing a debug register.
                UserArea::DebugReg(_) if data == 0 => Ok(()),
                _ => Err(Errno::EIO),
            }?;
            Ok(0)
        }
        req::GETREGS => {
            let regs = with_stop(k, target, |s| s.regs)?;
            k.copy_to_user(out, &state::regs_to_bytes(&regs))?;
            Ok(0)
        }
        req::SETREGS => {
            let mut b = [0u8; REGS_SIZE];
            k.copy_from_user(out, &mut b)?;
            set_regs(k, target, &b)?;
            Ok(0)
        }
        req::GETFPREGS => {
            let fpu = with_stop(k, target, |s| fxsave(&s.fpu))?;
            k.copy_to_user(out, &fpu)?;
            Ok(0)
        }
        req::SETFPREGS => {
            let mut b = [0u8; FPREGS_SIZE];
            k.copy_from_user(out, &mut b)?;
            set_fpregs(k, target, &b)?;
            Ok(0)
        }
        req::GETREGSET => regset(k, target, addr, out, false),
        req::SETREGSET => regset(k, target, addr, out, true),
        req::GETSIGINFO => {
            let info = with_stop(k, target, |s| s.info)?;
            k.copy_to_user(out, &info.to_bytes())?;
            Ok(0)
        }
        req::SETSIGINFO => {
            let mut b = [0u8; SigInfo::SIZE];
            k.copy_from_user(out, &mut b)?;
            let info = SigInfo::from_bytes(&b);
            with_stop(k, target, |s| s.info = info)?;
            Ok(0)
        }
        req::GETEVENTMSG => {
            let msg = with_stop(k, target, |s| s.message)?;
            k.copy_to_user(out, &msg.to_le_bytes())?;
            Ok(0)
        }
        req::SETOPTIONS => {
            if data & !u64::from(opt::MASK) != 0 {
                return Err(Errno::EINVAL);
            }
            with_tracee(k, target, |l| l.options = data as u32)?;
            Ok(0)
        }
        req::GETSIGMASK => {
            if addr != 8 {
                return Err(Errno::EINVAL);
            }
            let mask = with_stop(k, target, |s| s.mask)?;
            k.copy_to_user(out, &mask.to_le_bytes())?;
            Ok(0)
        }
        req::SETSIGMASK => {
            if addr != 8 {
                return Err(Errno::EINVAL);
            }
            let mut b = [0u8; 8];
            k.copy_from_user(out, &mut b)?;
            let mask = SigSet(u64::from_le_bytes(b)).without(SigSet::UNBLOCKABLE).0;
            with_stop(k, target, |s| {
                s.mask = mask;
                s.mask_changed = true;
            })?;
            Ok(0)
        }
        req::GET_SYSCALL_INFO => {
            let (b, len) =
                with_stop(k, target, |s| state::syscall_info(&s.op, s.regs[reg::RIP], s.regs[reg::RSP]))?;
            let n = (addr as usize).min(len);
            k.copy_to_user(out, &b[..n])?;
            Ok(len as u64)
        }
        req::CONT | req::SYSCALL | req::SINGLESTEP | req::DETACH => {
            if data > NSIG {
                return Err(Errno::EIO);
            }
            let how = match request {
                req::CONT => How::Cont,
                req::SYSCALL => How::Syscall,
                req::SINGLESTEP => How::Step,
                _ => How::Detach,
            };
            restart(k, target, Resume { how, signal: data as u32 })?;
            Ok(0)
        }
        req::KILL => {
            signal_process(k, target, SigInfo::new(SIGKILL, SI_USER));
            let _ = restart(k, target, Resume { how: How::Cont, signal: 0 });
            Ok(0)
        }
        req::INTERRUPT => {
            let seized = with_tree(k, |t| {
                let Some(l) = t.tracee_mut(target) else { return false };
                if !l.seized {
                    return false;
                }
                if l.stopped().is_none() {
                    l.interrupt = true;
                }
                t.touch();
                true
            })?;
            if !seized {
                return Err(Errno::EIO);
            }
            k.notify_signal(target);
            Ok(0)
        }
        _ => Err(Errno::EIO),
    }
}

/// The FXSAVE area (`struct user_fpregs_struct`) of an FPU image.
fn fxsave(image: &[u8]) -> [u8; FPREGS_SIZE] {
    let mut b = [0u8; FPREGS_SIZE];
    let n = image.len().min(FPREGS_SIZE);
    b[..n].copy_from_slice(&image[..n]);
    b
}

/// A register value `POKEUSER` / `SETREGS` may store: the selectors stay
/// Linux's user ones, the bases user addresses (`EIO`).
fn check_reg(i: usize, v: u64) -> Result<(), Errno> {
    let ok = match i {
        reg::CS => v == state::USER_CS,
        reg::SS => v == state::USER_DS,
        reg::DS | reg::ES | reg::FS | reg::GS => v == 0 || v == state::USER_DS,
        reg::FS_BASE | reg::GS_BASE => v < USER_ADDRESS_LIMIT,
        _ => true,
    };
    if ok { Ok(()) } else { Err(Errno::EIO) }
}

fn set_regs(k: &mut dyn LinuxKernel, target: i32, b: &[u8]) -> Result<(), Errno> {
    let mut r = state::regs_from_bytes(b);
    // The selectors are not the tracer's to change.
    r[reg::CS] = state::USER_CS;
    r[reg::SS] = state::USER_DS;
    for i in [reg::FS_BASE, reg::GS_BASE] {
        check_reg(i, r[i])?;
    }
    with_stop(k, target, |s| {
        let keep = (s.regs[reg::DS], s.regs[reg::ES], s.regs[reg::FS], s.regs[reg::GS]);
        s.regs = r;
        (s.regs[reg::DS], s.regs[reg::ES], s.regs[reg::FS], s.regs[reg::GS]) = keep;
        s.regs_changed = true;
    })
}

fn set_fpregs(k: &mut dyn LinuxKernel, target: i32, b: &[u8]) -> Result<(), Errno> {
    with_stop(k, target, |s| {
        let n = b.len().min(s.fpu.len());
        s.fpu[..n].copy_from_slice(&b[..n]);
        s.fpu_changed = true;
    })
}

/// `PTRACE_GETREGSET` / `PTRACE_SETREGSET` of note type `note` through the
/// `struct iovec` at `iov`: at most `iov_len` bytes, and `iov_len` set to
/// what was copied. `EINVAL` for a note type Poly does not know.
fn regset(k: &mut dyn LinuxKernel, target: i32, note: u64, iov: UserPtr, set: bool) -> SysResult {
    let mut v = [0u8; 16];
    k.copy_from_user(iov, &mut v)?;
    let base = UserPtr(u64::from_le_bytes(v[0..8].try_into().expect("8 bytes")));
    let len = u64::from_le_bytes(v[8..16].try_into().expect("8 bytes")) as usize;
    let bytes: Vec<u8> = match note {
        NT_PRSTATUS => state::regs_to_bytes(&with_stop(k, target, |s| s.regs)?).to_vec(),
        NT_PRFPREG => fxsave(&with_stop(k, target, |s| fxsave(&s.fpu))?).to_vec(),
        NT_X86_XSTATE => {
            let image = with_stop(k, target, |s| s.fpu.clone())?;
            if image.len() <= FPREGS_SIZE {
                return Err(Errno::ENODEV);
            }
            image
        }
        _ => return Err(Errno::EINVAL),
    };
    let n = len.min(bytes.len());
    if set {
        let mut new = bytes;
        k.copy_from_user(base, &mut new[..n])?;
        match note {
            NT_PRSTATUS => set_regs(k, target, &new)?,
            _ => set_fpregs(k, target, &new)?,
        }
    } else {
        k.copy_to_user(base, &bytes[..n])?;
    }
    k.copy_to_user(UserPtr(iov.0 + 8), &(n as u64).to_le_bytes())?;
    Ok(0)
}

/// Run `f` on the tracee's link (`ESRCH` if gone meanwhile).
fn with_tracee<R>(k: &mut dyn LinuxKernel, target: i32, f: impl FnOnce(&mut Tracee) -> R) -> Result<R, Errno> {
    let me = k.pid();
    with_tree(k, |t| t.tracee_mut(target).filter(|l| l.traced_by(me)).map(f))?.ok_or(Errno::ESRCH)
}

/// Run `f` on the stopped tracee's saved state (`ESRCH` if it runs).
fn with_stop<R>(k: &mut dyn LinuxKernel, target: i32, f: impl FnOnce(&mut Stop) -> R) -> Result<R, Errno> {
    with_tracee(k, target, |l| l.stopped_mut().map(f))?.ok_or(Errno::ESRCH)
}

/// Restart the stopped tracee (a detach also ends the link for the
/// tracer at once).
fn restart(k: &mut dyn LinuxKernel, target: i32, resume: Resume) -> Result<(), Errno> {
    let me = k.pid();
    let done = with_tree(k, |t| {
        let Some(l) = t.tracee_mut(target).filter(|l| l.traced_by(me) && l.stopped().is_some()) else {
            return false;
        };
        l.resume = Some(resume);
        if resume.how == How::Detach {
            l.detaching = true;
        }
        t.touch();
        true
    })?;
    if done { Ok(()) } else { Err(Errno::ESRCH) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::task::ptrace::Tracee;

    fn stopped(tracer: i32) -> Tracee {
        let mut l = Tracee::new(tracer, true, 0);
        l.stop = Some(Box::new(Stop {
            signal: SIGTRAP,
            event: event::STOP,
            delivery: false,
            info: state::trap_info(SIGTRAP, event::STOP),
            message: 0,
            regs: [0; NREGS],
            regs_changed: false,
            fpu: alloc::vec![0; 512],
            fpu_changed: false,
            mask: 0,
            mask_changed: false,
            op: SyscallOp::None,
            reported: false,
        }));
        l
    }

    #[test]
    fn inheritance_follows_the_options() {
        let mut l = Tracee::new(1, true, opt::TRACEFORK);
        assert_eq!(inherits(&l, 0, SIGCHLD), Some(event::FORK));
        assert_eq!(inherits(&l, 0x4000 | 0x100, SIGCHLD), None, "vfork not asked for");
        assert_eq!(inherits(&l, 0x0080_0000, SIGCHLD), None, "CLONE_UNTRACED");
        l.options |= opt::TRACEVFORK;
        assert_eq!(inherits(&l, 0x4000 | 0x100, SIGCHLD), Some(event::VFORK));
        assert_eq!(inherits(&Tracee::new(1, false, 0), 0x2000, SIGCHLD), Some(event::FORK), "CLONE_PTRACE");
    }

    #[test]
    fn registers_checked_like_linux() {
        assert_eq!(check_reg(reg::CS, 0x33), Ok(()));
        assert_eq!(check_reg(reg::CS, 0x10), Err(Errno::EIO));
        assert_eq!(check_reg(reg::FS_BASE, USER_ADDRESS_LIMIT), Err(Errno::EIO));
        assert_eq!(check_reg(reg::RAX, u64::MAX), Ok(()));
    }

    #[test]
    fn a_stop_is_waited_for_once_and_restart_needs_it() {
        use crate::task::tree::{Clones, Poll, Selector, WaitFilter};
        let mut t = ProcessTree::new();
        t.insert_root(1, 0);
        t.insert_child(1, 1, 10, SIGCHLD, 0);
        t.insert_child(10, 10, 11, SIGCHLD, 0);
        // 1 traces its grandchild 11 (not its child).
        t.attach(11, stopped(1));
        let filter =
            WaitFilter { selector: Selector::Pid(11), ended: true, stopped: false, continued: false, consume: true, clones: Clones::Include };
        match t.poll(1, &filter) {
            Poll::Ready(w) => assert_eq!(wait::status_word(w.event), (5 << 8) | 0x7f | (128 << 16)),
            other => panic!("{other:?}"),
        }
        assert_eq!(t.poll(1, &filter), Poll::Pending, "reported once");
        // The parent does not see the tracer's stops.
        let any = WaitFilter { selector: Selector::Any, stopped: true, ..filter };
        assert_eq!(t.poll(10, &any), Poll::Pending);
        // The tracee's end goes to the tracer first, then to the parent.
        t.tracee_mut(11).unwrap().stop = None;
        t.exit(11, crate::task::tree::Termination::Exited(5), false);
        assert_eq!(t.poll(10, &any), Poll::Pending, "hidden from the parent while traced");
        assert!(matches!(t.poll(1, &filter), Poll::Ready(_)));
        assert!(t.tracee(11).is_none());
        assert!(matches!(t.poll(10, &any), Poll::Ready(_)), "then the parent reaps it");
    }
}
