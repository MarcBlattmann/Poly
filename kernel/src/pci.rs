//! PCI: configuration space through ECAM, the list of functions found at
//! boot, and mappings of memory BARs for drivers.
//!
//! All protocol logic (header parsing, BAR sizing, enumeration, capability
//! walks, MSI-X) lives in `poly-pci`; this module gives it hardware access
//! through two traits ([ADR 0009](../../docs/adr/0009-drivers-behind-access-traits.md)):
//!
//! - [`EcamSpace`] implements [`ConfigSpace`] over one ECAM window from the
//!   ACPI MCFG, mapped uncached with [`mm::map_mmio`]. Accesses are volatile
//!   and exactly as wide as asked (PCI Express Base Specification §7.2.2).
//! - [`MmioRegion`] (`arch/x86_64/mmio.rs`, the kernel's one `poly_pci::Mmio`)
//!   over a mapped memory BAR, or a part of one.
//!
//! [`init`] maps every window, finds every function (every bus the window
//! decodes is probed, so root buses that no bridge leads to are found too),
//! sizes their BARs and prints one line per function. Drivers then look
//! their device up in [`devices`] and map its BARs with [`Device::map_bar`].
//! Only ECAM is supported; machines without an MCFG (QEMU's `pc`) have no
//! PCI for now.
//!
//! Configuration space is not locked: enumeration runs once at boot on one
//! CPU, and afterwards each driver only touches its own functions.

use crate::sync::SpinLock;
use crate::{acpi, kprint, kprintln, mm};
use alloc::vec::Vec;
use core::fmt;
use core::ptr;
use poly_acpi::EcamRegion;
use poly_pci::{
    Bar, BarKind, Bdf, Capabilities, ConfigSpace, Ecam, Header, MsiError, MsiX, cap_id, probe_bars, reg, scan_all,
};

/// The configuration space of one PCI segment group, through its ECAM
/// window.
#[derive(Debug)]
pub struct EcamSpace {
    pub segment: u16,
    ecam: Ecam,
    /// Physical start of the window: bus `start_bus`, not bus 0.
    window_phys: u64,
    /// Where the window is mapped.
    window_virt: u64,
}

impl EcamSpace {
    /// Map the window an MCFG entry describes.
    fn map(r: &EcamRegion) -> Result<EcamSpace, &'static str> {
        // The MCFG base is where bus 0 would be, whatever the start bus
        // (see `poly_pci::Ecam`), so it is copied over unchanged.
        let ecam = Ecam { base: r.base, start_bus: r.start_bus, end_bus: r.end_bus };
        let (window_phys, len) = ecam.window().ok_or("bus range inverted or window past the end of memory")?;
        let window_virt = mm::map_mmio(window_phys, len).ok_or("cannot map the window")?;
        Ok(EcamSpace { segment: r.segment, ecam, window_phys, window_virt })
    }

    pub const fn start_bus(&self) -> u8 {
        self.ecam.start_bus
    }

    pub const fn end_bus(&self) -> u8 {
        self.ecam.end_bus
    }

    /// Virtual address of a register `width` bytes wide, if the window
    /// decodes it and it is naturally aligned. ECAM accesses must not cross
    /// a dword, and natural alignment is also what `read_volatile` needs.
    fn register(&self, bdf: Bdf, offset: u16, width: u16) -> Option<u64> {
        if !offset.is_multiple_of(width) {
            return None;
        }
        let phys = self.ecam.address(bdf, offset)?;
        Some(self.window_virt + (phys - self.window_phys))
    }
}

// Every access below goes through `register`, which only returns addresses
// inside the window `map` mapped (uncached, for the kernel's lifetime: MMIO
// mappings are never removed), aligned for the access width. Reads outside
// the window return all ones, as a missing function does; writes there are
// dropped.
impl ConfigSpace for EcamSpace {
    fn read32(&self, bdf: Bdf, offset: u16) -> u32 {
        match self.register(bdf, offset, 4) {
            // SAFETY: mapped device memory, aligned (see above).
            Some(v) => unsafe { ptr::read_volatile(v as *const u32) },
            None => u32::MAX,
        }
    }

    fn read16(&self, bdf: Bdf, offset: u16) -> u16 {
        match self.register(bdf, offset, 2) {
            // SAFETY: as for `read32`.
            Some(v) => unsafe { ptr::read_volatile(v as *const u16) },
            None => u16::MAX,
        }
    }

    fn read8(&self, bdf: Bdf, offset: u16) -> u8 {
        match self.register(bdf, offset, 1) {
            // SAFETY: as for `read32`.
            Some(v) => unsafe { ptr::read_volatile(v as *const u8) },
            None => u8::MAX,
        }
    }

    fn write32(&self, bdf: Bdf, offset: u16, value: u32) {
        if let Some(v) = self.register(bdf, offset, 4) {
            // SAFETY: as for `read32`.
            unsafe { ptr::write_volatile(v as *mut u32, value) }
        }
    }

    fn write16(&self, bdf: Bdf, offset: u16, value: u16) {
        if let Some(v) = self.register(bdf, offset, 2) {
            // SAFETY: as for `read32`.
            unsafe { ptr::write_volatile(v as *mut u16, value) }
        }
    }

    fn write8(&self, bdf: Bdf, offset: u16, value: u8) {
        if let Some(v) = self.register(bdf, offset, 1) {
            // SAFETY: as for `read32`.
            unsafe { ptr::write_volatile(v as *mut u8, value) }
        }
    }
}

pub use crate::arch::mmio::MmioRegion;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MapError {
    /// No BAR at that index (unimplemented, or the upper half of a 64-bit
    /// one).
    NoSuchBar,
    /// An I/O port BAR; drivers use port instructions for those.
    NotMemory,
    /// The firmware left the BAR unassigned (address 0).
    Unassigned,
    /// `map_mmio` refused (overlaps RAM, or the MMIO window is full).
    MapFailed,
}

/// One PCI function, as found at boot.
#[derive(Debug)]
pub struct Device {
    pub header: Header,
    /// Indexed by register: a 64-bit BAR sits at its first register and the
    /// next reads `None`. All `None` where BARs were not probed (see
    /// [`init`]).
    pub bars: [Option<Bar>; 6],
    cfg: &'static EcamSpace,
}

impl Device {
    pub const fn bdf(&self) -> Bdf {
        self.header.bdf
    }

    /// The configuration space the function lives in.
    pub const fn config(&self) -> &'static EcamSpace {
        self.cfg
    }

    pub fn capabilities(&self) -> Capabilities<'static, EcamSpace> {
        Capabilities::new(self.cfg, &self.header)
    }

    /// The function's MSI-X capability, checked against its BARs. `None` if
    /// it has none.
    pub fn msix(&self) -> Option<Result<MsiX, MsiError>> {
        let cap = self.capabilities().find_id(cap_id::MSI_X)?;
        Some(MsiX::new(self.cfg, self.bdf(), cap).and_then(|m| m.check_bars(&self.bars).map(|()| m)))
    }

    /// Map memory BAR `index` (all of it) for the kernel. Mapping the same
    /// BAR again returns the first mapping. Does not touch the command
    /// register: the driver enables memory decoding (and bus mastering)
    /// with `poly_pci::enable`.
    pub fn map_bar(&self, index: usize) -> Result<MmioRegion, MapError> {
        let bar = self.bars.get(index).copied().flatten().ok_or(MapError::NoSuchBar)?;
        if !bar.is_memory() {
            return Err(MapError::NotMemory);
        }
        if bar.address == 0 {
            return Err(MapError::Unassigned);
        }
        let len = usize::try_from(bar.size).map_err(|_| MapError::MapFailed)?;
        let key = (self.cfg.segment, self.bdf(), index as u8);
        let mut mapped = MAPPED_BARS.lock();
        let virt = match mapped.iter().find(|(k, _)| *k == key) {
            Some(&(_, virt)) => virt,
            None => {
                let virt = mm::map_mmio(bar.address, bar.size).ok_or(MapError::MapFailed)?;
                mapped.push((key, virt));
                virt
            }
        };
        // SAFETY: `virt` is this BAR's `map_mmio` mapping (made here once and
        // kept in MAPPED_BARS), `len` bytes, never unmapped.
        Ok(unsafe { MmioRegion::from_mapping(virt, len) })
    }
}

impl fmt::Display for Device {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let h = &self.header;
        if self.cfg.segment != 0 {
            write!(f, "{:04x}:", self.cfg.segment)?;
        }
        let c = h.class;
        write!(
            f,
            "{} {:04x}:{:04x} class {:02x}.{:02x}.{:02x}",
            h.bdf, h.vendor_id, h.device_id, c.class, c.subclass, c.prog_if
        )?;
        if let Some(b) = h.bridge {
            write!(f, ", bridge to buses {:02x}-{:02x}", b.secondary, b.subordinate)?;
        }
        if skip_bar_probe(h) {
            return write!(f, ", BARs not probed");
        }
        for (i, bar) in self.bars.iter().enumerate() {
            let Some(bar) = bar else { continue };
            let kind = match bar.kind {
                BarKind::Memory32 => "mem32",
                BarKind::Memory64 => "mem64",
                BarKind::Io => "io",
            };
            let pf = if bar.prefetchable { " pf" } else { "" };
            write!(f, ", BAR{i} {kind}{pf} {:#x} {}", bar.address, Size(bar.size))?;
        }
        Ok(())
    }
}

/// A byte count in the largest binary unit that divides it.
#[derive(Clone, Copy, Debug)]
pub struct Size(pub u64);

impl fmt::Display for Size {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let units = [(30, "GiB"), (20, "MiB"), (10, "KiB")];
        match units.iter().find(|(shift, _)| self.0 >= 1 << shift && self.0.is_multiple_of(1 << shift)) {
            Some((shift, unit)) => write!(f, "{} {unit}", self.0 >> shift),
            None => write!(f, "{} B", self.0),
        }
    }
}

/// Host bridges and ISA (LPC) bridges are not BAR-probed: sizing a BAR
/// turns off the function's decoding for a moment, and these decode things
/// the kernel is using at that moment (RAM, the ECAM window itself, the
/// legacy serial port and ACPI registers). Their BARs, if any, are
/// chipset-specific anyway.
fn skip_bar_probe(h: &Header) -> bool {
    h.class.class == 0x06 && matches!(h.class.subclass, 0x00 | 0x01)
}

#[derive(Clone, Copy, Debug)]
struct Pci {
    segments: &'static [EcamSpace],
    devices: &'static [Device],
}

static PCI: SpinLock<Option<Pci>> = SpinLock::new(None);

/// A mapped BAR: segment, function and BAR index.
type BarKey = (u16, Bdf, u8);

/// BARs mapped so far, with their virtual addresses.
static MAPPED_BARS: SpinLock<Vec<(BarKey, u64)>> = SpinLock::new(Vec::new());

/// Every function found at boot; empty before [`init`] or without PCI.
pub fn devices() -> &'static [Device] {
    PCI.lock().map_or(&[], |p| p.devices)
}

/// The mapped ECAM windows.
pub fn segments() -> &'static [EcamSpace] {
    PCI.lock().map_or(&[], |p| p.segments)
}

/// Map the ECAM windows from the ACPI MCFG, find every function, size its
/// BARs and print one line per function. Call once, after `acpi::init`,
/// before any driver.
pub fn init() {
    assert!(PCI.lock().is_none(), "pci::init called twice");
    let regions = acpi::ecam_regions();
    if regions.is_empty() {
        kprintln!("pci: no ECAM window in the ACPI MCFG, so no PCI (the legacy port mechanism is not supported)");
        return;
    }
    let mut segments = Vec::new();
    for r in &regions {
        match EcamSpace::map(r) {
            Ok(s) => {
                kprintln!(
                    "pci: segment {} buses {:02x}-{:02x}: ECAM at {:#x}, mapped at {:#x}",
                    s.segment,
                    s.start_bus(),
                    s.end_bus(),
                    s.window_phys,
                    s.window_virt
                );
                segments.push(s);
            }
            Err(e) => kprintln!("pci: ECAM window at {:#x} (segment {}) unusable: {e}", r.base, r.segment),
        }
    }
    // Both lists live as long as the kernel; drivers keep references.
    let segments: &'static [EcamSpace] = segments.leak();
    let mut devices = Vec::new();
    for s in segments {
        scan_all(s, s.start_bus(), s.end_bus(), |h| {
            let bars = if skip_bar_probe(h) { [None; 6] } else { probe_bars(s, h) };
            devices.push(Device { header: *h, bars, cfg: s });
        });
    }
    for d in &devices {
        kprintln!("pci: {d}");
    }
    kprint!("pci: {} functions", devices.len());
    if segments.len() > 1 {
        kprint!(" in {} segments", segments.len());
    }
    kprintln!();
    *PCI.lock() = Some(Pci { segments, devices: devices.leak() });
}

crate::kernel_test! {
    fn host_bridge_config_space() -> TestResult {
        let cfg = segments().first().ok_or("no ECAM window mapped")?;
        let host = Bdf::new(cfg.start_bus(), 0, 0);
        let id = cfg.read32(host, reg::VENDOR_ID);
        let (vendor, device) = (id as u16, (id >> 16) as u16);
        if vendor == 0xffff || vendor == 0 || device == 0xffff {
            return Err("no function answers at 00:00.0");
        }
        // Narrow reads are real 8- and 16-bit accesses; they must agree with
        // the dword.
        let narrow_ok = cfg.read16(host, reg::VENDOR_ID) == vendor
            && cfg.read16(host, reg::DEVICE_ID) == device
            && cfg.read8(host, reg::VENDOR_ID + 1) == (vendor >> 8) as u8
            && cfg.read8(host, reg::DEVICE_ID + 1) == (device >> 8) as u8;
        if !narrow_ok {
            return Err("8- and 16-bit configuration reads disagree with the 32-bit read");
        }
        if (cfg.read8(host, reg::CLASS), cfg.read8(host, reg::SUBCLASS)) != (0x06, 0x00) {
            return Err("00:00.0 is not a host bridge");
        }
        if cfg.read32(host, reg::VENDOR_ID + 2) != u32::MAX || cfg.read32(host, 0x1000) != u32::MAX {
            return Err("misaligned or out-of-range configuration read did not return all ones");
        }
        // A device number nobody answers at reads all ones.
        let devs = devices();
        let present = |d: u8| devs.iter().any(|x| x.bdf().bus() == host.bus() && x.bdf().device() == d);
        let empty = (0..=Bdf::MAX_DEVICE).find(|&d| !present(d)).ok_or("every device number on bus 0 is in use")?;
        if cfg.read32(Bdf::new(host.bus(), empty, 0), reg::VENDOR_ID) != u32::MAX {
            return Err("an absent device did not read all ones");
        }
        // Enumeration found the host bridge, with the same IDs.
        match devs.iter().find(|d| d.bdf() == host) {
            Some(d) if (d.header.vendor_id, d.header.device_id) == (vendor, device) => Ok(()),
            Some(_) => Err("enumerated host bridge has different IDs"),
            None => Err("enumeration missed the host bridge"),
        }
    }
}

crate::kernel_test! {
    fn enumerated_bars_are_sane() -> TestResult {
        for d in devices() {
            for bar in d.bars.iter().flatten() {
                // BAR sizes are powers of two and assigned BARs are aligned to
                // their size (PCI 3.0 §6.2.5.1).
                if !bar.size.is_power_of_two() || (bar.address != 0 && !bar.address.is_multiple_of(bar.size)) {
                    kprintln!("pci: {d}: bad BAR {bar:x?}");
                    return Err("BAR size not a power of two or address not aligned to it");
                }
            }
        }
        Ok(())
    }
}
