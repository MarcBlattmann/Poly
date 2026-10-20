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

use crate::arch::irq::{self, Handler};
use crate::sync::SpinLock;
use crate::{acpi, kprint, kprintln, mm};
use alloc::vec;
use alloc::vec::Vec;
use core::fmt;
use core::ptr;
use poly_acpi::EcamRegion;
use poly_irq::MsiMessage;
use poly_pci::{
    Bar, BarKind, Bdf, Capabilities, ConfigSpace, Ecam, Header, Message, Msi, MsiError, MsiX, cap_id, command,
    probe_bars, reg, scan_all,
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

// --- Message signalled interrupts --------------------------------------------

/// Why a function's interrupts could not be set up.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IrqError {
    /// The function has no MSI-X capability (or, for MSI, no MSI one).
    NoCapability,
    /// The capability is malformed or cannot do what was asked.
    Msi(MsiError),
    /// The MSI-X table or PBA could not be mapped.
    Bar(MapError),
    /// Every device vector is taken.
    NoVector,
    /// No message can reach the boot CPU (its local APIC is not set up, or
    /// its ID does not fit an MSI address).
    NoDestination,
    /// The MSI-X table entry already has a vector.
    EntryInUse,
}

/// The x86 message `poly-irq` builds, as `poly-pci` writes it into a
/// function (the same address / data pair; the crates do not know each
/// other).
pub const fn to_pci_message(m: MsiMessage) -> Message {
    Message { address: m.address, data: m.data }
}

/// A message read back from a function, for `poly-irq` to decode.
pub const fn from_pci_message(m: Message) -> MsiMessage {
    MsiMessage { address: m.address, data: m.data }
}

/// The message that raises kernel vector `vector` on the boot CPU.
pub fn msi_message(vector: u8) -> Option<Message> {
    irq::msi_message(vector).map(to_pci_message)
}

/// A function's MSI-X, enabled: which table entries raise which kernel
/// vector. Made by [`Device::enable_msix`]; [`MsixVectors::disable`] turns
/// it off again and frees the vectors (dropping it without that leaves
/// both as they are, which suits a device that lives as long as the
/// kernel).
#[derive(Debug)]
pub struct MsixVectors {
    cfg: &'static EcamSpace,
    bdf: Bdf,
    cap: MsiX,
    table: MmioRegion,
    pba: MmioRegion,
    /// The kernel vector of each table entry, once attached.
    vectors: Vec<Option<u8>>,
}

impl MsixVectors {
    /// The kernel vector table entry `entry` raises, if attached.
    pub fn vector(&self, entry: u16) -> Option<u8> {
        self.vectors.get(entry as usize).copied().flatten()
    }

    /// Give table entry `entry` a fresh kernel vector running
    /// `handler(data)`, and program it (unmasked) with the message for that
    /// vector. Nothing arrives while the function mask is set (as
    /// [`Device::enable_msix`] leaves it). Returns the vector.
    pub fn attach(&mut self, entry: u16, handler: Handler, data: usize) -> Result<u8, IrqError> {
        if entry >= self.cap.table_size {
            return Err(IrqError::Msi(MsiError::NoSuchVector));
        }
        if self.vectors[entry as usize].is_some() {
            return Err(IrqError::EntryInUse);
        }
        let vector = irq::allocate_vector(handler, data).ok_or(IrqError::NoVector)?;
        let result = match msi_message(vector) {
            Some(msg) => self.cap.write_entry(&self.table, entry, msg, false).map_err(IrqError::Msi),
            None => Err(IrqError::NoDestination),
        };
        if let Err(e) = result {
            irq::free_vector(vector);
            return Err(e);
        }
        self.vectors[entry as usize] = Some(vector);
        Ok(vector)
    }

    /// Mask table entry `entry` and free its vector. False if it had none.
    pub fn detach(&mut self, entry: u16) -> bool {
        let Some(vector) = self.vector(entry) else { return false };
        // In range: it has a vector.
        let _ = self.cap.set_vector_masked(&self.table, entry, true);
        self.vectors[entry as usize] = None;
        irq::free_vector(vector)
    }

    /// Hold back (or let through) every entry at once.
    pub fn set_function_masked(&self, masked: bool) {
        self.cap.set_function_mask(self.cfg, self.bdf, masked);
    }

    pub fn is_function_masked(&self) -> bool {
        self.cap.is_function_masked(self.cfg, self.bdf)
    }

    /// Mask or unmask one entry. A masked entry's interrupt waits in the
    /// PBA ([`MsixVectors::is_pending`]) and is sent once it is unmasked.
    pub fn set_masked(&self, entry: u16, masked: bool) -> Result<(), IrqError> {
        self.cap.set_vector_masked(&self.table, entry, masked).map_err(IrqError::Msi)
    }

    /// Table entry `entry` as the function sees it: message and mask.
    pub fn entry(&self, entry: u16) -> Result<(Message, bool), IrqError> {
        self.cap.read_entry(&self.table, entry).map_err(IrqError::Msi)
    }

    pub fn is_pending(&self, entry: u16) -> Result<bool, IrqError> {
        self.cap.is_pending(&self.pba, entry).map_err(IrqError::Msi)
    }

    /// Turn MSI-X off: mask the function and every attached entry, clear
    /// the enable bit and free the vectors. The function cannot interrupt
    /// afterwards (INTx stays disabled).
    pub fn disable(mut self) {
        self.set_function_masked(true);
        for entry in 0..self.cap.table_size {
            self.detach(entry);
        }
        self.cap.set_enabled(self.cfg, self.bdf, false);
    }
}

/// A function's plain MSI, enabled with one vector: made by
/// [`Device::enable_msi`], undone by [`MsiVector::disable`].
#[derive(Debug)]
pub struct MsiVector {
    cfg: &'static EcamSpace,
    bdf: Bdf,
    cap: Msi,
    vector: u8,
}

impl MsiVector {
    pub const fn vector(&self) -> u8 {
        self.vector
    }

    pub fn is_enabled(&self) -> bool {
        self.cap.is_enabled(self.cfg, self.bdf)
    }

    /// The message the function sends, as programmed.
    pub fn message(&self) -> Message {
        self.cap.read_message(self.cfg, self.bdf).0
    }

    /// Mask or unmask the vector; only functions with per-vector masking
    /// can.
    pub fn set_masked(&self, masked: bool) -> Result<(), IrqError> {
        self.cap.set_masked(self.cfg, self.bdf, 0, masked).map_err(IrqError::Msi)
    }

    /// Turn MSI off and free the vector (INTx stays disabled).
    pub fn disable(self) {
        self.cap.set_enabled(self.cfg, self.bdf, false);
        irq::free_vector(self.vector);
    }
}

/// A function's interrupt, whichever way it is delivered; see
/// [`Device::enable_interrupt`].
#[derive(Debug)]
pub enum Interrupt {
    /// MSI-X table entry 0.
    MsiX(MsixVectors),
    Msi(MsiVector),
}

impl Interrupt {
    /// The kernel vector it raises (`None` if MSI-X entry 0 was detached).
    pub fn vector(&self) -> Option<u8> {
        match self {
            Interrupt::MsiX(v) => v.vector(0),
            Interrupt::Msi(v) => Some(v.vector()),
        }
    }

    pub fn disable(self) {
        match self {
            Interrupt::MsiX(v) => v.disable(),
            Interrupt::Msi(v) => v.disable(),
        }
    }
}

impl Device {
    /// Turn on the function's MSI-X with everything masked (the function
    /// mask and every entry), and memory decoding (the table is in a BAR),
    /// INTx and MSI off. Then [`MsixVectors::attach`] the entries the driver
    /// uses and clear the function mask; this order sends no interrupt
    /// before its entry is programmed (PCI 3.0 §6.8.2). The function needs
    /// bus mastering to send messages, which its driver turns on.
    pub fn enable_msix(&self) -> Result<MsixVectors, IrqError> {
        let (cfg, bdf) = (self.cfg, self.bdf());
        let cap = self.msix().ok_or(IrqError::NoCapability)?.map_err(IrqError::Msi)?;
        let part = |bar: u8, offset: u32, len: u64| {
            let region = self.map_bar(bar as usize).map_err(IrqError::Bar)?;
            region.subregion(offset as usize, len as usize).ok_or(IrqError::Msi(MsiError::OutsideBar))
        };
        let table = part(cap.table_bar, cap.table_offset, cap.table_len())?;
        let pba = part(cap.pba_bar, cap.pba_offset, cap.pba_len())?;
        if let Some(Ok(msi)) = self.capabilities().find_id(cap_id::MSI).map(|c| Msi::new(cfg, bdf, c)) {
            msi.set_enabled(cfg, bdf, false);
        }
        poly_pci::enable(cfg, bdf, command::MEMORY_SPACE | command::INTX_DISABLE);
        cap.set_function_mask(cfg, bdf, true);
        cap.set_enabled(cfg, bdf, true);
        for entry in 0..cap.table_size {
            cap.set_vector_masked(&table, entry, true).map_err(IrqError::Msi)?;
        }
        let vectors = vec![None; cap.table_size as usize];
        Ok(MsixVectors { cfg, bdf, cap, table, pba, vectors })
    }

    /// Turn on plain MSI with one vector for `handler(data)`, INTx off: for
    /// functions without MSI-X. (Several MSI vectors would need a block of
    /// consecutive kernel vectors, aligned to its size, since the function
    /// fills in the low data bits; the allocator hands out single ones.)
    /// Bus mastering is the driver's to turn on.
    pub fn enable_msi(&self, handler: Handler, data: usize) -> Result<MsiVector, IrqError> {
        let (cfg, bdf) = (self.cfg, self.bdf());
        let cap = self.capabilities().find_id(cap_id::MSI).ok_or(IrqError::NoCapability)?;
        let cap = Msi::new(cfg, bdf, cap).map_err(IrqError::Msi)?;
        cap.set_enabled(cfg, bdf, false);
        let vector = irq::allocate_vector(handler, data).ok_or(IrqError::NoVector)?;
        let configured = match msi_message(vector) {
            Some(msg) => cap.configure(cfg, bdf, msg, 1).map_err(IrqError::Msi),
            None => Err(IrqError::NoDestination),
        };
        if let Err(e) = configured {
            irq::free_vector(vector);
            return Err(e);
        }
        if cap.per_vector_masking() {
            // Vector 0 exists, and masking is supported.
            let _ = cap.set_masked(cfg, bdf, 0, false);
        }
        poly_pci::enable(cfg, bdf, command::INTX_DISABLE);
        cap.set_enabled(cfg, bdf, true);
        Ok(MsiVector { cfg, bdf, cap, vector })
    }

    /// One interrupt for `handler(data)`: MSI-X table entry 0 if the
    /// function has usable MSI-X, plain MSI otherwise.
    pub fn enable_interrupt(&self, handler: Handler, data: usize) -> Result<Interrupt, IrqError> {
        if let Some(Ok(_)) = self.msix() {
            let mut v = self.enable_msix()?;
            return match v.attach(0, handler, data) {
                Ok(_) => {
                    v.set_function_masked(false);
                    Ok(Interrupt::MsiX(v))
                }
                Err(e) => {
                    v.disable();
                    Err(e)
                }
            };
        }
        self.enable_msi(handler, data).map(Interrupt::Msi)
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

/// The boot test attaches QEMU's `edu` device (a function with plain MSI
/// and no MSI-X) and says so with this flag.
pub const TEST_MSI_FLAG: &str = "poly.test.msi";

/// QEMU's educational device (QEMU documentation, `docs/specs/edu.rst`):
/// IDs, and registers in BAR 0.
mod edu {
    pub const VENDOR: u16 = 0x1234;
    pub const DEVICE: u16 = 0x11e8;
    /// Identification, 0xRRrr00ed (major, minor version).
    pub const ID: usize = 0x00;
    /// Interrupt status: the bits raised and not yet acknowledged.
    pub const IRQ_STATUS: usize = 0x24;
    /// Writing ORs the value into the status and raises the interrupt.
    pub const IRQ_RAISE: usize = 0x60;
    /// Writing clears those status bits.
    pub const IRQ_ACK: usize = 0x64;
}

use core::sync::atomic::{AtomicU32, AtomicU64, Ordering::SeqCst};
use poly_pci::Mmio;

/// The edu device's registers, for its interrupt handler.
static EDU: SpinLock<Option<MmioRegion>> = SpinLock::new(None);
static EDU_INTERRUPTS: AtomicU64 = AtomicU64::new(0);
static EDU_STATUS: AtomicU32 = AtomicU32::new(0);

/// The edu device's interrupt: note and acknowledge what was raised.
fn edu_interrupt(_: usize) {
    let Some(regs) = *EDU.lock() else { return };
    let status = regs.read32(edu::IRQ_STATUS);
    regs.write32(edu::IRQ_ACK, status);
    EDU_STATUS.fetch_or(status, SeqCst);
    EDU_INTERRUPTS.fetch_add(1, SeqCst);
}

fn is_edu(d: &&Device) -> bool {
    (d.header.vendor_id, d.header.device_id) == (edu::VENDOR, edu::DEVICE)
}

crate::kernel_test! {
    /// The MSI fallback of `enable_interrupt`, on a function without MSI-X:
    /// each interrupt the device raises arrives at the handler through the
    /// message `msi_message` built; after `disable` none does.
    fn msi_without_msix_reaches_its_handler(env: &TestEnv) -> TestResult {
        let expected = env.boot.has_flag(TEST_MSI_FLAG);
        let Some(dev) = devices().iter().find(is_edu) else {
            return if expected { Err("poly.test.msi is set but there is no edu device") } else { Ok(()) };
        };
        if dev.msix().is_some() {
            return Err("the edu device has MSI-X, so it cannot test the MSI fallback");
        }
        let regs = dev.map_bar(0).map_err(|_| "cannot map the edu device's BAR 0")?;
        // Messages are memory writes: they need bus mastering.
        poly_pci::enable(dev.config(), dev.bdf(), command::MEMORY_SPACE | command::BUS_MASTER);
        if regs.read32(edu::ID) & 0xff != 0xed {
            return Err("the edu device does not identify itself");
        }
        *EDU.lock() = Some(regs);
        EDU_INTERRUPTS.store(0, SeqCst);
        EDU_STATUS.store(0, SeqCst);
        let unhandled = irq::stats().unhandled;
        let irq = dev.enable_interrupt(edu_interrupt, 0).map_err(|_| "enable_interrupt failed")?;
        let vector = irq.vector().unwrap_or(0);
        let result = (|| {
            let Interrupt::Msi(msi) = &irq else { return Err("enable_interrupt did not fall back to MSI") };
            let (cfg, bdf) = (dev.config(), dev.bdf());
            if Some(msi.message()) != msi_message(vector) || from_pci_message(msi.message()).vector() != vector {
                return Err("the MSI capability does not hold the message for the vector");
            }
            if !msi.is_enabled() || poly_pci::read_command(cfg, bdf) & command::INTX_DISABLE == 0 {
                return Err("MSI is not enabled, or INTx is not disabled");
            }
            for i in 0..3 {
                let before = EDU_INTERRUPTS.load(SeqCst);
                regs.write32(edu::IRQ_RAISE, 1 << i);
                if !wait_until(|| EDU_INTERRUPTS.load(SeqCst) > before) {
                    return Err("an interrupt the edu device raised did not arrive");
                }
                if EDU_STATUS.load(SeqCst) & (1 << i) == 0 {
                    return Err("the handler did not see the status bit that was raised");
                }
            }
            if msi.set_masked(true) != Err(IrqError::Msi(MsiError::NoMasking)) {
                return Err("masking was accepted on a function without per-vector masking");
            }
            let n = EDU_INTERRUPTS.load(SeqCst);
            kprintln!("pci: {} MSI -> vector {vector:#x}: {n} interrupts delivered", dev.bdf());
            Ok(())
        })();
        irq.disable();
        let before = EDU_INTERRUPTS.load(SeqCst);
        regs.write32(edu::IRQ_RAISE, 1 << 7);
        let quiet = !wait_until(|| EDU_INTERRUPTS.load(SeqCst) > before);
        regs.write32(edu::IRQ_ACK, regs.read32(edu::IRQ_STATUS));
        *EDU.lock() = None;
        result?;
        if !quiet || irq::free_vector(vector) {
            return Err("after disable the function still interrupts, or its vector is still allocated");
        }
        if irq::stats().unhandled != unhandled {
            return Err("an edu interrupt arrived on a vector without a handler");
        }
        Ok(())
    }
}

/// Wait up to 100 ms (with interrupts on) for something an interrupt does.
fn wait_until(mut done: impl FnMut() -> bool) -> bool {
    let start = crate::time::now_ns();
    while crate::time::now_ns() - start < 100 * crate::time::NANOS_PER_MILLI {
        if done() {
            return true;
        }
        core::hint::spin_loop();
    }
    done()
}
