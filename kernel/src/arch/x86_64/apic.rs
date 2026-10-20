//! The interrupt controllers, driven through `poly-irq`: the legacy 8259
//! pair (remapped and fully masked), this CPU's local APIC (x2APIC mode when
//! the CPU has it, else xAPIC through `map_mmio`) and every I/O APIC in the
//! MADT (all inputs masked until a driver routes one).
//!
//! The kernel's part is the hardware access poly-irq asks for: [`CpuMsr`]
//! (`rdmsr` / `wrmsr`, limited to the APIC's MSRs), [`CpuPorts`] (`in` /
//! `out`, limited to the 8259's ports) and [`MmioRegion`].
//!
//! Single CPU: only the boot CPU's local APIC is set up, and I/O APIC and
//! MSI interrupts are delivered to it. Each application processor will run
//! [`init_local_apic`] for its own APIC (same physical address in xAPIC
//! mode, so the mapping can be shared).

use super::cpu;
use super::irq::{ERROR_VECTOR, PIC_BASE, SPURIOUS_VECTOR};
use super::mmio::MmioRegion;
use super::port;
use crate::acpi::Platform;
use crate::kprintln;
use crate::sync::{SetOnce, SpinLock};
use alloc::vec::Vec;
use poly_irq::lapic::{self, IA32_APIC_BASE, IA32_TSC_DEADLINE, X2APIC_MSR_BASE};
use poly_irq::pic::port as pic_port;
use poly_irq::{
    Backend, CpuFeatures, DeliveryMode, Destination, IoApic, IoApicError, Ipi, IsaOverride, IsaRoute, IsaRoutes,
    LocalApic, Lvt, LvtReg, Msr, Pic8259, Polarity, PortIo, RedirectionEntry, Trigger, X2Apic, XApic,
};

/// `rdmsr` / `wrmsr` for poly-irq, restricted to the MSRs it needs:
/// `IA32_APIC_BASE`, the x2APIC registers and `IA32_TSC_DEADLINE`. Anything
/// else panics, so no caller can reach an MSR that changes how the kernel
/// itself runs.
#[derive(Clone, Copy, Debug)]
pub struct CpuMsr;

impl CpuMsr {
    fn check(msr: u32) {
        let apic = msr == IA32_APIC_BASE || msr == IA32_TSC_DEADLINE || (X2APIC_MSR_BASE..0x900).contains(&msr);
        assert!(apic, "MSR {msr:#x} is not an APIC register");
    }
}

impl Msr for CpuMsr {
    fn read(&self, msr: u32) -> u64 {
        Self::check(msr);
        // SAFETY: only APIC MSRs get here. poly-irq reads the x2APIC ones
        // only in x2APIC mode and the deadline only with CPUID's
        // TSC-deadline bit, as `LocalApic` is told the features; a missing
        // one raises #GP, which panics rather than misbehaving.
        unsafe { cpu::read_msr(msr) }
    }

    fn write(&self, msr: u32, value: u64) {
        Self::check(msr);
        // SAFETY: as for `read`. Writes to these registers only change how
        // the local APIC delivers interrupts, which is what poly-irq is for;
        // none of them affects memory the kernel uses.
        unsafe { cpu::write_msr(msr, value) }
    }
}

/// Port I/O for poly-irq's 8259 driver: its four ports and the delay port.
/// Other ports read as all ones and ignore writes.
#[derive(Clone, Copy, Debug)]
pub struct CpuPorts;

impl CpuPorts {
    fn allowed(p: u16) -> bool {
        use pic_port::*;
        matches!(p, MASTER_COMMAND | MASTER_DATA | SLAVE_COMMAND | SLAVE_DATA | DELAY)
    }
}

impl PortIo for CpuPorts {
    fn read8(&self, p: u16) -> u8 {
        if !Self::allowed(p) {
            return u8::MAX;
        }
        // SAFETY: an 8259 port; reading its mask or status registers has no
        // side effect beyond what the driver intends.
        unsafe { port::inb(p) }
    }

    fn write8(&self, p: u16, value: u8) {
        if Self::allowed(p) {
            // SAFETY: an 8259 port (or the unused POST port 0x80, a delay);
            // the driver only remaps and masks the controllers.
            unsafe { port::outb(p, value) }
        }
    }
}

pub type Apic = LocalApic<Backend<MmioRegion, CpuMsr>>;

static LAPIC: SetOnce<Apic> = SetOnce::new();

/// The boot CPU's local APIC, once [`init_local_apic`] ran.
pub fn local_apic() -> Option<&'static Apic> {
    LAPIC.get()
}

/// Signal the end of the interrupt being handled to the local APIC.
pub fn eoi() {
    if let Some(apic) = LAPIC.get() {
        apic.eoi();
    }
}

/// Remap the 8259s to vectors `PIC_BASE..PIC_BASE + 16`, away from the CPU
/// exceptions, and mask every input. With the local APIC's LINT0 masked they
/// cannot even raise a spurious interrupt, but if one arrives,
/// [`legacy_interrupt`] handles it.
pub fn init_pic() {
    match Pic8259::new(CpuPorts).init(PIC_BASE, PIC_BASE + 8) {
        Ok(()) => kprintln!("irq: 8259 PICs remapped to vectors {PIC_BASE:#x}-{:#x}, all masked", PIC_BASE + 15),
        Err(e) => kprintln!("irq: 8259 PICs not remapped: {e:?}"),
    }
}

/// An interrupt on one of the 8259's vectors (input `irq`). They are all
/// masked, so this is a spurious IRQ 7 or 15 (no EOI) or something that got
/// in before the masking (EOI to the 8259s, not to the local APIC: the
/// 8259 delivers through LINT0 as ExtINT, which the APIC does not track).
pub fn legacy_interrupt(irq: u8) -> bool {
    let pic = Pic8259::new(CpuPorts);
    if pic.is_spurious(irq) {
        return true;
    }
    pic.eoi(irq);
    false
}

/// Set up this CPU's local APIC: x2APIC mode if the CPU supports it, else
/// xAPIC mode through a mapping of its register page. Every LVT entry is
/// masked except the error entry (and LINT inputs the MADT wires to NMI);
/// the timer is programmed later by the time module.
pub fn init_local_apic(features: &CpuFeatures, platform: Option<&Platform>) -> Result<&'static Apic, &'static str> {
    if !features.apic {
        return Err("CPUID reports no local APIC");
    }
    let backend = if features.x2apic {
        lapic::enable_x2apic(&CpuMsr, features).map_err(|_| "x2APIC mode could not be enabled")?;
        Backend::X2Apic(X2Apic(CpuMsr))
    } else {
        let base = lapic::enable_xapic(&CpuMsr).map_err(|_| "firmware left the APIC in x2APIC mode")?;
        if let Some(p) = platform.filter(|p| p.local_apic != base.address()) {
            kprintln!("apic: MADT says {:#x}, IA32_APIC_BASE {:#x}; using the MSR", p.local_apic, base.address());
        }
        let regs = MmioRegion::map(base.address(), 0x1000).ok_or("could not map the local APIC")?;
        Backend::XApic(XApic(regs))
    };
    let apic = LocalApic::new(backend, features);
    apic.mask_all_lvts();
    apic.set_task_priority(0);
    apic.enable(SPURIOUS_VECTOR);
    apic.set_lvt(LvtReg::Error, Lvt::new(ERROR_VECTOR)).map_err(|_| "could not program the APIC error entry")?;
    let nmis = configure_lint_nmis(&apic, platform);
    // Clear errors left over from before (the register latches them).
    apic.error_status();
    let mode = match apic.registers() {
        Backend::X2Apic(_) => alloc::string::String::from("x2APIC mode"),
        Backend::XApic(_) => alloc::format!("xAPIC mode at {:#x}", lapic::read_apic_base(&CpuMsr).address()),
    };
    kprintln!(
        "apic: local APIC id {} version {:#x}, {mode}, {} LVT entries, spurious vector {SPURIOUS_VECTOR:#x}, \
         {nmis} LINT input(s) wired to NMI",
        apic.id(),
        apic.version().version(),
        apic.version().max_lvt() + 1,
    );
    LAPIC.set(apic).map_err(|_| "local APIC initialised twice")
}

/// Program the LINT inputs the MADT wires to NMI for this processor.
fn configure_lint_nmis(apic: &Apic, platform: Option<&Platform>) -> usize {
    let Some(p) = platform else { return 0 };
    let id = apic.id();
    let uid = p.processors.iter().find(|c| c.apic_id == id).map(|c| c.uid);
    let mut n = 0;
    for nmi in p.lint_nmis.iter().filter(|nmi| nmi.processor_uid.is_none() || nmi.processor_uid == uid) {
        let which = match nmi.lint {
            0 => LvtReg::Lint0,
            1 => LvtReg::Lint1,
            _ => continue,
        };
        // NMI is always edge-triggered (SDM §11.5.1); the polarity comes
        // from the MPS INTI flags (ISA default: active high).
        let polarity = if nmi.flags & 0b11 == 0b11 { Polarity::ActiveLow } else { Polarity::ActiveHigh };
        let lvt = Lvt::new(0).with_delivery(DeliveryMode::Nmi).with_polarity(polarity);
        if apic.set_lvt(which, lvt).is_ok() {
            n += 1;
        }
    }
    n
}

/// The local APIC ID that I/O APIC routes and MSI messages target: the boot
/// CPU's.
pub fn interrupt_destination() -> Option<u32> {
    LAPIC.get().map(|a| a.id())
}

/// Send fixed interrupt `vector` to this CPU through the local APIC.
pub fn send_self_ipi(vector: u8) -> bool {
    LAPIC.get().is_some_and(|a| a.send_ipi(&Ipi::fixed(Destination::ToSelf, vector)).is_ok())
}

/// Every I/O APIC, and where the ISA IRQs arrive on them. The
/// select-then-access register pair of an I/O APIC is not atomic, so all
/// access goes through this lock.
struct IoApics {
    chips: Vec<IoApic<MmioRegion>>,
    isa: IsaRoutes,
}

static IO_APICS: SpinLock<Option<IoApics>> = SpinLock::new(None);

/// Map every I/O APIC in the MADT and mask all of its inputs, and work out
/// the ISA IRQ routes from the interrupt source overrides.
pub fn init_io_apics(platform: &Platform) {
    let mut chips = Vec::new();
    for info in &platform.io_apics {
        // IOREGSEL at 0x00 and IOWIN at 0x10, 32 bits each.
        let Some(regs) = MmioRegion::map(info.address, 0x20) else {
            kprintln!("apic: could not map the I/O APIC at {:#x}", info.address);
            continue;
        };
        match IoApic::new(regs, info.gsi_base) {
            Ok(io) => {
                io.mask_all();
                kprintln!(
                    "apic: I/O APIC {} at {:#x}: version {:#x}, GSI {}-{}, all masked",
                    io.id(),
                    info.address,
                    io.version(),
                    io.gsi_base(),
                    io.gsi_base() + u32::from(io.inputs()).saturating_sub(1)
                );
                chips.push(io);
            }
            Err(e) => kprintln!("apic: no I/O APIC at {:#x}: {e:?}", info.address),
        }
    }
    let isa = IsaRoutes::from_overrides(platform.overrides.iter().map(|o| IsaOverride {
        bus: o.bus,
        source: o.source,
        gsi: o.gsi,
        flags: o.flags,
    }));
    *IO_APICS.lock() = Some(IoApics { chips, isa });
}

/// Where ISA IRQ `irq` arrives (after the MADT's overrides).
pub fn isa_route(irq: u8) -> Option<IsaRoute> {
    IO_APICS.lock().as_ref()?.isa.route(irq)
}

/// Total I/O APIC inputs, for the boot log and tests.
pub fn io_apic_inputs() -> usize {
    IO_APICS.lock().as_ref().map_or(0, |io| io.chips.iter().map(|c| usize::from(c.inputs())).sum())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RouteError {
    /// The interrupt controllers are not set up yet.
    NotReady,
    /// No I/O APIC handles that GSI.
    NoSuchGsi,
    /// The local APIC ID does not fit an 8-bit destination field.
    DestinationTooWide,
    IoApic(IoApicError),
}

/// Deliver global system interrupt `gsi` as fixed interrupt `vector` to the
/// boot CPU, with the given signalling, and unmask it.
pub fn route_gsi(gsi: u32, vector: u8, polarity: Polarity, trigger: Trigger) -> Result<(), RouteError> {
    let dest = interrupt_destination().ok_or(RouteError::NotReady)?;
    let dest = u8::try_from(dest).map_err(|_| RouteError::DestinationTooWide)?;
    let io = IO_APICS.lock();
    let chip = io.as_ref().ok_or(RouteError::NotReady)?.chips.iter().find(|c| c.input_for(gsi).is_some());
    let entry = RedirectionEntry::new(vector, dest).with_polarity(polarity).with_trigger(trigger);
    chip.ok_or(RouteError::NoSuchGsi)?.route(gsi, entry).map_err(RouteError::IoApic)
}

/// Mask global system interrupt `gsi` again.
pub fn mask_gsi(gsi: u32) -> Result<(), RouteError> {
    let io = IO_APICS.lock();
    let chip = io.as_ref().ok_or(RouteError::NotReady)?.chips.iter().find(|c| c.input_for(gsi).is_some());
    let chip = chip.ok_or(RouteError::NoSuchGsi)?;
    let input = chip.input_for(gsi).ok_or(RouteError::NoSuchGsi)?;
    chip.set_masked(input, true).map_err(RouteError::IoApic)
}

/// The redirection entry of `gsi`, for tests and diagnostics.
pub fn gsi_entry(gsi: u32) -> Option<RedirectionEntry> {
    let io = IO_APICS.lock();
    let chip = io.as_ref()?.chips.iter().find(|c| c.input_for(gsi).is_some())?;
    chip.entry(chip.input_for(gsi)?).ok()
}

crate::kernel_test! {
    #[boot_only]
    fn io_apic_inputs_start_masked() -> TestResult {
        if io_apic_inputs() == 0 {
            return Err("no I/O APIC was set up");
        }
        let io = IO_APICS.lock();
        let io = io.as_ref().ok_or("I/O APICs not initialised")?;
        for chip in &io.chips {
            for input in 0..chip.inputs() {
                let gsi = chip.gsi_base() + u32::from(input);
                // Tests may route an input for a moment, but never leave it so.
                if chip.entry(input).map_or(true, |e| !e.is_masked()) {
                    crate::kprintln!("apic: GSI {gsi} is unmasked: {:x?}", chip.entry(input));
                    return Err("an I/O APIC input is unmasked with no driver for it");
                }
            }
        }
        // QEMU's (and most PCs') timer override: ISA IRQ 0 arrives on GSI 2.
        match io.isa.route(0) {
            Some(r) if r.gsi < 24 => Ok(()),
            _ => Err("ISA IRQ 0 has no route"),
        }
    }
}

crate::kernel_test! {
    fn legacy_pics_are_masked() -> TestResult {
        let masks = Pic8259::new(CpuPorts).masks();
        if masks == u16::MAX { Ok(()) } else { Err("an 8259 input is unmasked") }
    }
}
