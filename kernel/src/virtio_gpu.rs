//! virtio-gpu over PCI: the kernel's side of the `poly-virtio-gpu` driver
//! ([ADR 0102](../../docs/adr/0102-virtio-gpu-2d-display.md)).
//!
//! The driver (feature negotiation, the control queue's commands and
//! responses, fences, display info and EDID, scanout planning) is the
//! host-tested `poly_virtio_gpu` crate. This module gives it what only the
//! kernel has, as `virtio_blk.rs` and `net/virtio.rs` do (VIRTIO 1.2
//! §3.1.1 up to where [`poly_virtio_gpu::init`] takes over, §5.7.5):
//!
//! - finds the function (vendor 0x1af4, virtio device type 16; QEMU's
//!   `virtio-gpu-pci` is PCI device 0x1050), locates its structures
//!   ([`PciTransport::discover`]) and maps them;
//! - turns on memory decoding with INTx off, resets the device, and only
//!   then turns on bus mastering;
//! - gives it DMA memory ([`DmaBuffer`]) for the control queue and the
//!   command buffers, and a monotonic clock for the command timeout.
//!
//! Synchronous commands are waited for by polling (the console draws with
//! interrupts off), and display-change events are read from the device
//! configuration when the display flushes ([`crate::display`]). With MSI-X
//! the control queue interrupts through entry 0 when the device gives
//! buffers back, which is how 3D fences complete (ADR 0119: [`crate::drm`]
//! connects the entry, [`connect_interrupt`]); without it they are polled.
//! The host-visible shared memory region (blobs are mapped into it) is
//! found here too ([`hostmem`]). The framebuffer memory itself belongs to
//! the display, which hands it to the device with
//! `RESOURCE_ATTACH_BACKING`. Everything here lives as long as the kernel.

use crate::dma::DmaBuffer;
use crate::pci::{self, Device, IrqError, MapError, MmioRegion, MsixVectors};
use crate::sync::SpinLock;
use crate::{kprintln, time};
use alloc::boxed::Box;
use alloc::vec;
use poly_pci::{Bdf, command};
use poly_virtio::{CommonCfg, DescState, InitError, Layout, PciTransport, Region, TransportError, device_id};
use poly_virtio_gpu::edid::Edid;
use poly_virtio_gpu::{COMMAND_MEMORY_LEN, Clock, DmaRegion, Setup};

/// The driver as the kernel instantiates it.
pub type Driver = poly_virtio_gpu::VirtioGpu<'static, MmioRegion, MmioRegion, MmioRegion>;

/// Entries of the control queue: room for every slot of the driver with a
/// request, a command stream in a piece or two, and the response (the
/// driver needs at least 4: a transfer and its flush).
const QUEUE_ENTRIES: u16 = 64;
/// The MSI-X table entry the control queue interrupts through.
const MSIX_ENTRY: u16 = 0;
/// Status reads after a reset before giving up on the device.
const RESET_POLLS: u32 = 1_000_000;
/// How long one command may take. A full-screen transfer under TCG takes
/// a few milliseconds; a device that takes seconds is not coming back.
const COMMAND_TIMEOUT_NS: u64 = 2 * time::NANOS_PER_SEC;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Transport(TransportError),
    Bar(MapError),
    /// The function has no device-specific configuration structure.
    NoDeviceConfig,
    /// The device did not finish the reset before bus mastering went on.
    Reset(InitError),
    OutOfMemory,
    Init(poly_virtio_gpu::InitError),
}

/// A virtio-gpu function brought up by [`probe`].
#[derive(Debug)]
pub struct Gpu {
    pub bdf: Bdf,
    pub driver: Driver,
}

/// The host-visible shared memory region: its bus address and length.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HostMem {
    pub phys: u64,
    pub len: u64,
}

/// The device's MSI-X (on, function masked until [`connect_interrupt`]),
/// if the control queue was set up to interrupt.
static MSIX: SpinLock<Option<MsixVectors>> = SpinLock::new(None);
/// The device's host-visible region, if it has one.
static HOSTMEM: SpinLock<Option<HostMem>> = SpinLock::new(None);

/// The host-visible shared memory region of the device [`probe`] brought up.
pub fn hostmem() -> Option<HostMem> {
    *HOSTMEM.lock()
}

/// Point the control queue's MSI-X entry at `handler(data)` and let the
/// function interrupt. Returns the kernel vector; an error if the queue
/// was set up polled.
pub fn connect_interrupt(handler: crate::arch::irq::Handler, data: usize) -> Result<u8, IrqError> {
    let mut guard = MSIX.lock();
    let msix = guard.as_mut().ok_or(IrqError::NoCapability)?;
    let vector = msix.attach(MSIX_ENTRY, handler, data)?;
    msix.set_function_masked(false);
    Ok(vector)
}

fn map_region(dev: &Device, r: Region) -> Result<MmioRegion, Error> {
    let bar = dev.map_bar(r.bar as usize).map_err(Error::Bar)?;
    bar.subregion(r.offset as usize, r.length as usize).ok_or(Error::Transport(TransportError::OutsideBar))
}

fn region(b: &DmaBuffer) -> DmaRegion {
    DmaRegion { ptr: b.ptr(), bus: b.phys(), len: b.len() }
}

pub fn is_virtio_gpu(d: &Device) -> bool {
    poly_virtio::device_id_of(&d.header) == Some(device_id::GPU)
}

/// Bring the device up and hand it to `poly-virtio-gpu`, its control queue
/// interrupting through MSI-X entry `vector` if given.
fn bring_up(dev: &Device, vector: Option<u16>) -> Result<Driver, Error> {
    let cfg = dev.config();
    let bdf = dev.bdf();
    let t = PciTransport::discover(cfg, &dev.header).map_err(Error::Transport)?;
    t.check_bars(&dev.bars).map_err(Error::Transport)?;
    let device_region = t.device.ok_or(Error::NoDeviceConfig)?;
    let device_cfg = map_region(dev, device_region)?;
    let common = map_region(dev, t.common)?;
    let notify = map_region(dev, t.notify)?;

    // As for virtio-blk: decoding on and INTx off first, bus mastering only
    // after a reset made the device forget any queues firmware set up.
    poly_pci::enable(cfg, bdf, command::MEMORY_SPACE | command::INTX_DISABLE);
    CommonCfg::new(common).reset(RESET_POLLS).map_err(Error::Reset)?;
    poly_pci::enable(cfg, bdf, command::BUS_MASTER);

    let layout = Layout::new(QUEUE_ENTRIES).unwrap_or_else(|| unreachable!("a valid queue size"));
    let ring = DmaBuffer::new(layout.total()).ok_or(Error::OutOfMemory)?;
    let commands = DmaBuffer::new(COMMAND_MEMORY_LEN).ok_or(Error::OutOfMemory)?;
    // If bring-up fails this table leaks; it happens once, at boot.
    let desc: &'static mut [DescState] =
        Box::leak(vec![DescState::default(); QUEUE_ENTRIES as usize].into_boxed_slice());
    let setup = Setup {
        common: CommonCfg::new(common),
        device_cfg,
        device_cfg_len: device_cfg.len() as u32,
        transport: t,
        notify,
        ring: region(&ring),
        commands: region(&commands),
        desc,
        clock: Clock { now_ns: time::now_ns, timeout_ns: COMMAND_TIMEOUT_NS },
        queue_vector: vector,
    };
    let memory = [ring, commands];
    // SAFETY: both DMA buffers are fresh, zeroed, direct-mapped blocks of
    // frames that the device reaches at their physical addresses (no
    // IOMMU); on success they are leaked below, so they stay valid and used
    // by nothing else for the kernel's lifetime ('static). The regions all
    // come from this one function's capabilities.
    match unsafe { poly_virtio_gpu::init(setup) } {
        Ok(driver) => {
            memory.into_iter().for_each(core::mem::forget);
            Ok(driver)
        }
        Err(e) => {
            // The device may know the queue's address by now: take the
            // memory back with a reset, or never reuse it.
            if CommonCfg::new(common).reset(RESET_POLLS).is_err() {
                memory.into_iter().for_each(core::mem::forget);
            }
            Err(Error::Init(e))
        }
    }
}

/// Find the first virtio-gpu function [`pci::init`] found and bring it up.
/// Logs what it found: the features, the scanouts and their preferred
/// modes, and what each enabled scanout's EDID says. `None` if there is
/// none or it cannot be used.
pub fn probe() -> Option<Gpu> {
    let mut found = pci::devices().iter().filter(|d| is_virtio_gpu(d));
    let dev = found.next()?;
    for other in found {
        kprintln!("virtio-gpu: {}: ignored, only one device is driven for now", other.bdf());
    }
    let bdf = dev.bdf();
    // MSI-X on but masked: the control queue gets entry 0, which DRM
    // connects once it has 3D work whose fences an interrupt completes.
    let mut msix = match dev.msix() {
        Some(Ok(_)) => dev.enable_msix().ok(),
        _ => None,
    };
    let mut result = bring_up(dev, msix.as_ref().map(|_| MSIX_ENTRY));
    if let Err(e) = &result
        && let Some(m) = msix.take()
    {
        kprintln!("virtio-gpu: {bdf}: bring-up with MSI-X failed ({e:?}); trying again, polled");
        m.disable();
        result = bring_up(dev, None);
    }
    let mut driver = match result {
        Ok(d) => d,
        Err(e) => {
            kprintln!("virtio-gpu: {bdf}: bring-up failed: {e:?}");
            if let Some(m) = msix {
                m.disable();
            }
            return None;
        }
    };
    let c = driver.config();
    kprintln!(
        "virtio-gpu: {bdf}: features {:#x}{}{}{}{}, {} scanout(s), {} capset(s), control queue {} entries, {}",
        driver.features(),
        if driver.has_edid() { " (EDID)" } else { "" },
        if driver.has_virgl() { " (3D)" } else { "" },
        if driver.has_blob() { " (blobs)" } else { "" },
        if driver.has_context_init() { " (context init)" } else { "" },
        c.num_scanouts,
        c.num_capsets,
        driver.queue_size(),
        if driver.interrupts() { "MSI-X" } else { "polled" },
    );
    *MSIX.lock() = msix;
    let shm =
        poly_virtio::find_shared_memory(dev.config(), &dev.header, poly_virtio_gpu::protocol::SHM_ID_HOST_VISIBLE);
    if let Some(region) = shm {
        match region.address(&dev.bars) {
            Some(phys) => {
                kprintln!("virtio-gpu: {bdf}: host-visible region {:#x}, {} MiB", phys, region.length >> 20);
                *HOSTMEM.lock() = Some(HostMem { phys, len: region.length });
            }
            None => kprintln!("virtio-gpu: {bdf}: host-visible region outside its BAR; blobs are not mappable"),
        }
    }
    match driver.display_info() {
        Ok(info) => {
            let mut any = false;
            for (i, s) in info.enabled(c.num_scanouts) {
                any = true;
                kprintln!("virtio-gpu: {bdf}: scanout {i}: enabled, preferred {}x{}", s.rect.width, s.rect.height);
                log_edid(&mut driver, bdf, i);
            }
            if !any {
                kprintln!("virtio-gpu: {bdf}: no scanout reports a display");
            }
        }
        Err(e) => {
            kprintln!("virtio-gpu: {bdf}: GET_DISPLAY_INFO failed: {e:?}");
            if driver.is_broken() {
                driver.reset();
                return None;
            }
        }
    }
    Some(Gpu { bdf, driver })
}

/// Log what scanout `i`'s EDID says, if the device has EDIDs.
fn log_edid(driver: &mut Driver, bdf: Bdf, i: u32) {
    if !driver.has_edid() {
        return;
    }
    match driver.edid(i).map(|blob| (blob.len, Edid::parse(blob.bytes()))) {
        Ok((_, Ok(e))) => kprintln!(
            "virtio-gpu: {bdf}: scanout {i}: EDID {}.{} from {} product {:#06x}{}{}",
            e.version.0,
            e.version.1,
            e.vendor(),
            e.product,
            e.name().map_or(alloc::string::String::new(), |n| alloc::format!(" \"{n}\"")),
            e.preferred.map_or(alloc::string::String::new(), |(w, h)| alloc::format!(", preferred timing {w}x{h}")),
        ),
        Ok((len, Err(e))) => kprintln!("virtio-gpu: {bdf}: scanout {i}: EDID of {len} bytes unusable: {e:?}"),
        Err(e) => kprintln!("virtio-gpu: {bdf}: scanout {i}: GET_EDID failed: {e:?}"),
    }
}
