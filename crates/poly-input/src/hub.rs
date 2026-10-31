//! The input event layer: every input device, its state, the readers of
//! its events and the kernel console's keyboard queue.
//!
//! ```text
//!  driver ──report(dev, type, code, value)──▶ Device: capabilities, key/LED/axis state,
//!   (PS/2, virtio-input)                        the frame being built, soft repeat
//!                                                   │ SYN_REPORT
//!                     ┌─────────────────────────────┼──────────────────────────────┐
//!                     ▼                             ▼                              ▼
//!            Client queue (each open         Client queue ...              console keys (every keyboard,
//!            /dev/input/eventN)                                            unless a client grabbed it)
//! ```
//!
//! The vocabulary is evdev's ([`crate::codes`]): a device reports events
//! of a type and code with a value, and a `SYN_REPORT` ends a frame of
//! events that happened at once. The hub follows the semantics the Linux
//! input documentation gives the event interface (`input.rst`,
//! `event-codes.rst`), so `/dev/input/event*` can be served from it:
//!
//! - Events a device does not declare are dropped, and so are events that
//!   change nothing: a release of a key that is up, a press of a key that
//!   is down (reported as an auto-repeat, value 2, by a device without
//!   soft repeat), relative motion of 0, an absolute axis or LED set to
//!   the value it has.
//! - A frame is delivered at its `SYN_REPORT`, to every client, or only to
//!   the client that grabbed the device (`EVIOCGRAB`). An empty frame is
//!   not delivered.
//! - A client whose queue cannot take a frame loses what it had queued and
//!   gets `SYN_DROPPED` first: it should skip to the next `SYN_REPORT` and
//!   ask for the state with the `EVIOCG*` ioctls.
//! - A device with soft repeat (`EV_REP`, the driver asks for it) repeats
//!   the last key pressed while it is held: value 2 after the delay, then
//!   every period ([`Hub::tick`], driven by a timer).
//! - Keys of devices marked as console keyboards also go to one console
//!   queue, from every such device, which the kernel's own terminal
//!   reads; a grab takes the device's keys away from it too.
//!
//! The hub never allocates while reporting: queues are made when a client
//! opens (by the caller, [`Queue::new`]), frames have a fixed capacity.
//! So drivers may report from interrupt handlers, with the hub under an
//! interrupt-safe lock. Times are the monotonic clock in nanoseconds.

use crate::codes::{self, KEY_CNT, abs, ev, key, led, rep, syn};
use crate::{Key, keycode};
use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

/// One input event.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Event {
    /// When it happened: the monotonic clock, in nanoseconds.
    pub time_ns: u64,
    /// `EV_*`.
    pub kind: u16,
    pub code: u16,
    pub value: i32,
}

impl Event {
    pub const fn new(time_ns: u64, kind: u16, code: u16, value: i32) -> Event {
        Event { time_ns, kind, code, value }
    }

    pub const fn syn(time_ns: u64) -> Event {
        Event::new(time_ns, ev::SYN, syn::REPORT, 0)
    }
}

/// `struct input_id`: what kind of device this is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct InputId {
    /// `BUS_*`.
    pub bustype: u16,
    pub vendor: u16,
    pub product: u16,
    pub version: u16,
}

/// `struct input_absinfo`: an absolute axis's range and current value.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct AbsInfo {
    pub value: i32,
    pub minimum: i32,
    pub maximum: i32,
    pub fuzz: i32,
    pub flat: i32,
    /// Units per millimetre (per radian for rotation axes).
    pub resolution: i32,
}

/// A set of bits `0..64 * W`, stored as the little-endian array of 64-bit
/// words the evdev bitmaps are on x86-64 (bit `n` is bit `n % 8` of byte
/// `n / 8`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Bits<const W: usize>(pub [u64; W]);

impl<const W: usize> Default for Bits<W> {
    fn default() -> Self {
        Bits([0; W])
    }
}

impl<const W: usize> Bits<W> {
    pub const LEN: usize = 64 * W;

    pub const fn new() -> Self {
        Bits([0; W])
    }

    pub fn get(&self, bit: u16) -> bool {
        let b = usize::from(bit);
        b < Self::LEN && self.0[b / 64] & (1 << (b % 64)) != 0
    }

    pub fn set(&mut self, bit: u16, on: bool) {
        let b = usize::from(bit);
        if b >= Self::LEN {
            return;
        }
        if on {
            self.0[b / 64] |= 1 << (b % 64);
        } else {
            self.0[b / 64] &= !(1 << (b % 64));
        }
    }

    pub fn any(&self) -> bool {
        self.0.iter().any(|&w| w != 0)
    }

    /// The bits, as bytes.
    pub fn to_bytes(&self) -> Vec<u8> {
        self.0.iter().flat_map(|w| w.to_le_bytes()).collect()
    }

    /// The bits set, in order.
    pub fn iter(&self) -> impl Iterator<Item = u16> + '_ {
        (0..Self::LEN as u16).filter(|&b| self.get(b))
    }
}

/// What a device can report: one bitmap per event type, plus its
/// properties.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Capabilities {
    /// Event types (`EV_*`).
    pub ev: Bits<1>,
    pub key: Bits<{ KEY_CNT / 64 }>,
    pub rel: Bits<1>,
    pub abs: Bits<1>,
    pub msc: Bits<1>,
    pub led: Bits<1>,
    pub sw: Bits<1>,
    pub snd: Bits<1>,
    pub ff: Bits<2>,
    /// `INPUT_PROP_*`.
    pub prop: Bits<1>,
}

impl Capabilities {
    /// Whether events of `kind` with `code` are declared.
    pub fn has(&self, kind: u16, code: u16) -> bool {
        if !self.ev.get(kind) {
            return false;
        }
        match kind {
            ev::SYN | ev::REP => true,
            ev::KEY => self.key.get(code),
            ev::REL => self.rel.get(code),
            ev::ABS => self.abs.get(code),
            ev::MSC => self.msc.get(code),
            ev::LED => self.led.get(code),
            ev::SW => self.sw.get(code),
            ev::SND => self.snd.get(code),
            ev::FF => self.ff.get(code),
            _ => false,
        }
    }

    /// The bitmap of codes of `kind` as `EVIOCGBIT` returns it (for type 0,
    /// the event types), or `None` for a type the interface does not have.
    /// Each is a whole number of 64-bit words long.
    pub fn bitmap(&self, kind: u16) -> Option<Vec<u8>> {
        Some(match kind {
            0 => self.ev.to_bytes(),
            ev::KEY => self.key.to_bytes(),
            ev::REL => self.rel.to_bytes(),
            ev::ABS => self.abs.to_bytes(),
            ev::MSC => self.msc.to_bytes(),
            ev::LED => self.led.to_bytes(),
            ev::SW => self.sw.to_bytes(),
            ev::SND => self.snd.to_bytes(),
            ev::FF => self.ff.to_bytes(),
            // Types without codes of their own (EV_REP, EV_PWR, ...).
            ev::REP | ev::PWR | ev::FF_STATUS => Bits::<1>::new().to_bytes(),
            _ => return None,
        })
    }
}

/// A device as its driver describes it.
#[derive(Clone, Debug)]
pub struct DeviceInfo {
    pub name: String,
    /// Where the device is attached (`EVIOCGPHYS`); empty if unknown.
    pub phys: String,
    /// A unique identifier, a serial number (`EVIOCGUNIQ`); empty if none.
    pub uniq: String,
    pub id: InputId,
    pub caps: Capabilities,
    /// Each absolute axis's range (and, in the hub, current value).
    pub abs: [AbsInfo; abs::CNT],
    /// Auto-repeat delay and period, milliseconds (`EV_REP`).
    pub rep: [i32; rep::CNT],
    /// The hub repeats held keys itself; otherwise presses of a key that
    /// is down are the device's own repeats.
    pub soft_repeat: bool,
    /// The device's keys also go to the kernel console.
    pub console: bool,
}

/// The repeat parameters of PC keyboards: 250 ms, then 30 per second.
pub const DEFAULT_REPEAT: [i32; 2] = [250, 33];

impl DeviceInfo {
    /// A device that reports nothing yet (`EV_SYN` only).
    pub fn new(name: &str, id: InputId) -> DeviceInfo {
        let mut caps = Capabilities::default();
        caps.ev.set(ev::SYN, true);
        DeviceInfo {
            name: String::from(name),
            phys: String::new(),
            uniq: String::new(),
            id,
            caps,
            abs: [AbsInfo::default(); abs::CNT],
            rep: DEFAULT_REPEAT,
            soft_repeat: false,
            console: false,
        }
    }

    /// Declare events of `kind` with `code`.
    pub fn with(mut self, kind: u16, code: u16) -> DeviceInfo {
        self.declare(kind, code);
        self
    }

    pub fn declare(&mut self, kind: u16, code: u16) {
        self.caps.ev.set(kind, true);
        let c = &mut self.caps;
        match kind {
            ev::KEY => c.key.set(code, true),
            ev::REL => c.rel.set(code, true),
            ev::ABS => c.abs.set(code, true),
            ev::MSC => c.msc.set(code, true),
            ev::LED => c.led.set(code, true),
            ev::SW => c.sw.set(code, true),
            ev::SND => c.snd.set(code, true),
            ev::FF => c.ff.set(code, true),
            _ => {}
        }
    }

    /// Declare the absolute axis `code` with this range.
    pub fn with_abs(mut self, code: u16, info: AbsInfo) -> DeviceInfo {
        self.declare(ev::ABS, code);
        if let Some(a) = self.abs.get_mut(usize::from(code)) {
            *a = info;
        }
        self
    }

    pub fn with_prop(mut self, prop: u16) -> DeviceInfo {
        self.caps.prop.set(prop, true);
        self
    }

    /// Auto-repeat with these parameters; by the hub if `soft`.
    pub fn with_repeat(mut self, rep: [i32; 2], soft: bool) -> DeviceInfo {
        self.caps.ev.set(ev::REP, true);
        self.rep = rep;
        self.soft_repeat = soft;
        self
    }

    /// A PC keyboard: every key [`Key`] names, the three lock LEDs,
    /// auto-repeat by the keyboard itself, and a console keyboard.
    pub fn keyboard(name: &str, id: InputId) -> DeviceInfo {
        let mut d = DeviceInfo::new(name, id).with_repeat(DEFAULT_REPEAT, false);
        for k in Key::ALL {
            d.declare(ev::KEY, keycode::code(k));
        }
        for l in [led::NUML, led::CAPSL, led::SCROLLL] {
            d.declare(ev::LED, l);
        }
        d.console = true;
        d
    }

    /// A mouse with these buttons, X and Y motion, and the wheels asked
    /// for.
    pub fn mouse(name: &str, id: InputId, buttons: &[u16], wheel: bool, hwheel: bool) -> DeviceInfo {
        let mut d = DeviceInfo::new(name, id).with(ev::REL, codes::rel::X).with(ev::REL, codes::rel::Y);
        if wheel {
            d.declare(ev::REL, codes::rel::WHEEL);
        }
        if hwheel {
            d.declare(ev::REL, codes::rel::HWHEEL);
        }
        for &b in buttons {
            d.declare(ev::KEY, b);
        }
        d.with_prop(codes::prop::POINTER)
    }

    /// Whether this looks like a keyboard: it has the letter keys.
    pub fn has_letter_keys(&self) -> bool {
        (key::Q..=key::P).chain(key::A..=key::L).chain(key::Z..=key::M).all(|c| self.caps.key.get(c))
    }
}

/// A client's clock for event times (`EVIOCSCLOCKID`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Clock {
    /// `CLOCK_REALTIME`, the default.
    #[default]
    Realtime,
    /// `CLOCK_MONOTONIC`.
    Monotonic,
    /// `CLOCK_BOOTTIME` (the monotonic clock: Poly does not suspend).
    Boottime,
}

/// Events waiting for one client: a ring that never grows.
#[derive(Clone, Debug)]
pub struct Queue {
    buf: Box<[Event]>,
    head: usize,
    len: usize,
}

/// Events a client queue holds unless asked otherwise.
pub const QUEUE_EVENTS: usize = 256;

impl Queue {
    /// A queue for `capacity` events (at least 8).
    pub fn new(capacity: usize) -> Queue {
        Queue { buf: vec![Event::default(); capacity.max(8)].into_boxed_slice(), head: 0, len: 0 }
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn capacity(&self) -> usize {
        self.buf.len()
    }

    fn push(&mut self, e: Event) -> bool {
        if self.len == self.buf.len() {
            return false;
        }
        let i = (self.head + self.len) % self.buf.len();
        self.buf[i] = e;
        self.len += 1;
        true
    }

    fn pop(&mut self) -> Option<Event> {
        if self.len == 0 {
            return None;
        }
        let e = self.buf[self.head];
        self.head = (self.head + 1) % self.buf.len();
        self.len -= 1;
        Some(e)
    }

    fn clear(&mut self) {
        self.head = 0;
        self.len = 0;
    }

    /// Queue a frame; if it does not fit, drop what was queued and put
    /// `SYN_DROPPED` before it. Returns false if events were dropped.
    fn push_frame(&mut self, frame: &[Event]) -> bool {
        if self.buf.len() - self.len >= frame.len() {
            frame.iter().for_each(|&e| {
                self.push(e);
            });
            return true;
        }
        self.clear();
        let t = frame.first().map_or(0, |e| e.time_ns);
        self.push(Event::new(t, ev::SYN, syn::DROPPED, 0));
        for &e in frame {
            if !self.push(e) {
                break;
            }
        }
        false
    }
}

/// Identifies one reader of a device (one open file).
pub type ClientId = u32;

#[derive(Debug)]
struct Client {
    id: ClientId,
    queue: Queue,
    clock: Clock,
    /// `EVIOCREVOKE`: no more events, reads fail.
    revoked: bool,
    /// Frames this client lost to a full queue.
    overruns: u64,
}

/// A key the soft repeat is repeating.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Repeat {
    code: u16,
    next_ns: u64,
}

/// The most events in one frame (a device that sends more without a
/// `SYN_REPORT` gets frames cut at this size).
pub const FRAME_EVENTS: usize = 64;

/// A device's live state.
#[derive(Debug)]
struct Device {
    info: DeviceInfo,
    keys: Bits<{ KEY_CNT / 64 }>,
    leds: Bits<1>,
    frame: Vec<Event>,
    /// Console keys of the frame being built.
    console_keys: Vec<(Key, i32)>,
    repeat: Option<Repeat>,
    clients: Vec<Client>,
    grab: Option<ClientId>,
    stats: DeviceStats,
}

/// Counters per device.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DeviceStats {
    /// Events reported by the driver (before filtering).
    pub reported: u64,
    /// Frames delivered (to at least the console or one client).
    pub frames: u64,
    /// Repeats made by the soft repeat.
    pub repeats: u64,
    /// Frames some client lost to a full queue.
    pub overruns: u64,
}

/// A key for the console: the key, and 0 (released), 1 (pressed) or 2
/// (repeated).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ConsoleKey {
    pub time_ns: u64,
    pub key: Key,
    pub value: i32,
}

/// Console keys buffered: about 60 key presses and releases.
pub const CONSOLE_KEYS: usize = 128;

#[derive(Debug)]
struct ConsoleRing {
    buf: [ConsoleKey; CONSOLE_KEYS],
    head: usize,
    len: usize,
    dropped: u64,
}

/// What a call delivered, so the caller wakes the right readers.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Delivered {
    /// Bit `n`: clients of device `n` got events (devices from 64 on all
    /// share bit 63).
    pub devices: u64,
    /// The console queue got keys.
    pub console: bool,
}

impl Delivered {
    pub fn merge(self, other: Delivered) -> Delivered {
        Delivered { devices: self.devices | other.devices, console: self.console || other.console }
    }

    pub fn device_bit(dev: usize) -> u64 {
        1 << dev.min(63)
    }

    pub fn is_empty(&self) -> bool {
        self.devices == 0 && !self.console
    }
}

/// Why a client's request failed (the kernel turns these into errnos).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// Nothing to read now (`EAGAIN`).
    Again,
    /// A bad argument (`EINVAL`).
    Invalid,
    /// Not a request of this device (`ENOTTY`).
    NotTty,
    /// Someone else grabbed the device (`EBUSY`).
    Busy,
    /// The client was revoked or the device is gone (`ENODEV`).
    NoDevice,
    /// The device has no such string (`ENOENT`).
    NoEntry,
}

/// Every input device and its readers. See the [module documentation](self).
#[derive(Debug)]
pub struct Hub {
    devices: Vec<Device>,
    console: ConsoleRing,
    next_client: ClientId,
}

impl Default for Hub {
    fn default() -> Hub {
        Hub::new()
    }
}

impl Hub {
    pub const fn new() -> Hub {
        Hub {
            devices: Vec::new(),
            console: ConsoleRing {
                buf: [ConsoleKey { time_ns: 0, key: Key::A, value: 0 }; CONSOLE_KEYS],
                head: 0,
                len: 0,
                dropped: 0,
            },
            next_client: 1,
        }
    }

    /// Add a device; returns its number (`/dev/input/event<n>`).
    pub fn add(&mut self, info: DeviceInfo) -> usize {
        let leds = Bits::new();
        self.devices.push(Device {
            info,
            keys: Bits::new(),
            leds,
            frame: Vec::with_capacity(FRAME_EVENTS),
            console_keys: Vec::with_capacity(FRAME_EVENTS),
            repeat: None,
            clients: Vec::new(),
            grab: None,
            stats: DeviceStats::default(),
        });
        self.devices.len() - 1
    }

    pub fn len(&self) -> usize {
        self.devices.len()
    }

    pub fn is_empty(&self) -> bool {
        self.devices.is_empty()
    }

    pub fn info(&self, dev: usize) -> Option<&DeviceInfo> {
        self.devices.get(dev).map(|d| &d.info)
    }

    pub fn stats(&self, dev: usize) -> Option<DeviceStats> {
        self.devices.get(dev).map(|d| d.stats)
    }

    /// The keys down now (`EVIOCGKEY`).
    pub fn keys(&self, dev: usize) -> Option<&Bits<{ KEY_CNT / 64 }>> {
        self.devices.get(dev).map(|d| &d.keys)
    }

    /// The LEDs on now, bit `n` for `LED_*` code `n` (`EVIOCGLED`).
    pub fn leds(&self, dev: usize) -> u64 {
        self.devices.get(dev).map_or(0, |d| d.leds.0[0])
    }

    pub fn is_key_down(&self, dev: usize, code: u16) -> bool {
        self.devices.get(dev).is_some_and(|d| d.keys.get(code))
    }

    // --- Reporting -------------------------------------------------------

    /// The driver of `dev` reports one event (`SYN_REPORT` ends a frame).
    pub fn report(&mut self, dev: usize, time_ns: u64, kind: u16, code: u16, value: i32) -> Delivered {
        let Some(d) = self.devices.get_mut(dev) else { return Delivered::default() };
        d.stats.reported += 1;
        if kind == ev::SYN {
            if code == syn::REPORT {
                return self.flush(dev, time_ns);
            }
            return Delivered::default();
        }
        if !d.info.caps.has(kind, code) {
            return Delivered::default();
        }
        let value = match kind {
            ev::KEY => match d.key(code, value, time_ns) {
                Some(v) => v,
                None => return Delivered::default(),
            },
            ev::REL if value == 0 => return Delivered::default(),
            ev::ABS => {
                let a = &mut d.info.abs[usize::from(code)];
                if a.value == value {
                    return Delivered::default();
                }
                a.value = value;
                value
            }
            ev::LED => {
                let on = value != 0;
                if d.leds.get(code) == on {
                    return Delivered::default();
                }
                d.leds.set(code, on);
                i32::from(on)
            }
            ev::REP => {
                if let Some(r) = d.info.rep.get_mut(usize::from(code)) {
                    *r = value;
                }
                return Delivered::default();
            }
            _ => value,
        };
        let mut out = Delivered::default();
        if d.frame.len() + 1 >= FRAME_EVENTS {
            out = self.flush(dev, time_ns);
        }
        let d = &mut self.devices[dev];
        d.frame.push(Event::new(time_ns, kind, code, value));
        if kind == ev::KEY
            && d.info.console
            && let Some(k) = keycode::key(code)
        {
            d.console_keys.push((k, value));
        }
        out
    }

    /// [`report`](Self::report) each of `events`, all at `time_ns`.
    pub fn report_all(&mut self, dev: usize, time_ns: u64, events: &[(u16, u16, i32)]) -> Delivered {
        events.iter().fold(Delivered::default(), |acc, &(k, c, v)| acc.merge(self.report(dev, time_ns, k, c, v)))
    }

    /// End the frame of `dev`: deliver it to its clients (or the one that
    /// grabbed it) and its console keys to the console.
    fn flush(&mut self, dev: usize, time_ns: u64) -> Delivered {
        let d = &mut self.devices[dev];
        if d.frame.is_empty() {
            d.console_keys.clear();
            return Delivered::default();
        }
        d.frame.push(Event::syn(time_ns));
        let mut out = Delivered::default();
        let grab = d.grab;
        let mut overrun = false;
        for c in d.clients.iter_mut().filter(|c| !c.revoked && grab.is_none_or(|g| g == c.id)) {
            if !c.queue.push_frame(&d.frame) {
                c.overruns += 1;
                overrun = true;
            }
            out.devices |= Delivered::device_bit(dev);
        }
        if overrun {
            d.stats.overruns += 1;
        }
        d.frame.clear();
        let keys = core::mem::take(&mut d.console_keys);
        if grab.is_none() && !keys.is_empty() {
            for &(key, value) in &keys {
                self.console.push(ConsoleKey { time_ns, key, value });
            }
            out.console = true;
        }
        let d = &mut self.devices[dev];
        d.console_keys = keys;
        d.console_keys.clear();
        if !out.is_empty() {
            d.stats.frames += 1;
        }
        out
    }

    // --- Soft repeat -----------------------------------------------------

    /// Make the repeats due by `now_ns`; returns what was delivered.
    pub fn tick(&mut self, now_ns: u64) -> Delivered {
        let mut out = Delivered::default();
        for dev in 0..self.devices.len() {
            let d = &mut self.devices[dev];
            let Some(r) = d.repeat else { continue };
            if r.next_ns > now_ns {
                continue;
            }
            let period = ms_to_ns(d.info.rep[usize::from(rep::PERIOD)]);
            let next = r.next_ns.saturating_add(period);
            d.repeat = Some(Repeat { code: r.code, next_ns: if next <= now_ns { now_ns + period } else { next } });
            d.stats.repeats += 1;
            d.frame.push(Event::new(now_ns, ev::KEY, r.code, 2));
            if d.info.console
                && let Some(k) = keycode::key(r.code)
            {
                d.console_keys.push((k, 2));
            }
            out = out.merge(self.flush(dev, now_ns));
        }
        out
    }

    /// When [`tick`](Self::tick) next has work, if ever.
    pub fn next_deadline(&self) -> Option<u64> {
        self.devices.iter().filter_map(|d| d.repeat.map(|r| r.next_ns)).min()
    }

    // --- LEDs ------------------------------------------------------------

    /// Light the LEDs of `dev` as `leds` says (bit `n` for `LED_*` code
    /// `n`), as events to its clients. Returns whether any changed.
    pub fn set_leds(&mut self, dev: usize, leds: u64, time_ns: u64) -> (bool, Delivered) {
        let Some(d) = self.devices.get(dev) else { return (false, Delivered::default()) };
        let before = d.leds;
        let codes: Vec<u16> = d.info.caps.led.iter().collect();
        let mut out = Delivered::default();
        for code in codes {
            out = out.merge(self.report(dev, time_ns, ev::LED, code, i32::from(leds & (1 << code) != 0)));
        }
        out = out.merge(self.flush(dev, time_ns));
        (self.devices[dev].leds != before, out)
    }

    /// The devices that feed the console and have lock LEDs.
    pub fn console_keyboards(&self) -> impl Iterator<Item = usize> + '_ {
        self.devices.iter().enumerate().filter(|(_, d)| d.info.console && d.info.caps.led.any()).map(|(i, _)| i)
    }

    // --- The console -----------------------------------------------------

    /// The oldest console key, if any.
    pub fn console_pop(&mut self) -> Option<ConsoleKey> {
        self.console.pop()
    }

    /// Put a key in the console queue directly (keys injected without a
    /// device). False if it was full.
    pub fn console_push(&mut self, key: Key, value: i32, time_ns: u64) -> bool {
        self.console.push(ConsoleKey { time_ns, key, value })
    }

    pub fn console_len(&self) -> usize {
        self.console.len
    }

    /// Console keys lost because the queue was full.
    pub fn console_dropped(&self) -> u64 {
        self.console.dropped
    }

    // --- Clients ---------------------------------------------------------

    /// A new reader of `dev` with `queue`; `None` if there is no such
    /// device.
    pub fn open(&mut self, dev: usize, queue: Queue) -> Option<ClientId> {
        let id = self.next_client;
        let d = self.devices.get_mut(dev)?;
        self.next_client = self.next_client.wrapping_add(1).max(1);
        d.clients.push(Client { id, queue, clock: Clock::default(), revoked: false, overruns: 0 });
        Some(id)
    }

    /// The reader is gone (its grab with it). Returns its queue, so the
    /// caller frees it outside its lock.
    pub fn close(&mut self, dev: usize, client: ClientId) -> Option<Queue> {
        let d = self.devices.get_mut(dev)?;
        if d.grab == Some(client) {
            d.grab = None;
        }
        let i = d.clients.iter().position(|c| c.id == client)?;
        Some(d.clients.swap_remove(i).queue)
    }

    fn client(&self, dev: usize, client: ClientId) -> Result<&Client, Error> {
        let d = self.devices.get(dev).ok_or(Error::NoDevice)?;
        let c = d.clients.iter().find(|c| c.id == client).ok_or(Error::NoDevice)?;
        if c.revoked { Err(Error::NoDevice) } else { Ok(c) }
    }

    fn client_mut(&mut self, dev: usize, client: ClientId) -> Result<&mut Client, Error> {
        let d = self.devices.get_mut(dev).ok_or(Error::NoDevice)?;
        let c = d.clients.iter_mut().find(|c| c.id == client).ok_or(Error::NoDevice)?;
        if c.revoked { Err(Error::NoDevice) } else { Ok(c) }
    }

    /// Take up to `out.len()` queued events of a client; `Again` if there
    /// are none, `NoDevice` once it was revoked.
    pub fn read(&mut self, dev: usize, client: ClientId, out: &mut [Event]) -> Result<usize, Error> {
        let c = self.client_mut(dev, client)?;
        if c.queue.is_empty() {
            return Err(Error::Again);
        }
        let mut n = 0;
        while n < out.len()
            && let Some(e) = c.queue.pop()
        {
            out[n] = e;
            n += 1;
        }
        Ok(n)
    }

    /// Events queued for a client; `NoDevice` once it was revoked.
    pub fn pending(&self, dev: usize, client: ClientId) -> Result<usize, Error> {
        self.client(dev, client).map(|c| c.queue.len())
    }

    pub fn clock(&self, dev: usize, client: ClientId) -> Clock {
        self.client(dev, client).map_or(Clock::default(), |c| c.clock)
    }

    pub fn set_clock(&mut self, dev: usize, client: ClientId, clock: Clock) -> Result<(), Error> {
        self.client_mut(dev, client)?.clock = clock;
        Ok(())
    }

    /// Frames a client lost to a full queue.
    pub fn overruns(&self, dev: usize, client: ClientId) -> u64 {
        self.client(dev, client).map_or(0, |c| c.overruns)
    }

    /// `EVIOCGRAB`: take (`on`) or give back the device's events.
    pub fn grab(&mut self, dev: usize, client: ClientId, on: bool) -> Result<(), Error> {
        self.client(dev, client)?;
        let d = &mut self.devices[dev];
        match (on, d.grab) {
            (true, None) => {
                d.grab = Some(client);
                Ok(())
            }
            (true, Some(_)) => Err(Error::Busy),
            (false, Some(g)) if g == client => {
                d.grab = None;
                Ok(())
            }
            (false, _) => Err(Error::Invalid),
        }
    }

    pub fn grabbed_by(&self, dev: usize) -> Option<ClientId> {
        self.devices.get(dev).and_then(|d| d.grab)
    }

    /// `EVIOCREVOKE`: the client gets no more events and its reads fail.
    pub fn revoke(&mut self, dev: usize, client: ClientId) -> Result<(), Error> {
        let c = self.client_mut(dev, client)?;
        c.revoked = true;
        c.queue.clear();
        let d = &mut self.devices[dev];
        if d.grab == Some(client) {
            d.grab = None;
        }
        Ok(())
    }

    /// A client writes an event to the device (`write(2)` on the event
    /// file): it goes through the device as if the driver had reported it,
    /// so an `EV_LED` lights the LED and every client sees it.
    pub fn inject(
        &mut self,
        dev: usize,
        client: ClientId,
        time_ns: u64,
        kind: u16,
        code: u16,
        value: i32,
    ) -> Result<Delivered, Error> {
        self.client(dev, client)?;
        Ok(self.report(dev, time_ns, kind, code, value))
    }

    /// Change an absolute axis's range (`EVIOCSABS`).
    pub fn set_abs(&mut self, dev: usize, code: u16, info: AbsInfo) -> Result<(), Error> {
        let d = self.devices.get_mut(dev).ok_or(Error::NoDevice)?;
        if !d.info.caps.has(ev::ABS, code) {
            return Err(Error::Invalid);
        }
        d.info.abs[usize::from(code)] = info;
        Ok(())
    }

    /// Repeat held keys in the hub (`soft`) or take the device's own
    /// repeats; a repeat in progress stops.
    pub fn set_soft_repeat(&mut self, dev: usize, soft: bool) -> Result<(), Error> {
        let d = self.devices.get_mut(dev).ok_or(Error::NoDevice)?;
        if !d.info.caps.ev.get(ev::REP) {
            return Err(Error::Invalid);
        }
        d.info.soft_repeat = soft;
        d.repeat = None;
        Ok(())
    }

    /// Change the repeat parameters (`EVIOCSREP`).
    pub fn set_repeat(&mut self, dev: usize, delay_ms: i32, period_ms: i32) -> Result<(), Error> {
        let d = self.devices.get_mut(dev).ok_or(Error::NoDevice)?;
        if !d.info.caps.ev.get(ev::REP) || delay_ms < 0 || period_ms < 0 {
            return Err(Error::Invalid);
        }
        d.info.rep = [delay_ms, period_ms];
        if period_ms == 0 {
            d.repeat = None;
        }
        Ok(())
    }
}

fn ms_to_ns(ms: i32) -> u64 {
    u64::try_from(ms).unwrap_or(0) * 1_000_000
}

impl Device {
    /// A key event: the value to report, or `None` if it changes nothing.
    fn key(&mut self, code: u16, value: i32, time_ns: u64) -> Option<i32> {
        let down = self.keys.get(code);
        let repeatable = self.info.soft_repeat && code < key::BTN_MISC;
        match (value, down) {
            (0, false) => None,
            (0, true) => {
                self.keys.set(code, false);
                if self.repeat.is_some_and(|r| r.code == code) {
                    self.repeat = None;
                }
                Some(0)
            }
            (_, false) if value != 2 => {
                self.keys.set(code, true);
                let [delay, period] = self.info.rep;
                if repeatable && period > 0 {
                    self.repeat = Some(Repeat { code, next_ns: time_ns.saturating_add(ms_to_ns(delay)) });
                }
                Some(1)
            }
            // The device's own repeat of a held key, unless the hub repeats.
            (_, true) if !self.info.soft_repeat => Some(2),
            _ => None,
        }
    }
}

impl ConsoleRing {
    fn push(&mut self, k: ConsoleKey) -> bool {
        if self.len == CONSOLE_KEYS {
            self.dropped += 1;
            return false;
        }
        self.buf[(self.head + self.len) % CONSOLE_KEYS] = k;
        self.len += 1;
        true
    }

    fn pop(&mut self) -> Option<ConsoleKey> {
        if self.len == 0 {
            return None;
        }
        let k = self.buf[self.head];
        self.head = (self.head + 1) % CONSOLE_KEYS;
        self.len -= 1;
        Some(k)
    }
}

#[cfg(test)]
mod tests;
