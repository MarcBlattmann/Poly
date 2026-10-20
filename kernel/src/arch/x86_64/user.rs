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
//! - [`exception`]: a page fault in ring 3 that demand paging can resolve
//!   (`user::page_fault`) returns to the faulting instruction; any other CPU
//!   exception in ring 3 kills the thread that caused it (with a log line),
//!   never the kernel.
//! - [`hwcap`] and [`random_bytes`]: what `exec` puts in the auxiliary
//!   vector (`AT_HWCAP`, `AT_RANDOM`).

pub use super::addrspace::{
    AddressSpace, Flush, LOWER_HALF_END, MapError, Mapping, Prot, USER_END, USER_START, UserFrame, user_frames,
};
pub use super::syscall::{Entry, enter_user, enter_user_regs, make_next_return_fail};
pub use super::usercopy::{Efault, cmpxchg_user_u32, copy_from_user, copy_to_user, is_user_range, strncpy_from_user};

use super::fpu::{self, FpuArea};
use super::idt::{self, TrapFrame};
use super::{addrspace, cpu, syscall, usercopy};
use alloc::sync::Arc;
use core::arch::x86_64::{__cpuid, __cpuid_count};
use core::sync::atomic::Ordering;
use poly_vm::Access;

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
    set_tsc_aux(0);
}

/// What [`init`] set up, on an application processor: SMEP, SMAP, PCIDs,
/// the FPU state format, `syscall`, and IA32_TSC_AUX (its CPU number, for
/// the vDSO's `getcpu`).
///
/// # Safety
/// Once per AP, after its GDT and GS base and with the kernel PML4 (PCID
/// 0) loaded, with interrupts off, before it runs a thread.
pub unsafe fn init_ap() {
    let f = features();
    let mut cr4 = cpu::read_cr4();
    if f.smep {
        cr4 |= cpu::CR4_SMEP;
    }
    if f.smap {
        cr4 |= cpu::CR4_SMAP;
    }
    // SAFETY: as in `init`; the same settings as the boot CPU's.
    unsafe {
        cpu::write_cr4(cr4);
        if addrspace::pcid_enabled() {
            addrspace::enable_pcid();
        }
        fpu::init_cpu();
        syscall::init();
    }
    set_tsc_aux(super::percpu::cpu_index() as u32);
}

/// IA32_TSC_AUX: what RDTSCP and RDPID read (SDM Vol. 4, Table 2-2).
const MSR_TSC_AUX: u32 = 0xc000_0103;

/// How user mode can learn its CPU without a system call: RDPID
/// (CPUID.(EAX=07H,ECX=0):ECX[22]) or RDTSCP (CPUID.80000001H:EDX[27]),
/// both reading IA32_TSC_AUX, which holds the CPU number.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CpuNumber {
    Rdpid,
    Rdtscp,
    Syscall,
}

pub fn cpu_number_instruction() -> CpuNumber {
    let rdpid = __cpuid(0).eax >= 7 && __cpuid_count(7, 0).ecx & (1 << 22) != 0;
    let rdtscp = __cpuid(0x8000_0000).eax >= 0x8000_0001 && __cpuid(0x8000_0001).edx & (1 << 27) != 0;
    if rdpid {
        CpuNumber::Rdpid
    } else if rdtscp {
        CpuNumber::Rdtscp
    } else {
        CpuNumber::Syscall
    }
}

/// Put this CPU's number in IA32_TSC_AUX, if the CPU has it.
fn set_tsc_aux(cpu: u32) {
    if cpu_number_instruction() != CpuNumber::Syscall {
        // SAFETY: the MSR exists with RDTSCP or RDPID (SDM Vol. 4); it
        // only feeds those instructions.
        unsafe { cpu::write_msr(MSR_TSC_AUX, u64::from(cpu)) };
    }
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

    /// A thread in `space` that starts with FPU state `fpu` (an image from
    /// [`fpu_image`](Self::fpu_image), or the initial state if it is
    /// refused) and FS / GS bases `fs_base` / `gs_base` (canonical): a
    /// child of `clone` or `fork`. `None` without memory for the FPU state.
    pub fn with_state(space: Arc<AddressSpace>, fpu: &[u8], fs_base: u64, gs_base: u64) -> Option<ThreadState> {
        let mut state = ThreadState::new(space)?;
        let _irq = cpu::InterruptsOff::new();
        let _ = state.fpu.load(fpu);
        state.fs_base = fs_base;
        state.gs_base = gs_base;
        Some(state)
    }

    /// Replace the address space of the running thread (`execve`) and load
    /// the new one; returns the old one, for the caller to drop once it is
    /// safe to. Call with interrupts off, on the running thread's state.
    pub fn replace_space(&mut self, space: Arc<AddressSpace>) -> Arc<AddressSpace> {
        debug_assert!(!cpu::interrupts_enabled());
        space.activate();
        core::mem::replace(&mut self.space, space)
    }

    /// Copy the thread's FPU / vector state into `out` ([`fpu_size`]
    /// bytes): an XSAVE image (returns `Some(XCR0)`) or an FXSAVE image
    /// (`None`). Call with interrupts off (`thread::with_current_user`).
    pub fn fpu_image(&mut self, out: &mut [u8]) -> Option<u64> {
        self.fpu.snapshot(out);
        fpu::xfeatures()
    }

    /// Load the thread's FPU state from `image` (an FXSAVE image, or an
    /// XSAVE image); refused if it would fault. Call with interrupts off.
    pub fn load_fpu(&mut self, image: &[u8]) -> Result<(), fpu::BadImage> {
        self.fpu.load(image)
    }

    /// Give the thread the initial FPU state. Call with interrupts off.
    pub fn reset_fpu(&mut self) {
        self.fpu.reset();
    }
}

/// Bytes in a thread's FPU image ([`ThreadState::fpu_image`]).
pub fn fpu_size() -> usize {
    fpu::area_size()
}

pub use super::fpu::BadImage;

/// A user thread's registers while it is in the kernel from ring 3: its
/// trap frame (general registers, RIP, CS, RFLAGS, RSP, SS).
pub use super::idt::TrapFrame as UserRegs;

/// The running user thread's registers as it will return to ring 3. Only
/// meaningful while it is in the kernel from ring 3 (a system call, an
/// interrupt or an exception there): the frame is then at the top of its
/// kernel stack.
pub fn user_regs() -> UserRegs {
    let frame = syscall::user_frame();
    // SAFETY: the frame slot at the top of the running thread's kernel
    // stack is mapped and holds a trap frame while the thread is in the
    // kernel from ring 3; the entry code wrote it and only `set_user_regs`
    // changes it before the return.
    let regs = unsafe { frame.read() };
    debug_assert!(regs.in_user_mode(), "user_regs outside a ring-3 entry");
    regs
}

/// Replace the running user thread's registers for its return to ring 3
/// (see [`user_regs`]). CS and SS stay the user selectors and RFLAGS is
/// sanitised (interrupts on, nothing privileged); a non-canonical RIP makes
/// the return fail, which kills the thread. The caller makes the return
/// restore every register (`user::set_registers`).
pub fn set_user_regs(regs: &UserRegs) {
    let frame = syscall::user_frame();
    let mut new = *regs;
    new.cs = u64::from(super::gdt::USER_CODE);
    new.ss = u64::from(super::gdt::USER_DATA);
    new.rflags = syscall::user_rflags(regs.rflags);
    // SAFETY: as in `user_regs`; the vector and error code slots are
    // ignored by the exit path, everything else is what IRETQ and the
    // register pops consume.
    unsafe {
        let old = frame.read();
        debug_assert!(old.in_user_mode(), "set_user_regs outside a ring-3 entry");
        new.vector = old.vector;
        new.error_code = old.error_code;
        frame.write(new);
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
        // Its next CPU may be another one: its FPU state goes to memory.
        from.fpu.save();
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
    /// Why a page fault could not be resolved (`SEGV_MAPERR`, ...).
    pub reason: Option<&'static str>,
    /// The same, for a fault in a thread with a memory map: what the
    /// personality turns into a signal (`SIGSEGV` with `SEGV_MAPERR` or
    /// `SEGV_ACCERR`, `SIGBUS`).
    pub page: Option<crate::user::FaultError>,
}

impl core::fmt::Display for Fault {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let whence = if self.on_return { "returning to" } else { "at" };
        write!(f, "{} (vector {}) {whence} rip {:#x}, error {:#x}", self.name, self.vector, self.rip, self.error_code)?;
        if let Some(addr) = self.address {
            write!(f, ", address {addr:#x}")?;
        }
        if let Some(reason) = self.reason {
            write!(f, " ({reason})")?;
        }
        Ok(())
    }
}

/// Page-fault error code bits (SDM Vol. 3A §4.7): W/R (a write), RSVD (a
/// reserved bit was set in a paging entry), I/D (an instruction fetch).
const PF_WRITE: u64 = 1 << 1;
const PF_RESERVED: u64 = 1 << 3;
const PF_FETCH: u64 = 1 << 4;

/// The access a page-fault error code describes, or `None` for a fault
/// demand paging must not touch (a reserved bit in a paging entry is a
/// kernel bug, not a missing page).
pub(super) fn fault_access(error_code: u64) -> Option<Access> {
    if error_code & PF_RESERVED != 0 {
        return None;
    }
    Some(if error_code & PF_FETCH != 0 {
        Access::Execute
    } else if error_code & PF_WRITE != 0 {
        Access::Write
    } else {
        Access::Read
    })
}

/// A CPU exception in ring 3, on the thread's kernel stack with interrupts
/// off (`frame` is a copy of the thread's trap frame). A page fault that the
/// thread's memory map resolves (a page that is part of a region but not
/// present yet) returns, and the instruction is retried; anything else goes
/// to `user::fault`, which returns only if the thread goes on (into a
/// signal handler, with its frame rewritten), and otherwise ends it.
/// Returns with interrupts off.
pub(super) fn exception(frame: TrapFrame) {
    let vector = frame.vector as u8;
    let address = (frame.vector == idt::PAGE_FAULT).then(cpu::read_cr2);
    // SAFETY: the thread came from ring 3, so it holds no lock and no
    // `InterruptsOff`; CR2 has been read. Resolving the fault may block (a
    // mutex, a disk read), as a system call may, and so may turning it
    // into a signal or ending the thread.
    unsafe { cpu::enable_interrupts() };
    let mut reason = None;
    let mut fault_kind = None;
    if let (Some(addr), Some(access)) = (address, fault_access(frame.error_code)) {
        match crate::user::page_fault(addr, access) {
            Ok(()) => {
                cpu::disable_interrupts();
                return;
            }
            // A bare address space (no memory map): the old log line.
            Err(crate::user::FaultError::NoMemoryMap) => {}
            Err(why) => {
                reason = Some(why.name());
                fault_kind = Some(why);
            }
        }
    }
    let fault = Fault {
        vector,
        name: idt::exception_name(vector),
        error_code: frame.error_code,
        rip: frame.rip,
        address,
        on_return: false,
        reason,
        page: fault_kind,
    };
    crate::user::fault(&fault);
    cpu::disable_interrupts();
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
        reason: None,
        page: None,
    };
    // SAFETY: the thread was leaving the kernel, so it holds no lock and no
    // `InterruptsOff`; GS is the kernel's again. Ending it may block.
    unsafe { cpu::enable_interrupts() };
    crate::user::kill_current(&fault)
}

/// `AT_HWCAP` on x86-64: the feature bits of CPUID.01H:EDX (getauxval(3):
/// "an architecture and ABI dependent bit-mask whose settings indicate
/// detailed processor capabilities"; on x86 the C library reads it as
/// leaf 1's EDX).
pub fn hwcap() -> u64 {
    u64::from(__cpuid(1).edx)
}

/// 16 bytes for `AT_RANDOM`, which the C library seeds its stack protector
/// and pointer guard from. RDRAND when the CPU has it (CPUID.01H:ECX[30];
/// each call may fail and is retried, SDM Vol. 1 §7.3.17.1), else the TSC
/// stirred through a 64-bit mixer: unpredictable enough for now, not for
/// cryptography.
pub fn random_bytes() -> [u8; 16] {
    let rdrand = __cpuid(1).ecx & (1 << 30) != 0;
    let mut out = [0u8; 16];
    // The stack address varies a little between threads: one more input.
    let salt = &raw const out as u64;
    for (i, chunk) in out.chunks_exact_mut(8).enumerate() {
        let word = rdrand.then(rdrand64).flatten().unwrap_or_else(|| {
            let seed = cpu::rdtsc() ^ ((i as u64) << 56) ^ salt;
            mix64(seed.wrapping_add(cpu::rdtsc().rotate_left(29)))
        });
        chunk.copy_from_slice(&word.to_le_bytes());
    }
    out
}

/// One RDRAND result, retrying a few times when it reports no entropy.
fn rdrand64() -> Option<u64> {
    for _ in 0..10 {
        let (value, ok): (u64, u8);
        // SAFETY: the caller checked CPUID for RDRAND; it only writes the
        // destination register and CF.
        unsafe {
            core::arch::asm!("rdrand {v}", "setc {ok}", v = out(reg) value, ok = out(reg_byte) ok, options(nomem, nostack));
        }
        if ok != 0 {
            return Some(value);
        }
    }
    None
}

/// A bijective 64-bit mixer, the output function of the SplitMix64
/// generator (Steele, Lea and Flood, "Fast splittable pseudorandom number
/// generators", OOPSLA 2014): every input bit affects every output bit.
fn mix64(mut z: u64) -> u64 {
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}
