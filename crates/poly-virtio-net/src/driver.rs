//! The driver: frames out through the transmit queue, frames in from the
//! receive queue, which is kept full.

use crate::{
    BUF_LEN, ConfigError, ETHERNET_HEADER_LEN, HEADER_LEN, NetConfig, NetHeader, RX_QUEUE, TX_QUEUE, effective_mtu,
    feature, link_status, local_mac, select_features,
};
use core::ptr::{self, NonNull};
use poly_net::{MacAddr, NetDevice, TxError};
use poly_pci::Mmio;
use poly_virtio::{
    AddError, Buffer, CommonCfg, Configuring, DescState, Layout, Notifier, PciTransport, SetupError, SplitQueue, Used,
    begin, status,
};

/// Polls of the status register while waiting for a reset to finish.
const RESET_POLLS: u32 = 1000;

/// Memory shared with the device: its CPU address, bus address and length.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DmaRegion {
    pub ptr: NonNull<u8>,
    pub bus: u64,
    pub len: usize,
}

/// The memory of one virtqueue: the ring (see [`Layout`], 16-byte aligned)
/// and its packet buffers, [`BUF_LEN`] bytes each, one per queue entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PacketMemory {
    pub ring: DmaRegion,
    pub buffers: DmaRegion,
}

/// Caller-provided tables. The queues get no more entries than the
/// shortest table of their side has.
#[derive(Debug)]
pub struct Storage<'a> {
    pub rx_desc: &'a mut [DescState],
    pub tx_desc: &'a mut [DescState],
    /// For each descriptor: the packet buffer whose chain starts there.
    pub rx_owner: &'a mut [u16],
    pub tx_owner: &'a mut [u16],
    /// The free transmit buffers (a stack).
    pub tx_free: &'a mut [u16],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InitError {
    Virtio(poly_virtio::InitError),
    Config(ConfigError),
    Queue(SetupError),
    /// A queue's notification register lies outside the notification
    /// region.
    NoNotifier,
    /// Not enough memory or table space for queues of 2 entries.
    NoRoom,
}

impl From<poly_virtio::InitError> for InitError {
    fn from(e: poly_virtio::InitError) -> InitError {
        InitError::Virtio(e)
    }
}

/// What [`init`] needs.
#[derive(Debug)]
pub struct Setup<'a, M, C, N> {
    /// The common configuration registers; the driver keeps them, to reset
    /// the device if it misbehaves.
    pub common: CommonCfg<M>,
    /// The device configuration region and its length; kept, for the link
    /// status.
    pub device_cfg: C,
    pub device_cfg_len: u32,
    pub transport: PciTransport,
    /// The notification region.
    pub notify: N,
    pub rx: PacketMemory,
    pub tx: PacketMemory,
    pub storage: Storage<'a>,
    /// MSI-X table entries for the receive queue, the transmit queue and
    /// configuration changes (`None`: no interrupt). MSI-X must be enabled
    /// on the function for the device to take them. Several may share one
    /// entry.
    pub rx_vector: Option<u16>,
    pub tx_vector: Option<u16>,
    pub config_vector: Option<u16>,
    /// Random bytes for a station address, used when the device has none
    /// (no VIRTIO_NET_F_MAC; see [`local_mac`]).
    pub mac_seed: [u8; 6],
}

/// The largest power-of-two queue size, at most `max` (the device's limit)
/// and `room` (table entries), whose ring and buffers fit `mem`.
fn queue_size(max: u16, room: usize, mem: &PacketMemory) -> u16 {
    let room = (max as usize).min(room).min(Layout::MAX_SIZE as usize);
    (1..=15)
        .rev()
        .map(|k| 1u16 << k)
        .find(|&s| {
            s as usize <= room
                && Layout::new(s).is_some_and(|l| l.total() <= mem.ring.len)
                && s as usize * BUF_LEN <= mem.buffers.len
        })
        .unwrap_or(0)
}

/// Set up queue `index` of `size` entries in `mem.ring`, interrupting on
/// MSI-X entry `vector`, and find its notification register.
///
/// # Safety
///
/// As for [`init`]: the ring memory is the caller's to give to the device.
unsafe fn setup_queue<'a, M: Mmio>(
    cfg: &Configuring<'_, M>,
    transport: &PciTransport,
    index: u16,
    size: u16,
    mem: &PacketMemory,
    desc: &'a mut [DescState],
    vector: Option<u16>,
) -> Result<(SplitQueue<'a>, Notifier), InitError> {
    let layout = Layout::new(size).ok_or(InitError::NoRoom)?;
    let event_idx = cfg.has(poly_virtio::feature::EVENT_IDX);
    // SAFETY: our caller vouches for the memory; the layout fits in it (the
    // size was chosen so), and the queue is not enabled yet.
    let queue =
        unsafe { SplitQueue::new(mem.ring.ptr, mem.ring.bus, layout, desc, event_idx) }.map_err(InitError::Queue)?;
    let notify_off = cfg.enable_queue(index, size, queue.addresses(), vector)?;
    let notifier = transport.notifier(index, notify_off).ok_or(InitError::NoNotifier)?;
    Ok((queue, notifier))
}

/// Initialise a virtio network device (§3.1.1, §5.1.5) and return the
/// running driver: features negotiated, the configuration read, receive
/// queue 0 and transmit queue 1 set up (each as large as the device, the
/// memory and the tables allow), every receive buffer handed to the device,
/// DRIVER_OK set. On failure the device is left marked FAILED (or, if it
/// refused the reset, untouched).
///
/// # Safety
///
/// The ring and buffer memory of `rx` and `tx` must be valid for reads and
/// writes for `'a`, reachable by the device at their bus addresses, and
/// used by nothing but the driver and the device. `common`, `device_cfg`,
/// `transport` and `notify` must all belong to the same device.
pub unsafe fn init<'a, M: Mmio, C: Mmio, N: Mmio>(
    setup: Setup<'a, M, C, N>,
) -> Result<VirtioNet<'a, M, C, N>, InitError> {
    let Setup {
        common,
        device_cfg,
        device_cfg_len,
        transport,
        notify,
        rx,
        tx,
        storage,
        rx_vector,
        tx_vector,
        config_vector,
        mac_seed,
    } = setup;
    let negotiation = begin(&common, RESET_POLLS)?;
    let offered = negotiation.offered();
    // The MTU decides whether VIRTIO_NET_F_MTU can be taken, so it is read
    // before accepting (allowed by §3.1.1 step 4).
    let device_mtu = if offered & feature::MTU != 0 && device_cfg_len >= 12 {
        match NetConfig::read(&common, &device_cfg, device_cfg_len, feature::MTU) {
            Ok(c) => c.mtu,
            Err(_) => None,
        }
    } else {
        None
    };
    let cfg = negotiation.accept(select_features(offered, device_mtu))?;
    macro_rules! fail {
        ($e:expr) => {{
            cfg.fail();
            return Err($e);
        }};
    }
    let config = match NetConfig::read(&common, &device_cfg, device_cfg_len, cfg.features()) {
        Ok(c) => c,
        Err(e) => fail!(InitError::Config(e)),
    };
    if let Some(v) = config_vector
        && let Err(e) = cfg.set_config_vector(v)
    {
        fail!(InitError::Virtio(e));
    }
    let Storage { rx_desc, tx_desc, rx_owner, tx_owner, tx_free } = storage;
    let max = |q| cfg.queue_max_size(q);
    let (rx_max, tx_max) = match (max(RX_QUEUE), max(TX_QUEUE)) {
        (Ok(r), Ok(t)) => (r, t),
        (Err(e), _) | (_, Err(e)) => fail!(InitError::Virtio(e)),
    };
    let rx_size = queue_size(rx_max, rx_desc.len().min(rx_owner.len()), &rx);
    let tx_size = queue_size(tx_max, tx_desc.len().min(tx_owner.len()).min(tx_free.len()), &tx);
    if rx_size < 2 || tx_size < 2 {
        fail!(InitError::NoRoom);
    }
    // SAFETY: forwarded from our caller.
    let (mut rx_queue, rx_notifier) =
        match unsafe { setup_queue(&cfg, &transport, RX_QUEUE, rx_size, &rx, rx_desc, rx_vector) } {
            Ok(q) => q,
            Err(e) => fail!(e),
        };
    // SAFETY: forwarded from our caller.
    let (tx_queue, tx_notifier) =
        match unsafe { setup_queue(&cfg, &transport, TX_QUEUE, tx_size, &tx, tx_desc, tx_vector) } {
            Ok(q) => q,
            Err(e) => fail!(e),
        };
    // §5.1.5 step 3: fill the receive queue before the device goes live.
    // The device may not look at it before DRIVER_OK (§3.1.1), so the
    // notification waits until then.
    for slot in 0..rx_size {
        match rx_queue.add(&[], &[rx_buffer(&rx.buffers, slot)]) {
            Ok(head) => rx_owner[head as usize] = slot,
            Err(_) => fail!(InitError::NoRoom),
        }
    }
    for (i, f) in tx_free.iter_mut().take(tx_size as usize).enumerate() {
        *f = i as u16;
    }
    let negotiated = cfg.features();
    let features = cfg.driver_ok();
    debug_assert_eq!(features, negotiated);
    let mut net = VirtioNet {
        common,
        device_cfg,
        notify,
        rx: rx_queue,
        tx: tx_queue,
        rx_notifier,
        tx_notifier,
        rx_mem: rx.buffers,
        tx_mem: tx.buffers,
        rx_owner,
        tx_owner,
        tx_free,
        tx_free_len: tx_size,
        rx_pending: None,
        mac: config.mac.unwrap_or_else(|| local_mac(mac_seed)),
        mtu: effective_mtu(config.mtu),
        features,
        stats: Stats::default(),
        failed: false,
        reset_done: false,
    };
    net.kick_rx();
    Ok(net)
}

/// Receive buffer `slot` of `mem`, device-writable.
fn rx_buffer(mem: &DmaRegion, slot: u16) -> Buffer {
    Buffer { addr: mem.bus + (slot as usize * BUF_LEN) as u64, len: BUF_LEN as u32 }
}

/// Counters.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    pub rx_packets: u64,
    /// Frame bytes, without the virtio header.
    pub rx_bytes: u64,
    /// Frames longer than the stack's buffer, dropped.
    pub rx_dropped: u64,
    /// Buffers with a bad length or buffer count, and runt frames.
    pub rx_errors: u64,
    pub tx_packets: u64,
    pub tx_bytes: u64,
    /// Frames refused because every transmit buffer was in flight.
    pub tx_busy: u64,
    /// Notifications sent to the device.
    pub rx_kicks: u64,
    pub tx_kicks: u64,
}

/// A running virtio network device. See the [crate documentation](crate).
pub struct VirtioNet<'a, M, C, N> {
    common: CommonCfg<M>,
    device_cfg: C,
    notify: N,
    rx: SplitQueue<'a>,
    tx: SplitQueue<'a>,
    rx_notifier: Notifier,
    tx_notifier: Notifier,
    rx_mem: DmaRegion,
    tx_mem: DmaRegion,
    rx_owner: &'a mut [u16],
    tx_owner: &'a mut [u16],
    tx_free: &'a mut [u16],
    tx_free_len: u16,
    /// The first buffer of a packet whose other buffers (MRG_RXBUF) the
    /// device has not handed back yet.
    rx_pending: Option<Used>,
    mac: [u8; 6],
    mtu: usize,
    features: u64,
    stats: Stats,
    /// The device broke the queue protocol.
    failed: bool,
    /// ... and has since been reset, so it no longer touches memory.
    reset_done: bool,
}

// SAFETY: the queue and buffer memory belong to the driver alone (the
// contract of `init`), so moving the driver moves their only CPU-side user
// with it; the rest is plain data or the caller's own types.
unsafe impl<M: Send, C: Send, N: Send> Send for VirtioNet<'_, M, C, N> {}

impl<M, C, N> core::fmt::Debug for VirtioNet<'_, M, C, N> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("VirtioNet")
            .field("mac", &MacAddr(self.mac))
            .field("mtu", &self.mtu)
            .field("features", &self.features)
            .field("rx", &self.rx)
            .field("tx", &self.tx)
            .field("failed", &self.failed)
            .finish()
    }
}

impl<M: Mmio, C: Mmio, N: Mmio> VirtioNet<'_, M, C, N> {
    /// The negotiated features.
    pub const fn features(&self) -> u64 {
        self.features
    }

    pub const fn has(&self, f: u64) -> bool {
        self.features & f == f
    }

    pub const fn rx_queue_size(&self) -> u16 {
        self.rx.size()
    }

    pub const fn tx_queue_size(&self) -> u16 {
        self.tx.size()
    }

    /// Frames handed to the device and not yet sent.
    pub const fn tx_in_flight(&self) -> u16 {
        self.tx.in_flight()
    }

    /// Receive buffers the device holds.
    pub const fn rx_posted(&self) -> u16 {
        self.rx.in_flight()
    }

    pub const fn stats(&self) -> Stats {
        self.stats
    }

    /// The device broke the protocol and was reset; it sends and receives
    /// nothing any more.
    pub const fn is_failed(&self) -> bool {
        self.failed
    }

    /// Is the link up? Without VIRTIO_NET_F_STATUS the driver assumes so
    /// (§5.1.4.2). A failed device is down.
    pub fn link_up(&self) -> bool {
        if self.failed {
            return false;
        }
        if !self.has(feature::STATUS) {
            return true;
        }
        self.device_cfg.read16(6) & link_status::LINK_UP != 0
    }

    /// Ask the device for interrupts: for every received frame, and for
    /// transmit completions only while all transmit buffers are in flight
    /// (otherwise they are reclaimed when the next frame goes out,
    /// §5.1.6.2.2). Returns true if work arrived meanwhile, which may not
    /// raise an interrupt: poll the stack again before waiting for one.
    pub fn enable_interrupts(&mut self) -> bool {
        if self.failed {
            return false;
        }
        let mut pending = self.rx.enable_interrupts();
        if self.tx_free_len == 0 {
            pending |= self.tx.enable_interrupts();
        } else {
            self.tx.disable_interrupts();
        }
        pending
    }

    /// Ask the device not to interrupt (a hint, §2.7.7).
    pub fn disable_interrupts(&mut self) {
        self.rx.disable_interrupts();
        self.tx.disable_interrupts();
    }

    /// Take back the transmit buffers the device has finished with.
    pub fn reclaim(&mut self) {
        while !self.failed {
            match self.tx.pop_used() {
                Ok(Some(u)) => {
                    let slot = self.tx_owner[u.head as usize];
                    self.tx_free[self.tx_free_len as usize] = slot;
                    self.tx_free_len += 1;
                }
                Ok(None) => return,
                Err(_) => self.fail(),
            }
        }
    }

    /// The device broke the protocol: reset it, so it stops using our
    /// memory, and mark it FAILED.
    fn fail(&mut self) {
        self.failed = true;
        self.rx_pending = None;
        if !self.reset_done && self.common.reset(RESET_POLLS).is_ok() {
            self.reset_done = true;
            self.common.set_status(status::FAILED);
        }
    }

    /// Hand receive buffer `slot` to the device. False if the queue had no
    /// free descriptor (cannot happen: there is one per buffer).
    fn post_rx(&mut self, slot: u16) -> bool {
        match self.rx.add(&[], &[rx_buffer(&self.rx_mem, slot)]) {
            Ok(head) => {
                self.rx_owner[head as usize] = slot;
                true
            }
            Err(AddError::Full | AddError::TooLong | AddError::NoBuffers) => false,
        }
    }

    fn kick_rx(&mut self) {
        if self.rx.should_notify() {
            self.rx_notifier.notify(&self.notify);
            self.stats.rx_kicks += 1;
        }
    }

    /// Receive buffer `slot`, `len` bytes from `offset` (both within the
    /// buffer: `len` comes from a used element, which the queue clamps to
    /// the buffer's length).
    fn rx_bytes(&self, slot: u16, offset: usize, len: usize) -> &[u8] {
        debug_assert!((slot as usize) < self.rx.size() as usize && offset + len <= BUF_LEN);
        let len = len.min(BUF_LEN - offset.min(BUF_LEN));
        // SAFETY: the buffer is inside the receive memory (`init`'s
        // contract: one BUF_LEN slot per queue entry), and the device has
        // handed it back (it came out of the used ring, after the queue's
        // Acquire fence), so it no longer writes there until the buffer is
        // posted again, which needs `&mut self`.
        unsafe { core::slice::from_raw_parts(self.rx_mem.ptr.as_ptr().add(slot as usize * BUF_LEN + offset), len) }
    }

    /// Copy the data of received buffer `u` from `offset` into `out` at
    /// `*at`, unless it does not fit (then `*fits` turns false), and post
    /// the buffer again.
    fn take_rx(&mut self, u: Used, offset: usize, out: &mut [u8], at: &mut usize, fits: &mut bool) {
        let slot = self.rx_owner[u.head as usize];
        let len = (u.len as usize).saturating_sub(offset);
        if *fits && *at + len <= out.len() {
            out[*at..*at + len].copy_from_slice(self.rx_bytes(slot, offset, len));
            *at += len;
        } else {
            *fits = false;
        }
        if !self.post_rx(slot) {
            self.fail();
        }
    }
}

impl<M: Mmio, C: Mmio, N: Mmio> NetDevice for VirtioNet<'_, M, C, N> {
    fn mac(&self) -> MacAddr {
        MacAddr(self.mac)
    }

    fn mtu(&self) -> usize {
        self.mtu
    }

    fn transmit(&mut self, frame: &[u8]) -> Result<(), TxError> {
        if frame.len() > self.mtu + ETHERNET_HEADER_LEN || HEADER_LEN + frame.len() > BUF_LEN {
            return Err(TxError::TooLong);
        }
        self.reclaim();
        if self.failed || self.tx_free_len == 0 {
            self.stats.tx_busy += 1;
            return Err(TxError::Busy);
        }
        self.tx_free_len -= 1;
        let slot = self.tx_free[self.tx_free_len as usize];
        let offset = slot as usize * BUF_LEN;
        let header = NetHeader::TRANSMIT.to_bytes();
        // SAFETY: the slot is inside the transmit memory (one BUF_LEN slot
        // per queue entry, `init`'s contract) and free: the device handed it
        // back or never had it, so nothing else accesses it. Header and
        // frame fit (checked above). The queue's Release fence orders these
        // writes before the device can see the buffer.
        unsafe {
            let p = self.tx_mem.ptr.as_ptr().add(offset);
            ptr::copy_nonoverlapping(header.as_ptr(), p, HEADER_LEN);
            ptr::copy_nonoverlapping(frame.as_ptr(), p.add(HEADER_LEN), frame.len());
        }
        let buf = Buffer { addr: self.tx_mem.bus + offset as u64, len: (HEADER_LEN + frame.len()) as u32 };
        match self.tx.add(&[buf], &[]) {
            Ok(head) => self.tx_owner[head as usize] = slot,
            Err(_) => {
                // One descriptor per buffer, so a free buffer means a free
                // descriptor; keep going if that ever does not hold.
                self.tx_free[self.tx_free_len as usize] = slot;
                self.tx_free_len += 1;
                self.stats.tx_busy += 1;
                return Err(TxError::Busy);
            }
        }
        self.stats.tx_packets += 1;
        self.stats.tx_bytes += frame.len() as u64;
        if self.tx.should_notify() {
            self.tx_notifier.notify(&self.notify);
            self.stats.tx_kicks += 1;
        }
        Ok(())
    }

    fn receive(&mut self, buf: &mut [u8]) -> Option<usize> {
        let mrg = self.has(feature::MRG_RXBUF);
        let mut taken = false;
        let result = loop {
            if self.failed {
                break None;
            }
            let first = match self.rx_pending.take() {
                Some(u) => u,
                None => match self.rx.pop_used() {
                    Ok(Some(u)) => u,
                    Ok(None) => break None,
                    Err(_) => {
                        self.fail();
                        break None;
                    }
                },
            };
            let slot = self.rx_owner[first.head as usize];
            let (mut at, mut fits) = (0, true);
            if (first.len as usize) < HEADER_LEN {
                // Not even a header: not a packet.
                self.stats.rx_errors += 1;
                self.take_rx(first, 0, &mut [], &mut at, &mut fits);
                taken = true;
                continue;
            }
            let header: [u8; HEADER_LEN] = self.rx_bytes(slot, 0, HEADER_LEN).try_into().unwrap_or([0; HEADER_LEN]);
            let count = if mrg { NetHeader::from_bytes(&header).num_buffers } else { 1 };
            if count == 0 || count > self.rx.size() {
                // A count no packet can have; the next buffers cannot be
                // told apart from new packets, so treat them as such.
                self.stats.rx_errors += 1;
                self.take_rx(first, 0, &mut [], &mut at, &mut fits);
                taken = true;
                continue;
            }
            if count > 1 && self.rx.used_pending() < count - 1 {
                // The rest of the packet is not back yet.
                self.rx_pending = Some(first);
                break None;
            }
            self.take_rx(first, HEADER_LEN, buf, &mut at, &mut fits);
            for _ in 1..count {
                match self.rx.pop_used() {
                    Ok(Some(u)) => self.take_rx(u, 0, buf, &mut at, &mut fits),
                    // `used_pending` counted it, so this is the device
                    // going back on its word.
                    Ok(None) | Err(_) => self.fail(),
                }
            }
            taken = true;
            if self.failed {
                break None;
            }
            if !fits {
                self.stats.rx_dropped += 1;
                continue;
            }
            if at < ETHERNET_HEADER_LEN {
                self.stats.rx_errors += 1;
                continue;
            }
            self.stats.rx_packets += 1;
            self.stats.rx_bytes += at as u64;
            break Some(at);
        };
        if taken && !self.failed {
            self.kick_rx();
        }
        result
    }

    fn can_transmit(&self) -> bool {
        !self.failed && (self.tx_free_len > 0 || self.tx.has_used())
    }
}
