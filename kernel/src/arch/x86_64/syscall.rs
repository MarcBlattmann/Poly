//! The `syscall` entry, the return to ring 3 and the per-CPU area.
//!
//! # Entering the kernel
//!
//! `SYSCALL` (SDM Vol. 2B) loads CS/SS from IA32_STAR, RIP from IA32_LSTAR,
//! saves the user RIP in RCX and RFLAGS in R11, and clears the RFLAGS bits
//! in IA32_FMASK: here IF (interrupts stay off until the entry is done),
//! TF, DF, AC (no SMAP bypass), NT and IOPL. It does *not* switch stacks.
//! The entry stub (`syscall_entry`) therefore:
//!
//! 1. `swapgs`: GS base becomes this CPU's per-CPU area (`percpu.rs`; IA32_KERNEL_GS_BASE
//!    held it while user mode ran; the user's GS base goes there);
//! 2. parks the user RSP in the per-CPU area and loads the running thread's
//!    kernel stack top from it;
//! 3. pushes a [`TrapFrame`] like the one an interrupt from ring 3 leaves
//!    (user SS, RSP, RFLAGS, CS, RIP, a dummy error code and vector
//!    [`SYSCALL_VECTOR`], then every general register), so the rest of the
//!    kernel sees one format for "user registers";
//! 4. calls [`syscall_handler`] with `rbp = 0`, so a backtrace ends there.
//!
//! The handler copies the frame into a `poly_abi::SyscallRegs`, enables
//! interrupts (a system call may block and is preempted like any thread),
//! runs the call through `crate::user::syscall` (the thread's hook: by
//! default the process's personality), and writes the registers back.
//!
//! # Returning
//!
//! `SYSRET` is the fast way back: RIP from RCX, RFLAGS from R11, CS/SS from
//! IA32_STAR. It is only used when the return address is in the lower half
//! (so canonical). On Intel CPUs, SYSRET to a non-canonical RCX raises #GP
//! *in ring 0 but after RSP is already the user's*, which would let user
//! mode choose the kernel's stack. For any other return address the
//! handler falls back to `IRETQ` through the common trap exit
//! (`trap_return` in `idt.rs`). IRETQ checks the address before it changes
//! privilege, so a non-canonical one raises #GP in ring 0 on the kernel
//! stack, at the known `trap_return_iretq` instruction; the exception
//! handler recognises it and kills the thread (`user.rs`). (QEMU's TCG does
//! not check, and the thread page-faults in ring 3 instead; the tests use
//! [`make_next_return_fail`] to reach the ring-0 path there.) A canonical
//! kernel address just faults in user mode (the pages are supervisor-only),
//! and kills it too. The user range itself ends a page below the hole
//! (`addrspace.rs`), so ordinary system calls always return with SYSRET.
//!
//! RFLAGS on the way out is sanitised: the status flags, TF, DF, AC and ID
//! come from the thread; IF is always set; IOPL, NT, RF and VM are clear.
//! RCX and R11 are clobbered (they hold RIP and RFLAGS), as on Linux.
//!
//! # GS and the per-CPU area
//!
//! In ring 0, IA32_GS_BASE points at this CPU's area (`percpu.rs`) and IA32_KERNEL_GS_BASE holds
//! the user's GS base; in ring 3 the two are swapped. Every way into the
//! kernel from ring 3 swaps (`syscall_entry`, and `isr_common` when the
//! saved CS is a ring-3 selector) and every way out swaps back. The NMI,
//! the machine check and the double fault can arrive between `syscall`
//! and `swapgs` or between `swapgs` and `sysretq`/`iretq`; they run on
//! their own IST stacks (never the user's RSP) and set GS from there
//! (`idt.rs`, `isr_paranoid`). FSGSBASE (user-mode `wrgsbase`) stays
//! disabled, so user mode can only set its GS base through the kernel.

use super::addrspace::LOWER_HALF_END;
use super::cpu;
use super::gdt::{KERNEL_CODE, KERNEL_DATA, USER_CODE, USER_DATA};
use super::idt::TrapFrame;
use core::arch::{asm, global_asm};
use core::sync::atomic::{AtomicU64, Ordering};
use poly_abi::SyscallRegs;

/// The vector field of a frame built by the `syscall` entry (not a real
/// vector: those stop at 255).
pub const SYSCALL_VECTOR: u64 = 0x100;

/// Set the kernel stack ring 3 enters this CPU on, by `syscall` and by
/// interrupts.
pub fn set_kernel_stack(top: u64) {
    super::percpu::this().kernel_rsp.store(top, Ordering::Relaxed);
    super::gdt::set_kernel_stack(top);
}

/// RFLAGS bits (SDM Vol. 1 §3.4.3).
const RFLAGS_TF: u64 = 1 << 8;
const RFLAGS_IF: u64 = 1 << 9;
const RFLAGS_DF: u64 = 1 << 10;
const RFLAGS_IOPL: u64 = 3 << 12;
const RFLAGS_NT: u64 = 1 << 14;
const RFLAGS_AC: u64 = 1 << 18;
/// Bit 1 is reserved and always reads as 1.
const RFLAGS_FIXED: u64 = 1 << 1;
/// The flags user mode may set: CF, PF, AF, ZF, SF, TF, DF, OF, AC, ID.
const RFLAGS_USER: u64 = 0x1 | 0x4 | 0x10 | 0x40 | 0x80 | RFLAGS_TF | RFLAGS_DF | 0x800 | RFLAGS_AC | (1 << 21);

/// The RFLAGS a thread resumes ring 3 with: its own user-settable flags,
/// interrupts on, nothing privileged.
pub fn user_rflags(rflags: u64) -> u64 {
    (rflags & RFLAGS_USER) | RFLAGS_IF | RFLAGS_FIXED
}

/// Cleared by `syscall` (IA32_FMASK).
const SYSCALL_MASK: u64 = RFLAGS_TF | RFLAGS_IF | RFLAGS_DF | RFLAGS_IOPL | RFLAGS_NT | RFLAGS_AC;

// SYSRET takes CS = STAR[63:48] + 16 and SS = STAR[63:48] + 8 (both with
// RPL 3), SYSCALL CS = STAR[47:32] and SS = STAR[47:32] + 8 (SDM Vol. 2B):
// the GDT must have user data right after kernel data, then user code.
const SYSRET_BASE: u16 = KERNEL_DATA | 3;
const _: () = assert!(USER_DATA == (SYSRET_BASE + 8) | 3 && USER_CODE == (SYSRET_BASE + 16) | 3);
const _: () = assert!(KERNEL_DATA == KERNEL_CODE + 8);

/// Bytes from the frame's base to its fields, as the assembly uses them.
const _: () = assert!(core::mem::size_of::<TrapFrame>() == 22 * 8);

global_asm!(
    r#"
    .section .text.syscall, "ax", @progbits
    .global syscall_entry
syscall_entry:
    swapgs
    mov gs:[8], rsp
    mov rsp, gs:[0]
    push {user_ss}
    push qword ptr gs:[8]
    push r11
    push {user_cs}
    push rcx
    push 0
    push {vector}
    push rax
    push rbx
    push rcx
    push rdx
    push rsi
    push rdi
    push rbp
    push r8
    push r9
    push r10
    push r11
    push r12
    push r13
    push r14
    push r15
    mov rdi, rsp
    xor ebp, ebp
    call {handler}
    test al, al
    jz trap_return
    pop r15
    pop r14
    pop r13
    pop r12
    pop r11
    pop r10
    pop r9
    pop r8
    pop rbp
    pop rdi
    pop rsi
    pop rdx
    pop rcx
    pop rbx
    pop rax
    /* Skip vector and error code; RCX and R11 already hold RIP and
       RFLAGS (the handler set them so). Interrupts are off: nothing can
       arrive on the user stack in between except an NMI or machine
       check, which have their own stacks. */
    mov rsp, [rsp + 16 + 24]
    swapgs
    sysretq

    /* enter_user(frame: rdi) -> !: start ring 3 with the registers in a
       TrapFrame, through the common exit. */
    .global enter_user_frame
enter_user_frame:
    mov rsp, rdi
    jmp trap_return
"#,
    user_ss = const USER_DATA,
    user_cs = const USER_CODE,
    vector = const SYSCALL_VECTOR,
    handler = sym syscall_handler,
);

unsafe extern "sysv64" {
    static syscall_entry: u8;
    fn enter_user_frame(frame: *const TrapFrame) -> !;
}

/// Enable `syscall` / `sysret`: IA32_EFER.SCE, the selectors, the entry
/// point and the flag mask, on this CPU (GS base already points at its
/// per-CPU area).
///
/// # Safety
/// Call once on each CPU, after its GDT is loaded (loading a GS selector
/// afterwards would reset the GS base).
pub(super) unsafe fn init() {
    let star = (u64::from(SYSRET_BASE) << 48) | (u64::from(KERNEL_CODE) << 32);
    // SAFETY: these MSRs exist on every x86-64 CPU (SYSCALL is part of long
    // mode); the values follow SDM Vol. 2B `SYSCALL`, and nothing uses GS
    // before this.
    unsafe {
        cpu::write_msr(cpu::MSR_STAR, star);
        cpu::write_msr(cpu::MSR_LSTAR, (&raw const syscall_entry) as u64);
        cpu::write_msr(cpu::MSR_FMASK, SYSCALL_MASK);
        cpu::write_msr(cpu::MSR_KERNEL_GS_BASE, 0);
        cpu::write_msr(cpu::MSR_EFER, cpu::read_msr(cpu::MSR_EFER) | cpu::EFER_SCE);
    }
}

impl TrapFrame {
    /// Whether the interrupted code ran in ring 3.
    pub fn in_user_mode(&self) -> bool {
        self.cs & 3 == 3
    }

    fn syscall_regs(&self) -> SyscallRegs {
        SyscallRegs {
            rax: self.rax,
            rdi: self.rdi,
            rsi: self.rsi,
            rdx: self.rdx,
            r10: self.r10,
            r8: self.r8,
            r9: self.r9,
            rflags: self.rflags,
            rsp: self.rsp,
        }
    }

    fn set_syscall_regs(&mut self, regs: &SyscallRegs) {
        self.rax = regs.rax;
        self.rdi = regs.rdi;
        self.rsi = regs.rsi;
        self.rdx = regs.rdx;
        self.r10 = regs.r10;
        self.r8 = regs.r8;
        self.r9 = regs.r9;
        self.rflags = user_rflags(regs.rflags);
        self.rsp = regs.rsp;
    }
}

/// Where the running thread's ring-3 trap frame is while it is in the
/// kernel from ring 3: at the top of its kernel stack, where both the
/// `syscall` entry and the CPU (TSS.rsp0, already 16-byte aligned) put it.
pub(super) fn user_frame() -> *mut TrapFrame {
    let top = super::percpu::this().kernel_rsp.load(Ordering::Relaxed);
    (top - core::mem::size_of::<TrapFrame>() as u64) as *mut TrapFrame
}

/// The Rust half of the `syscall` entry, on the thread's kernel stack with
/// interrupts off. Returns whether to leave with SYSRET (else IRETQ). The
/// frame comes as a raw pointer: the system call may rewrite it through
/// [`user_frame`] (`user::set_registers`).
extern "sysv64" fn syscall_handler(frame: *mut TrapFrame) -> bool {
    crate::cputime::entered_kernel();
    let sysret = system_call(frame);
    // The IRETQ path tells the CPU-time accounting in `trap_return`.
    if sysret {
        crate::cputime::returning_to_user();
    }
    sysret
}

/// The system call itself, for [`syscall_handler`]: whether to leave with
/// SYSRET.
fn system_call(frame: *mut TrapFrame) -> bool {
    // SAFETY: the entry stub passes the complete frame it pushed; nothing
    // else refers to it during this read.
    let mut regs = unsafe { (*frame).syscall_regs() };
    // SAFETY: this thread came from ring 3, so it holds no lock and no
    // `InterruptsOff`; the frame is complete.
    unsafe { cpu::enable_interrupts() };
    let ret = crate::user::syscall(&mut regs);
    cpu::disable_interrupts();
    // SAFETY: the system call is over; nothing else refers to the frame.
    let frame = unsafe { &mut *frame };
    if ret.restore_all {
        // Every register was replaced (and sanitised) in the frame: only
        // the result comes from the call, and IRETQ restores RCX and R11.
        frame.rax = regs.rax;
        return false;
    }
    frame.set_syscall_regs(&regs);
    if let Some(rip) = ret.rip {
        frame.rip = rip;
    }
    // What SYSRET puts in RCX and R11 anyway; the IRETQ path restores them
    // from the frame, so both leave the same registers behind.
    frame.rcx = frame.rip;
    frame.r11 = frame.rflags;
    let me = super::percpu::this().kernel_rsp.load(Ordering::Relaxed);
    if FAIL_RETURN.compare_exchange(me, 0, Ordering::Relaxed, Ordering::Relaxed).is_ok() {
        // A kernel-mode SS selector: IRETQ refuses it (RPL 0 for a return
        // to ring 3) with #GP before it changes privilege.
        frame.ss = u64::from(KERNEL_DATA);
        return false;
    }
    frame.rip < LOWER_HALF_END
}

/// Kernel stack top of the thread whose next return from a system call
/// must fail (0: none); see [`make_next_return_fail`].
static FAIL_RETURN: AtomicU64 = AtomicU64::new(0);

/// Test support: make the running user thread's return from its current
/// system call fail in IRETQ, as a non-canonical return address does on
/// real CPUs. QEMU's TCG does not check RIP for canonicality on IRETQ, so
/// the tests use this to reach the kernel's handling of that fault.
pub fn make_next_return_fail() {
    FAIL_RETURN.store(super::percpu::this().kernel_rsp.load(Ordering::Relaxed), Ordering::Relaxed);
}

/// Where a new user thread starts and with what.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Entry {
    pub rip: u64,
    pub rsp: u64,
    /// In RDI, RSI and RDX. Every other register starts at 0.
    pub args: [u64; 3],
}

/// Leave for ring 3 at `entry`, never to return: the thread's kernel stack
/// is only re-entered from the top, by system calls and interrupts. The
/// thread's address space, FS/GS bases, FPU state and kernel stack must be
/// loaded (the scheduler did that when it switched to it).
pub fn enter_user(entry: &Entry) -> ! {
    cpu::disable_interrupts();
    let frame = TrapFrame {
        r15: 0,
        r14: 0,
        r13: 0,
        r12: 0,
        r11: 0,
        r10: 0,
        r9: 0,
        r8: 0,
        rbp: 0,
        rdi: entry.args[0],
        rsi: entry.args[1],
        rdx: entry.args[2],
        rcx: 0,
        rbx: 0,
        rax: 0,
        vector: 0,
        error_code: 0,
        rip: entry.rip,
        cs: u64::from(USER_CODE),
        rflags: user_rflags(0),
        rsp: entry.rsp,
        ss: u64::from(USER_DATA),
    };
    // SAFETY: the frame is a complete ring-3 frame on this stack;
    // `enter_user_frame` makes it the stack top, swaps GS and IRETQs to
    // ring 3. Nothing on this stack is used again.
    unsafe { enter_user_frame(&frame) }
}

/// Leave for ring 3 with every register from `regs` (a child of `clone`
/// or `fork`, which starts where its parent made the call), never to
/// return, like [`enter_user`]. CS and SS are the user selectors and
/// RFLAGS is sanitised, whatever `regs` holds.
pub fn enter_user_regs(regs: &TrapFrame) -> ! {
    cpu::disable_interrupts();
    let mut frame = *regs;
    frame.vector = 0;
    frame.error_code = 0;
    frame.cs = u64::from(USER_CODE);
    frame.ss = u64::from(USER_DATA);
    frame.rflags = user_rflags(regs.rflags);
    // SAFETY: as in `enter_user`: a complete ring-3 frame on this stack. A
    // non-canonical RIP faults in IRETQ, which kills the thread.
    unsafe { enter_user_frame(&frame) }
}

/// Load `value` into IA32_FS_BASE, the running user thread's FS base.
///
/// # Safety
/// `value` is canonical (else WRMSR raises #GP), and the running thread is
/// the user thread it is meant for.
pub(super) unsafe fn write_fs_base(value: u64) {
    // SAFETY: forwarded from the caller.
    unsafe { cpu::write_msr(cpu::MSR_FS_BASE, value) }
}

/// Undo the `swapgs` of a return to ring 3 that faulted (see `user.rs`).
///
/// # Safety
/// Only on that path: GS base holds the user's value, the per-CPU pointer
/// is in IA32_KERNEL_GS_BASE.
pub(super) unsafe fn swapgs() {
    // SAFETY: forwarded from the caller.
    unsafe { asm!("swapgs", options(nostack, preserves_flags)) }
}
