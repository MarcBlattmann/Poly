//! virtio-blk over PCI, polled: the kernel's side of the `poly-virtio-blk`
//! driver.
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
//! - wraps the driver in a [`Watchdog`], so a request that never comes back
//!   resets the device and fails instead of hanging the kernel.
//!
//! The block layer (`block.rs`) puts a request queue and a buffer cache in
//! front of it. Everything here lives as long as the kernel: there is no
//! hot-unplug, so the DMA memory is never freed once the device has it.
//!
//! # Interrupts (not yet)
//!
//! The device is driven with interrupts off: INTx is disabled in the
//! command register, MSI-X stays disabled, the queue asks the device not to
//! interrupt, and no MSI-X vector is assigned (`vector: None`). The boot log
//! shows the MSI-X capability that is there. Switching to interrupts, once
//! the kernel has an APIC driver and an IDT vector to spare:
//!
//! 1. Map the MSI-X table BAR ([`pci::Device::map_bar`]), set the function
//!    mask and the enable bit (`MsiX::set_function_mask`,
//!    `MsiX::set_enabled`), write table entry 0 with the local APIC message
//!    for the vector (`MsiX::write_entry`), clear the function mask.
//! 2. Pass `vector: Some(0)` in the [`Setup`] in [`bring_up`]; `init` fails
//!    with `VectorRejected` if the device cannot map it.
//! 3. Call `enable_interrupts` on the driver (through
//!    `Watchdog::device_mut`) instead of `disable_interrupts`. The interrupt
//!    handler runs `RequestQueue::process` on the device's queue (`block.rs`)
//!    and wakes the threads whose tickets `RequestQueue::finished` returns;
//!    waiters sleep on their ticket instead of spinning in
//!    `RequestQueue::wait`.
//! 4. Turn the watchdog's poll budget into a time limit checked from the
//!    timer, since nothing polls in a loop any more.

use crate::dma::{DmaBuffer, KernelDma};
use crate::pci::{self, Device, MapError, MmioRegion, Size};
use crate::{block, kernel_test, kprintln};
use alloc::boxed::Box;
use alloc::vec;
use poly_block::{BlockDevice, Watchdog};
use poly_pci::{Bdf, ConfigSpace, command};
use poly_virtio::{Buffer, CommonCfg, DescState, InitError, Layout, PciTransport, Region, TransportError, device_id};
use poly_virtio::{device_id_of, feature};
use poly_virtio_blk::{DmaRegion, ReqSlot, SECTOR_SIZE, Setup, Storage, request_area_len};

/// The driver as the kernel instantiates it.
pub type Driver = poly_virtio_blk::VirtioBlk<'static, MmioRegion, MmioRegion, KernelDma>;

/// What the block layer gets: the driver behind a watchdog.
pub type Disk = Watchdog<Driver>;

/// Queue entries the driver gets memory for; it uses fewer if the device
/// offers fewer. Without indirect descriptors a request takes two entries
/// plus one per segment, so this bounds the requests in flight too.
const QUEUE_ENTRIES: u16 = 128;
/// Status reads after a reset before giving up on the device.
const RESET_POLLS: u32 = 1_000_000;
/// Polls of the used ring in a row that may bring nothing back, while
/// requests are out, before the watchdog resets the device and fails them.
/// Requests to QEMU's device come back after a few hundred to a few
/// thousand polls. One turn of the request queue's wait loop costs about
/// 1.5 µs under QEMU's TCG (debug kernel; measured with a device that never
/// answers), so a lost request fails after about 30 seconds there, and
/// after a second or two with KVM. Generous on purpose: a false alarm
/// breaks the disk for good. Becomes a time limit once there is a timer.
const POLL_BUDGET: u64 = 20_000_000;

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

/// Bring the device up and hand it to `poly-virtio-blk`.
fn bring_up(dev: &Device) -> Result<Driver, Error> {
    let cfg = dev.config();
    let bdf = dev.bdf();
    let t = PciTransport::discover(cfg, &dev.header).map_err(Error::Transport)?;
    t.check_bars(&dev.bars).map_err(Error::Transport)?;
    let device_region = t.device.ok_or(Error::NoDeviceConfig)?;
    let device_cfg = map_region(dev, device_region)?;
    let common = map_region(dev, t.common)?;
    let notify = map_region(dev, t.notify)?;

    // Registers answer once memory decoding is on. INTx stays off, since
    // the kernel polls. Bus mastering waits until a reset has made the
    // device forget whatever queues firmware set up (`init` resets it
    // again, which does no harm).
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
        // Interrupt integration point: `Some(0)` once MSI-X is set up.
        vector: None,
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
            // A hint only (§2.7.7); the function cannot interrupt anyway.
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

/// Find the first virtio-blk function [`pci::init`] found, bring it up and
/// return it with its address. Call once, after `pci::init`.
pub fn probe() -> Option<(Bdf, Disk)> {
    let mut found = pci::devices().iter().filter(|d| is_virtio_blk(d));
    let Some(dev) = found.next() else {
        kprintln!("virtio-blk: no device");
        return None;
    };
    for other in found {
        kprintln!("virtio-blk: {}: ignored, only one device is driven for now", other.bdf());
    }
    let bdf = dev.bdf();
    match dev.msix() {
        Some(Ok(m)) => kprintln!(
            "virtio-blk: {bdf}: MSI-X with {} vectors, table BAR{}+{:#x}, PBA BAR{}+{:#x}; left disabled, polling",
            m.table_size,
            m.table_bar,
            m.table_offset,
            m.pba_bar,
            m.pba_offset
        ),
        Some(Err(e)) => kprintln!("virtio-blk: {bdf}: MSI-X capability unusable ({e:?}); polling"),
        None => kprintln!("virtio-blk: {bdf}: no MSI-X; polling"),
    }
    let driver = match bring_up(dev) {
        Ok(d) => d,
        Err(e) => {
            kprintln!("virtio-blk: {bdf}: bring-up failed: {e:?}");
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
    Some((bdf, Watchdog::new(driver, POLL_BUDGET)))
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
        match dev.msix() {
            Some(Ok(m)) => {
                // Polling only: MSI-X must still be off (message control bit 15).
                if cfg.read16(dev.bdf(), m.offset as u16 + 2) & (1 << 15) != 0 {
                    return Err("MSI-X is enabled although the driver polls");
                }
            }
            Some(Err(_)) => return Err("MSI-X table or PBA outside the function's memory BARs"),
            None => {}
        }
        let cmd = poly_pci::read_command(cfg, dev.bdf());
        let want = command::MEMORY_SPACE | command::BUS_MASTER | command::INTX_DISABLE;
        let disk = block::root().ok_or("virtio-blk device found but not brought up")?;
        if cmd & want != want {
            return Err("virtio-blk device brought up without memory decoding and bus mastering, or with INTx");
        }
        let cached = disk.lock();
        let block::Device::Virtio(w) = cached.device() else {
            return Err("the root disk is not the virtio-blk device");
        };
        if w.device().features() & feature::VERSION_1 == 0 || w.device().queue_size() < 4 {
            return Err("the driver runs without VERSION_1 or with a tiny queue");
        }
        // Runs after the block tests (names sort): none of their requests
        // may have been lost.
        if w.aborts() != 0 || w.device().is_failed() || w.device().in_flight() != 0 {
            return Err("the virtio-blk device lost a request or broke its protocol");
        }
        Ok(())
    }
}
