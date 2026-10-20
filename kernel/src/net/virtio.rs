//! virtio-net over PCI: the kernel's side of the `poly-virtio-net` driver.
//!
//! The driver (feature negotiation, the `virtio_net_hdr`, the receive and
//! transmit queues, refilling, recovery from a misbehaving device) is the
//! host-tested `poly_virtio_net` crate. This module gives it what only the
//! kernel has, as `virtio_blk.rs` does for disks (VIRTIO 1.2 §3.1.1 up to
//! where [`poly_virtio_net::init`] takes over):
//!
//! - finds the function (vendor 0x1af4, virtio device type 1; QEMU's
//!   modern-only `virtio-net-pci,disable-legacy=on` is PCI device 0x1041),
//!   locates its structures ([`PciTransport::discover`]) and maps them;
//! - turns on memory decoding, resets the device and only then bus
//!   mastering;
//! - gives it DMA memory ([`DmaBuffer`]) for both virtqueues and their
//!   packet buffers, 2 KiB per queue entry, and bookkeeping tables from the
//!   heap. Frames are copied between these buffers and the stack's, so no
//!   other memory is ever handed to the device.
//!
//! # Interrupts (VIRTIO 1.2 §4.1.5.1.2; PCI 3.0 §6.8.2)
//!
//! With MSI-X, the function is enabled masked before `init`, which maps
//! the receive queue, the transmit queue and configuration changes all to
//! table entry 0. [`connect_interrupt`] later points entry 0 at the network
//! thread's wakeup and clears the function mask. A device that refuses the
//! vectors is brought up again without them, and is then polled by the
//! network thread. Everything here lives as long as the kernel.

use crate::dma::DmaBuffer;
use crate::kprintln;
use crate::pci::{self, Device, IrqError, MapError, MmioRegion, MsixVectors};
use alloc::boxed::Box;
use alloc::vec;
use poly_net::NetDevice;
use poly_pci::{Bdf, command};
use poly_virtio::{CommonCfg, DescState, InitError, Layout, PciTransport, Region, TransportError, device_id};
use poly_virtio_net::{BUF_LEN, DmaRegion, PacketMemory, Setup, Storage};

/// The driver as the kernel instantiates it.
pub type Driver = poly_virtio_net::VirtioNet<'static, MmioRegion, MmioRegion, MmioRegion>;

/// Entries per queue the driver gets memory for (fewer if the device
/// offers fewer): 128 receive buffers and 128 transmit buffers of 2 KiB.
const QUEUE_ENTRIES: u16 = 128;
/// Status reads after a reset before giving up on the device.
const RESET_POLLS: u32 = 1_000_000;
/// The MSI-X table entry both queues and configuration changes use.
pub const MSIX_ENTRY: u16 = 0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Transport(TransportError),
    Bar(MapError),
    /// The function has no device-specific configuration structure.
    NoDeviceConfig,
    /// The device did not finish the reset before bus mastering went on.
    Reset(InitError),
    OutOfMemory,
    Init(poly_virtio_net::InitError),
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

/// DMA memory for one queue: the ring and its packet buffers.
fn queue_memory() -> Option<(DmaBuffer, DmaBuffer)> {
    let layout = Layout::new(QUEUE_ENTRIES).unwrap_or_else(|| unreachable!("a valid queue size"));
    Some((DmaBuffer::new(layout.total())?, DmaBuffer::new(QUEUE_ENTRIES as usize * BUF_LEN)?))
}

/// Bring the device up and hand it to `poly-virtio-net`, with its queues
/// and configuration changes on MSI-X entry `vector` if given.
fn bring_up(dev: &Device, vector: Option<u16>, mac_seed: [u8; 6]) -> Result<Driver, Error> {
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

    let (rx_ring, rx_bufs) = queue_memory().ok_or(Error::OutOfMemory)?;
    let (tx_ring, tx_bufs) = queue_memory().ok_or(Error::OutOfMemory)?;
    // If bring-up fails these tables leak; it happens once, at boot.
    let storage = Storage {
        rx_desc: table(DescState::default()),
        tx_desc: table(DescState::default()),
        rx_owner: table(0u16),
        tx_owner: table(0u16),
        tx_free: table(0u16),
    };
    let setup = Setup {
        common: CommonCfg::new(common),
        device_cfg,
        device_cfg_len: device_cfg.len() as u32,
        transport: t,
        notify,
        rx: PacketMemory { ring: region(&rx_ring), buffers: region(&rx_bufs) },
        tx: PacketMemory { ring: region(&tx_ring), buffers: region(&tx_bufs) },
        storage,
        rx_vector: vector,
        tx_vector: vector,
        config_vector: vector,
        mac_seed,
    };
    let memory = [rx_ring, rx_bufs, tx_ring, tx_bufs];
    // SAFETY: the four DMA buffers are fresh, zeroed, direct-mapped blocks
    // of frames that the device reaches at their physical addresses (no
    // IOMMU); on success they are leaked below, so they stay valid and used
    // by nothing else for the kernel's lifetime ('static). The regions all
    // come from this one function's capabilities.
    match unsafe { poly_virtio_net::init(setup) } {
        Ok(mut driver) => {
            memory.into_iter().for_each(core::mem::forget);
            // Until the interrupt is connected: a hint only, and the
            // function is masked anyway.
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

pub fn is_virtio_net(d: &Device) -> bool {
    poly_virtio::device_id_of(&d.header) == Some(device_id::NETWORK)
}

/// A virtio-net function brought up by [`probe`].
#[derive(Debug)]
pub struct Found {
    pub bdf: Bdf,
    pub driver: Driver,
    /// MSI-X, on but masked, with everything mapped to entry 0; `None` if
    /// the device must be polled.
    pub msix: Option<MsixVectors>,
}

/// Find the first virtio-net function [`pci::init`] found and bring it up
/// (MSI-X on but masked if it has it). Logs what it found; `None` if there
/// is none or it cannot be used.
pub fn probe(mac_seed: [u8; 6]) -> Option<Found> {
    let mut found = pci::devices().iter().filter(|d| is_virtio_net(d));
    let dev = found.next()?;
    for other in found {
        kprintln!("virtio-net: {}: ignored, only one device is driven for now", other.bdf());
    }
    let bdf = dev.bdf();
    let mut msix = match dev.msix() {
        Some(Ok(m)) => {
            kprintln!(
                "virtio-net: {bdf}: MSI-X with {} vectors, table BAR{}+{:#x}, PBA BAR{}+{:#x}",
                m.table_size,
                m.table_bar,
                m.table_offset,
                m.pba_bar,
                m.pba_offset
            );
            match dev.enable_msix() {
                Ok(v) => Some(v),
                Err(e) => {
                    kprintln!("virtio-net: {bdf}: cannot enable MSI-X ({e:?}); polling");
                    None
                }
            }
        }
        Some(Err(e)) => {
            kprintln!("virtio-net: {bdf}: MSI-X capability unusable ({e:?}); polling");
            None
        }
        None => {
            kprintln!("virtio-net: {bdf}: no MSI-X; polling");
            None
        }
    };
    let mut result = bring_up(dev, msix.as_ref().map(|_| MSIX_ENTRY), mac_seed);
    if let Err(e) = &result
        && let Some(m) = msix.take()
    {
        kprintln!("virtio-net: {bdf}: bring-up with MSI-X failed ({e:?}); trying again, polled");
        m.disable();
        result = bring_up(dev, None, mac_seed);
    }
    let driver = match result {
        Ok(d) => d,
        Err(e) => {
            kprintln!("virtio-net: {bdf}: bring-up failed: {e:?}");
            if let Some(m) = msix {
                m.disable();
            }
            return None;
        }
    };
    kprintln!(
        "virtio-net: {bdf}: MAC {}, MTU {}, link {}, features {:#x}, queues {} rx / {} tx entries",
        driver.mac(),
        driver.mtu(),
        if driver.link_up() { "up" } else { "down" },
        driver.features(),
        driver.rx_queue_size(),
        driver.tx_queue_size(),
    );
    Some(Found { bdf, driver, msix })
}

/// Point MSI-X entry 0 at `handler(data)` and let the function interrupt.
/// The caller then asks the device for interrupts (`enable_interrupts`) and
/// polls once, to collect what arrived before. Returns the kernel vector.
pub fn connect_interrupt(
    msix: &mut MsixVectors,
    handler: crate::arch::irq::Handler,
    data: usize,
) -> Result<u8, IrqError> {
    let vector = msix.attach(MSIX_ENTRY, handler, data)?;
    msix.set_function_masked(false);
    Ok(vector)
}
