//! virtio-blk over PCI, polled: the kernel's first disk driver.
//!
//! Bring-up follows VIRTIO 1.2 §3.1.1 with the pieces from `poly-virtio`:
//! find the function (vendor 0x1af4, virtio device type 2; QEMU's
//! modern-only `virtio-blk-pci,disable-legacy=on` is PCI device 0x1042),
//! locate its structures through the vendor capabilities
//! ([`PciTransport::discover`]), map them out of its BARs, reset it,
//! negotiate features (VERSION_1 is required), read the capacity and block
//! size from the device configuration, set up the request queue (queue 0)
//! in a DMA buffer and set DRIVER_OK.
//!
//! Requests go out one at a time and the driver spins on the used ring
//! until the device hands the request back, with a spin budget instead of a
//! timer (there is none yet). Data moves through a one-page bounce buffer,
//! so callers may pass any memory. Everything block-specific on the wire
//! (feature bits, configuration layout, request header and status) is in
//! [`protocol`], so a shared virtio-blk library can replace it.
//!
//! # Interrupts (not yet)
//!
//! The device is driven with interrupts off: INTx is disabled in the
//! command register, MSI-X stays disabled, the queue asks the device not to
//! interrupt, and no MSI-X vector is assigned (`NO_VECTOR`). The boot log
//! shows the MSI-X capability that is there. Switching to interrupts, once
//! the kernel has an APIC driver and an IDT vector to spare:
//!
//! 1. Map the MSI-X table BAR ([`pci::Device::map_bar`]), set the function
//!    mask and the enable bit (`MsiX::set_function_mask`,
//!    `MsiX::set_enabled`), write table entry 0 with the local APIC message
//!    for the vector (`MsiX::write_entry`), clear the function mask.
//! 2. Pass `Some(0)` instead of `None` to `enable_queue` in
//!    [`VirtioBlk::bring_up`] (and `set_config_vector` for configuration
//!    changes); both fail with `VectorRejected` if the device cannot map it.
//! 3. Call `enable_interrupts` on the queue and replace the spin in
//!    [`VirtioBlk::wait`] with sleeping until the vector fires (harvesting
//!    with `pop_used` in the handler or after wake-up).

mod protocol;

use self::protocol::{RequestType, SECTOR_SIZE, Status};
use crate::dma::DmaBuffer;
use crate::pci::{self, Device, MapError, MmioRegion, Size};
use crate::sync::SpinLock;
use crate::{kernel_test, kprint, kprintln};
use alloc::boxed::Box;
use alloc::vec;
use core::mem::ManuallyDrop;
use poly_pci::{Bdf, ConfigSpace, Mmio, command};
use poly_virtio::{
    AddError, Buffer, CommonCfg, Configuring, DescState, InitError, Layout, Notifier, PciTransport, Region, SetupError,
    SplitQueue, TransportError, Used, UsedError, begin, device_id, device_id_of, feature,
};

/// The request queue (§5.2.2; the only one without VIRTIO_BLK_F_MQ).
const REQUEST_QUEUE: u16 = 0;
/// One request is in flight at a time and takes three descriptors.
const QUEUE_SIZE: u16 = 16;
const MIN_QUEUE_SIZE: u16 = 4;
/// Status reads after a reset before giving up on the device.
const RESET_POLLS: u32 = 1_000_000;
/// Attempts at a consistent read of the device configuration (§2.5.1).
const CONFIG_TRIES: u32 = 16;
/// Used-ring polls before a request counts as lost. Requests to QEMU's
/// device come back after a few hundred to a few thousand polls; a poll
/// costs about 0.25 µs under QEMU's TCG (debug kernel) and less with KVM,
/// so this gives up after about 5 seconds at most, long before a boot
/// test's timeout. Becomes a real timeout once there is a timer.
const POLL_SPINS: u64 = 20_000_000;
/// Features the driver understands, besides VERSION_1 (which `begin`
/// requires). ACCESS_PLATFORM is accepted when offered, as §6.1 asks: the
/// kernel sets up no IOMMU, so bus addresses stay physical addresses.
const WANTED: u64 =
    protocol::feature::RO | protocol::feature::BLK_SIZE | protocol::feature::FLUSH | feature::ACCESS_PLATFORM;

/// Data moves through a bounce buffer of this size, so a longer transfer
/// becomes several requests.
const BOUNCE_SIZE: usize = 4096;
/// The request header buffer holds the header, then the status byte.
const STATUS_OFFSET: usize = protocol::HEADER_LEN;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Transport(TransportError),
    Bar(MapError),
    Init(InitError),
    Queue(SetupError),
    /// No device configuration, or it is too short or never reads
    /// consistently.
    BadConfig,
    /// The device's request queue is smaller than one request needs.
    QueueTooSmall(u16),
    /// The device's notification offset lies outside its notify region.
    NoNotifier,
    OutOfMemory,
    /// The request runs past the end of the disk.
    OutOfRange,
    /// The length is not a multiple of 512 bytes.
    Unaligned,
    ReadOnly,
    /// The device does not take FLUSH requests.
    NoFlush,
    /// The device completed the request with this status.
    Device(Status),
    /// The device wrote less than a read asked for.
    ShortTransfer,
    /// The request did not come back within the spin budget.
    Timeout,
    Add(AddError),
    /// The device broke the queue protocol.
    Protocol(UsedError),
    /// An earlier timeout or protocol error stopped the driver; the device
    /// has been reset.
    Broken,
}

/// A virtio-blk device that has been brought up.
pub struct VirtioBlk {
    bdf: Bdf,
    common: CommonCfg<MmioRegion>,
    notify: MmioRegion,
    notifier: Notifier,
    queue: SplitQueue<'static>,
    /// Size of the disk in 512-byte sectors.
    capacity: u64,
    features: u64,
    broken: bool,
    // The device holds the bus addresses of these: they are only freed once
    // it has been reset (see `Drop`).
    queue_mem: ManuallyDrop<DmaBuffer>,
    /// Request header at 0, status byte at [`STATUS_OFFSET`].
    req: ManuallyDrop<DmaBuffer>,
    /// The bounce buffer, [`BOUNCE_SIZE`] bytes.
    data: ManuallyDrop<DmaBuffer>,
}

impl core::fmt::Debug for VirtioBlk {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("VirtioBlk")
            .field("bdf", &self.bdf)
            .field("capacity", &self.capacity)
            .field("features", &self.features)
            .field("queue", &self.queue)
            .field("broken", &self.broken)
            .finish()
    }
}

/// What [`VirtioBlk::bring_up`] reports for the boot log.
struct Facts {
    offered: u64,
    block_size: u32,
    queue_max: u16,
}

/// Map the part of a BAR a virtio structure occupies.
fn map_region(dev: &Device, r: Region) -> Result<MmioRegion, Error> {
    let bar = dev.map_bar(r.bar as usize).map_err(Error::Bar)?;
    bar.subregion(r.offset as usize, r.length as usize).ok_or(Error::Transport(TransportError::OutsideBar))
}

/// The queue and buffers, once the device knows the queue's addresses.
struct QueueSetup {
    queue: SplitQueue<'static>,
    queue_mem: DmaBuffer,
    notifier: Notifier,
    queue_max: u16,
}

impl VirtioBlk {
    fn bring_up(dev: &Device) -> Result<(VirtioBlk, Facts), Error> {
        let cfg = dev.config();
        let bdf = dev.bdf();
        let t = PciTransport::discover(cfg, &dev.header).map_err(Error::Transport)?;
        t.check_bars(&dev.bars).map_err(Error::Transport)?;
        let device_cfg = map_region(dev, t.device.ok_or(Error::BadConfig)?)?;
        if device_cfg.len() < protocol::config::LEN {
            return Err(Error::BadConfig);
        }
        let common = CommonCfg::new(map_region(dev, t.common)?);
        let notify = map_region(dev, t.notify)?;
        let req = DmaBuffer::new(STATUS_OFFSET + 1).ok_or(Error::OutOfMemory)?;
        let data = DmaBuffer::new(BOUNCE_SIZE).ok_or(Error::OutOfMemory)?;

        // Registers answer once memory decoding is on. INTx stays off, since
        // the driver polls. Bus mastering waits until the reset in `begin`
        // has made the device forget whatever queues firmware set up.
        poly_pci::enable(cfg, bdf, command::MEMORY_SPACE | command::INTX_DISABLE);
        let negotiation = begin(&common, RESET_POLLS).map_err(Error::Init)?;
        let offered = negotiation.offered();
        let c = negotiation.accept(WANTED).map_err(Error::Init)?;
        let features = c.features();

        let read = || {
            let lo = device_cfg.read32(protocol::config::CAPACITY) as u64;
            let hi = device_cfg.read32(protocol::config::CAPACITY + 4) as u64;
            (hi << 32 | lo, device_cfg.read32(protocol::config::BLK_SIZE))
        };
        let Some((capacity, blk_size)) = c.common().read_config(CONFIG_TRIES, read) else {
            c.fail();
            return Err(Error::BadConfig);
        };
        let block_size = if c.has(protocol::feature::BLK_SIZE) { blk_size } else { SECTOR_SIZE as u32 };

        poly_pci::enable(cfg, bdf, command::BUS_MASTER);
        let setup = match Self::set_up_queue(&c, &t) {
            Ok(s) => s,
            Err(e) => {
                c.fail();
                return Err(e);
            }
        };
        c.driver_ok();
        let disk = VirtioBlk {
            bdf,
            common,
            notify,
            notifier: setup.notifier,
            queue: setup.queue,
            capacity,
            features,
            broken: false,
            queue_mem: ManuallyDrop::new(setup.queue_mem),
            req: ManuallyDrop::new(req),
            data: ManuallyDrop::new(data),
        };
        Ok((disk, Facts { offered, block_size, queue_max: setup.queue_max }))
    }

    /// Set up the request queue in fresh DMA memory and enable it.
    fn set_up_queue(c: &Configuring<'_, MmioRegion>, t: &PciTransport) -> Result<QueueSetup, Error> {
        let queue_max = c.queue_max_size(REQUEST_QUEUE).map_err(Error::Init)?;
        // Split queue sizes are powers of two: the largest one that fits.
        let wanted = queue_max.min(QUEUE_SIZE);
        let size = 1u16 << (15 - wanted.leading_zeros());
        if size < MIN_QUEUE_SIZE {
            return Err(Error::QueueTooSmall(queue_max));
        }
        let layout = Layout::new(size).ok_or(Error::QueueTooSmall(queue_max))?;
        let queue_mem = DmaBuffer::new(layout.total()).ok_or(Error::OutOfMemory)?;
        // The driver lives as long as the kernel, and so does its queue.
        let state = Box::leak(vec![DescState::default(); size as usize].into_boxed_slice());
        // SAFETY: `queue_mem` is a fresh DMA buffer of at least
        // `layout.total()` bytes that the device reaches at its physical
        // address. It is moved into the driver together with the queue and
        // only freed after the device has been reset (`Drop`), and the
        // device's queue is not enabled yet.
        let mut queue = unsafe { SplitQueue::new(queue_mem.ptr(), queue_mem.phys(), layout, state, false) }
            .map_err(Error::Queue)?;
        // A hint only (§2.7.7); the function cannot interrupt anyway.
        queue.disable_interrupts();
        // Interrupt integration point: `Some(vector)` once MSI-X is set up.
        let notify_off = c.enable_queue(REQUEST_QUEUE, size, queue.addresses(), None).map_err(Error::Init)?;
        let Some(notifier) = t.notifier(REQUEST_QUEUE, notify_off) else {
            // The device has the queue's address now: take it back before
            // the memory can be reused, or leak the memory.
            if c.common().reset(RESET_POLLS).is_err() {
                core::mem::forget(queue_mem);
            }
            return Err(Error::NoNotifier);
        };
        Ok(QueueSetup { queue, queue_mem, notifier, queue_max })
    }

    /// Size of the disk in 512-byte sectors.
    pub const fn capacity(&self) -> u64 {
        self.capacity
    }

    pub const fn read_only(&self) -> bool {
        self.features & protocol::feature::RO != 0
    }

    /// Read `buf.len()` bytes (a multiple of 512) starting at `sector`.
    pub fn read(&mut self, sector: u64, buf: &mut [u8]) -> Result<(), Error> {
        self.check_range(sector, buf.len())?;
        let mut sector = sector;
        for chunk in buf.chunks_mut(BOUNCE_SIZE) {
            self.request(RequestType::In, sector, chunk.len())?;
            self.data.read(0, chunk);
            sector += (chunk.len() / SECTOR_SIZE) as u64;
        }
        Ok(())
    }

    /// Write `buf` (a multiple of 512 bytes long) starting at `sector`. The
    /// data may sit in the device's write cache until [`VirtioBlk::flush`].
    pub fn write(&mut self, sector: u64, buf: &[u8]) -> Result<(), Error> {
        if self.read_only() {
            return Err(Error::ReadOnly);
        }
        self.check_range(sector, buf.len())?;
        let mut sector = sector;
        for chunk in buf.chunks(BOUNCE_SIZE) {
            self.data.write(0, chunk);
            self.request(RequestType::Out, sector, chunk.len())?;
            sector += (chunk.len() / SECTOR_SIZE) as u64;
        }
        Ok(())
    }

    /// Make every completed write durable (§5.2.6.2).
    pub fn flush(&mut self) -> Result<(), Error> {
        if self.features & protocol::feature::FLUSH == 0 {
            return Err(Error::NoFlush);
        }
        self.request(RequestType::Flush, 0, 0)
    }

    fn check_range(&self, sector: u64, len: usize) -> Result<(), Error> {
        if !len.is_multiple_of(SECTOR_SIZE) {
            return Err(Error::Unaligned);
        }
        match sector.checked_add((len / SECTOR_SIZE) as u64) {
            Some(end) if end <= self.capacity => Ok(()),
            _ => Err(Error::OutOfRange),
        }
    }

    /// Send one request with `len` bytes of data in the bounce buffer and
    /// wait for it. `len` is at most [`BOUNCE_SIZE`] and 0 for a flush.
    fn request(&mut self, kind: RequestType, sector: u64, len: usize) -> Result<(), Error> {
        if self.broken {
            return Err(Error::Broken);
        }
        debug_assert!(len <= BOUNCE_SIZE && len.is_multiple_of(SECTOR_SIZE));
        self.req.write(0, &protocol::header(kind, sector));
        self.req.write(STATUS_OFFSET, &[Status::UNWRITTEN]);
        let header = Buffer { addr: self.req.phys(), len: protocol::HEADER_LEN as u32 };
        let status = Buffer { addr: self.req.phys() + STATUS_OFFSET as u64, len: 1 };
        let data = Buffer { addr: self.data.phys(), len: len as u32 };
        let head = match kind {
            RequestType::In => self.queue.add(&[header], &[data, status]),
            RequestType::Out => self.queue.add(&[header, data], &[status]),
            RequestType::Flush => self.queue.add(&[header], &[status]),
        }
        .map_err(Error::Add)?;
        if self.queue.should_notify() {
            self.notifier.notify(&self.notify);
        }
        let used = self.wait()?;
        if used.head != head {
            // Only one request is ever in flight.
            self.stop();
            return Err(Error::Protocol(UsedError::BadId(used.head as u32)));
        }
        let mut byte = [0u8];
        self.req.read(STATUS_OFFSET, &mut byte);
        match Status::from_byte(byte[0]) {
            Status::Ok => {}
            s => return Err(Error::Device(s)),
        }
        // A read must have filled the data and the status byte.
        if kind == RequestType::In && (used.len as usize) < len + 1 {
            return Err(Error::ShortTransfer);
        }
        Ok(())
    }

    /// Spin on the used ring until the request comes back.
    fn wait(&mut self) -> Result<Used, Error> {
        for _ in 0..POLL_SPINS {
            match self.queue.pop_used() {
                Ok(Some(used)) => return Ok(used),
                Ok(None) => core::hint::spin_loop(),
                Err(e) => {
                    self.stop();
                    return Err(Error::Protocol(e));
                }
            }
        }
        self.stop();
        Err(Error::Timeout)
    }

    /// Give up on the device after a lost request or a protocol error. The
    /// request in flight still owns its buffers until the device lets go of
    /// them, which a reset makes it do (§2.4); every later request fails.
    fn stop(&mut self) {
        self.broken = true;
        if self.common.reset(RESET_POLLS).is_err() {
            kprintln!("virtio-blk: {}: the device does not reset", self.bdf);
        }
    }
}

impl Drop for VirtioBlk {
    fn drop(&mut self) {
        if self.common.reset(RESET_POLLS).is_err() {
            // The device may still write to the queue and the buffers: they
            // must never be reused, so they stay allocated.
            return;
        }
        // SAFETY: each buffer is dropped once, here, and never used again;
        // the device was reset, so it no longer uses them either.
        unsafe {
            ManuallyDrop::drop(&mut self.queue_mem);
            ManuallyDrop::drop(&mut self.req);
            ManuallyDrop::drop(&mut self.data);
        }
    }
}

static DISK: SpinLock<Option<VirtioBlk>> = SpinLock::new(None);

/// Run `f` on the virtio-blk disk, if one was brought up.
pub fn with_disk<R>(f: impl FnOnce(&mut VirtioBlk) -> R) -> Option<R> {
    DISK.lock().as_mut().map(f)
}

fn is_virtio_blk(d: &Device) -> bool {
    device_id_of(&d.header) == Some(device_id::BLOCK)
}

/// Bring up the first virtio-blk device [`pci::init`] found and read its
/// first sector. Call once, after `pci::init`.
pub fn init() {
    let mut found = pci::devices().iter().filter(|d| is_virtio_blk(d));
    let Some(dev) = found.next() else {
        kprintln!("virtio-blk: no device");
        return;
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
    let (mut disk, facts) = match VirtioBlk::bring_up(dev) {
        Ok(d) => d,
        Err(e) => {
            kprintln!("virtio-blk: {bdf}: bring-up failed: {e:?}");
            return;
        }
    };
    kprintln!(
        "virtio-blk: {bdf}: {} ({} sectors), block size {}, features {:#x} of {:#x}{}, queue {} entries of {}",
        Size(disk.capacity.saturating_mul(SECTOR_SIZE as u64)),
        disk.capacity,
        facts.block_size,
        disk.features,
        facts.offered,
        if disk.read_only() { ", read-only" } else { "" },
        disk.queue.size(),
        facts.queue_max
    );
    let mut sector = [0u8; SECTOR_SIZE];
    match disk.read(0, &mut sector) {
        Ok(()) => {
            kprint!("virtio-blk: sector 0:");
            for b in &sector[..16] {
                kprint!(" {b:02x}");
            }
            let text = sector[..16].iter().map(|&b| if b.is_ascii_graphic() { b as char } else { '.' });
            kprint!("  ");
            text.for_each(|c| kprint!("{c}"));
            kprintln!();
        }
        Err(e) => kprintln!("virtio-blk: reading sector 0 failed: {e:?}"),
    }
    *DISK.lock() = Some(disk);
}

/// The boot test attaches the disk image `cargo xtask` builds and says so
/// with this flag; without it the disk tests only check what is there.
const TEST_DISK_FLAG: &str = "poly.test.disk";
/// Size of that image in sectors (1 MiB).
const TEST_DISK_SECTORS: u64 = 2048;

/// Sector `sector` of the test image: "POLY", the sector number (le32),
/// then bytes that depend on the sector and the position. Must match
/// `test_disk_sector` in tools/xtask.
fn test_pattern(sector: u64) -> [u8; SECTOR_SIZE] {
    let mut s = [0u8; SECTOR_SIZE];
    s[..4].copy_from_slice(b"POLY");
    s[4..8].copy_from_slice(&(sector as u32).to_le_bytes());
    for (i, b) in s.iter_mut().enumerate().skip(8) {
        *b = (sector as u32).wrapping_mul(31).wrapping_add(i as u32) as u8;
    }
    s
}

/// A driver error as a test failure, printed first.
fn io<T>(what: &'static str, r: Result<T, Error>) -> Result<T, &'static str> {
    r.map_err(|e| {
        kprintln!("virtio-blk: {what}: {e:?}");
        what
    })
}

kernel_test! {
    fn device_found_with_sane_bars(env: &TestEnv) -> TestResult {
        let expected = env.boot.has_flag(TEST_DISK_FLAG);
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
        if DISK.lock().is_none() || cmd & want != want {
            return Err("virtio-blk device found but not brought up");
        }
        Ok(())
    }
}

kernel_test! {
    fn reads_the_test_pattern(env: &TestEnv) -> TestResult {
        if !env.boot.has_flag(TEST_DISK_FLAG) {
            return Ok(());
        }
        with_disk(|d| {
            if d.capacity() != TEST_DISK_SECTORS {
                return Err("the test disk does not have 2048 sectors");
            }
            let mut buf = [0u8; SECTOR_SIZE];
            for sector in [0, TEST_DISK_SECTORS - 1] {
                io("read", d.read(sector, &mut buf))?;
                if buf != test_pattern(sector) {
                    return Err("sector 0 or the last sector does not hold the test pattern");
                }
            }
            // Longer than the bounce buffer: several requests.
            let mut long = vec![0u8; 20 * SECTOR_SIZE];
            io("long read", d.read(3, &mut long))?;
            for (i, s) in long.chunks(SECTOR_SIZE).enumerate() {
                if *s != test_pattern(3 + i as u64) {
                    return Err("a multi-request read returned the wrong data");
                }
            }
            // Refused before anything reaches the device.
            if d.read(TEST_DISK_SECTORS - 1, &mut [0; 2 * SECTOR_SIZE]) != Err(Error::OutOfRange)
                || d.read(u64::MAX, &mut buf) != Err(Error::OutOfRange)
                || d.read(0, &mut [0; 100]) != Err(Error::Unaligned)
            {
                return Err("a read past the end or of a partial sector was not refused");
            }
            Ok(())
        })
        .ok_or("no virtio-blk disk")?
    }
}

kernel_test! {
    fn write_flush_read_round_trip(env: &TestEnv) -> TestResult {
        if !env.boot.has_flag(TEST_DISK_FLAG) {
            return Ok(());
        }
        const SECTOR: u64 = 1;
        with_disk(|d| {
            let original = test_pattern(SECTOR);
            let mut changed = original;
            changed[..4].copy_from_slice(b"WRIT");
            changed[8..].iter_mut().for_each(|b| *b ^= 0xa5);
            let mut back = [0u8; SECTOR_SIZE];
            io("write", d.write(SECTOR, &changed))?;
            io("flush", d.flush())?;
            io("read back", d.read(SECTOR, &mut back))?;
            if back != changed {
                return Err("a written sector did not read back");
            }
            // The neighbours are untouched.
            for s in [SECTOR - 1, SECTOR + 1] {
                io("read neighbour", d.read(s, &mut back))?;
                if back != test_pattern(s) {
                    return Err("writing one sector changed another");
                }
            }
            // Put the image back the way xtask made it.
            io("restore", d.write(SECTOR, &original))?;
            io("flush", d.flush())?;
            io("read restored", d.read(SECTOR, &mut back))?;
            if back != original { Err("the restored sector did not read back") } else { Ok(()) }
        })
        .ok_or("no virtio-blk disk")?
    }
}
