//! ptrace(2) state: who traces whom, and what a tracee in a ptrace-stop
//! shows its tracer (ADR 0134).
//!
//! The links live in the process tree ([`super::tree::ProcessTree`]),
//! keyed by the tracee's thread id, so the tracer's wait(2) finds its
//! tracees' stops and ends in the same pass as its children's, and both
//! sides block on the tree's generation. Poly traces processes as single
//! threads (the tracee's id is its main thread's; new threads of a
//! tracee are not attached).
//!
//! A tracee stops itself: at a stop it saves its registers, FPU area and
//! signal mask in its [`Stop`] and waits; the tracer's requests read and
//! change that copy, and the tracee loads what changed when it is
//! restarted ([`Resume`]). Nothing reaches into another thread's state.
//!
//! Constants are those of `linux/ptrace.h` and `asm/ptrace-abi.h`; the
//! register layout is `struct user_regs_struct` and the `PEEKUSER` area
//! `struct user` of `asm/user.h` (x86-64).

use crate::abi::SigInfo;
use crate::abi::signal::SIGTRAP;
use crate::kernel::UserContext;
use alloc::boxed::Box;
use alloc::vec::Vec;

/// Requests (`linux/ptrace.h`, `asm/ptrace-abi.h`).
pub mod req {
    pub const TRACEME: u64 = 0;
    pub const PEEKTEXT: u64 = 1;
    pub const PEEKDATA: u64 = 2;
    pub const PEEKUSR: u64 = 3;
    pub const POKETEXT: u64 = 4;
    pub const POKEDATA: u64 = 5;
    pub const POKEUSR: u64 = 6;
    pub const CONT: u64 = 7;
    pub const KILL: u64 = 8;
    pub const SINGLESTEP: u64 = 9;
    pub const GETREGS: u64 = 12;
    pub const SETREGS: u64 = 13;
    pub const GETFPREGS: u64 = 14;
    pub const SETFPREGS: u64 = 15;
    pub const ATTACH: u64 = 16;
    pub const DETACH: u64 = 17;
    pub const SYSCALL: u64 = 24;
    pub const SETOPTIONS: u64 = 0x4200;
    pub const GETEVENTMSG: u64 = 0x4201;
    pub const GETSIGINFO: u64 = 0x4202;
    pub const SETSIGINFO: u64 = 0x4203;
    pub const GETREGSET: u64 = 0x4204;
    pub const SETREGSET: u64 = 0x4205;
    pub const SEIZE: u64 = 0x4206;
    pub const INTERRUPT: u64 = 0x4207;
    pub const LISTEN: u64 = 0x4208;
    pub const GETSIGMASK: u64 = 0x420a;
    pub const SETSIGMASK: u64 = 0x420b;
    pub const GET_SYSCALL_INFO: u64 = 0x420e;
}

/// Options (`PTRACE_O_*`, `linux/ptrace.h`).
pub mod opt {
    pub const TRACESYSGOOD: u32 = 0x1;
    pub const TRACEFORK: u32 = 0x2;
    pub const TRACEVFORK: u32 = 0x4;
    pub const TRACECLONE: u32 = 0x8;
    pub const TRACEEXEC: u32 = 0x10;
    pub const TRACEVFORKDONE: u32 = 0x20;
    pub const TRACEEXIT: u32 = 0x40;
    pub const TRACESECCOMP: u32 = 0x80;
    pub const EXITKILL: u32 = 0x10_0000;
    pub const SUSPEND_SECCOMP: u32 = 0x20_0000;
    /// Every option bit Linux knows (`PTRACE_O_MASK`).
    pub const MASK: u32 = 0xff | EXITKILL | SUSPEND_SECCOMP;
}

/// Event numbers (`PTRACE_EVENT_*`, `linux/ptrace.h`): a stop's status
/// carries one in bits 16..23.
pub mod event {
    pub const FORK: u32 = 1;
    pub const VFORK: u32 = 2;
    pub const CLONE: u32 = 3;
    pub const EXEC: u32 = 4;
    pub const VFORK_DONE: u32 = 5;
    pub const EXIT: u32 = 6;
    pub const STOP: u32 = 128;
}

/// The option that asks for the stop of event `e` (`PTRACE_O_TRACEFORK`
/// is `1 << PTRACE_EVENT_FORK`, and so on up to `PTRACE_EVENT_EXIT`).
pub const fn option_for(e: u32) -> u32 {
    if e >= 1 && e <= event::EXIT { 1 << e } else { 0 }
}

/// `PTRACE_SYSCALL_INFO_*` (`linux/ptrace.h`).
pub const SYSCALL_INFO_NONE: u8 = 0;
pub const SYSCALL_INFO_ENTRY: u8 = 1;
pub const SYSCALL_INFO_EXIT: u8 = 2;
/// `AUDIT_ARCH_X86_64` (`linux/audit.h`).
pub const AUDIT_ARCH_X86_64: u32 = 0xc000_003e;

/// Register set note types (`NT_*`, `linux/elf.h`).
pub const NT_PRSTATUS: u64 = 1;
pub const NT_PRFPREG: u64 = 2;
pub const NT_X86_XSTATE: u64 = 0x202;

/// `struct user_regs_struct`: 27 registers in this order.
pub const NREGS: usize = 27;
pub const REGS_SIZE: usize = NREGS * 8;
/// `struct user_fpregs_struct`: the 512-byte FXSAVE area.
pub const FPREGS_SIZE: usize = 512;

/// Indexes into `struct user_regs_struct` (`asm/user.h`).
pub mod reg {
    pub const R15: usize = 0;
    pub const R14: usize = 1;
    pub const R13: usize = 2;
    pub const R12: usize = 3;
    pub const RBP: usize = 4;
    pub const RBX: usize = 5;
    pub const R11: usize = 6;
    pub const R10: usize = 7;
    pub const R9: usize = 8;
    pub const R8: usize = 9;
    pub const RAX: usize = 10;
    pub const RCX: usize = 11;
    pub const RDX: usize = 12;
    pub const RSI: usize = 13;
    pub const RDI: usize = 14;
    pub const ORIG_RAX: usize = 15;
    pub const RIP: usize = 16;
    pub const CS: usize = 17;
    pub const EFLAGS: usize = 18;
    pub const RSP: usize = 19;
    pub const SS: usize = 20;
    pub const FS_BASE: usize = 21;
    pub const GS_BASE: usize = 22;
    pub const DS: usize = 23;
    pub const ES: usize = 24;
    pub const FS: usize = 25;
    pub const GS: usize = 26;
}

/// The 64-bit user code and stack selectors Linux gives every process
/// (`__USER_CS`, `__USER_DS`, `asm/segment.h`).
pub const USER_CS: u64 = 0x33;
pub const USER_DS: u64 = 0x2b;

/// `struct user` (`asm/user.h`, x86-64): `regs` (216 bytes), `u_fpvalid`
/// and padding, `i387` (512), `u_tsize`, `u_dsize`, `u_ssize`,
/// `start_code`, `start_stack`, `signal`, `reserved` and padding, `u_ar0`,
/// `u_fpstate`, `magic`, `u_comm[32]`, `u_debugreg[8]`: 912 bytes, the
/// debug registers at 848.
pub const USER_SIZE: u64 = 912;
pub const USER_DEBUGREG: u64 = 848;

/// The registers of a [`UserContext`] as `struct user_regs_struct`.
pub fn regs_of(ctx: &UserContext, orig_rax: u64, fs_base: u64, gs_base: u64) -> [u64; NREGS] {
    let mut r = [0u64; NREGS];
    r[reg::R15] = ctx.r15;
    r[reg::R14] = ctx.r14;
    r[reg::R13] = ctx.r13;
    r[reg::R12] = ctx.r12;
    r[reg::RBP] = ctx.rbp;
    r[reg::RBX] = ctx.rbx;
    r[reg::R11] = ctx.r11;
    r[reg::R10] = ctx.r10;
    r[reg::R9] = ctx.r9;
    r[reg::R8] = ctx.r8;
    r[reg::RAX] = ctx.rax;
    r[reg::RCX] = ctx.rcx;
    r[reg::RDX] = ctx.rdx;
    r[reg::RSI] = ctx.rsi;
    r[reg::RDI] = ctx.rdi;
    r[reg::ORIG_RAX] = orig_rax;
    r[reg::RIP] = ctx.rip;
    r[reg::CS] = USER_CS;
    r[reg::EFLAGS] = ctx.rflags;
    r[reg::RSP] = ctx.rsp;
    r[reg::SS] = USER_DS;
    r[reg::FS_BASE] = fs_base;
    r[reg::GS_BASE] = gs_base;
    r
}

/// Load `r` into `ctx` (the selectors stay as they are).
pub fn apply_regs(r: &[u64; NREGS], ctx: &mut UserContext) {
    ctx.r15 = r[reg::R15];
    ctx.r14 = r[reg::R14];
    ctx.r13 = r[reg::R13];
    ctx.r12 = r[reg::R12];
    ctx.rbp = r[reg::RBP];
    ctx.rbx = r[reg::RBX];
    ctx.r11 = r[reg::R11];
    ctx.r10 = r[reg::R10];
    ctx.r9 = r[reg::R9];
    ctx.r8 = r[reg::R8];
    ctx.rax = r[reg::RAX];
    ctx.rcx = r[reg::RCX];
    ctx.rdx = r[reg::RDX];
    ctx.rsi = r[reg::RSI];
    ctx.rdi = r[reg::RDI];
    ctx.rip = r[reg::RIP];
    ctx.rflags = r[reg::EFLAGS];
    ctx.rsp = r[reg::RSP];
}

pub fn regs_to_bytes(r: &[u64; NREGS]) -> [u8; REGS_SIZE] {
    let mut b = [0u8; REGS_SIZE];
    for (i, v) in r.iter().enumerate() {
        b[i * 8..i * 8 + 8].copy_from_slice(&v.to_le_bytes());
    }
    b
}

pub fn regs_from_bytes(b: &[u8]) -> [u64; NREGS] {
    let mut r = [0u64; NREGS];
    for (i, chunk) in b.chunks_exact(8).take(NREGS).enumerate() {
        r[i] = u64::from_le_bytes(chunk.try_into().expect("8 bytes"));
    }
    r
}

/// Where `PEEKUSER` / `POKEUSER` offset `off` falls: `EIO` (`None`) for an
/// unaligned offset or one past `struct user`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UserArea {
    Reg(usize),
    DebugReg(usize),
    Other,
}

pub const fn user_area(off: u64) -> Option<UserArea> {
    if off & 7 != 0 || off >= USER_SIZE {
        return None;
    }
    Some(if off < REGS_SIZE as u64 {
        UserArea::Reg((off / 8) as usize)
    } else if off >= USER_DEBUGREG {
        UserArea::DebugReg(((off - USER_DEBUGREG) / 8) as usize)
    } else {
        UserArea::Other
    })
}

/// What `PTRACE_GET_SYSCALL_INFO` reports about the stop.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SyscallOp {
    None,
    Entry { nr: u64, args: [u64; 6] },
    Exit { rval: i64, is_error: bool },
}

/// `struct ptrace_syscall_info` for `op` (`linux/ptrace.h`): `op`, three
/// pad bytes, `arch`, `instruction_pointer`, `stack_pointer`, then the
/// entry (`nr`, `args[6]`) or exit (`rval`, `is_error`) member. Returns
/// the bytes and the size the call reports (24, 80 or 33).
pub fn syscall_info(op: &SyscallOp, ip: u64, sp: u64) -> ([u8; 80], usize) {
    let mut b = [0u8; 80];
    b[4..8].copy_from_slice(&AUDIT_ARCH_X86_64.to_le_bytes());
    b[8..16].copy_from_slice(&ip.to_le_bytes());
    b[16..24].copy_from_slice(&sp.to_le_bytes());
    let len = match *op {
        SyscallOp::None => 24,
        SyscallOp::Entry { nr, args } => {
            b[0] = SYSCALL_INFO_ENTRY;
            b[24..32].copy_from_slice(&nr.to_le_bytes());
            for (i, a) in args.iter().enumerate() {
                b[32 + i * 8..40 + i * 8].copy_from_slice(&a.to_le_bytes());
            }
            80
        }
        SyscallOp::Exit { rval, is_error } => {
            b[0] = SYSCALL_INFO_EXIT;
            b[24..32].copy_from_slice(&rval.to_le_bytes());
            b[32] = u8::from(is_error);
            33
        }
    };
    (b, len)
}

/// How the tracer restarted a stopped tracee.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum How {
    /// `PTRACE_CONT` (and `PTRACE_KILL` of a stopped tracee).
    Cont,
    /// `PTRACE_SYSCALL`: stop at the next system call entry or exit.
    Syscall,
    /// `PTRACE_SINGLESTEP`: run one instruction (the trap flag).
    Step,
    /// `PTRACE_DETACH`: run on untraced.
    Detach,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Resume {
    pub how: How,
    /// The signal to deliver (signal-delivery-stops only; 0: none).
    pub signal: u32,
}

/// What a tracee in a ptrace-stop shows its tracer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stop {
    /// `WSTOPSIG`: the signal, `SIGTRAP`, or `SIGTRAP | 0x80`.
    pub signal: u32,
    /// The event (status >> 16), 0 for none.
    pub event: u32,
    /// The stop is for a signal about to be delivered (its signal may be
    /// replaced by the restarting request's).
    pub delivery: bool,
    /// `PTRACE_GETSIGINFO`.
    pub info: SigInfo,
    /// `PTRACE_GETEVENTMSG`.
    pub message: u64,
    pub regs: [u64; NREGS],
    pub regs_changed: bool,
    /// The FPU state as the tracee keeps it (an FXSAVE or XSAVE image).
    pub fpu: Vec<u8>,
    pub fpu_changed: bool,
    pub mask: u64,
    pub mask_changed: bool,
    pub op: SyscallOp,
    /// A wait of the tracer has reported it.
    pub reported: bool,
}

impl Stop {
    /// The status word's stop signal with the event: what
    /// [`super::tree::ChildEvent::Trapped`] carries.
    pub const fn code(&self) -> u32 {
        (self.signal & 0xff) | (self.event << 8)
    }
}

/// A tracee: its tracer and its state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tracee {
    /// The tracer's process id.
    pub tracer: i32,
    /// Attached with `PTRACE_SEIZE` (`PTRACE_INTERRUPT` is allowed, new
    /// children start in a `PTRACE_EVENT_STOP`).
    pub seized: bool,
    pub options: u32,
    /// In a ptrace-stop.
    pub stop: Option<Box<Stop>>,
    /// The tracer's restart, for the tracee to pick up.
    pub resume: Option<Resume>,
    /// `PTRACE_DETACH` was asked: no longer the tracer's for requests and
    /// waits; the tracee drops the link when it runs.
    pub detaching: bool,
    /// `PTRACE_INTERRUPT` of a running tracee: it stops at once.
    pub interrupt: bool,
    /// Auto-attached child: its first stop is still to come.
    pub start_stop: bool,
    /// `PTRACE_SYSCALL`: stop at the next system call entry or exit.
    pub syscall_trace: bool,
    /// `PTRACE_SINGLESTEP` set the trap flag, which the next stop clears.
    pub stepping: bool,
    /// The system call the tracee is in (`orig_rax`), if any.
    pub in_syscall: Option<u64>,
}

impl Tracee {
    pub fn new(tracer: i32, seized: bool, options: u32) -> Tracee {
        Tracee {
            tracer,
            seized,
            options,
            stop: None,
            resume: None,
            detaching: false,
            interrupt: false,
            start_stop: false,
            syscall_trace: false,
            stepping: false,
            in_syscall: None,
        }
    }

    /// Traced by `tracer` for its requests and waits.
    pub fn traced_by(&self, tracer: i32) -> bool {
        self.tracer == tracer && !self.detaching
    }

    /// In a ptrace-stop the tracer has not restarted yet: the state
    /// requests other than `KILL` and `INTERRUPT` need.
    pub fn stopped(&self) -> Option<&Stop> {
        self.stop.as_deref().filter(|_| self.resume.is_none())
    }

    pub fn stopped_mut(&mut self) -> Option<&mut Stop> {
        if self.resume.is_some() { None } else { self.stop.as_deref_mut() }
    }
}

/// The event a clone(2) reports, by its flags (`linux/sched.h`):
/// `CLONE_VFORK` is a vfork, a plain `SIGCHLD` child a fork, anything else
/// a clone.
pub const fn clone_event(flags: u64, exit_signal: u32) -> u32 {
    const CLONE_VFORK: u64 = 0x4000;
    if flags & CLONE_VFORK != 0 {
        event::VFORK
    } else if exit_signal == crate::abi::signal::SIGCHLD {
        event::FORK
    } else {
        event::CLONE
    }
}

/// The siginfo of a `SIGTRAP` stop that is no signal (an event or system
/// call stop): `si_code` is the stop signal with the event above it.
pub fn trap_info(signal: u32, event: u32) -> SigInfo {
    SigInfo::new(SIGTRAP, (signal | (event << 8)) as i32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_area_offsets() {
        assert_eq!(user_area(17 * 8), Some(UserArea::Reg(reg::CS)));
        assert_eq!(user_area(3), None, "unaligned");
        assert_eq!(user_area(0x10000), None, "past struct user");
        assert_eq!(user_area(u64::MAX - 7), None, "negative");
        assert_eq!(user_area(USER_DEBUGREG + 8), Some(UserArea::DebugReg(1)));
        assert_eq!(user_area(216), Some(UserArea::Other));
        assert_eq!(user_area(USER_SIZE - 8), Some(UserArea::DebugReg(7)));
    }

    #[test]
    fn registers_round_trip() {
        let ctx = UserContext { r12: 0x1234, rax: 7, rip: 0x401000, rsp: 0x7ff0, rflags: 0x246, ..Default::default() };
        let r = regs_of(&ctx, 62, 0x10, 0);
        assert_eq!((r[reg::ORIG_RAX], r[reg::CS], r[reg::SS]), (62, 0x33, 0x2b));
        let b = regs_to_bytes(&r);
        assert_eq!(b.len(), 216);
        assert_eq!(regs_from_bytes(&b), r);
        let mut back = UserContext::default();
        apply_regs(&r, &mut back);
        assert_eq!((back.r12, back.rax, back.rip, back.rsp), (0x1234, 7, 0x401000, 0x7ff0));
    }

    #[test]
    fn syscall_info_sizes() {
        assert_eq!(syscall_info(&SyscallOp::None, 1, 2).1, 24);
        let (b, n) = syscall_info(&SyscallOp::Entry { nr: 39, args: [1, 2, 3, 4, 5, 6] }, 1, 2);
        assert_eq!((n, b[0], u64::from_le_bytes(b[24..32].try_into().unwrap())), (80, 1, 39));
        assert_eq!(u32::from_le_bytes(b[4..8].try_into().unwrap()), AUDIT_ARCH_X86_64);
        let (b, n) = syscall_info(&SyscallOp::Exit { rval: -2, is_error: true }, 1, 2);
        assert_eq!((n, b[0], b[32]), (33, 2, 1));
    }

    #[test]
    fn events_and_options() {
        assert_eq!(option_for(event::FORK), opt::TRACEFORK);
        assert_eq!(option_for(event::VFORK_DONE), opt::TRACEVFORKDONE);
        assert_eq!(option_for(event::EXIT), opt::TRACEEXIT);
        assert_eq!(option_for(event::STOP), 0);
        assert_eq!(clone_event(0x4000 | 0x100, 17), event::VFORK);
        assert_eq!(clone_event(0, 17), event::FORK);
        assert_eq!(clone_event(0x10000 | 0x100, 0), event::CLONE);
        assert_eq!(trap_info(SIGTRAP, event::STOP).code, 5 | (128 << 8));
    }
}
