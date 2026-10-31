//! virtio-input over PCI (ADR 0103): the kernel's side of the
//! `poly-virtio-input` driver, for every virtio keyboard, mouse and tablet.
//!
//! The driver (the configuration as an event-layer device, the event and
//! status queues, recovery from a misbehaving device) is the host-tested
//! `poly_virtio_input` crate. This module, like `net/virtio.rs` for
//! virtio-net, gives it what only the kernel has (VIRTIO 1.2 §3.1.1 up to
//! where `poly_virtio_input::init` takes over):
//!
//! - finds the functions (vendor 0x1af4, virtio device type 18: PCI device
//!   0x1052; QEMU's `virtio-keyboard-pci`, `virtio-mouse-pci`,
//!   `virtio-tablet-pci`), locates their structures and maps them;
//! - turns on memory decoding, resets each device and only then bus
//!   mastering; gives it DMA memory for its two queues (64 and 16 eight-byte
//!   event buffers);
//! - with MSI-X (enabled masked before bring-up, both queues and
//!   configuration changes on table entry 0), the interrupt handler takes
//!   the events and reports them to the event layer; a device without
//!   usable MSI-X is polled by the input thread every 10 ms;
//! - a keyboard's LEDs follow the console's locks through the status queue.

use crate::dma::DmaBuffer;
use crate::pci::{self, Device, MapError, MmioRegion, MsixVectors};
use crate::sync::{SetOnce, SpinLock};
use crate::{input, kprintln, time};
use alloc::boxed::Box;
use alloc::format;
use alloc::vec;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, Ordering::SeqCst};
use poly_input::codes::{ev, syn};
use poly_pci::{Bdf, command};
use poly_virtio::{CommonCfg, DescState, InitError, Layout, PciTransport, Region, TransportError, device_id};
use poly_virtio_input::{DmaRegion, EVENT_LEN, QueueMemory, RawEvent, Setup, Storage, VirtioInput};

/// The driver as the kernel instantiates it.
pub type Driver = VirtioInput<'static, MmioRegion, MmioRegion>;

/// Entries the queues get memory for (fewer if the device offers fewer).
const EVENT_ENTRIES: u16 = poly_virtio_input::MAX_EVENT_ENTRIES;
const STATUS_ENTRIES: u16 = poly_virtio_input::MAX_STATUS_ENTRIES;
/// Status reads after a reset before giving up on the device.
const RESET_POLLS: u32 = 1_000_000;
/// The MSI-X table entry both queues and configuration changes use.
const MSIX_ENTRY: u16 = 0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Transport(TransportError),
    Bar(MapError),
    /// The function has no device-specific configuration structure.
    NoDeviceConfig,
    /// The device did not finish the reset before bus mastering went on.
    Reset(InitError),
    OutOfMemory,
    Init(poly_virtio_input::InitError),
}

/// One device brought up.
struct Entry {
    bdf: Bdf,
    driver: SpinLock<Driver>,
    /// Its device in the event layer.
    dev: usize,
    /// MSI-X, on but masked until connected; `None`: polled.
    msix: SpinLock<Option<MsixVectors>>,
    polled: AtomicBool,
}

static ENTRIES: SetOnce<Vec<&'static Entry>> = SetOnce::new();

fn map_region(dev: &Device, r: Region) -> Result<MmioRegion, Error> {
    let bar = dev.map_bar(r.bar as usize).map_err(Error::Bar)?;
    bar.subregion(r.offset as usize, r.length as usize).ok_or(Error::Transport(TransportError::OutsideBar))
}

fn region(b: &DmaBuffer) -> DmaRegion {
    DmaRegion { ptr: b.ptr(), bus: b.phys(), len: b.len() }
}

/// A table that lives as long as the kernel (the driver keeps it).
fn table(entries: u16) -> &'static mut [DescState] {
    Box::leak(vec![DescState::default(); entries as usize].into_boxed_slice())
}

/// DMA memory for one queue of `entries`: the ring and its buffers.
fn queue_memory(entries: u16) -> Option<(DmaBuffer, DmaBuffer)> {
    let layout = Layout::new(entries)?;
    Some((DmaBuffer::new(layout.total())?, DmaBuffer::new(entries as usize * EVENT_LEN)?))
}

/// Bring the device up and hand it to `poly-virtio-input`, its queues and
/// configuration changes on MSI-X entry `vector` if given.
fn bring_up(dev: &Device, vector: Option<u16>) -> Result<Driver, Error> {
    let cfg = dev.config();
    let bdf = dev.bdf();
    let t = PciTransport::discover(cfg, &dev.header).map_err(Error::Transport)?;
    t.check_bars(&dev.bars).map_err(Error::Transport)?;
    let device_region = t.device.ok_or(Error::NoDeviceConfig)?;
    let device_cfg = map_region(dev, device_region)?;
    let common = map_region(dev, t.common)?;
    let notify = map_region(dev, t.notify)?;

    // As for the other virtio devices: decoding on and INTx off first, bus
    // mastering only after a reset made the device forget any queues.
    poly_pci::enable(cfg, bdf, command::MEMORY_SPACE | command::INTX_DISABLE);
    CommonCfg::new(common).reset(RESET_POLLS).map_err(Error::Reset)?;
    poly_pci::enable(cfg, bdf, command::BUS_MASTER);

    let (event_ring, event_bufs) = queue_memory(EVENT_ENTRIES).ok_or(Error::OutOfMemory)?;
    let (status_ring, status_bufs) = queue_memory(STATUS_ENTRIES).ok_or(Error::OutOfMemory)?;
    let setup = Setup {
        common: CommonCfg::new(common),
        device_cfg,
        device_cfg_len: device_cfg.len() as u32,
        transport: t,
        notify,
        event: QueueMemory { ring: region(&event_ring), buffers: region(&event_bufs) },
        status: QueueMemory { ring: region(&status_ring), buffers: region(&status_bufs) },
        // If bring-up fails these tables leak; it happens once, at boot.
        storage: Storage { event_desc: table(EVENT_ENTRIES), status_desc: table(STATUS_ENTRIES) },
        vector,
    };
    let memory = [event_ring, event_bufs, status_ring, status_bufs];
    // SAFETY: the four DMA buffers are fresh, zeroed, direct-mapped blocks
    // of frames that the device reaches at their physical addresses (no
    // IOMMU); on success they are leaked below, so they stay valid and used
    // by nothing else for the kernel's lifetime ('static). The regions all
    // come from this one function's capabilities.
    match unsafe { poly_virtio_input::init(setup) } {
        Ok(mut driver) => {
            memory.into_iter().for_each(core::mem::forget);
            // Until the interrupt is connected (a hint only: the function
            // is masked anyway).
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

pub fn is_virtio_input(d: &Device) -> bool {
    poly_virtio::device_id_of(&d.header) == Some(device_id::INPUT)
}

/// Bring up one function: MSI-X masked if it has it, else polled.
fn probe(dev: &'static Device) -> Option<(Driver, Option<MsixVectors>)> {
    let bdf = dev.bdf();
    let mut msix = match dev.msix() {
        Some(Ok(_)) => match dev.enable_msix() {
            Ok(v) => Some(v),
            Err(e) => {
                kprintln!("virtio-input: {bdf}: cannot enable MSI-X ({e:?}); polling");
                None
            }
        },
        _ => None,
    };
    let mut result = bring_up(dev, msix.as_ref().map(|_| MSIX_ENTRY));
    if let Err(e) = &result
        && let Some(m) = msix.take()
    {
        kprintln!("virtio-input: {bdf}: bring-up with MSI-X failed ({e:?}); trying again, polled");
        m.disable();
        result = bring_up(dev, None);
    }
    match result {
        Ok(d) => Some((d, msix)),
        Err(e) => {
            kprintln!("virtio-input: {bdf}: bring-up failed: {e:?}");
            if let Some(m) = msix {
                m.disable();
            }
            None
        }
    }
}

/// Find every virtio input function, bring each up and add it to the event
/// layer. Once, after `pci::init` and `input::init`.
pub fn init() {
    let mut list: Vec<&'static Entry> = Vec::new();
    for dev in pci::devices().iter().filter(|d| is_virtio_input(d)) {
        let Some((driver, msix)) = probe(dev) else { continue };
        let slot = list.len();
        let mut info = driver.info().clone();
        info.phys = format!("pci-{}/virtio-input", dev.bdf());
        let leds = info.caps.led.any().then_some((set_leds_hook as input::LedHook, slot));
        let polled = msix.is_none();
        let name = info.name.clone();
        let (features, queues) = (driver.features(), (driver.event_queue_size(), driver.status_queue_size()));
        let ids = info.id;
        let ev_bits = info.caps.ev.0[0];
        let dev_no = input::register(info, leds);
        kprintln!(
            "virtio-input: {}: \"{name}\" (bus {:#04x} vendor {:#06x} product {:#06x}), event types {ev_bits:#x}, features {features:#x}, queues {} / {} entries; event{dev_no}",
            dev.bdf(),
            ids.bustype,
            ids.vendor,
            ids.product,
            queues.0,
            queues.1,
        );
        list.push(Box::leak(Box::new(Entry {
            bdf: dev.bdf(),
            driver: SpinLock::new(driver),
            dev: dev_no,
            msix: SpinLock::new(msix),
            polled: AtomicBool::new(polled),
        })));
    }
    let Ok(entries) = ENTRIES.set(list) else { return };
    // The handlers find their device in ENTRIES, so connect them now.
    for (slot, e) in entries.iter().enumerate() {
        let mut msix = e.msix.lock();
        let connected = match msix.as_mut() {
            Some(m) => match m.attach(MSIX_ENTRY, on_irq, slot) {
                Ok(v) => {
                    m.set_function_masked(false);
                    Some(v)
                }
                Err(err) => {
                    kprintln!("virtio-input: {}: cannot attach MSI-X entry 0 ({err:?}); polling", e.bdf);
                    None
                }
            },
            None => None,
        };
        drop(msix);
        match connected {
            Some(v) => {
                kprintln!("virtio-input: {}: MSI-X entry {MSIX_ENTRY} -> vector {v:#x}; interrupt-driven", e.bdf)
            }
            None => {
                e.polled.store(true, SeqCst);
                kprintln!("virtio-input: {}: polled every 10 ms", e.bdf);
            }
        }
        // Collect what arrived before, and ask for interrupts.
        drain(e);
    }
}

/// Devices found.
pub fn count() -> usize {
    ENTRIES.get().map_or(0, |l| l.len())
}

/// Whether some device has no interrupt, so the input thread polls.
pub fn needs_polling() -> bool {
    ENTRIES.get().is_some_and(|l| l.iter().any(|e| e.polled.load(SeqCst)))
}

/// Poll the devices without interrupts (the input thread).
pub fn poll_all() {
    for e in ENTRIES.get().into_iter().flatten() {
        if e.polled.load(SeqCst) {
            drain(e);
        }
    }
}

/// Take every event the device has delivered into the event layer, then
/// ask for an interrupt for the next (unless polled).
fn drain(e: &Entry) {
    let mut buf: [RawEvent; 32] = [(0, 0, 0); 32];
    loop {
        let (n, more) = {
            let mut d = e.driver.lock();
            let n = d.poll(&mut buf);
            let more = n == buf.len() || (!e.polled.load(SeqCst) && d.enable_interrupts());
            (n, more)
        };
        if n > 0 {
            input::report_at(e.dev, time::now_ns(), &buf[..n]);
        }
        if !more {
            return;
        }
    }
}

fn on_irq(slot: usize) {
    if let Some(e) = ENTRIES.get().and_then(|l| l.get(slot)) {
        drain(e);
    }
}

/// The event layer lights a virtio keyboard's LEDs: `EV_LED` events for
/// every LED it has, then `SYN_REPORT`, on the status queue.
fn set_leds_hook(slot: usize, bits: u64) {
    let Some(e) = ENTRIES.get().and_then(|l| l.get(slot)) else { return };
    let mut d = e.driver.lock();
    let mut events: Vec<RawEvent> =
        d.info().caps.led.iter().map(|code| (ev::LED, code, i32::from(bits & (1 << code) != 0))).collect();
    events.push((ev::SYN, syn::REPORT, 0));
    if let Err(err) = d.send(&events) {
        kprintln!("virtio-input: {}: setting the LEDs failed: {err:?}", e.bdf);
    }
}

// --- Tests ------------------------------------------------------------------

use crate::kernel_test;

/// Kernel flag: `poly.test.virtio_input=<n>` says the boot has `n` virtio
/// input devices, which `devices_found` then expects.
pub const TEST_FLAG: &str = "poly.test.virtio_input";

kernel_test! {
    fn devices_found(env: &TestEnv) -> TestResult {
        let Some(want) = env.boot.arg(TEST_FLAG) else { return Ok(()) };
        let want: usize = want.parse().map_err(|_| "poly.test.virtio_input is not a number")?;
        if count() != want {
            kprintln!("virtio-input test: {} device(s), expected {want}", count());
            return Err("not every virtio input device was brought up");
        }
        for e in ENTRIES.get().into_iter().flatten() {
            if e.driver.lock().is_failed() || input::info(e.dev).is_none() {
                return Err("a virtio input device failed or is not in the event layer");
            }
        }
        Ok(())
    }
}
