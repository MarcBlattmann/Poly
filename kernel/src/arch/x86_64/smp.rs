//! Starting the application processors, and stopping CPUs.
//!
//! # Start-up
//!
//! [`start_aps`] runs on the boot CPU once interrupts and the clock work.
//! `poly_smp::plan` decides which processors from the MADT to start
//! (`poly.smp=off`, `poly.cpus=<n>`); each is started in turn:
//!
//! 1. The start-up area below 1 MiB (`poly_smp::trampoline`, kept out of
//!    every allocator by `kernel_entry`) gets the trampoline code (below,
//!    copied there once) and its page tables: an identity map of the first
//!    2 MiB and the kernel's higher half.
//! 2. The scheduler gets the CPU and its idle thread, whose stack becomes
//!    the AP's stack (`thread::prepare_cpu`); the AP's per-CPU area, GDT,
//!    TSS and IST stacks are allocated.
//! 3. The parameter block gets the stack, `ap_entry` and the CPU number,
//!    then INIT, SIPI, SIPI with the SDM's delays (`poly_smp::startup`).
//! 4. The AP goes from real mode through protected mode to long mode on
//!    the area's tables, calls [`ap_entry`] on its idle stack, loads the
//!    kernel's page tables, its own GDT, TSS and IDT and its GS base, and
//!    reports [`STARTED`]: the area is free for the next AP. It then sets
//!    up what the boot CPU set up for itself (NXE, PAT and global pages,
//!    SMEP / SMAP / PCIDs, the FPU, `syscall`, its local APIC and tick),
//!    reports [`ONLINE`], joins the scheduler and runs its idle thread.
//!
//! The boot CPU waits for each AP (up to two seconds per step) before
//! starting the next; one that does not report is left out and logged.
//!
//! # Stopping
//!
//! - A panic stops every other CPU with an NMI ([`panic_stop_others`]): the
//!   handler halts them, so they neither print over the report nor go on.
//!   A second panic on the reporting CPU (a fault while it formats the
//!   report) is reported too, once.
//! - [`Exclusive`] drains every other CPU (its threads move away) and parks
//!   it in its idle thread with interrupts off until it is dropped: only
//!   one CPU runs threads meanwhile (the old single-CPU guarantee of
//!   "interrupts off", which tests that count frames rely on). Parked CPUs
//!   keep serving TLB shootdowns.

use super::cpu::{self, InterruptsOff};
use super::gdt::CpuTables;
use super::irq::RESCHEDULE_VECTOR;
use super::percpu::{self, MAX_CPUS, PerCpu};
use super::{apic, paging, tlb};
use crate::acpi::Platform;
use crate::kprintln;
use alloc::boxed::Box;
use core::arch::global_asm;
use core::sync::atomic::{AtomicBool, AtomicPtr, AtomicU64, AtomicUsize, Ordering};
use poly_smp::CpuSet;
use poly_smp::plan::{self, Processor};
use poly_smp::startup::{self, StartError, Startup};
use poly_smp::trampoline::{self, Params, param};

/// Bring-up states in `PerCpu::state`.
const STARTED: u32 = 1;
const ONLINE: u32 = 2;

/// How long the boot CPU waits for an AP at each step.
const TIMEOUT_US: u64 = 2_000_000;

// The trampoline: real mode to long mode, then `call ENTRY(ARG)` on
// `STACK`. Copied to the start-up area; it addresses the parameter block
// relative to its own base (in `ebx`), so it runs at any page below 1 MiB.
global_asm!(
    r#"
    .section .rodata.ap_trampoline, "a", @progbits
    .balign 16
    .global ap_trampoline_start
ap_trampoline_start:
    .code16
    cli
    cld
    mov %cs, %ax
    mov %ax, %ds
    xor %ebx, %ebx
    mov %ax, %bx
    shl $4, %ebx                /* ebx = the area's physical base */
    lgdtl {gdtr}
    mov %cr0, %eax
    or $1, %eax                 /* PE */
    mov %eax, %cr0
    ljmpl *{far32}

    .code32
    .global ap_trampoline_32
ap_trampoline_32:
    mov ${data32}, %ax
    mov %ax, %ds
    mov %ax, %es
    mov %ax, %ss
    mov {cr4}(%ebx), %eax       /* PAE (and PGE) */
    mov %eax, %cr4
    mov {cr3}(%ebx), %eax
    mov %eax, %cr3
    mov $0xc0000080, %ecx       /* IA32_EFER: LME (and NXE) */
    rdmsr
    or {efer}(%ebx), %eax
    wrmsr
    mov %cr0, %eax
    or $0x80010001, %eax        /* PG, WP, PE */
    mov %eax, %cr0
    ljmpl *{far64}(%ebx)

    .code64
    .global ap_trampoline_64
ap_trampoline_64:
    mov %ebx, %ebx              /* upper half undefined after the switch */
    mov {stack}(%rbx), %rsp
    mov {arg}(%rbx), %rdi
    mov {entry}(%rbx), %rax
    xor %ebp, %ebp
    call *%rax
    ud2
    .global ap_trampoline_end
ap_trampoline_end:
"#,
    gdtr = const param::GDTR,
    far32 = const param::FAR32,
    far64 = const param::FAR64,
    cr3 = const param::CR3,
    cr4 = const param::CR4,
    efer = const param::EFER,
    stack = const param::STACK,
    entry = const param::ENTRY,
    arg = const param::ARG,
    data32 = const trampoline::DATA32,
    options(att_syntax)
);

unsafe extern "C" {
    static ap_trampoline_start: u8;
    static ap_trampoline_32: u8;
    static ap_trampoline_64: u8;
    static ap_trampoline_end: u8;
}

/// Physical address of the start-up area, 0 if there is none.
static AREA: AtomicU64 = AtomicU64::new(0);

/// The CPUs that run the scheduler (bit = CPU number).
static ONLINE_CPUS: AtomicU64 = AtomicU64::new(1);

/// The start-up area's place among the firmware's usable RAM ranges
/// `(start, end)`, if there is room below 1 MiB; remembered for
/// [`start_aps`]. The caller keeps it out of every allocator.
pub fn reserve_area(usable: impl IntoIterator<Item = (u64, u64)>) -> Option<u64> {
    let base = trampoline::find_base(usable)?;
    AREA.store(base, Ordering::Relaxed);
    Some(base)
}

/// Bytes of the start-up area.
pub const AREA_SIZE: u64 = trampoline::AREA_SIZE;

/// The CPUs that are online (running the scheduler).
pub fn online_cpus() -> CpuSet {
    CpuSet::from_bits(ONLINE_CPUS.load(Ordering::Acquire))
}

/// What one AP's entry needs besides its per-CPU area.
struct ApSetup {
    tables: AtomicPtr<CpuTables>,
    double_fault_end: AtomicU64,
    nmi_end: AtomicU64,
}

static AP_SETUP: [ApSetup; MAX_CPUS] = [const {
    ApSetup {
        tables: AtomicPtr::new(core::ptr::null_mut()),
        double_fault_end: AtomicU64::new(0),
        nmi_end: AtomicU64::new(0),
    }
}; MAX_CPUS];

/// How bring-up went, for the boot log and the tests.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Summary {
    /// Processors the plan chose (boot CPU included).
    pub planned: usize,
    /// Of those, the ones running now.
    pub online: usize,
    /// Processors the MADT lists that the options or the limits left out.
    pub left_out: usize,
    /// Processors the MADT lists that cannot be started in xAPIC mode.
    pub unreachable: usize,
}

static SUMMARY: AtomicU64 = AtomicU64::new(0);

/// The bring-up summary (all zero before [`start_aps`]).
pub fn summary() -> Summary {
    let v = SUMMARY.load(Ordering::Acquire);
    let field = |i: u32| ((v >> (16 * i)) & 0xffff) as usize;
    Summary { planned: field(0), online: field(1), left_out: field(2), unreachable: field(3) }
}

/// Start the application processors the plan chooses. Call once, on the
/// boot CPU, with interrupts and the clock running.
pub fn start_aps(platform: Option<&Platform>, smp: Option<&str>, cpus: Option<&str>) -> Summary {
    let options = plan::parse_options(smp, cpus).unwrap_or_else(|e| {
        kprintln!("smp: {}; starting every CPU", e.describe());
        plan::Options::default()
    });
    let processors: alloc::vec::Vec<Processor> = platform
        .map(|p| p.processors.iter().map(|c| Processor { uid: c.uid, apic_id: c.apic_id }).collect())
        .unwrap_or_default();
    let bsp = apic::current_id().unwrap_or(0);
    let plan = plan::plan(&processors, bsp, options, MAX_CPUS, apic::is_x2apic());
    let mut online = 1;
    let base = AREA.load(Ordering::Relaxed);
    if !plan.aps().is_empty() && base == 0 {
        kprintln!("smp: no room below 1 MiB for the start-up code; running on the boot CPU only");
    } else if !plan.aps().is_empty() {
        // SAFETY: once, on the boot CPU, before any AP runs; the area was
        // kept out of every allocator.
        unsafe { prepare_area(base) };
        for (i, p) in plan.aps().iter().enumerate() {
            match start_one(base, i + 1, p) {
                Ok(()) => online += 1,
                Err(why) => kprintln!("smp: CPU {} (APIC ID {}) did not start: {why}", i + 1, p.apic_id),
            }
        }
    }
    let unreachable = if plan.unreachable > 0 {
        alloc::format!(", {} unreachable in xAPIC mode", plan.unreachable)
    } else {
        "".into()
    };
    kprintln!(
        "smp: {online} of {} CPUs online ({}){}{unreachable}",
        processors.len().max(1),
        online_cpus(),
        if plan.left_out > 0 {
            alloc::format!(", {} left out by poly.smp / poly.cpus", plan.left_out)
        } else {
            "".into()
        },
    );
    let s = Summary { planned: plan.cpus.len(), online, left_out: plan.left_out, unreachable: plan.unreachable };
    let packed = [s.planned, s.online, s.left_out, s.unreachable]
        .iter()
        .enumerate()
        .fold(0u64, |v, (i, &n)| v | (n.min(0xffff) as u64) << (16 * i));
    SUMMARY.store(packed, Ordering::Release);
    s
}

/// Copy the trampoline into the area at `base` and build its page tables.
///
/// # Safety
/// The area is reserved RAM nothing else uses, reachable through the
/// direct map.
unsafe fn prepare_area(base: u64) {
    let virt = |phys: u64| super::phys_to_virt(phys).expect("the start-up area is in the direct map") as *mut u8;
    let start = (&raw const ap_trampoline_start) as usize;
    let len = (&raw const ap_trampoline_end) as usize - start;
    assert!(len <= param::GDT, "the trampoline code overlaps its parameter block");
    let kernel = paging::kernel_root();
    let [pml4_0, pdpt_0, pd_0] = trampoline::identity_entries(base);
    // SAFETY: the caller vouches for the area (four pages at `base`); the
    // code is `len` bytes of the kernel image.
    unsafe {
        core::ptr::write_bytes(virt(base), 0, trampoline::AREA_SIZE as usize);
        core::ptr::copy_nonoverlapping(start as *const u8, virt(base), len);
        let pml4 = virt(base + trampoline::PML4_OFFSET) as *mut u64;
        let kernel_pml4 = virt(kernel) as *const u64;
        for i in 256..512 {
            pml4.add(i).write(kernel_pml4.add(i).read());
        }
        pml4.write(pml4_0);
        (virt(base + trampoline::PDPT_OFFSET) as *mut u64).write(pdpt_0);
        (virt(base + trampoline::PD_OFFSET) as *mut u64).write(pd_0);
    }
}

/// A stack of `pages` pages for an AP's IST, from the frame allocator;
/// returns its end.
fn ist_stack() -> Result<u64, &'static str> {
    const ORDER: usize = 2;
    let phys =
        crate::frames::FRAMES.lock().as_mut().and_then(|f| f.allocate(ORDER)).ok_or("no memory for IST stacks")?;
    let virt = super::phys_to_virt(phys).ok_or("an IST stack outside the direct map")?;
    Ok(virt + (poly_mem::PAGE_SIZE << ORDER))
}

/// Start AP `index` (MADT entry `p`) through the area at `base`.
fn start_one(base: u64, index: usize, p: &Processor) -> Result<(), &'static str> {
    let (_, stack_top) =
        crate::thread::prepare_cpu(index).map_err(|_| "no thread slot or stack for its idle thread")?;
    let area = percpu::register(index, Box::leak(Box::new(PerCpu::new())), p.apic_id);
    super::irqstack::set_up(index, area)?;
    let setup = &AP_SETUP[index];
    setup.tables.store(Box::leak(Box::new(CpuTables::new())), Ordering::Release);
    setup.double_fault_end.store(ist_stack()?, Ordering::Release);
    setup.nmi_end.store(ist_stack()?, Ordering::Release);

    let features = cpu::features();
    let code = |sym: *const u8| (sym as usize - (&raw const ap_trampoline_start) as usize) as u32;
    let params = Params {
        base,
        entry32: code(&raw const ap_trampoline_32),
        entry64: code(&raw const ap_trampoline_64),
        cr4: if features.pge { trampoline::CR4_PGE } else { 0 },
        efer: if features.nx { trampoline::EFER_NXE } else { 0 },
        stack: stack_top,
        entry: ap_entry as *const () as u64,
        arg: index as u64,
    };
    let page = super::phys_to_virt(base).ok_or("the start-up area is not mapped")? as *mut u8;
    // SAFETY: the area's first page, which no AP uses now (the previous one
    // reported STARTED, after which it no longer reads it).
    let page = unsafe { core::slice::from_raw_parts_mut(page, 4096) };
    params.write(page).map_err(|_| "bad start-up parameters")?;
    let vector = trampoline::startup_vector(base).ok_or("bad start-up page")?;
    let mut sequence = Sequence { area, waiting_for: STARTED };
    match startup::start_ap(&mut sequence, p.apic_id, vector, TIMEOUT_US) {
        Ok(_) => {}
        Err(StartError::Ipi(_)) => return Err("the local APIC did not send the IPIs"),
        Err(StartError::Timeout) => return Err("it never reached long mode"),
    }
    sequence.waiting_for = ONLINE;
    let start = sequence.now_us();
    while !sequence.started() {
        if sequence.now_us() - start > TIMEOUT_US {
            return Err("it reached the kernel but did not come online");
        }
        core::hint::spin_loop();
    }
    Ok(())
}

/// [`Startup`] with the local APIC and the clock, waiting for `area`'s
/// state to reach `waiting_for`.
struct Sequence {
    area: &'static PerCpu,
    waiting_for: u32,
}

impl Startup for Sequence {
    type Error = poly_irq::ApicError;

    fn send_init(&mut self, apic_id: u32) -> Result<(), Self::Error> {
        apic::send_init(apic_id)
    }

    fn send_startup(&mut self, apic_id: u32, vector: u8) -> Result<(), Self::Error> {
        apic::send_startup(apic_id, vector)
    }

    fn delay_us(&mut self, us: u64) {
        let end = self.now_us() + us;
        while self.now_us() < end {
            core::hint::spin_loop();
        }
    }

    fn now_us(&mut self) -> u64 {
        crate::time::now_ns() / 1000
    }

    fn started(&mut self) -> bool {
        self.area.state.load(Ordering::Acquire) >= self.waiting_for
    }
}

/// Where an application processor enters the kernel, on its idle thread's
/// stack, with the start-up area's page tables and GDT still loaded and
/// interrupts off.
extern "sysv64" fn ap_entry(index: u64) -> ! {
    let index = index as usize;
    let area = percpu::cpu(index).expect("an AP without a per-CPU area");
    let setup = &AP_SETUP[index];
    // SAFETY: the kernel's tables map everything this code and its stack
    // use; the area's tables are not needed again. Then this CPU's own GDT,
    // TSS (with its IST stacks) and the shared IDT, and its GS base; all of
    // it was allocated for this CPU alone and lives forever.
    unsafe {
        cpu::write_cr3(paging::kernel_root());
        super::gdt::init_ap(
            setup.tables.load(Ordering::Acquire),
            setup.double_fault_end.load(Ordering::Acquire),
            setup.nmi_end.load(Ordering::Acquire),
            area,
        );
        super::idt::load();
        percpu::load(area);
    }
    area.state.store(STARTED, Ordering::Release);
    // SAFETY: once on this CPU, with interrupts off, before it runs any
    // thread: the same per-CPU setup the boot CPU did.
    unsafe {
        paging::init_ap();
        super::user::init_ap();
    }
    let features = super::irq_features();
    let platform = crate::acpi::platform();
    let started = super::apic::init_ap(&features, platform.as_ref()).and_then(|id| {
        area.apic_id.store(id, Ordering::Relaxed);
        super::time::start_ap_timer(apic::local_apic().expect("checked by init_ap"))
    });
    if let Err(why) = started {
        kprintln!("smp: CPU {index}: {why}; halting it");
        cpu::halt_forever();
    }
    drop(platform);
    ONLINE_CPUS.fetch_or(1 << index, Ordering::AcqRel);
    crate::thread::cpu_online(index);
    area.state.store(ONLINE, Ordering::Release);
    crate::thread::run_idle()
}

/// Ask CPU `index` to look at its run queue (it is idle, or its running
/// thread must move).
pub fn send_reschedule(index: usize) {
    if let Some(area) = percpu::cpu(index).filter(|_| online_cpus().contains(index)) {
        apic::send_ipi(area.apic_id(), RESCHEDULE_VECTOR);
    }
}

// --- Stopping CPUs ---------------------------------------------------------------

/// The number + 1 of the first CPU that panicked (0: none yet).
static PANICKING: AtomicUsize = AtomicUsize::new(0);
/// Panics on that CPU while it reported the first one.
static NESTED_PANICS: AtomicUsize = AtomicUsize::new(0);

/// Who reports a panic ([`panic_stop_others`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PanicRole {
    /// The first panic: this CPU reports it, the others are stopped.
    First,
    /// A panic on the CPU that is reporting one (a fault while formatting
    /// the report, such as its stack running into the guard): report this
    /// one too, or nothing would say why the report stopped.
    Nested,
    /// Another CPU reports, or this CPU's nested report failed too: halt.
    Halt,
}

/// A panic on this CPU: stop the others with an NMI.
pub fn panic_stop_others() -> PanicRole {
    let me = super::percpu::cpu_index() + 1;
    match PANICKING.compare_exchange(0, me, Ordering::AcqRel, Ordering::Acquire) {
        Ok(_) => {
            if online_cpus().len() > 1 {
                apic::send_nmi_to_others();
            }
            PanicRole::First
        }
        Err(cpu) if cpu == me && NESTED_PANICS.fetch_add(1, Ordering::AcqRel) == 0 => PanicRole::Nested,
        Err(_) => PanicRole::Halt,
    }
}

/// An NMI: whether it is the panic's stop request (the CPU halts).
pub(super) fn stopped_by_nmi() -> bool {
    PANICKING.load(Ordering::Acquire) != 0
}

/// Whoever holds the [`Exclusive`] (CPU number + 1; 0: nobody).
static EXCLUSIVE: AtomicUsize = AtomicUsize::new(0);
/// The other CPUs should park once idle.
static STOP: AtomicBool = AtomicBool::new(false);
/// CPUs parked.
static PARKED: AtomicUsize = AtomicUsize::new(0);

/// Only this CPU runs threads until dropped, as on a machine with one CPU:
/// every other online CPU is drained (its threads move to the others, in
/// effect to this one, `thread::drain_cpu`) and then parks in its idle
/// thread with interrupts off; this CPU's thread is pinned to it and runs
/// with interrupts off. Threads that held locks on the other CPUs run on
/// this one when it blocks, so waiting for them is fine. Parked CPUs keep
/// answering TLB shootdowns. Take it with interrupts on and no spinlock
/// held; it does not nest.
#[derive(Debug)]
pub struct Exclusive {
    // Dropped in this order, after `Drop::drop` released the others.
    _irq: InterruptsOff,
    _pin: crate::thread::Pinned,
    others: CpuSet,
}

impl Exclusive {
    pub fn new() -> Exclusive {
        assert!(cpu::interrupts_enabled(), "Exclusive::new with interrupts off would deadlock");
        // Claimed with interrupts on: a CPU waiting here can still be
        // drained by the holder.
        while EXCLUSIVE.compare_exchange(0, usize::MAX, Ordering::Acquire, Ordering::Relaxed).is_err() {
            core::hint::spin_loop();
        }
        let pin = crate::thread::pin_current();
        let me = pin.cpu();
        EXCLUSIVE.store(me + 1, Ordering::Release);
        let mut others = online_cpus();
        others.remove(me);
        STOP.store(true, Ordering::SeqCst);
        for c in others.iter() {
            crate::thread::drain_cpu(c, true);
        }
        let irq = InterruptsOff::new();
        while PARKED.load(Ordering::SeqCst) < others.len() {
            tlb::service();
            core::hint::spin_loop();
        }
        Exclusive { _irq: irq, _pin: pin, others }
    }
}

impl Default for Exclusive {
    fn default() -> Self {
        Exclusive::new()
    }
}

impl Drop for Exclusive {
    fn drop(&mut self) {
        STOP.store(false, Ordering::SeqCst);
        while PARKED.load(Ordering::SeqCst) != 0 {
            tlb::service();
            core::hint::spin_loop();
        }
        for c in self.others.iter() {
            crate::thread::drain_cpu(c, false);
        }
        EXCLUSIVE.store(0, Ordering::Release);
    }
}

/// Called by an idle thread with interrupts off when its CPU has nothing
/// to run: park while an [`Exclusive`] held on another CPU wants it to.
pub fn park_if_stopped() {
    let me = percpu::cpu_index() + 1;
    if !STOP.load(Ordering::SeqCst) || EXCLUSIVE.load(Ordering::Acquire) == me {
        return;
    }
    PARKED.fetch_add(1, Ordering::SeqCst);
    while STOP.load(Ordering::SeqCst) {
        tlb::service();
        core::hint::spin_loop();
    }
    PARKED.fetch_sub(1, Ordering::SeqCst);
}
