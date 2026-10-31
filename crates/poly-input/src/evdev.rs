//! The Linux event device interface (`/dev/input/event*`) over the
//! [`Hub`]: the bytes of `struct input_event` and the `EVIOC*` ioctls.
//!
//! Sources (ABI facts): the public uapi headers `linux/input.h` (`struct
//! input_event`, `struct input_id`, `struct input_absinfo`, the `EVIOC*`
//! request numbers, `EV_VERSION`), `asm-generic/ioctl.h` (how a request
//! number encodes its direction, size, type and number) and the kernel's
//! input documentation (`input.rst`: "struct input_event", reads of whole
//! events; `event-codes.rst`: `SYN_DROPPED`; `input-programming.rst`), plus
//! the `ioctl(2)` and `read(2)` manual pages for the errors.
//!
//! - **Reads** return whole 24-byte events (x86-64: `struct timeval` of two
//!   64-bit fields, then `__u16 type`, `__u16 code`, `__s32 value`), as
//!   many as fit; a buffer smaller than one event is `EINVAL`.
//! - **Writes** take whole events and inject them into the device
//!   ([`Hub::inject`]); a trailing partial event is ignored.
//! - **ioctls**: `EVIOCGVERSION`, `EVIOCGID`, `EVIOCGREP`/`EVIOCSREP`,
//!   `EVIOCGNAME`/`EVIOCGPHYS`/`EVIOCGUNIQ`/`EVIOCGPROP`,
//!   `EVIOCGKEY`/`EVIOCGLED`/`EVIOCGSND`/`EVIOCGSW`, `EVIOCGBIT`,
//!   `EVIOCGABS`/`EVIOCSABS`, `EVIOCGRAB`, `EVIOCREVOKE`, `EVIOCSCLOCKID`,
//!   `EVIOCGEFFECTS` (no force feedback: 0). Variable-length reads copy at
//!   most the length the request encodes and return how many bytes they
//!   copied. Keymap changes (`EVIOCGKEYCODE`/`EVIOCSKEYCODE`), multitouch
//!   slots, event masks and force feedback are `EINVAL`.

use crate::hub::{AbsInfo, ClientId, Clock, Delivered, Error, Event, Hub};
use alloc::vec::Vec;

/// `EV_VERSION`: the event interface version `EVIOCGVERSION` reports.
pub const EV_VERSION: u32 = 0x01_0001;

/// Bytes of `struct input_event` on x86-64.
pub const EVENT_SIZE: usize = 24;

/// `CLOCK_*` ids `EVIOCSCLOCKID` takes (`linux/time.h`).
pub const CLOCK_REALTIME: i32 = 0;
pub const CLOCK_MONOTONIC: i32 = 1;
pub const CLOCK_BOOTTIME: i32 = 7;

/// `struct input_event` for `e` on `clock`; `realtime_offset_ns` is what
/// the realtime clock is ahead of the monotonic one.
pub fn encode(e: &Event, clock: Clock, realtime_offset_ns: u64) -> [u8; EVENT_SIZE] {
    let ns = match clock {
        Clock::Realtime => e.time_ns.wrapping_add(realtime_offset_ns),
        Clock::Monotonic | Clock::Boottime => e.time_ns,
    };
    let mut b = [0u8; EVENT_SIZE];
    b[0..8].copy_from_slice(&((ns / 1_000_000_000) as i64).to_le_bytes());
    b[8..16].copy_from_slice(&(((ns % 1_000_000_000) / 1000) as i64).to_le_bytes());
    b[16..18].copy_from_slice(&e.kind.to_le_bytes());
    b[18..20].copy_from_slice(&e.code.to_le_bytes());
    b[20..24].copy_from_slice(&e.value.to_le_bytes());
    b
}

/// The type, code and value of a `struct input_event` (its time is
/// ignored, as for events written to a device).
pub fn decode(b: &[u8; EVENT_SIZE]) -> (u16, u16, i32) {
    (
        u16::from_le_bytes([b[16], b[17]]),
        u16::from_le_bytes([b[18], b[19]]),
        i32::from_le_bytes([b[20], b[21], b[22], b[23]]),
    )
}

/// `read(2)` on an event file: whole events into `buf`, in the client's
/// clock. `EINVAL` for a buffer smaller than one event; `Again` if
/// nothing is queued.
pub fn read(
    hub: &mut Hub,
    dev: usize,
    client: ClientId,
    buf: &mut [u8],
    realtime_offset_ns: u64,
) -> Result<usize, Error> {
    if buf.len() < EVENT_SIZE {
        return Err(Error::Invalid);
    }
    let clock = hub.clock(dev, client);
    let mut events = [Event::default(); 32];
    let mut done = 0;
    while buf.len() - done >= EVENT_SIZE {
        let want = ((buf.len() - done) / EVENT_SIZE).min(events.len());
        let n = match hub.read(dev, client, &mut events[..want]) {
            Ok(n) => n,
            Err(Error::Again) if done > 0 => break,
            Err(e) => return Err(e),
        };
        for e in &events[..n] {
            buf[done..done + EVENT_SIZE].copy_from_slice(&encode(e, clock, realtime_offset_ns));
            done += EVENT_SIZE;
        }
        if n < want {
            break;
        }
    }
    Ok(done)
}

/// `write(2)` on an event file: inject each whole event. Returns the bytes
/// taken and what was delivered.
pub fn write(
    hub: &mut Hub,
    dev: usize,
    client: ClientId,
    buf: &[u8],
    now_ns: u64,
) -> Result<(usize, Delivered), Error> {
    if buf.len() < EVENT_SIZE {
        return Err(Error::Invalid);
    }
    let mut out = Delivered::default();
    let mut done = 0;
    for chunk in buf.chunks_exact(EVENT_SIZE) {
        let (kind, code, value) = decode(chunk.try_into().map_err(|_| Error::Invalid)?);
        out = out.merge(hub.inject(dev, client, now_ns, kind, code, value)?);
        done += EVENT_SIZE;
    }
    Ok((done, out))
}

// --- ioctl request numbers (asm-generic/ioctl.h) --------------------------

const IOC_WRITE: u32 = 1;
const IOC_READ: u32 = 2;

/// `_IOC(dir, type, nr, size)`: direction in bits 30-31, size in bits
/// 16-29, type in bits 8-15, number in bits 0-7.
pub const fn ioc(dir: u32, ty: u8, nr: u8, size: u32) -> u32 {
    dir << 30 | (size & 0x3fff) << 16 | (ty as u32) << 8 | nr as u32
}

/// The ioctl type of the event interface, `'E'`.
pub const EVIOC_TYPE: u8 = b'E';

const fn ior(nr: u8, size: u32) -> u32 {
    ioc(IOC_READ, EVIOC_TYPE, nr, size)
}

const fn iow(nr: u8, size: u32) -> u32 {
    ioc(IOC_WRITE, EVIOC_TYPE, nr, size)
}

pub const EVIOCGVERSION: u32 = ior(0x01, 4);
pub const EVIOCGID: u32 = ior(0x02, 8);
pub const EVIOCGREP: u32 = ior(0x03, 8);
pub const EVIOCSREP: u32 = iow(0x03, 8);
pub const EVIOCGKEYCODE: u32 = ior(0x04, 8);
pub const EVIOCSKEYCODE: u32 = iow(0x04, 8);
pub const EVIOCGEFFECTS: u32 = ior(0x84, 4);
pub const EVIOCGRAB: u32 = iow(0x90, 4);
pub const EVIOCREVOKE: u32 = iow(0x91, 4);
pub const EVIOCSCLOCKID: u32 = iow(0xa0, 4);

/// `EVIOCGNAME(len)`.
pub const fn eviocgname(len: u32) -> u32 {
    ioc(IOC_READ, EVIOC_TYPE, 0x06, len)
}

/// `EVIOCGPHYS(len)`.
pub const fn eviocgphys(len: u32) -> u32 {
    ioc(IOC_READ, EVIOC_TYPE, 0x07, len)
}

/// `EVIOCGUNIQ(len)`.
pub const fn eviocguniq(len: u32) -> u32 {
    ioc(IOC_READ, EVIOC_TYPE, 0x08, len)
}

/// `EVIOCGPROP(len)`.
pub const fn eviocgprop(len: u32) -> u32 {
    ioc(IOC_READ, EVIOC_TYPE, 0x09, len)
}

/// `EVIOCGKEY(len)`: the keys down.
pub const fn eviocgkey(len: u32) -> u32 {
    ioc(IOC_READ, EVIOC_TYPE, 0x18, len)
}

/// `EVIOCGLED(len)`: the LEDs on.
pub const fn eviocgled(len: u32) -> u32 {
    ioc(IOC_READ, EVIOC_TYPE, 0x19, len)
}

/// `EVIOCGBIT(ev, len)`: the codes of type `ev` (0: the types).
pub const fn eviocgbit(ev: u8, len: u32) -> u32 {
    ioc(IOC_READ, EVIOC_TYPE, 0x20 + ev, len)
}

/// `EVIOCGABS(abs)`: `struct input_absinfo` of axis `abs`.
pub const fn eviocgabs(abs: u8) -> u32 {
    ior(0x40 + abs, 24)
}

/// `EVIOCSABS(abs)`.
pub const fn eviocsabs(abs: u8) -> u32 {
    iow(0xc0 + abs, 24)
}

/// A request of the event interface, decoded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Request {
    Version,
    Id,
    GetRepeat,
    /// Needs 8 bytes from the caller.
    SetRepeat,
    Name(usize),
    Phys(usize),
    Uniq(usize),
    Prop(usize),
    Keys(usize),
    Leds(usize),
    Sounds(usize),
    Switches(usize),
    Bits {
        kind: u16,
        len: usize,
    },
    GetAbs(u16),
    /// Needs 24 bytes from the caller.
    SetAbs(u16),
    /// The argument is the value itself.
    Grab,
    Revoke,
    /// Needs 4 bytes from the caller.
    SetClock,
    Effects,
    /// Recognised but not supported: `EINVAL`.
    Unsupported,
}

impl Request {
    /// Decode `request`; `None` if it is not an event interface request
    /// (`ENOTTY`).
    pub fn decode(request: u32) -> Option<Request> {
        let dir = request >> 30;
        let size = ((request >> 16) & 0x3fff) as usize;
        let ty = (request >> 8) as u8;
        let nr = request as u8;
        if ty != EVIOC_TYPE {
            return None;
        }
        let read = dir == IOC_READ;
        let write = dir == IOC_WRITE;
        Some(match nr {
            0x01 if read && size == 4 => Request::Version,
            0x02 if read && size == 8 => Request::Id,
            0x03 if read && size == 8 => Request::GetRepeat,
            0x03 if write && size == 8 => Request::SetRepeat,
            // EVIOCG/SKEYCODE and their _V2 forms.
            0x04 | 0x05 => Request::Unsupported,
            0x06 if read => Request::Name(size),
            0x07 if read => Request::Phys(size),
            0x08 if read => Request::Uniq(size),
            0x09 if read => Request::Prop(size),
            // EVIOCGMTSLOTS.
            0x0a if read => Request::Unsupported,
            0x18 if read => Request::Keys(size),
            0x19 if read => Request::Leds(size),
            0x1a if read => Request::Sounds(size),
            0x1b if read => Request::Switches(size),
            0x20..=0x3f if read => Request::Bits { kind: u16::from(nr - 0x20), len: size },
            0x40..=0x7f if read && size == 24 => Request::GetAbs(u16::from(nr - 0x40)),
            0xc0..=0xff if write && size == 24 => Request::SetAbs(u16::from(nr - 0xc0)),
            // EVIOCSFF, EVIOCRMFF.
            0x80 | 0x81 => Request::Unsupported,
            0x84 if read && size == 4 => Request::Effects,
            0x90 if write => Request::Grab,
            0x91 if write => Request::Revoke,
            // EVIOCGMASK, EVIOCSMASK.
            0x92 | 0x93 => Request::Unsupported,
            0xa0 if write && size == 4 => Request::SetClock,
            _ => return None,
        })
    }

    /// Bytes the request reads from the caller's pointer before it runs.
    pub const fn input_len(self) -> usize {
        match self {
            Request::SetRepeat => 8,
            Request::SetAbs(_) => 24,
            Request::SetClock => 4,
            _ => 0,
        }
    }
}

/// What a request returns: the system call's value, and bytes to copy to
/// the caller's pointer (empty for none).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reply {
    pub value: u64,
    pub out: Vec<u8>,
}

impl Reply {
    fn value(v: u64) -> Reply {
        Reply { value: v, out: Vec::new() }
    }

    /// `data` cut to `len`, returning the length copied.
    fn bytes(mut data: Vec<u8>, len: usize) -> Reply {
        data.truncate(len);
        Reply { value: data.len() as u64, out: data }
    }

    /// A fixed-size structure: returns 0.
    fn data(data: Vec<u8>) -> Reply {
        Reply { value: 0, out: data }
    }
}

/// A string as the `EVIOCG*` string requests return it: with its NUL,
/// cut to `len`; `ENOENT` if the device has none.
fn string(s: &str, len: usize) -> Result<Reply, Error> {
    if s.is_empty() {
        return Err(Error::NoEntry);
    }
    let mut v: Vec<u8> = s.as_bytes().to_vec();
    v.push(0);
    Ok(Reply::bytes(v, len))
}

fn le_i32(b: &[u8], i: usize) -> i32 {
    i32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]])
}

fn absinfo_bytes(a: &AbsInfo) -> Vec<u8> {
    [a.value, a.minimum, a.maximum, a.fuzz, a.flat, a.resolution].iter().flat_map(|v| v.to_le_bytes()).collect()
}

/// Carry out `req` for `client` of `dev`. `arg` is the ioctl's argument as
/// a value (for `EVIOCGRAB` and `EVIOCREVOKE`); `input` the
/// [`Request::input_len`] bytes the caller's pointer held.
pub fn ioctl(
    hub: &mut Hub,
    dev: usize,
    client: ClientId,
    req: Request,
    arg: u64,
    input: &[u8],
) -> Result<Reply, Error> {
    // A revoked client may do nothing but close.
    hub.pending(dev, client)?;
    if input.len() < req.input_len() {
        return Err(Error::Invalid);
    }
    let info = hub.info(dev).ok_or(Error::NoDevice)?;
    let caps = info.caps;
    Ok(match req {
        Request::Version => Reply::data(EV_VERSION.to_le_bytes().to_vec()),
        Request::Id => {
            let id = info.id;
            Reply::data([id.bustype, id.vendor, id.product, id.version].iter().flat_map(|v| v.to_le_bytes()).collect())
        }
        Request::GetRepeat => {
            if !caps.ev.get(crate::codes::ev::REP) {
                return Err(Error::Invalid);
            }
            Reply::data(info.rep.iter().flat_map(|v| v.to_le_bytes()).collect())
        }
        Request::SetRepeat => {
            hub.set_repeat(dev, le_i32(input, 0), le_i32(input, 4))?;
            Reply::value(0)
        }
        Request::Name(len) => string(&info.name, len)?,
        Request::Phys(len) => string(&info.phys, len)?,
        Request::Uniq(len) => string(&info.uniq, len)?,
        Request::Prop(len) => Reply::bytes(caps.prop.to_bytes(), len),
        Request::Keys(len) => {
            let keys = hub.keys(dev).ok_or(Error::NoDevice)?.to_bytes();
            Reply::bytes(keys, len)
        }
        Request::Leds(len) => Reply::bytes(hub.leds(dev).to_le_bytes().to_vec(), len),
        // No sounds or switches are ever on.
        Request::Sounds(len) | Request::Switches(len) => Reply::bytes(alloc::vec![0; 8], len),
        Request::Bits { kind, len } => Reply::bytes(caps.bitmap(kind).ok_or(Error::Invalid)?, len),
        Request::GetAbs(code) => {
            if !caps.has(crate::codes::ev::ABS, code) {
                return Err(Error::Invalid);
            }
            Reply::data(absinfo_bytes(&info.abs[usize::from(code)]))
        }
        Request::SetAbs(code) => {
            let a = AbsInfo {
                value: le_i32(input, 0),
                minimum: le_i32(input, 4),
                maximum: le_i32(input, 8),
                fuzz: le_i32(input, 12),
                flat: le_i32(input, 16),
                resolution: le_i32(input, 20),
            };
            hub.set_abs(dev, code, a)?;
            Reply::value(0)
        }
        Request::Grab => {
            hub.grab(dev, client, arg != 0)?;
            Reply::value(0)
        }
        Request::Revoke => {
            if arg != 0 {
                return Err(Error::Invalid);
            }
            hub.revoke(dev, client)?;
            Reply::value(0)
        }
        Request::SetClock => {
            let clock = match le_i32(input, 0) {
                CLOCK_REALTIME => Clock::Realtime,
                CLOCK_MONOTONIC => Clock::Monotonic,
                CLOCK_BOOTTIME => Clock::Boottime,
                _ => return Err(Error::Invalid),
            };
            hub.set_clock(dev, client, clock)?;
            Reply::value(0)
        }
        Request::Effects => Reply::data(0i32.to_le_bytes().to_vec()),
        Request::Unsupported => return Err(Error::Invalid),
    })
}

/// [`Request::decode`] and [`ioctl`] together, for callers that have the
/// input bytes already (the tests).
pub fn ioctl_raw(
    hub: &mut Hub,
    dev: usize,
    client: ClientId,
    request: u32,
    arg: u64,
    input: &[u8],
) -> Result<Reply, Error> {
    let req = Request::decode(request).ok_or(Error::NotTty)?;
    ioctl(hub, dev, client, req, arg, input)
}

#[cfg(test)]
mod tests;
