//! virtio-blk over PCI, interrupt-driven through MSI-X (polled without it):
//! the kernel's side of the `poly-virtio-blk` driver.
//!
//! The driver itself (feature negotiation, the request format, the request
//! queue, status bytes, recovery from a misbehaving device) is the
//! host-tested `poly_virtio_blk` crate (ADR 0014). This module gives it what
//! only the kernel has, following VIRTIO 1.2 §3.1.1 up to the point where
//! [`poly_virtio_blk::init`] takes over:
//!
//! - finds the function (vendor 0x1af4, virtio device type 2; QEMU's
//!   modern-only `virtio-blk-pci,disable-legacy=on` is PCI device 0x1042) in
//!   the list [`pci::init`] built, locates its structures through the vendor
//!   capabilities ([`PciTransport::discover`]) and maps them out of its BARs;
//! - turns on memory decoding, resets the device and only then bus
//!   mastering, so it forgets whatever queues firmware may have set up
//!   before it can reach memory;
//! - gives it DMA memory for the virtqueue and for request headers and
//!   status bytes ([`DmaBuffer`]), bookkeeping tables from the heap, and the
//!   kernel's address translation ([`KernelDma`]): data moves straight
//!   between the device and the caller's buffers, which must therefore be
//!   in the direct map (heap memory is; see `dma.rs`);
//! - wraps the driver in a [`Watchdog`] with a time limit on the monotonic
//!   clock, so a request that never comes back resets the device and fails
//!   instead of hanging its waiter.
//!
//! The block layer (`block.rs`) puts a request queue and a buffer cache in
//! front of it. Everything here lives as long as the kernel: there is no
//! hot-unplug, so the DMA memory is never freed once the device has it.
//!
//! # Interrupts (VIRTIO 1.2 §4.1.5.1.2, MSI-X vector configuration; PCI 3.0 §6.8.2)
//!
//! INTx is never used. When the function has MSI-X:
//!
//! 1. [`bring_up_device`] turns MSI-X on with the function mask set and
//!    every entry masked ([`pci::Device::enable_msix`]), and has `init` map
//!    the request queue to MSI-X entry 0 (`vector: Some(0)`). A device that
//!    refuses the mapping is brought up again without it, polled.
//! 2. Once the block layer has made the disk ([`start`], or
//!    [`connect_interrupt`] for a disk of one's own), entry 0 gets a kernel
//!    vector whose handler is the disk's ([`block::Disk::interrupt_handler`]:
//!    it runs the request queue and wakes the waiters) and the local APIC
//!    message for it, the driver asks the device for interrupts again
//!    (`enable_interrupts`), the function mask is cleared, and the disk's
//!    waiters switch from polling to sleeping.
//!
//! Without MSI-X (or if any of that fails) the disk stays polled, and the
//! queue asks the device not to interrupt. The boot log says which.

use crate::block::{self, Waiting};
use crate::dma::{DmaBuffer, KernelDma};
use crate::ktest::TestResult;
use crate::pci::{self, Device, IrqError, MapError, MmioRegion, MsixVectors, Size};
use crate::sync::SpinLock;
use crate::{kernel_test, kprintln, thread, time};
use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;
use poly_block::{BlockDevice, Watchdog};
use poly_pci::{Bdf, command};
use poly_virtio::{Buffer, CommonCfg, DescState, InitError, Layout, PciTransport, Region, TransportError, device_id};
use poly_virtio::{device_id_of, feature};
use poly_virtio_blk::{DmaRegion, ReqSlot, SECTOR_SIZE, Setup, Storage, request_area_len};

/// The driver as the kernel instantiates it.
pub type Driver = poly_virtio_blk::VirtioBlk<'static, MmioRegion, MmioRegion, KernelDma>;

/// The clock the watchdog measures silence with: [`time::now_ns`].
pub type Clock = fn() -> u64;

/// What the block layer gets: the driver behind a watchdog.
pub type Disk = Watchdog<Driver, Clock>;

/// Queue entries the driver gets memory for; it uses fewer if the device
/// offers fewer. Without indirect descriptors a request takes two entries
/// plus one per segment, so this bounds the requests in flight too.
const QUEUE_ENTRIES: u16 = 128;
/// Status reads after a reset before giving up on the device.
const RESET_POLLS: u32 = 1_000_000;
/// How long requests may be out with none coming back before the watchdog
/// resets the device and fails them. QEMU's device answers within
/// milliseconds; on a host stalled for longer than this the disk breaks for
/// good, so the limit is generous.
pub const WATCHDOG_LIMIT_NS: u64 = 10 * time::NANOS_PER_SEC;
/// The MSI-X table entry the request queue uses.
const QUEUE_ENTRY: u16 = 0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Transport(TransportError),
    Bar(MapError),
    /// The function has no device-specific configuration structure.
    NoDeviceConfig,
    /// The device did not finish the reset before bus mastering went on.
    Reset(InitError),
    OutOfMemory,
    Init(poly_virtio_blk::InitError),
}

/// Map the part of a BAR a virtio structure occupies.
fn map_region(dev: &Device, r: Region) -> Result<MmioRegion, Error> {
    let bar = dev.map_bar(r.bar as usize).map_err(Error::Bar)?;
    bar.subregion(r.offset as usize, r.length as usize).ok_or(Error::Transport(TransportError::OutsideBar))
}

fn region(b: &DmaBuffer) -> DmaRegion {
    DmaRegion { ptr: b.ptr(), bus: b.phys(), len: b.len() }
}

/// A table that lives as long as the kernel (the driver keeps it).
fn table<T: Clone>(entry: T) -> &'static mut [T] {
    Box::leak(vec![entry; QUEUE_ENTRIES as usize].into_boxed_slice())
}

/// Bring the device up and hand it to `poly-virtio-blk`, with its request
/// queue on MSI-X entry `vector` if given (MSI-X must be on for that).
fn bring_up(dev: &Device, vector: Option<u16>) -> Result<Driver, Error> {
    let cfg = dev.config();
    let bdf = dev.bdf();
    let t = PciTransport::discover(cfg, &dev.header).map_err(Error::Transport)?;
    t.check_bars(&dev.bars).map_err(Error::Transport)?;
    let device_region = t.device.ok_or(Error::NoDeviceConfig)?;
    let device_cfg = map_region(dev, device_region)?;
    let common = map_region(dev, t.common)?;
    let notify = map_region(dev, t.notify)?;

    // Registers answer once memory decoding is on. INTx stays off: the
    // device interrupts through MSI-X or not at all. Bus mastering waits
    // until a reset has made the device forget whatever queues firmware set
    // up (`init` resets it again, which does no harm).
    poly_pci::enable(cfg, bdf, command::MEMORY_SPACE | command::INTX_DISABLE);
    CommonCfg::new(common).reset(RESET_POLLS).map_err(Error::Reset)?;
    poly_pci::enable(cfg, bdf, command::BUS_MASTER);

    let layout = Layout::new(QUEUE_ENTRIES).unwrap_or_else(|| unreachable!("a valid queue size"));
    let queue_mem = DmaBuffer::new(layout.total()).ok_or(Error::OutOfMemory)?;
    let area = DmaBuffer::new(request_area_len(QUEUE_ENTRIES)).ok_or(Error::OutOfMemory)?;
    // If bring-up fails these tables leak; it happens once, at boot.
    let storage = Storage {
        desc_state: table(DescState::default()),
        slots: table(ReqSlot::EMPTY),
        buffers: table(Buffer { addr: 0, len: 0 }),
    };
    let setup = Setup {
        common: CommonCfg::new(common),
        device_cfg,
        device_cfg_len: device_cfg.len() as u32,
        transport: t,
        notify,
        dma: KernelDma,
        queue_mem: region(&queue_mem),
        area: region(&area),
        storage,
        vector,
    };
    // SAFETY: both DMA buffers are fresh, zeroed, direct-mapped blocks of
    // frames that the device reaches at their physical addresses (there is
    // no IOMMU); on success they are leaked below, so they stay valid and
    // unused by anything else for the kernel's lifetime ('static). The
    // regions all come from this one function's capabilities.
    match unsafe { poly_virtio_blk::init(setup) } {
        Ok(mut driver) => {
            core::mem::forget(queue_mem);
            core::mem::forget(area);
            // Until the interrupt is wired up (`connect_interrupt`): a hint
            // only (§2.7.7), and the function is masked anyway.
            driver.disable_interrupts();
            Ok(driver)
        }
        Err(e) => {
            // The device may know the queue's address by now: take the
            // memory back with a reset, or never reuse it.
            if CommonCfg::new(common).reset(RESET_POLLS).is_err() {
                core::mem::forget(queue_mem);
                core::mem::forget(area);
            }
            Err(Error::Init(e))
        }
    }
}

fn is_virtio_blk(d: &Device) -> bool {
    device_id_of(&d.header) == Some(device_id::BLOCK)
}

/// A virtio-blk function brought up by [`bring_up_device`].
#[derive(Debug)]
pub struct Found {
    pub bdf: Bdf,
    pub disk: Disk,
    /// MSI-X, on but masked, with the request queue mapped to entry 0;
    /// `None` if the disk must be polled.
    pub msix: Option<MsixVectors>,
}

/// Find the first virtio-blk function [`pci::init`] found and bring it up.
/// Call once, after `pci::init`; then [`start`] it once the block layer has
/// made it a disk.
pub fn probe() -> Option<Found> {
    let mut found = pci::devices().iter().filter(|d| is_virtio_blk(d));
    let Some(dev) = found.next() else {
        kprintln!("virtio-blk: no device");
        return None;
    };
    for other in found {
        kprintln!("virtio-blk: {}: ignored, only one device is driven for now", other.bdf());
    }
    bring_up_device(dev)
}

/// Bring up virtio-blk function `dev`: MSI-X on (masked) if it has it, the
/// driver initialised with its request queue on MSI-X entry 0 (or without
/// a vector, polled), behind a watchdog of [`WATCHDOG_LIMIT_NS`]. Logs what
/// it found; `None` if the device cannot be used.
pub fn bring_up_device(dev: &Device) -> Option<Found> {
    let bdf = dev.bdf();
    let mut msix = match dev.msix() {
        Some(Ok(m)) => {
            kprintln!(
                "virtio-blk: {bdf}: MSI-X with {} vectors, table BAR{}+{:#x}, PBA BAR{}+{:#x}",
                m.table_size,
                m.table_bar,
                m.table_offset,
                m.pba_bar,
                m.pba_offset
            );
            match dev.enable_msix() {
                Ok(v) => Some(v),
                Err(e) => {
                    kprintln!("virtio-blk: {bdf}: cannot enable MSI-X ({e:?}); polling");
                    None
                }
            }
        }
        Some(Err(e)) => {
            kprintln!("virtio-blk: {bdf}: MSI-X capability unusable ({e:?}); polling");
            None
        }
        None => {
            kprintln!("virtio-blk: {bdf}: no MSI-X; polling");
            None
        }
    };
    let mut result = bring_up(dev, msix.as_ref().map(|_| QUEUE_ENTRY));
    if let Err(e) = &result
        && let Some(m) = msix.take()
    {
        kprintln!("virtio-blk: {bdf}: bring-up with MSI-X failed ({e:?}); trying again, polled");
        m.disable();
        result = bring_up(dev, None);
    }
    let driver = match result {
        Ok(d) => d,
        Err(e) => {
            kprintln!("virtio-blk: {bdf}: bring-up failed: {e:?}");
            if let Some(m) = msix {
                m.disable();
            }
            return None;
        }
    };
    let info = driver.info();
    let sectors = info.blocks * (info.block_size / SECTOR_SIZE) as u64;
    kprintln!(
        "virtio-blk: {bdf}: {} ({sectors} sectors), block size {}, features {:#x}, queue {} entries, {} segments per request{}{}",
        Size(info.bytes()),
        info.block_size,
        driver.features(),
        driver.queue_size(),
        info.max_segments,
        if info.write_cache { ", write cache" } else { "" },
        if info.read_only { ", read-only" } else { "" },
    );
    let clock: Clock = time::now_ns;
    Some(Found { bdf, disk: Watchdog::new(driver, clock, WATCHDOG_LIMIT_NS), msix })
}

/// Point the request queue's MSI-X entry at `disk`'s interrupt handler,
/// let the device interrupt, and make the disk's waiters sleep. `disk`
/// must hold the virtio-blk device `msix` belongs to. Returns the kernel
/// vector. On failure the disk stays polled.
pub fn connect_interrupt(disk: &'static block::Disk, msix: &mut MsixVectors) -> Result<u8, IrqError> {
    let (handler, data) = disk.interrupt_handler();
    let vector = msix.attach(QUEUE_ENTRY, handler, data)?;
    // Completions that arrive before the function mask is cleared wait in
    // the PBA; `use_interrupt` collects whatever finished before that.
    disk.with_device(|d| {
        if let block::Device::Virtio(w) = d {
            w.device_mut().enable_interrupts();
        }
    });
    msix.set_function_masked(false);
    disk.use_interrupt(vector);
    Ok(vector)
}

/// MSI-X state of the disks [`start`] made interrupt-driven, kept for the
/// kernel's lifetime.
static CONNECTED: SpinLock<Vec<(Bdf, MsixVectors)>> = SpinLock::new(Vec::new());

/// Start `disk` (the device [`probe`] found at `bdf`): interrupt-driven if
/// MSI-X is set up, else polled. Logs which.
pub fn start(disk: &'static block::Disk, bdf: Bdf, msix: Option<MsixVectors>) {
    let Some(mut msix) = msix else {
        kprintln!("virtio-blk: {bdf}: polled: waiters run the queue and yield");
        return;
    };
    match connect_interrupt(disk, &mut msix) {
        Ok(vector) => {
            kprintln!(
                "virtio-blk: {bdf}: request queue on MSI-X entry {QUEUE_ENTRY} -> vector {vector:#x}; interrupt-driven, waiters sleep"
            );
            CONNECTED.lock().push((bdf, msix));
        }
        Err(e) => {
            kprintln!("virtio-blk: {bdf}: cannot connect the MSI-X interrupt ({e:?}); polled");
            msix.disable();
        }
    }
}

/// The kernel vector of the interrupt-driven device at `bdf`, if [`start`]
/// connected one.
pub fn connected_vector(bdf: Bdf) -> Option<u8> {
    CONNECTED.lock().iter().find(|(b, _)| *b == bdf).and_then(|(_, m)| m.vector(QUEUE_ENTRY))
}

kernel_test! {
    fn device_found_with_sane_bars(env: &TestEnv) -> TestResult {
        let expected = env.boot.has_flag(block::TEST_DISK_FLAG);
        let Some(dev) = pci::devices().iter().find(|d| is_virtio_blk(d)) else {
            return if expected { Err("poly.test.disk is set but there is no virtio-blk device") } else { Ok(()) };
        };
        if expected && dev.header.device_id != 0x1042 {
            return Err("the test disk is not a modern-only virtio-blk device (PCI device 0x1042)");
        }
        let cfg = dev.config();
        let t = PciTransport::discover(cfg, &dev.header).map_err(|_| "virtio capabilities missing")?;
        t.check_bars(&dev.bars).map_err(|_| "a virtio structure lies outside its BAR")?;
        for r in [Some(t.common), Some(t.notify), Some(t.isr), t.device].into_iter().flatten() {
            match dev.bars[r.bar as usize] {
                Some(bar) if bar.is_memory() && bar.address != 0 => {}
                _ => return Err("a virtio structure is not in an assigned memory BAR"),
            }
        }
        let cmd = poly_pci::read_command(cfg, dev.bdf());
        let want = command::MEMORY_SPACE | command::BUS_MASTER | command::INTX_DISABLE;
        let disk = block::root().ok_or("virtio-blk device found but not brought up")?;
        if cmd & want != want {
            return Err("virtio-blk device brought up without memory decoding and bus mastering, or with INTx");
        }
        match dev.msix() {
            Some(Ok(m)) => {
                // Interrupt-driven: MSI-X on and not masked, the queue's
                // entry carrying the message for the disk's vector.
                let Waiting::Interrupt(vector) = disk.waiting() else {
                    return Err("the device has MSI-X but the disk is polled");
                };
                if !m.is_enabled(cfg, dev.bdf()) || m.is_function_masked(cfg, dev.bdf()) {
                    return Err("MSI-X is off or masked although the disk is interrupt-driven");
                }
                if connected_vector(dev.bdf()) != Some(vector) {
                    return Err("the disk's vector is not the one its MSI-X entry was given");
                }
                let bar = dev.map_bar(m.table_bar as usize).map_err(|_| "cannot map the MSI-X table")?;
                let table = bar.subregion(m.table_offset as usize, m.table_len() as usize).ok_or("no MSI-X table")?;
                let (msg, masked) = m.read_entry(&table, QUEUE_ENTRY).map_err(|_| "no MSI-X entry 0")?;
                if masked || Some(msg) != pci::msi_message(vector) {
                    return Err("MSI-X entry 0 is masked or does not target the disk's vector");
                }
                if pci::from_pci_message(msg).vector() != vector {
                    return Err("the MSI-X message does not decode to the disk's vector");
                }
            }
            Some(Err(_)) => return Err("MSI-X table or PBA outside the function's memory BARs"),
            None => {
                if disk.waiting() != Waiting::Polled {
                    return Err("a device without MSI-X is not polled");
                }
            }
        }
        disk.with_device(|d| {
            let block::Device::Virtio(w) = d else {
                return Err("the root disk is not the virtio-blk device");
            };
            if w.device().features() & feature::VERSION_1 == 0 || w.device().queue_size() < 4 {
                return Err("the driver runs without VERSION_1 or with a tiny queue");
            }
            if w.limit() != WATCHDOG_LIMIT_NS {
                return Err("the watchdog does not have its time limit");
            }
            // Runs after the block tests (names sort): none of their
            // requests may have been lost.
            if w.aborts() != 0 || w.device().is_failed() || w.device().in_flight() != 0 {
                return Err("the virtio-blk device lost a request or broke its protocol");
            }
            Ok(())
        })
    }
}

kernel_test! {
    fn completions_arrive_as_interrupts(env: &TestEnv) -> TestResult {
        if !env.boot.has_flag(block::TEST_DISK_FLAG) {
            return Ok(());
        }
        let disk = block::root().ok_or("no virtio-blk disk")?;
        let Waiting::Interrupt(vector) = disk.waiting() else {
            // The polled boot (a device without MSI-X): nothing to count.
            return if pci::devices().iter().find(|d| is_virtio_blk(d)).is_some_and(|d| d.msix().is_none()) {
                Ok(())
            } else {
                Err("the test disk has MSI-X but is not interrupt-driven")
            };
        };
        let before = disk.stats();
        let unhandled = crate::arch::irq::stats().unhandled;
        let mut buf = vec![0u8; 8 * SECTOR_SIZE as usize];
        for first in [0u64, 100, 2040] {
            disk.read_uncached(first, &mut buf).map_err(|_| "an uncached read failed")?;
            if buf[..4] != *b"POLY" || buf[4..8] != (first as u32).to_le_bytes() {
                return Err("an uncached read returned the wrong sector");
            }
        }
        let after = disk.stats();
        let (interrupts, sleeps) = (after.interrupts - before.interrupts, after.sleeps - before.sleeps);
        kprintln!("virtio-blk: 3 reads: {interrupts} interrupts on vector {vector:#x}, {sleeps} sleeps");
        if interrupts == 0 {
            return Err("reads finished without an MSI-X interrupt");
        }
        if sleeps == 0 {
            return Err("an interrupt-driven waiter never slept");
        }
        if crate::arch::irq::stats().unhandled != unhandled {
            return Err("an interrupt arrived on a vector without a handler");
        }
        Ok(())
    }
}

/// How long the lost-request test lets the watchdog wait.
const LOST_LIMIT_NS: u64 = 200 * time::NANOS_PER_MILLI;

/// Thread body: read block 1 from the disk at address `disk` past the
/// cache. Exit code 0: success, 1: `DeviceFailed`, 2: another error.
fn read_block_one(disk: usize) -> usize {
    // SAFETY: the test passes the address of a `&'static block::Disk`.
    let disk = unsafe { &*(disk as *const block::Disk) };
    let mut buf = vec![0u8; SECTOR_SIZE as usize];
    match disk.read_uncached(1, &mut buf) {
        Ok(()) => 0,
        Err(poly_block::IoError::DeviceFailed) => 1,
        Err(_) => 2,
    }
}

kernel_test! {
    /// The second virtio-blk function the boot test attaches (a spare QEMU
    /// null device nobody else uses) gets a disk of its own, brought up
    /// like the root disk: interrupt-driven if it has MSI-X, else polled.
    /// With MSI-X, a masked entry must hold its interrupt back until it is
    /// unmasked. Then a request's notification is dropped on purpose, so
    /// the device never sees it and never interrupts: the waiter must wait
    /// (asleep, when interrupt-driven) until the watchdog's deadline, which
    /// then resets the device and fails the request. Afterwards MSI-X is
    /// torn down again.
    fn a_lost_request_fails_when_the_watchdog_runs_out(env: &TestEnv) -> TestResult {
        if !env.boot.has_flag(block::TEST_DISK_FLAG) {
            return Ok(());
        }
        let dev = pci::devices()
            .iter()
            .filter(|d| is_virtio_blk(d))
            .nth(1)
            .ok_or("poly.test.disk is set but there is no spare virtio-blk device")?;
        let found = bring_up_device(dev).ok_or("the spare virtio-blk device did not come up")?;
        let what = alloc::format!("virtio-blk {}", found.bdf);
        let disk = block::Disk::new("spare", what, block::Device::Virtio(Box::new(found.disk)))
            .map_err(|_| "no queue or cache for the spare device")?;
        let mut msix = found.msix;
        if msix.is_none() != dev.msix().is_none() {
            return Err("the spare device's MSI-X was not set up although it has it");
        }
        let vector = match &mut msix {
            Some(m) => Some(connect_interrupt(disk, m).map_err(|_| "cannot connect the spare device's interrupt")?),
            None => None,
        };
        let result = (|| {
            let mut buf = vec![0u8; SECTOR_SIZE as usize];
            disk.read_uncached(0, &mut buf).map_err(|_| "a read from the spare device failed")?;
            if let (Some(m), Some(v)) = (&msix, vector) {
                masked_entry_holds_back(disk, m, v)?;
            }
            lost_request(disk, vector.is_some())
        })();
        // Tear down: the disk polls again, MSI-X goes off, the vector is
        // free for others.
        disk.use_polling();
        if let (Some(m), Some(vector)) = (msix, vector) {
            m.disable();
            let cfg = dev.config();
            let off = dev.msix().is_some_and(|m| m.is_ok_and(|m| !m.is_enabled(cfg, dev.bdf())));
            if !off || crate::arch::irq::free_vector(vector) {
                return result.and(Err("disabling MSI-X left it on or did not free the vector"));
            }
        }
        result
    }
}

/// A masked MSI-X entry holds its interrupt back (pending in the PBA) and
/// sends it once unmasked. (The reader may or may not sleep meanwhile:
/// QEMU's null device can finish a request while it is being notified, and
/// the reader then finds its result when it runs the queue.)
fn masked_entry_holds_back(disk: &'static block::Disk, msix: &MsixVectors, vector: u8) -> TestResult {
    if disk.stats().interrupts == 0 || msix.is_function_masked() {
        return Err("the spare device's read finished without an interrupt");
    }
    msix.set_masked(QUEUE_ENTRY, true).map_err(|_| "cannot mask MSI-X entry 0")?;
    match msix.entry(QUEUE_ENTRY) {
        Ok((msg, true)) if Some(msg) == pci::msi_message(vector) => {}
        _ => return Err("masking MSI-X entry 0 changed its message or did not mask it"),
    }
    let interrupts = disk.stats().interrupts;
    let reader = thread::spawn(read_block_one, disk as *const block::Disk as usize).map_err(|_| "spawn failed")?;
    let start = time::now_ns();
    let mut pending = false;
    while !pending && time::now_ns() - start < time::NANOS_PER_SEC {
        thread::sleep(core::time::Duration::from_millis(1));
        pending = msix.is_pending(QUEUE_ENTRY) == Ok(true);
    }
    let held = disk.stats().interrupts == interrupts;
    msix.set_masked(QUEUE_ENTRY, false).map_err(|_| "cannot unmask MSI-X entry 0")?;
    let code = thread::join(reader).map_err(|_| "join failed")?;
    let start = time::now_ns();
    while disk.stats().interrupts == interrupts && time::now_ns() - start < time::NANOS_PER_SEC {
        thread::sleep(core::time::Duration::from_millis(1));
    }
    if !pending || !held || code != 0 || disk.stats().interrupts == interrupts {
        return Err("a masked MSI-X entry did not hold its interrupt back until it was unmasked");
    }
    Ok(())
}

/// Drop the next notification: the request never reaches the device, and
/// only the watchdog gets it back, at its deadline.
fn lost_request(disk: &'static block::Disk, interrupt_driven: bool) -> TestResult {
    disk.with_device(|d| {
        if let block::Device::Virtio(w) = d {
            w.set_limit(LOST_LIMIT_NS);
            w.device_mut().set_drop_notifications(true);
        }
    });
    let sleeps = disk.stats().sleeps;
    let start = time::now_ns();
    let reader = thread::spawn(read_block_one, disk as *const block::Disk as usize).map_err(|_| "spawn failed")?;
    // An interrupt-driven reader sleeps meanwhile; it does not spin.
    let mut blocked = false;
    while !blocked && time::now_ns() - start < LOST_LIMIT_NS / 2 {
        thread::sleep(core::time::Duration::from_millis(1));
        blocked = thread::state(reader) == Some(thread::State::Blocked);
    }
    let code = thread::join(reader).map_err(|_| "join failed")?;
    let took = time::now_ns() - start;
    let how = if interrupt_driven { "interrupt-driven, asleep" } else { "polled" };
    kprintln!("virtio-blk: lost request ({how}) failed after {} ms", took / time::NANOS_PER_MILLI);
    if code != 1 {
        return Err("a request the device never saw did not fail with DeviceFailed");
    }
    if interrupt_driven && (!blocked || disk.stats().sleeps == sleeps) {
        return Err("the thread waiting for the lost request did not sleep");
    }
    if !(LOST_LIMIT_NS..=LOST_LIMIT_NS + 2 * time::NANOS_PER_SEC).contains(&took) {
        return Err("the lost request did not fail at the watchdog's deadline");
    }
    disk.with_device(|d| match d {
        block::Device::Virtio(w) if w.aborts() == 1 && w.device().is_failed() => Ok(()),
        _ => Err("the watchdog did not abort the device exactly once"),
    })?;
    let mut buf = vec![0u8; SECTOR_SIZE as usize];
    if disk.read_uncached(0, &mut buf) != Err(poly_block::IoError::DeviceFailed) {
        return Err("the aborted device took another request");
    }
    Ok(())
}
