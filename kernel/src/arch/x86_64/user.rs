//! User mode on x86-64: CPU setup, what each user thread carries, and what
//! happens when ring 3 faults.
//!
//! - [`init`] turns on what user mode needs and what protects the kernel
//!   from it: SYSCALL/SYSRET and the per-CPU area (`syscall.rs`), the
//!   x87/SSE/AVX state (`fpu.rs`), SMEP and SMAP when the CPU has them, and
//!   PCIDs (`addrspace.rs`).
//! - [`ThreadState`] is the machine state of a user thread that is not in
//!   the general registers: its address space, FPU state, and FS and GS
//!   bases. [`switch`] moves it in and out when the scheduler switches
//!   threads, and points TSS.rsp0 and the `syscall` entry at the incoming
//!   thread's kernel stack.
//! - [`exception`]: a CPU exception in ring 3 kills the thread that caused
//!   it (with a log line), never the kernel.

pub use super::addrspace::{AddressSpace, LOWER_HALF_END, MapError, Mapping, Prot, USER_END, USER_START};
pub use super::syscall::{Entry, enter_user, make_next_return_fail};
pub use super::usercopy::{Efault, copy_from_user, copy_to_user, is_user_range, strncpy_from_user};

use super::fpu::{self, FpuArea};
use super::idt::{self, TrapFrame};
use super::{addrspace, cpu, syscall, usercopy};
use alloc::sync::Arc;
use core::arch::x86_64::{__cpuid, __cpuid_count};
use core::sync::atomic::Ordering;

/// The optional protections and accelerators user mode can use.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Features {
    /// CPUID.01H:ECX.PCID [bit 17].
    pub pcid: bool,
    /// CPUID.(EAX=07H,ECX=0):EBX.SMEP [bit 7].
    pub smep: bool,
    /// CPUID.(EAX=07H,ECX=0):EBX.SMAP [bit 20].
    pub smap: bool,
}

pub fn features() -> Features {
    let leaf1 = __cpuid(1);
    let leaf7_ebx = if __cpuid(0).eax >= 7 { __cpuid_count(7, 0).ebx } else { 0 };
    Features { pcid: leaf1.ecx & (1 << 17) != 0, smep: leaf7_ebx & (1 << 7) != 0, smap: leaf7_ebx & (1 << 20) != 0 }
}

/// Set up the CPU for user mode. With PCIDs, SMEP or SMAP missing (QEMU's
/// default CPU has none of them) the kernel works without: every CR3 load
/// flushes the TLB, and the kernel is not stopped in hardware from running
/// or touching user pages by mistake (`copy_from_user` and friends still
/// check every user range).
///
/// # Safety
/// Call once on the boot CPU, after `paging::init` (SMEP would forbid
/// running the boot stub's user-accessible mappings) and the GDT, with
/// interrupts off, before any user thread exists.
pub unsafe fn init() {
    let f = features();
    let mut cr4 = cpu::read_cr4();
    if f.smep {
        cr4 |= cpu::CR4_SMEP;
    }
    if f.smap {
        cr4 |= cpu::CR4_SMAP;
    }
    // SAFETY: CPUID reports both; the kernel page tables map nothing for
    // user mode, so neither changes what the kernel itself may do.
    unsafe { cpu::write_cr4(cr4) };
    usercopy::SMAP.store(f.smap, Ordering::Relaxed);
    // SAFETY: once, on the boot CPU, with the kernel PML4 (PCID 0) loaded,
    // before any user thread: forwarded from the caller.
    unsafe {
        if f.pcid {
            addrspace::enable_pcid();
        }
        fpu::init();
        syscall::init();
    }
    let yes_no = |b: bool| if b { "yes" } else { "no" };
    let fpu = fpu::info();
    let (how, xcr0) = if fpu.xsave { ("XSAVE", fpu.xcr0) } else { ("FXSAVE", 0) };
    crate::kprintln!(
        "user: syscall/sysret on; PCID: {}, SMEP: {}, SMAP: {}; FPU state: {how}, {} bytes, XCR0 {xcr0:#x}",
        yes_no(f.pcid),
        yes_no(f.smep),
        yes_no(f.smap),
        fpu.area_size
    );
}

/// What a user thread carries besides its general registers (which are on
/// its kernel stack while it is in the kernel).
#[derive(Debug)]
pub struct ThreadState {
    space: Arc<AddressSpace>,
    fpu: FpuArea,
    /// FS and GS bases while the thread is switched out. While it runs they
    /// are in IA32_FS_BASE and IA32_KERNEL_GS_BASE (swapped into GS base in
    /// ring 3).
    fs_base: u64,
    gs_base: u64,
}

impl ThreadState {
    /// A thread in `space` with the initial FPU state and zero FS/GS bases.
    /// `None` if there is no memory for the FPU state.
    pub fn new(space: Arc<AddressSpace>) -> Option<ThreadState> {
        Some(ThreadState { space, fpu: FpuArea::new()?, fs_base: 0, gs_base: 0 })
    }

    pub fn space(&self) -> &Arc<AddressSpace> {
        &self.space
    }
}

/// Switch the user state from thread `from` to thread `to` (`None`: a
/// kernel thread), right before the context switch: save `from`'s FS and GS
/// bases, then for a user thread load its address space, FPU state and
/// bases and point the ring-3 entries at `to_stack_top`; for a kernel thread
/// load the kernel PML4. Interrupts must be off.
pub fn switch(from: Option<&mut ThreadState>, to: Option<&ThreadState>, to_stack_top: Option<u64>) {
    debug_assert!(!cpu::interrupts_enabled());
    if let Some(from) = from {
        // SAFETY: both MSRs exist on every x86-64 CPU; reading has no
        // side effects.
        unsafe {
            from.fs_base = cpu::read_msr(cpu::MSR_FS_BASE);
            from.gs_base = cpu::read_msr(cpu::MSR_KERNEL_GS_BASE);
        }
    }
    let Some(to) = to else {
        addrspace::activate_kernel();
        return;
    };
    to.space.activate();
    to.fpu.make_current();
    // SAFETY: the values were read from these MSRs or checked canonical by
    // `set_fs_base` / `set_gs_base`; the kernel uses neither FS nor (in
    // ring 0) IA32_KERNEL_GS_BASE.
    unsafe {
        cpu::write_msr(cpu::MSR_FS_BASE, to.fs_base);
        cpu::write_msr(cpu::MSR_KERNEL_GS_BASE, to.gs_base);
    }
    syscall::set_kernel_stack(to_stack_top.expect("a user thread has its own kernel stack"));
}

/// Whether `addr` is canonical (bits 63:47 equal).
fn is_canonical(addr: u64) -> bool {
    poly_paging::is_canonical(addr)
}

/// Set the running user thread's FS base (`arch_prctl(ARCH_SET_FS)`, the
/// TLS pointer on Linux). Refuses a non-canonical address.
pub fn set_fs_base(base: u64) -> Result<(), Efault> {
    if !is_canonical(base) {
        return Err(Efault);
    }
    // SAFETY: canonical; the MSR is the running thread's until `switch`
    // saves it.
    unsafe { syscall::write_fs_base(base) };
    Ok(())
}

/// The running user thread's FS base.
pub fn fs_base() -> u64 {
    // SAFETY: the MSR exists; reading it has no side effects.
    unsafe { cpu::read_msr(cpu::MSR_FS_BASE) }
}

/// Set the running user thread's GS base (the TEB pointer on Windows).
/// Refuses a non-canonical address.
pub fn set_gs_base(base: u64) -> Result<(), Efault> {
    if !is_canonical(base) {
        return Err(Efault);
    }
    // SAFETY: in ring 0 the user's GS base lives in IA32_KERNEL_GS_BASE
    // until the return to ring 3 swaps it in; canonical, so WRMSR accepts it.
    unsafe { cpu::write_msr(cpu::MSR_KERNEL_GS_BASE, base) };
    Ok(())
}

/// The running user thread's GS base.
pub fn gs_base() -> u64 {
    // SAFETY: the MSR exists; reading it has no side effects.
    unsafe { cpu::read_msr(cpu::MSR_KERNEL_GS_BASE) }
}

/// A CPU exception that ends a user thread.
#[derive(Clone, Copy, Debug)]
pub struct Fault {
    pub vector: u8,
    pub name: &'static str,
    pub error_code: u64,
    /// The user RIP of the faulting instruction (or of the failed return).
    pub rip: u64,
    /// CR2 for a page fault.
    pub address: Option<u64>,
    /// True if the fault came from returning to ring 3, not from user code.
    pub on_return: bool,
}

impl core::fmt::Display for Fault {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let whence = if self.on_return { "returning to" } else { "at" };
        write!(f, "{} (vector {}) {whence} rip {:#x}, error {:#x}", self.name, self.vector, self.rip, self.error_code)?;
        if let Some(addr) = self.address {
            write!(f, ", address {addr:#x}")?;
        }
        Ok(())
    }
}

/// A CPU exception in ring 3: kill the thread. Runs on its kernel stack with
/// interrupts off.
pub(super) fn exception(frame: &TrapFrame) -> ! {
    let vector = frame.vector as u8;
    let address = (frame.vector == idt::PAGE_FAULT).then(cpu::read_cr2);
    // TODO(vm): a page fault on a page that belongs to a mapping but is not
    // present yet (demand paging, copy-on-write) is resolved here instead.
    let fault = Fault {
        vector,
        name: idt::exception_name(vector),
        error_code: frame.error_code,
        rip: frame.rip,
        address,
        on_return: false,
    };
    crate::user::kill_current(&fault)
}

/// A #GP in ring 0: if it is IRETQ refusing a return to ring 3 (a
/// non-canonical RIP; see `syscall.rs`), kill the thread that was
/// returning. Otherwise returns, and the fault is the kernel's.
pub(super) fn failed_return(frame: &TrapFrame) {
    if frame.vector != idt::GENERAL_PROTECTION || frame.rip != idt::return_iretq() {
        return;
    }
    // The fault left the IRETQ frame where RSP pointed: on this thread's
    // kernel stack, below the #GP's own frame.
    // SAFETY: `frame.rsp` is the kernel stack pointer at the IRETQ, which
    // points at the five words it was popping (RIP, CS, RFLAGS, RSP, SS).
    let (rip, cs) = unsafe { ((frame.rsp as *const u64).read(), (frame.rsp as *const u64).add(1).read()) };
    if cs & 3 != 3 {
        return;
    }
    // The exit had already swapped in the user's GS base; take it back out
    // so the kernel's state is as on any other entry from ring 3.
    // SAFETY: exactly that path.
    unsafe { syscall::swapgs() };
    let fault = Fault {
        vector: idt::GENERAL_PROTECTION as u8,
        name: idt::exception_name(idt::GENERAL_PROTECTION as u8),
        error_code: frame.error_code,
        rip,
        address: None,
        on_return: true,
    };
    crate::user::kill_current(&fault)
}
