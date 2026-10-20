//! ACPI tables, read through the direct map.
//!
//! All parsing lives in `poly-acpi`; this module only gives it access to
//! physical memory, prints what the firmware describes and keeps a summary
//! for the self-test, a [`Platform`] description for interrupt and timer
//! setup (and, later, SMP), and the PCI ECAM windows from the MCFG for
//! [`crate::pci`].

use crate::arch::phys_range_to_virt;
use crate::sync::SpinLock;
use crate::{kprint, kprintln};
use alloc::vec::Vec;
use poly_acpi::{
    Acpi, AddressSpace, EcamRegion, Error, Fadt, Hpet, Madt, MadtEntry, Mcfg, PhysMemory, ResetRegister, Table,
};
use poly_mem::{MemoryMap, RegionKind};

/// What the self-test checks.
#[derive(Clone, Copy, Debug)]
pub struct Summary {
    /// Processors whose local APIC entry is enabled.
    pub cpus: usize,
    pub io_apics: usize,
}

static SUMMARY: SpinLock<Option<Summary>> = SpinLock::new(None);
static ECAM: SpinLock<Vec<EcamRegion>> = SpinLock::new(Vec::new());

/// The summary from [`init`], if the tables could be read.
pub fn summary() -> Option<Summary> {
    *SUMMARY.lock()
}

/// What interrupt and timer setup needs from the tables, copied out as plain
/// values (from the MADT, HPET and FADT).
#[derive(Clone, Debug, Default)]
pub struct Platform {
    /// Physical address of the local APICs (xAPIC mode).
    pub local_apic: u64,
    /// Legacy 8259 PICs present, to be remapped and masked.
    pub has_8259: bool,
    /// Enabled processors.
    pub processors: Vec<Cpu>,
    pub io_apics: Vec<IoApicInfo>,
    /// Interrupt source overrides.
    pub overrides: Vec<Override>,
    /// Local APIC inputs wired to NMI.
    pub lint_nmis: Vec<LintNmi>,
    /// Physical address of the HPET block (memory space only).
    pub hpet: Option<u64>,
    /// ACPI PM timer I/O port and counter width (24 or 32).
    pub pm_timer: Option<(u16, u8)>,
    /// The FADT's reset register and value, for rebooting.
    pub reset: Option<ResetRegister>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cpu {
    pub uid: u32,
    pub apic_id: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IoApicInfo {
    pub id: u8,
    pub address: u64,
    pub gsi_base: u32,
}

/// An Interrupt Source Override (ACPI 6.5 §5.2.12.5); `flags` are the MPS
/// INTI flags.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Override {
    pub bus: u8,
    pub source: u8,
    pub gsi: u32,
    pub flags: u16,
}

/// A Local APIC NMI structure (§5.2.12.7, §5.2.12.13).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LintNmi {
    /// `None` for every processor.
    pub processor_uid: Option<u32>,
    /// LINT0 or LINT1.
    pub lint: u8,
    pub flags: u16,
}

static PLATFORM: SpinLock<Option<Platform>> = SpinLock::new(None);

/// The platform description from [`init`], if the tables could be read.
pub fn platform() -> Option<Platform> {
    PLATFORM.lock().clone()
}

/// The PCI ECAM windows the MCFG describes, one per segment group and bus
/// range. Empty without an MCFG (no PCI Express, or a machine like QEMU's
/// `pc` that only has the legacy port mechanism).
pub fn ecam_regions() -> Vec<EcamRegion> {
    ECAM.lock().clone()
}

/// Physical memory through the kernel's direct map, limited to memory the
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
        let start = phys_range_to_virt(phys, len as u64)?;
        // SAFETY: the whole range is direct-mapped (checked above), and the
        // direct map stays in place for the kernel's lifetime. It lies
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
        Ok((s, p)) => {
            *SUMMARY.lock() = Some(s);
            *PLATFORM.lock() = Some(p);
        }
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

fn probe(mem: &FirmwareMemory, rsdp: u64) -> Result<(Summary, Platform), Error> {
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
    let mut platform = platform_from_madt(&madt);

    if let Some(hpet) = optional::<Hpet>(&acpi) {
        kprintln!(
            "acpi: HPET at {:#x}, {} comparators, minimum tick {}",
            hpet.base.address,
            hpet.comparator_count(),
            hpet.minimum_tick
        );
        if hpet.base.space == AddressSpace::SystemMemory && hpet.base.address != 0 {
            platform.hpet = Some(hpet.base.address);
        }
    }
    if let Some(mcfg) = optional::<Mcfg>(&acpi) {
        let regions: Vec<EcamRegion> = mcfg.regions().collect();
        for r in &regions {
            kprintln!("acpi: PCI ECAM at {:#x}, segment {} buses {}-{}", r.base, r.segment, r.start_bus, r.end_bus);
        }
        *ECAM.lock() = regions;
    }
    if let Some(fadt) = optional::<Fadt>(&acpi) {
        kprint!("acpi: FADT SCI {}, century {:#x}", fadt.sci_interrupt(), fadt.century());
        if let Some(t) = fadt.pm_timer() {
            let space = if t.block.space == AddressSpace::SystemIo { "port" } else { "address" };
            kprint!(", PM timer {space} {:#x} ({}-bit)", t.block.address, t.bits);
            if let (AddressSpace::SystemIo, Ok(port)) = (t.block.space, u16::try_from(t.block.address)) {
                platform.pm_timer = Some((port, t.bits));
            }
        }
        if fadt.boot_arch().has_8042() {
            kprint!(", 8042");
        }
        if let Some(r) = fadt.reset() {
            kprint!(", reset {:#x}={:#x}", r.register.address, r.value);
            platform.reset = Some(r);
        }
        kprintln!();
    }

    Ok((Summary { cpus, io_apics }, platform))
}

fn platform_from_madt(madt: &Madt) -> Platform {
    let mut p = Platform {
        local_apic: madt.local_apic_address(),
        has_8259: madt.has_8259(),
        processors: madt.processors().filter(|c| c.enabled()).map(|c| Cpu { uid: c.uid, apic_id: c.apic_id }).collect(),
        io_apics: madt
            .io_apics()
            .map(|io| IoApicInfo { id: io.id, address: io.address.into(), gsi_base: io.gsi_base })
            .collect(),
        overrides: madt
            .interrupt_overrides()
            .map(|o| Override { bus: o.bus, source: o.source, gsi: o.gsi, flags: o.flags.0 })
            .collect(),
        ..Platform::default()
    };
    for e in madt.entries() {
        let nmi = match e {
            MadtEntry::LocalApicNmi(n) => LintNmi {
                processor_uid: (n.processor_uid != 0xff).then_some(n.processor_uid.into()),
                lint: n.lint,
                flags: n.flags.0,
            },
            MadtEntry::LocalX2ApicNmi(n) => LintNmi {
                processor_uid: (n.processor_uid != u32::MAX).then_some(n.processor_uid),
                lint: n.lint,
                flags: n.flags.0,
            },
            _ => continue,
        };
        p.lint_nmis.push(nmi);
    }
    p
}
