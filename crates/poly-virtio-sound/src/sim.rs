//! A simulated virtio sound device for the unit tests, written from the
//! device requirements of VIRTIO 1.2 (§2.7 split virtqueues, §4.1.4.3
//! common configuration, §5.14 sound device), not from the driver code.
//!
//! One shared state sits behind three register blocks: the common
//! configuration, the device configuration and the notification region.
//! Control requests are answered when the control queue is notified.
//! Output messages wait in the device until a test "plays" them
//! ([`Sim::play`]): the frames are appended to the stream's `played`
//! bytes and the message comes back with an OK status. Events are written
//! into the driver's event buffers by [`Sim::send_event`]. Bus addresses
//! are host addresses.

extern crate std;

use crate::proto::{
    ChmapInfo, Event, JackInfo, PcmInfo, PcmStatus, SetParams, XFER_LEN, code, format, rate, status as snd,
};
use core::cell::{Cell, RefCell};
use core::sync::atomic::{Ordering, fence};
use poly_pci::Mmio;
use poly_virtio::status;
use std::collections::VecDeque;
use std::rc::Rc;
use std::{alloc, vec, vec::Vec};

/// A zeroed, page-aligned allocation standing in for DMA memory.
#[derive(Debug)]
pub struct DmaMemory {
    pub ptr: core::ptr::NonNull<u8>,
    pub len: usize,
    layout: alloc::Layout,
}

impl DmaMemory {
    pub fn new(len: usize) -> DmaMemory {
        let layout = alloc::Layout::from_size_align(len.max(1), 4096).unwrap();
        // SAFETY: the layout has a non-zero size.
        let ptr = core::ptr::NonNull::new(unsafe { alloc::alloc_zeroed(layout) }).unwrap();
        DmaMemory { ptr, len, layout }
    }

    pub fn region(&self) -> crate::DmaRegion {
        crate::DmaRegion { ptr: self.ptr, bus: self.ptr.as_ptr() as u64, len: self.len }
    }
}

impl Drop for DmaMemory {
    fn drop(&mut self) {
        // SAFETY: allocated in `new` with this layout.
        unsafe { alloc::dealloc(self.ptr.as_ptr(), self.layout) }
    }
}

fn rd<T: Copy>(addr: u64) -> T {
    // SAFETY: the driver handed these addresses to the device; tests keep
    // the memory alive while the device works.
    unsafe { core::ptr::read_volatile(addr as *const T) }
}

fn wr<T: Copy>(addr: u64, v: T) {
    // SAFETY: as for `rd`.
    unsafe { core::ptr::write_volatile(addr as *mut T, v) }
}

fn read_bytes(addr: u64, len: u32) -> Vec<u8> {
    // SAFETY: as for `rd`: a buffer the driver lent the device.
    unsafe { core::slice::from_raw_parts(addr as *const u8, len as usize) }.to_vec()
}

fn write_bytes(addr: u64, bytes: &[u8]) {
    // SAFETY: as for `rd`: a device-writable buffer the driver lent us.
    unsafe { core::ptr::copy_nonoverlapping(bytes.as_ptr(), addr as *mut u8, bytes.len()) }
}

/// §2.7.7 / §2.7.10: has the index crossed `event` going from `old` to `new`?
fn need_event(event: u16, new: u16, old: u16) -> bool {
    new.wrapping_sub(event).wrapping_sub(1) < new.wrapping_sub(old)
}

#[derive(Clone, Copy, Debug, Default)]
pub struct QueueRegs {
    pub max_size: u16,
    pub size: u16,
    pub msix: u16,
    pub enable: u16,
    pub desc: u64,
    pub driver: u64,
    pub device: u64,
}

/// The device side of one queue.
#[derive(Debug, Default)]
pub struct QueueState {
    last_avail: u16,
    used_idx: u16,
    /// Used buffer notifications the device would have sent.
    pub interrupts: u32,
    pub notifies: u32,
}

/// A descriptor chain: (address, length, device-writable) per descriptor.
type Chain = (u16, Vec<(u64, u32, bool)>);

/// The lifecycle of a stream, from the device's side (§5.14.6.6.1).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DevState {
    #[default]
    Fresh,
    Params,
    Prepared,
    Running,
    Stopped,
    Released,
}

#[derive(Debug, Default)]
pub struct SimStream {
    pub info: PcmInfo,
    pub state: DevState,
    pub params: Option<SetParams>,
    /// Frames of every output message played, in order.
    pub played: Vec<u8>,
    /// Sizes of the messages played.
    pub messages: Vec<u32>,
}

/// An input message waiting in the device: its header's stream, its data
/// buffer and its status.
#[derive(Debug)]
struct RxPending {
    head: u16,
    stream: u32,
    buf: (u64, u32),
    status_addr: u64,
}

/// An output message waiting in the device.
#[derive(Debug)]
struct Pending {
    head: u16,
    stream: u32,
    data: Vec<u8>,
    status_addr: u64,
}

#[derive(Debug)]
pub struct Device {
    pub queues: [QueueState; 4],
    pub jacks: Vec<JackInfo>,
    pub streams: Vec<SimStream>,
    pub chmaps: Vec<ChmapInfo>,
    /// Every control request, by code.
    pub requests: Vec<u32>,
    /// Answer control requests at all (false: a hung device).
    pub answer: bool,
    /// Answer the next request with this status instead.
    pub force_status: Option<u32>,
    /// Hand back this id in the next used element (a broken device).
    pub bogus_id: Option<u32>,
    /// Write only this many status bytes in the next played message.
    pub short_status: Option<u32>,
    /// Report a used length of just the header for control responses (as
    /// QEMU 8.2 does for item queries).
    pub short_control: bool,
    pending: VecDeque<Pending>,
    /// Input messages waiting to be filled.
    rx_pending: VecDeque<RxPending>,
    /// Report this used length for the next filled input message (a
    /// device that reports less than it wrote).
    pub rx_used_len: Option<u32>,
    /// Event buffers waiting for events: (head, address, length).
    events: VecDeque<(u16, u64, u32)>,
}

#[derive(Debug)]
pub struct Inner {
    pub offered: u64,
    pub status: Cell<u8>,
    pub driver_features: Cell<u64>,
    dfselect: Cell<u32>,
    gfselect: Cell<u32>,
    queue_select: Cell<u16>,
    pub queues: RefCell<Vec<QueueRegs>>,
    pub config_msix: Cell<u16>,
    pub generation: Cell<u8>,
    pub config: RefCell<Vec<u8>>,
    pub dev: RefCell<Device>,
}

/// Handles on the shared state, one per register block.
#[derive(Clone, Debug)]
pub struct Sim(pub Rc<Inner>);
#[derive(Debug)]
pub struct CommonRegs(pub Rc<Inner>);
#[derive(Debug)]
pub struct ConfigRegs(pub Rc<Inner>);
#[derive(Debug)]
pub struct NotifyRegs(pub Rc<Inner>);

/// Length of the configuration structure the simulated device offers.
pub const CONFIG_LEN: u32 = 12;

/// An output stream like QEMU's: S16 and a few others, 8 to 48 kHz,
/// 1 or 2 channels.
pub fn output_stream() -> PcmInfo {
    let formats = [format::U8, format::S16, format::S32, format::FLOAT];
    let rates = [1u8, 4, 6, 7];
    PcmInfo {
        hda_fn_nid: 0,
        features: 0,
        formats: formats.iter().fold(0, |m, &f| m | 1 << f),
        rates: rates.iter().fold(0, |m, &r| m | 1 << r),
        direction: crate::proto::direction::OUTPUT,
        channels_min: 1,
        channels_max: 2,
    }
}

impl Sim {
    /// A device offering `offered` with queues of at most `max_queue`
    /// entries, and these streams, jacks and channel maps.
    pub fn new(
        offered: u64,
        max_queue: u16,
        streams: Vec<PcmInfo>,
        jacks: Vec<JackInfo>,
        chmaps: Vec<ChmapInfo>,
    ) -> Sim {
        let mut config = vec![0u8; CONFIG_LEN as usize];
        config[0..4].copy_from_slice(&(jacks.len() as u32).to_le_bytes());
        config[4..8].copy_from_slice(&(streams.len() as u32).to_le_bytes());
        config[8..12].copy_from_slice(&(chmaps.len() as u32).to_le_bytes());
        let q = QueueRegs { max_size: max_queue, size: max_queue, msix: 0xffff, ..Default::default() };
        Sim(Rc::new(Inner {
            offered,
            status: Cell::new(0),
            driver_features: Cell::new(0),
            dfselect: Cell::new(0),
            gfselect: Cell::new(0),
            queue_select: Cell::new(0),
            queues: RefCell::new(vec![q; 4]),
            config_msix: Cell::new(0xffff),
            generation: Cell::new(0),
            config: RefCell::new(config),
            dev: RefCell::new(Device {
                queues: Default::default(),
                jacks,
                streams: streams.into_iter().map(|info| SimStream { info, ..Default::default() }).collect(),
                chmaps,
                requests: Vec::new(),
                answer: true,
                force_status: None,
                bogus_id: None,
                short_status: None,
                short_control: false,
                pending: VecDeque::new(),
                rx_pending: VecDeque::new(),
                rx_used_len: None,
                events: VecDeque::new(),
            }),
        }))
    }

    pub fn common(&self) -> CommonRegs {
        CommonRegs(self.0.clone())
    }

    pub fn config(&self) -> ConfigRegs {
        ConfigRegs(self.0.clone())
    }

    pub fn notify(&self) -> NotifyRegs {
        NotifyRegs(self.0.clone())
    }

    pub fn dev(&self) -> std::cell::RefMut<'_, Device> {
        self.0.dev.borrow_mut()
    }

    pub fn negotiated(&self, f: u64) -> bool {
        self.0.driver_features.get() & f != 0
    }

    pub fn queue(&self, i: usize) -> QueueRegs {
        self.0.queues.borrow()[i]
    }

    fn live(&self, q: &QueueRegs) -> bool {
        q.enable != 0 && self.0.status.get() & status::DRIVER_OK != 0
    }

    fn event_idx(&self) -> bool {
        self.negotiated(poly_virtio::feature::EVENT_IDX)
    }

    /// The next available chain of queue `q`, if any.
    fn take_chain(q: &QueueRegs, st: &mut QueueState, event_idx: bool) -> Option<Chain> {
        if rd::<u16>(q.driver + 2) == st.last_avail {
            return None;
        }
        fence(Ordering::Acquire);
        let head = rd::<u16>(q.driver + 4 + 2 * (st.last_avail % q.size) as u64);
        st.last_avail = st.last_avail.wrapping_add(1);
        if event_idx {
            wr(q.device + 4 + 8 * q.size as u64, st.last_avail);
        }
        let mut descs = Vec::new();
        let mut i = head;
        loop {
            assert!(i < q.size, "descriptor index out of range");
            assert!(descs.len() < q.size as usize, "descriptor loop");
            let d = q.desc + 16 * i as u64;
            let (addr, len, flags, next) = (rd::<u64>(d), rd::<u32>(d + 8), rd::<u16>(d + 12), rd::<u16>(d + 14));
            descs.push((addr, len, flags & 2 != 0));
            if flags & 1 == 0 {
                break;
            }
            i = next;
        }
        Some((head, descs))
    }

    /// Write and publish a used element, counting the interrupt the device
    /// would send unless the driver suppressed it (§2.7.7).
    fn put_used(q: &QueueRegs, st: &mut QueueState, bogus: &mut Option<u32>, head: u16, len: u32, event_idx: bool) {
        let id = bogus.take().unwrap_or(head as u32);
        let e = q.device + 4 + 8 * (st.used_idx % q.size) as u64;
        wr(e, id);
        wr(e + 4, len);
        st.used_idx = st.used_idx.wrapping_add(1);
        fence(Ordering::Release);
        let old = rd::<u16>(q.device + 2);
        wr(q.device + 2, st.used_idx);
        fence(Ordering::SeqCst);
        let suppressed = if event_idx {
            !need_event(rd::<u16>(q.driver + 4 + 2 * q.size as u64), st.used_idx, old)
        } else {
            rd::<u16>(q.driver) & 1 != 0
        };
        if !suppressed {
            st.interrupts += 1;
        }
    }

    /// Answer every control request waiting.
    fn control(&self) {
        let q = self.queue(0);
        if !self.live(&q) {
            return;
        }
        let event_idx = self.event_idx();
        let mut dev = self.0.dev.borrow_mut();
        let dev = &mut *dev;
        if !dev.answer {
            return;
        }
        while let Some((head, descs)) = Self::take_chain(&q, &mut dev.queues[0], event_idx) {
            let mut req = Vec::new();
            let mut out = Vec::new();
            for &(addr, len, writable) in &descs {
                if writable {
                    out.push((addr, len));
                } else {
                    assert!(out.is_empty(), "a readable descriptor after a writable one");
                    req.extend_from_slice(&read_bytes(addr, len));
                }
            }
            let room: u32 = out.iter().map(|o| o.1).sum();
            let mut resp = Self::answer(dev, &req, room as usize);
            if let Some(s) = dev.force_status.take() {
                resp = s.to_le_bytes().to_vec();
            }
            resp.truncate(room as usize);
            let mut at = 0;
            for (addr, len) in out {
                let k = (len as usize).min(resp.len() - at);
                write_bytes(addr, &resp[at..at + k]);
                at += k;
            }
            let used = if dev.short_control { at.min(4) } else { at };
            Self::put_used(&q, &mut dev.queues[0], &mut dev.bogus_id, head, used as u32, event_idx);
        }
    }

    /// The response to `req`, given `room` bytes for it.
    fn answer(dev: &mut Device, req: &[u8], room: usize) -> Vec<u8> {
        let status = |s: u32| s.to_le_bytes().to_vec();
        if req.len() < 4 {
            return status(snd::BAD_MSG);
        }
        let le = |at: usize| u32::from_le_bytes(req[at..at + 4].try_into().unwrap());
        let c = le(0);
        dev.requests.push(c);
        match c {
            code::JACK_INFO | code::PCM_INFO | code::CHMAP_INFO => {
                if req.len() < 16 {
                    return status(snd::BAD_MSG);
                }
                let (start, count, size) = (le(4) as usize, le(8) as usize, le(12) as usize);
                let (items, len): (Vec<Vec<u8>>, usize) = match c {
                    code::JACK_INFO => (dev.jacks.iter().map(|j| j.encode().to_vec()).collect(), JackInfo::LEN),
                    code::PCM_INFO => (dev.streams.iter().map(|s| s.info.encode().to_vec()).collect(), PcmInfo::LEN),
                    _ => (dev.chmaps.iter().map(|m| m.encode().to_vec()).collect(), ChmapInfo::LEN),
                };
                if size != len || start + count > items.len() || room < 4 + count * size {
                    return status(snd::BAD_MSG);
                }
                let mut r = status(snd::OK);
                for item in &items[start..start + count] {
                    r.extend_from_slice(item);
                }
                r
            }
            code::PCM_SET_PARAMS => {
                let Some((id, p)) = SetParams::parse(req) else { return status(snd::BAD_MSG) };
                let Some(s) = dev.streams.get_mut(id as usize) else { return status(snd::BAD_MSG) };
                if !matches!(s.state, DevState::Fresh | DevState::Params | DevState::Prepared | DevState::Released) {
                    return status(snd::BAD_MSG);
                }
                let fmt_ok = p.format < 64 && s.info.formats & (1 << p.format) != 0;
                let rate_ok = p.rate < 64 && s.info.rates & (1 << p.rate) != 0 && rate::hz_of(p.rate).is_some();
                let ch_ok = p.channels >= s.info.channels_min && p.channels <= s.info.channels_max;
                let sizes_ok = p.period_bytes > 0 && p.buffer_bytes.is_multiple_of(p.period_bytes);
                if !(fmt_ok && rate_ok && ch_ok && sizes_ok) || p.features & !s.info.features != 0 {
                    return status(snd::NOT_SUPP);
                }
                s.params = Some(p);
                s.state = DevState::Params;
                status(snd::OK)
            }
            code::PCM_PREPARE | code::PCM_START | code::PCM_STOP | code::PCM_RELEASE => {
                if req.len() < 8 {
                    return status(snd::BAD_MSG);
                }
                let id = le(4);
                let Some(s) = dev.streams.get_mut(id as usize) else { return status(snd::BAD_MSG) };
                use DevState::*;
                let (ok, next) = match c {
                    code::PCM_PREPARE => (matches!(s.state, Params | Prepared | Released), Prepared),
                    code::PCM_START => (matches!(s.state, Prepared | Stopped), Running),
                    code::PCM_STOP => (s.state == Running, Stopped),
                    _ => (matches!(s.state, Prepared | Stopped), Released),
                };
                if !ok {
                    return status(snd::BAD_MSG);
                }
                s.state = next;
                if c == code::PCM_RELEASE {
                    // §5.14.6.6.5: release completes the stream's pending
                    // messages (they are returned below, after the lock on
                    // the request queue is given up).
                    for p in dev.pending.iter_mut().filter(|p| p.stream == id) {
                        p.stream |= RELEASED;
                    }
                    for p in dev.rx_pending.iter_mut().filter(|p| p.stream == id) {
                        p.stream |= RELEASED;
                    }
                }
                status(snd::OK)
            }
            _ => status(snd::NOT_SUPP),
        }
    }

    /// Take every output message the driver queued into the device.
    fn take_tx(&self) {
        let q = self.queue(2);
        if !self.live(&q) {
            return;
        }
        let event_idx = self.event_idx();
        let mut dev = self.0.dev.borrow_mut();
        let dev = &mut *dev;
        while let Some((head, descs)) = Self::take_chain(&q, &mut dev.queues[2], event_idx) {
            let mut bytes = Vec::new();
            let mut status_addr = None;
            for &(addr, len, writable) in &descs {
                if writable {
                    assert!(status_addr.is_none() && len as usize >= PcmStatus::LEN, "bad status descriptor");
                    status_addr = Some(addr);
                } else {
                    assert!(status_addr.is_none(), "a readable descriptor after the status");
                    bytes.extend_from_slice(&read_bytes(addr, len));
                }
            }
            let status_addr = status_addr.expect("an output message without a status");
            assert!(bytes.len() >= XFER_LEN, "an output message without a header");
            let stream = u32::from_le_bytes(bytes[..4].try_into().unwrap());
            dev.pending.push_back(Pending { head, stream, data: bytes[XFER_LEN..].to_vec(), status_addr });
        }
    }

    /// Return released messages (with an OK status: §5.14.6.6.5 does not
    /// make them errors).
    fn flush_released(&self) {
        let q = self.queue(2);
        let event_idx = self.event_idx();
        let mut dev = self.0.dev.borrow_mut();
        let dev = &mut *dev;
        while let Some(i) = dev.pending.iter().position(|p| p.stream & RELEASED != 0) {
            let p = dev.pending.remove(i).unwrap();
            write_bytes(p.status_addr, &PcmStatus { status: snd::OK, latency_bytes: 0 }.encode());
            Self::put_used(&q, &mut dev.queues[2], &mut dev.bogus_id, p.head, PcmStatus::LEN as u32, event_idx);
        }
        let q = self.queue(3);
        while let Some(i) = dev.rx_pending.iter().position(|p| p.stream & RELEASED != 0) {
            let p = dev.rx_pending.remove(i).unwrap();
            write_bytes(p.status_addr, &PcmStatus { status: snd::OK, latency_bytes: 0 }.encode());
            Self::put_used(&q, &mut dev.queues[3], &mut dev.bogus_id, p.head, PcmStatus::LEN as u32, event_idx);
        }
    }

    /// Take every input message the driver queued into the device.
    fn take_rx(&self) {
        let q = self.queue(3);
        if !self.live(&q) {
            return;
        }
        let event_idx = self.event_idx();
        let mut dev = self.0.dev.borrow_mut();
        let dev = &mut *dev;
        while let Some((head, descs)) = Self::take_chain(&q, &mut dev.queues[3], event_idx) {
            // §5.14.6.8: a readable header, then the writable buffer and
            // the writable status.
            assert!(descs.len() == 3, "an input message is header, buffer, status");
            assert!(!descs[0].2 && descs[1].2 && descs[2].2, "readable header, writable buffer and status");
            assert!(descs[0].1 as usize >= XFER_LEN && descs[2].1 as usize >= PcmStatus::LEN);
            let stream = u32::from_le_bytes(read_bytes(descs[0].0, 4).try_into().unwrap());
            dev.rx_pending.push_back(RxPending {
                head,
                stream,
                buf: (descs[1].0, descs[1].1),
                status_addr: descs[2].0,
            });
        }
    }

    /// Input messages waiting in the device.
    pub fn rx_pending(&self) -> usize {
        self.0.dev.borrow().rx_pending.len()
    }

    /// Fill up to `n` waiting input messages of running input streams,
    /// oldest first, with bytes from `source(stream, buffer)`; returns how
    /// many.
    pub fn record(&self, n: usize, mut source: impl FnMut(u32, &mut [u8])) -> usize {
        let q = self.queue(3);
        if !self.live(&q) {
            return 0;
        }
        let event_idx = self.event_idx();
        let mut dev = self.0.dev.borrow_mut();
        let dev = &mut *dev;
        let mut done = 0;
        while done < n {
            let running =
                |p: &RxPending| dev.streams.get(p.stream as usize).is_some_and(|s| s.state == DevState::Running);
            let Some(i) = dev.rx_pending.iter().position(running) else { break };
            let p = dev.rx_pending.remove(i).unwrap();
            let mut data = vec![0u8; p.buf.1 as usize];
            source(p.stream, &mut data);
            write_bytes(p.buf.0, &data);
            write_bytes(p.status_addr, &PcmStatus { status: snd::OK, latency_bytes: 0 }.encode());
            let len = dev.rx_used_len.take().unwrap_or(p.buf.1 + PcmStatus::LEN as u32);
            Self::put_used(&q, &mut dev.queues[3], &mut dev.bogus_id, p.head, len, event_idx);
            done += 1;
        }
        done
    }

    /// Output messages waiting in the device.
    pub fn pending(&self) -> usize {
        self.0.dev.borrow().pending.len()
    }

    /// Play up to `n` waiting messages of running streams, oldest first;
    /// returns how many.
    pub fn play(&self, n: usize) -> usize {
        let q = self.queue(2);
        if !self.live(&q) {
            return 0;
        }
        let event_idx = self.event_idx();
        let mut dev = self.0.dev.borrow_mut();
        let dev = &mut *dev;
        let mut done = 0;
        while done < n {
            let running =
                |p: &Pending| dev.streams.get(p.stream as usize).is_some_and(|s| s.state == DevState::Running);
            let Some(i) = dev.pending.iter().position(running) else { break };
            let p = dev.pending.remove(i).unwrap();
            let s = &mut dev.streams[p.stream as usize];
            s.played.extend_from_slice(&p.data);
            s.messages.push(p.data.len() as u32);
            let len = dev.short_status.take().unwrap_or(PcmStatus::LEN as u32);
            write_bytes(p.status_addr, &PcmStatus { status: snd::OK, latency_bytes: 0 }.encode()[..len as usize]);
            Self::put_used(&q, &mut dev.queues[2], &mut dev.bogus_id, p.head, len, event_idx);
            done += 1;
        }
        done
    }

    /// Note the driver's new event buffers.
    fn take_events(&self) {
        let q = self.queue(1);
        if !self.live(&q) {
            return;
        }
        let event_idx = self.event_idx();
        let mut dev = self.0.dev.borrow_mut();
        let dev = &mut *dev;
        while let Some((head, descs)) = Self::take_chain(&q, &mut dev.queues[1], event_idx) {
            assert!(descs.len() == 1 && descs[0].2, "an event buffer must be one device-writable descriptor");
            dev.events.push_back((head, descs[0].0, descs[0].1));
        }
    }

    /// Send `e` in the next event buffer; false if the driver gave none.
    pub fn send_event(&self, e: Event) -> bool {
        self.take_events();
        let q = self.queue(1);
        let event_idx = self.event_idx();
        let mut dev = self.0.dev.borrow_mut();
        let dev = &mut *dev;
        let Some((head, addr, len)) = dev.events.pop_front() else { return false };
        assert!(len as usize >= Event::LEN);
        write_bytes(addr, &e.encode());
        Self::put_used(&q, &mut dev.queues[1], &mut dev.bogus_id, head, Event::LEN as u32, event_idx);
        true
    }

    /// Event buffers the device holds.
    pub fn event_buffers(&self) -> usize {
        self.take_events();
        self.0.dev.borrow().events.len()
    }

    pub fn reset_device(&self) {
        for q in self.0.queues.borrow_mut().iter_mut() {
            *q = QueueRegs { max_size: q.max_size, size: q.max_size, msix: 0xffff, ..Default::default() };
        }
        self.0.driver_features.set(0);
        let mut dev = self.0.dev.borrow_mut();
        dev.queues = Default::default();
        dev.pending.clear();
        dev.rx_pending.clear();
        dev.events.clear();
        for s in &mut dev.streams {
            s.state = DevState::Fresh;
        }
    }
}

/// Marks a pending message whose stream was released.
const RELEASED: u32 = 1 << 31;

impl Mmio for CommonRegs {
    fn read8(&self, offset: usize) -> u8 {
        match offset {
            0x14 => self.0.status.get(),
            0x15 => self.0.generation.get(),
            _ => panic!("8-bit read at {offset:#x}"),
        }
    }

    fn read16(&self, offset: usize) -> u16 {
        let sel = self.0.queue_select.get() as usize;
        let qs = self.0.queues.borrow();
        let q = qs.get(sel).copied();
        match offset {
            0x10 => self.0.config_msix.get(),
            0x12 => qs.len() as u16,
            0x16 => sel as u16,
            0x18 => q.map_or(0, |q| q.size),
            0x1a => q.map_or(0xffff, |q| q.msix),
            0x1c => q.map_or(0, |q| q.enable),
            0x1e => sel as u16,
            _ => panic!("16-bit read at {offset:#x}"),
        }
    }

    fn read32(&self, offset: usize) -> u32 {
        match offset {
            0x00 => self.0.dfselect.get(),
            0x04 => match self.0.dfselect.get() {
                0 => self.0.offered as u32,
                1 => (self.0.offered >> 32) as u32,
                _ => 0,
            },
            0x08 => self.0.gfselect.get(),
            _ => panic!("32-bit read at {offset:#x}"),
        }
    }

    fn write8(&self, offset: usize, value: u8) {
        assert_eq!(offset, 0x14, "8-bit write at {offset:#x}");
        if value == 0 {
            self.0.status.set(0);
            Sim(self.0.clone()).reset_device();
            return;
        }
        let mut value = value;
        if value & status::FEATURES_OK != 0 && self.0.driver_features.get() & !self.0.offered != 0 {
            value &= !status::FEATURES_OK;
        }
        self.0.status.set(value);
    }

    fn write16(&self, offset: usize, value: u16) {
        let sel = self.0.queue_select.get() as usize;
        let mut qs = self.0.queues.borrow_mut();
        match offset {
            0x10 => self.0.config_msix.set(if value < 3 { value } else { 0xffff }),
            0x16 => self.0.queue_select.set(value),
            0x18 => qs[sel].size = value,
            0x1a => qs[sel].msix = if value < 3 { value } else { 0xffff },
            0x1c => qs[sel].enable = value,
            _ => panic!("16-bit write at {offset:#x}"),
        }
    }

    fn write32(&self, offset: usize, value: u32) {
        let lo = |old: u64| (old & !0xffff_ffff) | value as u64;
        let hi = |old: u64| (old & 0xffff_ffff) | (value as u64) << 32;
        let sel = self.0.queue_select.get() as usize;
        let mut qs = self.0.queues.borrow_mut();
        match offset {
            0x00 => self.0.dfselect.set(value),
            0x08 => self.0.gfselect.set(value),
            0x0c => {
                let old = self.0.driver_features.get();
                self.0.driver_features.set(if self.0.gfselect.get() == 0 { lo(old) } else { hi(old) });
            }
            0x20 => qs[sel].desc = lo(qs[sel].desc),
            0x24 => qs[sel].desc = hi(qs[sel].desc),
            0x28 => qs[sel].driver = lo(qs[sel].driver),
            0x2c => qs[sel].driver = hi(qs[sel].driver),
            0x30 => qs[sel].device = lo(qs[sel].device),
            0x34 => qs[sel].device = hi(qs[sel].device),
            _ => panic!("32-bit write at {offset:#x}"),
        }
    }
}

impl Mmio for ConfigRegs {
    fn read8(&self, offset: usize) -> u8 {
        self.0.config.borrow()[offset]
    }

    fn read16(&self, offset: usize) -> u16 {
        u16::from_le_bytes(self.0.config.borrow()[offset..offset + 2].try_into().unwrap())
    }

    fn read32(&self, offset: usize) -> u32 {
        u32::from_le_bytes(self.0.config.borrow()[offset..offset + 4].try_into().unwrap())
    }

    fn write8(&self, offset: usize, _: u8) {
        panic!("configuration write at {offset:#x}");
    }

    fn write16(&self, offset: usize, _: u16) {
        panic!("configuration write at {offset:#x}");
    }

    fn write32(&self, offset: usize, _: u32) {
        panic!("configuration write at {offset:#x}");
    }
}

impl Mmio for NotifyRegs {
    fn read8(&self, _: usize) -> u8 {
        unimplemented!()
    }

    fn read16(&self, _: usize) -> u16 {
        unimplemented!()
    }

    fn read32(&self, _: usize) -> u32 {
        unimplemented!()
    }

    fn write8(&self, _: usize, _: u8) {
        unimplemented!()
    }

    /// Queue `q`'s register is at `4 * q` (notify_off = q, multiplier 4).
    fn write16(&self, offset: usize, value: u16) {
        assert_eq!(offset, 4 * value as usize, "notification register and queue index disagree");
        let sim = Sim(self.0.clone());
        match value {
            0 => {
                sim.dev().queues[0].notifies += 1;
                sim.control();
                sim.flush_released();
            }
            1 => {
                sim.dev().queues[1].notifies += 1;
                sim.take_events();
            }
            2 => {
                sim.dev().queues[2].notifies += 1;
                sim.take_tx();
            }
            3 => {
                sim.dev().queues[3].notifies += 1;
                sim.take_rx();
            }
            q => panic!("notification for queue {q}"),
        }
    }

    fn write32(&self, _: usize, _: u32) {
        unimplemented!()
    }
}
