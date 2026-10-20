//! ACPI tables, read through the boot mapping.
//!
//! All parsing lives in `poly-acpi`; this module only gives it access to
//! physical memory, prints what the firmware describes and keeps a summary
//! for the self-test (and, later, for interrupt and SMP setup).

use crate::arch::phys_to_virt;
use crate::sync::SpinLock;
use crate::{kprint, kprintln};
use poly_acpi::{Acpi, AddressSpace, Error, Fadt, Hpet, Madt, Mcfg, PhysMemory, Table};
use poly_mem::{MemoryMap, RegionKind};

/// What the self-test checks.
#[derive(Clone, Copy, Debug)]
pub struct Summary {
    /// Processors whose local APIC entry is enabled.
    pub cpus: usize,
    pub io_apics: usize,
}

static SUMMARY: SpinLock<Option<Summary>> = SpinLock::new(None);

/// The summary from [`init`], if the tables could be read.
pub fn summary() -> Option<Summary> {
    *SUMMARY.lock()
}

/// Physical memory through the kernel's boot mapping, limited to memory the
/// kernel never writes: firmware tables must not live in usable RAM or in
/// the kernel image, so a pointer into those is rejected as bogus.
#[derive(Debug)]
struct FirmwareMemory<'m> {
    memory: &'m MemoryMap,
}

impl PhysMemory for FirmwareMemory<'_> {
    fn bytes(&self, phys: u64, len: usize) -> Option<&[u8]> {
        if len == 0 {
            return Some(&[]);
        }
        let last = phys.checked_add(len as u64 - 1)?;
        let owned = self
            .memory
            .regions()
            .iter()
            .any(|r| matches!(r.kind, RegionKind::Usable | RegionKind::Kernel) && r.start <= last && phys < r.end);
        if owned {
            return None;
        }
        // The boot mapping is linear, so both ends being mapped (and the
        // right distance apart) means everything in between is.
        let start = phys_to_virt(phys)?;
        if phys_to_virt(last)? != start + (len as u64 - 1) {
            return None;
        }
        // SAFETY: the range is mapped by the boot page tables (checked
        // above), which stay in place for the kernel's lifetime. It lies
        // outside usable RAM and the kernel image, so nothing in the kernel
        // writes it while the shared slice exists.
        Some(unsafe { core::slice::from_raw_parts(start as *const u8, len) })
    }
}

/// Read the ACPI tables, print a summary and remember it.
pub fn init(rsdp: Option<u64>, memory: &MemoryMap) {
    let Some(rsdp) = rsdp else {
        kprintln!("acpi: the boot loader passed no RSDP");
        return;
    };
    match probe(&FirmwareMemory { memory }, rsdp) {
        Ok(s) => *SUMMARY.lock() = Some(s),
        Err(e) => kprintln!("acpi: unavailable: {e:?}"),
    }
}

fn text(b: &[u8]) -> &str {
    core::str::from_utf8(b).unwrap_or("?").trim_end()
}

/// Look up an optional table: `Ok(None)` if the firmware has none.
fn optional<'a, T: Table<'a>>(acpi: &Acpi<'a, FirmwareMemory<'_>>) -> Option<T> {
    match acpi.table::<T>() {
        Ok(t) => Some(t),
        Err(Error::NotFound) => None,
        Err(e) => {
            kprintln!("acpi: {} table unusable: {e:?}", text(&T::SIGNATURE));
            None
        }
    }
}

fn probe(mem: &FirmwareMemory, rsdp: u64) -> Result<Summary, Error> {
    let acpi = Acpi::new(mem, rsdp)?;
    let root = acpi.root();
    kprint!(
        "acpi: revision {} OEM \"{}\", {} lists",
        acpi.rsdp().revision,
        text(&acpi.rsdp().oem_id),
        text(&root.signature())
    );
    for t in acpi.tables() {
        match t {
            Ok(t) => kprint!(" {}", text(&t.signature())),
            Err(e) => kprint!(" <{e:?}>"),
        }
    }
    kprintln!();

    let madt = acpi.table::<Madt>()?;
    let cpus = madt.processors().filter(|p| p.enabled()).count();
    let hotplug = madt.processors().filter(|p| !p.enabled() && p.online_capable()).count();
    kprintln!(
        "acpi: CPUs: {cpus} enabled, {hotplug} online-capable; local APIC at {:#x}{}",
        madt.local_apic_address(),
        if madt.has_8259() { ", 8259 PICs present" } else { "" }
    );
    let mut io_apics = 0;
    for io in madt.io_apics() {
        kprintln!("acpi: I/O APIC {} at {:#x}, GSI base {}", io.id, io.address, io.gsi_base);
        io_apics += 1;
    }
    kprint!("acpi: {} interrupt source overrides:", madt.interrupt_overrides().count());
    for o in madt.interrupt_overrides() {
        kprint!(" IRQ{}->GSI{}", o.source, o.gsi);
    }
    kprintln!();

    if let Some(hpet) = optional::<Hpet>(&acpi) {
        kprintln!(
            "acpi: HPET at {:#x}, {} comparators, minimum tick {}",
            hpet.base.address,
            hpet.comparator_count(),
            hpet.minimum_tick
        );
    }
    if let Some(mcfg) = optional::<Mcfg>(&acpi) {
        for r in mcfg.regions() {
            kprintln!("acpi: PCI ECAM at {:#x}, segment {} buses {}-{}", r.base, r.segment, r.start_bus, r.end_bus);
        }
    }
    if let Some(fadt) = optional::<Fadt>(&acpi) {
        kprint!("acpi: FADT SCI {}, century {:#x}", fadt.sci_interrupt(), fadt.century());
        if let Some(t) = fadt.pm_timer() {
            let space = if t.block.space == AddressSpace::SystemIo { "port" } else { "address" };
            kprint!(", PM timer {space} {:#x} ({}-bit)", t.block.address, t.bits);
        }
        if fadt.boot_arch().has_8042() {
            kprint!(", 8042");
        }
        if let Some(r) = fadt.reset() {
            kprint!(", reset {:#x}={:#x}", r.register.address, r.value);
        }
        kprintln!();
    }

    Ok(Summary { cpus, io_apics })
}
