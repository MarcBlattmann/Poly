//! virtio-sound over PCI: the kernel's side of the `poly-virtio-sound`
//! driver, as `net/virtio.rs` is for virtio-net (VIRTIO 1.2 §3.1.1 up to
//! where [`poly_virtio_sound::init`] takes over):
//!
//! - finds the function (vendor 0x1af4, virtio device type 25; QEMU's
//!   `virtio-sound-pci` is PCI device 0x1059), locates its structures and
//!   maps them;
//! - turns on memory decoding, resets the device and only then bus
//!   mastering;
//! - gives it DMA memory ([`DmaBuffer`]): the four queue rings, a page for
//!   control requests, the event buffers, the per-message headers and
//!   status, and the PCM ring buffer ([`PCM_BYTES`]) that output messages
//!   point into; bookkeeping tables come from the heap. Everything lives
//!   as long as the kernel.
//!
//! With MSI-X the function is enabled masked before `init`, which maps the
//! event and transmit queues and configuration changes to table entry 0;
//! [`connect_interrupt`] later points it at the audio thread's wakeup. A
//! device that refuses the vector is brought up again without it and is
//! then polled.

use crate::dma::DmaBuffer;
use crate::kprintln;
use crate::pci::{self, Device, IrqError, MapError, MmioRegion, MsixVectors};
use alloc::boxed::Box;
use alloc::vec;
use poly_pci::{Bdf, command};
use poly_virtio::{CommonCfg, DescState, InitError, Layout, PciTransport, Region, TransportError, device_id};
use poly_virtio_sound::{DmaRegion, Memory, Setup, Storage, TX_SLOT_LEN, proto::Event};

/// The driver as the kernel instantiates it.
pub type Driver = poly_virtio_sound::VirtioSound<'static, MmioRegion, MmioRegion>;

/// Entries per queue the driver gets memory for (fewer if the device
/// offers fewer): 21 output messages of three descriptors each.
const QUEUE_ENTRIES: u16 = 64;
/// The PCM ring buffer: the largest buffer a stream may use. 256 KiB holds
/// more than a second of 48 kHz stereo S16.
pub const PCM_BYTES: usize = 256 * 1024;
/// Status reads after a reset before giving up on the device.
const RESET_POLLS: u32 = 1_000_000;
/// The MSI-X table entry the queues and configuration changes use.
pub const MSIX_ENTRY: u16 = 0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Transport(TransportError),
    Bar(MapError),
    NoDeviceConfig,
    Reset(InitError),
    OutOfMemory,
    Init(poly_virtio_sound::InitError),
}

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

/// Bring the device up and hand it to `poly-virtio-sound`, with its queues
/// on MSI-X entry `vector` if given.
fn bring_up(dev: &Device, vector: Option<u16>) -> Result<Driver, Error> {
    let cfg = dev.config();
    let bdf = dev.bdf();
    let t = PciTransport::discover(cfg, &dev.header).map_err(Error::Transport)?;
    t.check_bars(&dev.bars).map_err(Error::Transport)?;
    let device_cfg = map_region(dev, t.device.ok_or(Error::NoDeviceConfig)?)?;
    let common = map_region(dev, t.common)?;
    let notify = map_region(dev, t.notify)?;

    // Decoding on and INTx off first, bus mastering only after a reset made
    // the device forget anything firmware set up.
    poly_pci::enable(cfg, bdf, command::MEMORY_SPACE | command::INTX_DISABLE);
    CommonCfg::new(common).reset(RESET_POLLS).map_err(Error::Reset)?;
    poly_pci::enable(cfg, bdf, command::BUS_MASTER);

    let layout = Layout::new(QUEUE_ENTRIES).unwrap_or_else(|| unreachable!("a valid queue size"));
    let mut memory = alloc::vec::Vec::new();
    for len in [
        layout.total(),
        layout.total(),
        layout.total(),
        layout.total(),
        4096,
        QUEUE_ENTRIES as usize * Event::LEN,
        QUEUE_ENTRIES as usize * TX_SLOT_LEN,
        PCM_BYTES,
    ] {
        memory.push(DmaBuffer::new(len).ok_or(Error::OutOfMemory)?);
    }
    let regions: alloc::vec::Vec<DmaRegion> = memory.iter().map(region).collect();
    // If bring-up fails these tables leak; it happens once, at boot.
    let storage = Storage {
        desc: [
            table(DescState::default()),
            table(DescState::default()),
            table(DescState::default()),
            table(DescState::default()),
        ],
        event_owner: table(0u16),
        tx_owner: table(0u16),
        tx_free: table(0u16),
        tx_len: table(0u32),
    };
    let setup = Setup {
        common: CommonCfg::new(common),
        device_cfg,
        device_cfg_len: device_cfg.len() as u32,
        transport: t,
        notify,
        memory: Memory {
            rings: [regions[0], regions[1], regions[2], regions[3]],
            control: regions[4],
            events: regions[5],
            tx_meta: regions[6],
            pcm: regions[7],
        },
        storage,
        vector,
    };
    // SAFETY: the DMA buffers are fresh, zeroed, direct-mapped blocks of
    // frames that the device reaches at their physical addresses (no
    // IOMMU); on success they are leaked below, so they stay valid and used
    // by nothing else for the kernel's lifetime ('static). The register
    // regions all come from this one function's capabilities.
    match unsafe { poly_virtio_sound::init(setup) } {
        Ok(mut driver) => {
            memory.into_iter().for_each(core::mem::forget);
            driver.disable_interrupts();
            Ok(driver)
        }
        Err(e) => {
            // The device may know the queues' addresses by now: take the
            // memory back with a reset, or never reuse it.
            if CommonCfg::new(common).reset(RESET_POLLS).is_err() {
                memory.into_iter().for_each(core::mem::forget);
            }
            Err(Error::Init(e))
        }
    }
}

pub fn is_virtio_sound(d: &Device) -> bool {
    poly_virtio::device_id_of(&d.header) == Some(device_id::SOUND)
}

/// A virtio-sound function brought up by [`probe`].
#[derive(Debug)]
pub struct Found {
    pub bdf: Bdf,
    pub driver: Driver,
    /// MSI-X, on but masked, with everything on entry 0; `None` if the
    /// device must be polled.
    pub msix: Option<MsixVectors>,
}

/// Find the first virtio-sound function and bring it up. Logs what it
/// found; `None` if there is none or it cannot be used.
pub fn probe() -> Option<Found> {
    let mut found = pci::devices().iter().filter(|d| is_virtio_sound(d));
    let dev = found.next()?;
    for other in found {
        kprintln!("sound: {}: ignored, only one virtio-sound device is driven for now", other.bdf());
    }
    let bdf = dev.bdf();
    let mut msix = match dev.msix() {
        Some(Ok(_)) => match dev.enable_msix() {
            Ok(v) => Some(v),
            Err(e) => {
                kprintln!("sound: {bdf}: cannot enable MSI-X ({e:?}); polling");
                None
            }
        },
        Some(Err(e)) => {
            kprintln!("sound: {bdf}: MSI-X capability unusable ({e:?}); polling");
            None
        }
        None => None,
    };
    let mut result = bring_up(dev, msix.as_ref().map(|_| MSIX_ENTRY));
    if let Err(e) = &result
        && let Some(m) = msix.take()
    {
        kprintln!("sound: {bdf}: bring-up with MSI-X failed ({e:?}); trying again, polled");
        m.disable();
        result = bring_up(dev, None);
    }
    match result {
        Ok(driver) => Some(Found { bdf, driver, msix }),
        Err(e) => {
            kprintln!("sound: {bdf}: bring-up failed: {e:?}");
            if let Some(m) = msix {
                m.disable();
            }
            None
        }
    }
}

/// Point MSI-X entry 0 at `handler(data)` and let the function interrupt.
/// Returns the kernel vector.
pub fn connect_interrupt(
    msix: &mut MsixVectors,
    handler: crate::arch::irq::Handler,
    data: usize,
) -> Result<u8, IrqError> {
    let vector = msix.attach(MSIX_ENTRY, handler, data)?;
    msix.set_function_masked(false);
    Ok(vector)
}
