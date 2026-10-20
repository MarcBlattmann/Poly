//! Secure computing (seccomp(2), prctl(2) `PR_SET_SECCOMP`): a thread's
//! strict mode or its stack of classic BPF filters, consulted before every
//! system call it makes (ADR 0054).
//!
//! - **State.** Each thread's [`Seccomp`] lives in its
//!   [`LinuxThread`](crate::signal::LinuxThread): disabled, strict, or a
//!   chain of [`Filter`]s, newest first, shared with `Arc` between the
//!   threads and processes that inherited it (fork(2), clone(2) and
//!   execve(2) keep it: seccomp(2), "the filters are inherited ... and
//!   preserved across execve(2)").
//! - **The check** ([`check`]) runs in [`Linux::dispatch_with`]
//!   before the call's handler, only when the kernel says the process has
//!   seccomp state at all ([`LinuxKernel::seccomp_active`]), so a process
//!   without filters pays one flag test per call. Every filter of the
//!   chain runs on the call's `struct seccomp_data`; the action of the
//!   highest precedence wins ([`precedence`]).
//! - **Actions**: `KILL_PROCESS` and unknown values (seccomp(2): "treated
//!   as ... SECCOMP_RET_KILL_PROCESS") end the process as if by `SIGSYS`
//!   (with a core dump status); `KILL_THREAD` ends the calling thread, or
//!   the process the same way when it is the only thread; `TRAP` sends
//!   `SIGSYS` (`SYS_SECCOMP`, `si_call_addr`, `si_syscall`, `si_arch`,
//!   `si_errno` = the data); `ERRNO` returns `-data` (at most 4095, the
//!   largest errno); `TRACE` without a tracer and `USER_NOTIF` without a
//!   listener (Poly has neither) fail the call with `ENOSYS`; `LOG` runs
//!   the call and tells the kernel to log it; `ALLOW` runs it. A call not
//!   run leaves `rax` as the action says; after `TRAP` it is `-ENOSYS`
//!   unless the handler changes it (Poly's choice, to be confirmed by the
//!   diff runner).
//! - **Strict mode**: only read(2), write(2), _exit(2) and rt_sigreturn(2);
//!   anything else is `SIGKILL` for the process.
//!
//! [`Linux::dispatch_with`]: crate::Linux::dispatch_with

pub mod abi;

use crate::abi::SigInfo;
use crate::abi::signal::{SIGKILL, SIGSYS};
use crate::errno::Errno;
use crate::kernel::{LinuxKernel, UserPtr};
use crate::signal::deliver::{force_signal, with_own};
use crate::table::nr;
use abi::*;
use alloc::sync::Arc;
use alloc::vec::Vec;
use poly_abi::SyscallRegs;
use poly_bpf::{Insn, Program, Rules};

/// What a filter may load: `struct seccomp_data`, 16 words.
pub const RULES: Rules = Rules { data_words: (SECCOMP_DATA_SIZE / 4) as u32 };

/// One installed filter and the ones installed before it.
#[derive(Debug)]
pub struct Filter {
    program: Program,
    /// `SECCOMP_FILTER_FLAG_LOG`: log every action but `ALLOW`.
    log: bool,
    prev: Option<Arc<Filter>>,
    /// Instructions of this filter and every earlier one, each earlier one
    /// counted with [`PATH_PENALTY`] more (seccomp(2), `ENOMEM`).
    path_len: usize,
}

impl Filter {
    /// The filters from this one back to the first, newest first.
    pub fn chain(self: &Arc<Self>) -> impl Iterator<Item = &Filter> {
        let mut next = Some(&**self);
        core::iter::from_fn(move || {
            let f = next?;
            next = f.prev.as_deref();
            Some(f)
        })
    }

    /// How many filters the chain holds.
    pub fn count(self: &Arc<Self>) -> usize {
        self.chain().count()
    }

    /// Whether `ancestor` is this filter or one installed before it.
    fn descends_from(self: &Arc<Self>, ancestor: &Arc<Filter>) -> bool {
        let mut f = Some(self);
        while let Some(x) = f {
            if Arc::ptr_eq(x, ancestor) {
                return true;
            }
            f = x.prev.as_ref();
        }
        false
    }
}

/// A thread's secure computing mode (seccomp(2), `PR_GET_SECCOMP`).
#[derive(Clone, Debug, Default)]
pub enum Seccomp {
    /// `SECCOMP_MODE_DISABLED`.
    #[default]
    Disabled,
    /// `SECCOMP_MODE_STRICT`.
    Strict,
    /// `SECCOMP_MODE_FILTER` with the newest filter of the chain.
    Filter(Arc<Filter>),
}

impl PartialEq for Seccomp {
    /// Equal when the same filters (not merely equal programs: seccomp(2)
    /// counts "identical filters in separate seccomp() calls" as
    /// different) are installed.
    fn eq(&self, other: &Seccomp) -> bool {
        match (self, other) {
            (Seccomp::Disabled, Seccomp::Disabled) | (Seccomp::Strict, Seccomp::Strict) => true,
            (Seccomp::Filter(a), Seccomp::Filter(b)) => Arc::ptr_eq(a, b),
            _ => false,
        }
    }
}

impl Eq for Seccomp {}

impl Seccomp {
    /// `PR_GET_SECCOMP`'s value: 0, 1 or 2.
    pub fn mode(&self) -> u32 {
        match self {
            Seccomp::Disabled => SECCOMP_MODE_DISABLED,
            Seccomp::Strict => SECCOMP_MODE_STRICT,
            Seccomp::Filter(_) => SECCOMP_MODE_FILTER,
        }
    }

    /// How many filters are installed (`/proc/[pid]/status`
    /// `Seccomp_filters`).
    pub fn filters(&self) -> usize {
        match self {
            Seccomp::Filter(f) => f.count(),
            _ => 0,
        }
    }

    /// Whether this thread's filters could be replaced by `target`'s
    /// (`SECCOMP_FILTER_FLAG_TSYNC`): it is not in strict mode and its
    /// filters, if any, are `target`'s or ones before them.
    fn can_sync_to(&self, target: &Arc<Filter>) -> bool {
        match self {
            Seccomp::Disabled => true,
            Seccomp::Strict => false,
            Seccomp::Filter(mine) => target.descends_from(mine),
        }
    }
}

/// `struct seccomp_data` (UAPI `linux/seccomp.h`): `int nr; __u32 arch;
/// __u64 instruction_pointer; __u64 args[6];`, as the 16 words filters
/// load.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SeccompData {
    pub nr: i32,
    pub arch: u32,
    pub instruction_pointer: u64,
    pub args: [u64; 6],
}

impl SeccompData {
    pub fn words(&self) -> [u32; 16] {
        let mut w = [0u32; 16];
        w[0] = self.nr as u32;
        w[1] = self.arch;
        w[2] = self.instruction_pointer as u32;
        w[3] = (self.instruction_pointer >> 32) as u32;
        for (i, a) in self.args.iter().enumerate() {
            w[4 + 2 * i] = *a as u32;
            w[5 + 2 * i] = (*a >> 32) as u32;
        }
        w
    }
}

/// The precedence of a filter's return value: lower is stronger.
/// seccomp(2): "the return value for the evaluation of a given system call
/// will always use the highest precedence value": `KILL_PROCESS`,
/// `KILL_THREAD`, `TRAP`, `ERRNO`, `USER_NOTIF`, `TRACE`, `LOG`, `ALLOW`,
/// which is the order of the full action field read as a signed number.
pub const fn precedence(ret: u32) -> i32 {
    (ret & SECCOMP_RET_ACTION_FULL) as i32
}

/// Run every filter of the chain on `data` and return the value of the
/// highest precedence; among equal actions the newest filter's (to be
/// confirmed by the diff runner: seccomp(2) does not say whose data
/// wins). With the `LOG` flag of the filter that decided.
pub fn evaluate(filter: &Arc<Filter>, data: &SeccompData) -> (u32, bool) {
    let words = data.words();
    let mut best: Option<(u32, bool)> = None;
    for f in filter.chain() {
        let ret = f.program.run(&words);
        if best.is_none_or(|(b, _)| precedence(ret) < precedence(b)) {
            best = Some((ret, f.log));
        }
    }
    best.unwrap_or((SECCOMP_RET_ALLOW, false))
}

/// What a filter's value asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    KillProcess,
    KillThread,
    Trap(u16),
    Errno(u16),
    UserNotif,
    Trace(u16),
    Log,
    Allow,
}

impl Action {
    /// The action of a filter's return value; unknown actions are
    /// `KILL_PROCESS` (seccomp(2)).
    pub const fn of(ret: u32) -> Action {
        let data = (ret & SECCOMP_RET_DATA) as u16;
        match ret & SECCOMP_RET_ACTION_FULL {
            SECCOMP_RET_KILL_PROCESS => Action::KillProcess,
            SECCOMP_RET_KILL_THREAD => Action::KillThread,
            SECCOMP_RET_TRAP => Action::Trap(data),
            SECCOMP_RET_ERRNO => Action::Errno(data),
            SECCOMP_RET_USER_NOTIF => Action::UserNotif,
            SECCOMP_RET_TRACE => Action::Trace(data),
            SECCOMP_RET_LOG => Action::Log,
            SECCOMP_RET_ALLOW => Action::Allow,
            _ => Action::KillProcess,
        }
    }

    /// Whether `SECCOMP_GET_ACTION_AVAIL` reports the action as
    /// supported. `USER_NOTIF` is not: Poly cannot create listeners
    /// (`SECCOMP_FILTER_FLAG_NEW_LISTENER` is `EINVAL`).
    pub const fn available(action: u32) -> bool {
        matches!(
            action,
            SECCOMP_RET_KILL_PROCESS
                | SECCOMP_RET_KILL_THREAD
                | SECCOMP_RET_TRAP
                | SECCOMP_RET_ERRNO
                | SECCOMP_RET_TRACE
                | SECCOMP_RET_LOG
                | SECCOMP_RET_ALLOW
        )
    }
}

/// What the dispatcher does after [`check`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// Run the call.
    Run,
    /// Do not run it: `rax` holds the result, or the thread or process is
    /// ending.
    Skip,
}

/// The largest errno a system call returns (`MAX_ERRNO`, 4095: the top
/// page of `-errno` values the C library recognises as errors).
const MAX_ERRNO: u16 = 4095;

/// The calls strict mode allows (seccomp(2): "read(2), write(2), _exit(2)
/// (but not exit_group(2)), and sigreturn(2)").
const STRICT_ALLOWED: [u64; 4] = [nr::READ, nr::WRITE, nr::EXIT, nr::RT_SIGRETURN];

/// The seccomp check of the system call in `regs` (number in `rax`, the
/// arguments in their registers), for the calling thread. Called by the
/// dispatcher when [`LinuxKernel::seccomp_active`] says so.
pub fn check(k: &mut dyn LinuxKernel, regs: &mut SyscallRegs) -> Verdict {
    let tid = k.tid();
    let state = with_own(k, |p| if p.has_thread(tid) { p.thread(tid).seccomp.clone() } else { Seccomp::Disabled });
    let filter = match state {
        Seccomp::Disabled => return Verdict::Run,
        Seccomp::Strict => {
            if STRICT_ALLOWED.contains(&regs.rax) {
                return Verdict::Run;
            }
            k.seccomp_log(regs.rax, SECCOMP_RET_KILL_PROCESS);
            k.kill_process(SIGKILL, false);
            return Verdict::Skip;
        }
        Seccomp::Filter(f) => f,
    };
    let ip = k.user_context().rip;
    let data = SeccompData {
        nr: regs.rax as u32 as i32,
        arch: AUDIT_ARCH_X86_64,
        instruction_pointer: ip,
        args: [regs.rdi, regs.rsi, regs.rdx, regs.r10, regs.r8, regs.r9],
    };
    let regs_nr = data.nr;
    let (ret, log) = evaluate(&filter, &data);
    let action = Action::of(ret);
    if log && action != Action::Allow || action == Action::Log {
        k.seccomp_log(regs.rax, ret);
    }
    match action {
        Action::Allow | Action::Log => Verdict::Run,
        Action::Errno(e) => {
            regs.rax = (-i64::from(e.min(MAX_ERRNO))) as u64;
            Verdict::Skip
        }
        Action::Trace(_) | Action::UserNotif => {
            regs.rax = Errno::ENOSYS.to_rax();
            Verdict::Skip
        }
        Action::Trap(data) => {
            regs.rax = Errno::ENOSYS.to_rax();
            force_signal(k, sigsys(ip, regs_nr, data));
            Verdict::Skip
        }
        Action::KillThread => {
            k.seccomp_log(regs.rax, ret);
            let alone = with_own(k, |p| p.thread_ids().count() <= 1);
            if alone {
                k.kill_process(SIGSYS, true);
            } else {
                k.exit_thread(0);
            }
            Verdict::Skip
        }
        Action::KillProcess => {
            k.seccomp_log(regs.rax, ret);
            k.kill_process(SIGSYS, true);
            Verdict::Skip
        }
    }
}

/// The `SIGSYS` of `SECCOMP_RET_TRAP` (sigaction(2)): `si_code`
/// `SYS_SECCOMP`, `si_call_addr` "the address of the system call
/// instruction" (2 bytes before the return address `ip`), `si_syscall`,
/// `si_arch`, and `si_errno` the action's data. `_sigsys` in
/// `asm-generic/siginfo.h`: `void *_call_addr; int _syscall; unsigned
/// int _arch;` at offset 16.
pub fn sigsys(ip: u64, nr: i32, data: u16) -> SigInfo {
    let mut info = SigInfo::new(SIGSYS, SYS_SECCOMP);
    info.errno = i32::from(data);
    info.fields[0..8].copy_from_slice(&ip.wrapping_sub(2).to_le_bytes());
    info.fields[8..12].copy_from_slice(&nr.to_le_bytes());
    info.fields[12..16].copy_from_slice(&AUDIT_ARCH_X86_64.to_le_bytes());
    info
}

/// Read a `struct sock_fprog` (`unsigned short len;` then, 8-aligned, the
/// `struct sock_filter *`) and the program it points to, and check it.
/// `EFAULT`; `EINVAL` for a length of 0 or above `BPF_MAXINSNS`, or a
/// program the checker refuses.
pub fn read_program(k: &mut dyn LinuxKernel, fprog: UserPtr) -> Result<Program, Errno> {
    let mut header = [0u8; 16];
    k.copy_from_user(fprog, &mut header)?;
    let len = usize::from(u16::from_le_bytes([header[0], header[1]]));
    let ptr = UserPtr(u64::from_le_bytes(header[8..16].try_into().expect("8 bytes")));
    if len == 0 || len > poly_bpf::MAXINSNS {
        return Err(Errno::EINVAL);
    }
    let mut bytes = alloc::vec![0u8; len * Insn::SIZE];
    k.copy_from_user(ptr, &mut bytes)?;
    let insns: Vec<Insn> = Insn::parse(&bytes);
    Program::new(&insns, RULES).map_err(|_| Errno::EINVAL)
}

/// What [`install`] did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Installed {
    Done,
    /// `TSYNC`: thread `tid` could not take the filter, so nothing was
    /// installed.
    Blocked(i32),
}

/// Install `program` on the calling thread (`SECCOMP_SET_MODE_FILTER`),
/// and on every other thread of the process with `tsync`. Fails with
/// `EINVAL` in strict mode, `ENOMEM` past [`MAX_INSNS_PER_PATH`].
pub fn install(k: &mut dyn LinuxKernel, program: Program, log: bool, tsync: bool) -> Result<Installed, Errno> {
    let tid = k.tid();
    let done = with_own(k, |p| {
        let mine = p.thread(tid).seccomp.clone();
        let nnp = p.thread(tid).creds.no_new_privs;
        let prev = match mine {
            Seccomp::Strict => return Err(Errno::EINVAL),
            Seccomp::Disabled => None,
            Seccomp::Filter(f) => Some(f),
        };
        let path_len = program.len() + prev.as_ref().map_or(0, |f| f.path_len + PATH_PENALTY);
        if path_len > MAX_INSNS_PER_PATH {
            return Err(Errno::ENOMEM);
        }
        let new = Arc::new(Filter { program, log, prev, path_len });
        if tsync {
            let others: Vec<i32> = p.thread_ids().filter(|&t| t != tid).collect();
            if let Some(&blocked) = others.iter().find(|&&t| !p.thread(t).seccomp.can_sync_to(&new)) {
                return Ok(Installed::Blocked(blocked));
            }
            for t in others {
                let th = p.thread_mut(t);
                th.seccomp = Seccomp::Filter(Arc::clone(&new));
                // The caller's no_new_privs goes with the filter, so no
                // synchronised thread can gain privileges past it (Poly's
                // choice, to be confirmed by the diff runner).
                th.creds.no_new_privs |= nnp;
            }
        }
        p.thread_mut(tid).seccomp = Seccomp::Filter(new);
        Ok(Installed::Done)
    })?;
    if done == Installed::Done {
        k.seccomp_installed();
    }
    Ok(done)
}

/// Put the calling thread in strict mode (`SECCOMP_SET_MODE_STRICT`,
/// `PR_SET_SECCOMP` with `SECCOMP_MODE_STRICT`). `EINVAL` if it has
/// filters (seccomp(2): "A secure computing mode has already been set,
/// and operation differs from the existing setting").
pub fn set_strict(k: &mut dyn LinuxKernel) -> Result<(), Errno> {
    let tid = k.tid();
    with_own(k, |p| {
        let t = p.thread_mut(tid);
        match t.seccomp {
            Seccomp::Filter(_) => Err(Errno::EINVAL),
            _ => {
                t.seccomp = Seccomp::Strict;
                Ok(())
            }
        }
    })?;
    k.seccomp_installed();
    Ok(())
}
