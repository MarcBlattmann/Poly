//! Mixer controls on the control device (`/dev/snd/controlC<c>`): the
//! `SNDRV_CTL_IOCTL_ELEM_*` requests over a fixed set of elements (ADR
//! 0109).
//!
//! The card has the controls ALSA's control-name conventions give a simple
//! card ("Master Playback Volume", "Master Playback Switch", and for
//! capture "Capture Volume", "Capture Switch"), two channels each. The
//! audio server applies them as software gains ([`Controls::playback_gains`],
//! [`Controls::capture_gains`]); neither virtio-sound (VIRTIO 1.2 has no
//! volume) nor our HDA path setup has a hardware volume behind them.
//!
//! Elements are numbered from 1 (`numid`) in table order, and also found
//! by interface, name and index. Layouts are those of the uapi header
//! `sound/asound.h` (x86-64), checked with `offsetof`: `snd_ctl_elem_id`
//! 64 bytes (name at 16, index at 60), `snd_ctl_elem_info` 272 (type at
//! 64, access 68, count 72, owner 76, value 80), `snd_ctl_elem_value` 1224
//! (value at 72), `snd_ctl_event` 72 (the element id at 8).

use super::abi::{
    CTL_EVENT_LEN, ELEM_ID_LEN, ELEM_INFO_LEN, ELEM_LIST_LEN, ELEM_VALUE_LEN, get32, get64, put_str, put32, put64,
};
use crate::errno::Errno;

/// `SNDRV_CTL_ELEM_TYPE_*`.
pub mod elem_type {
    pub const BOOLEAN: u32 = 1;
    pub const INTEGER: u32 = 2;
}

/// `SNDRV_CTL_ELEM_IFACE_*`.
pub mod iface {
    pub const CARD: u32 = 0;
    pub const MIXER: u32 = 2;
    pub const PCM: u32 = 3;
}

/// `SNDRV_CTL_ELEM_ACCESS_*`.
pub mod access {
    pub const READ: u32 = 1 << 0;
    pub const WRITE: u32 = 1 << 1;
    pub const READWRITE: u32 = READ | WRITE;
}

/// `SNDRV_CTL_EVENT_ELEM` and `SNDRV_CTL_EVENT_MASK_VALUE`.
pub const EVENT_ELEM: u32 = 0;
pub const EVENT_MASK_VALUE: u32 = 1 << 0;

/// The volume range of the volume controls: linear in amplitude, 0 is
/// silence and [`VOLUME_MAX`] unity gain.
pub const VOLUME_MAX: i64 = 100;
/// Values per element (left and right).
pub const CHANNELS: usize = 2;

/// `struct snd_ctl_elem_id`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ElemId {
    pub numid: u32,
    pub iface: u32,
    pub device: u32,
    pub subdevice: u32,
    pub name: [u8; 44],
    pub index: u32,
}

impl ElemId {
    pub fn decode(b: &[u8]) -> ElemId {
        let mut name = [0u8; 44];
        name.copy_from_slice(&b[16..60]);
        ElemId {
            numid: get32(b, 0),
            iface: get32(b, 4),
            device: get32(b, 8),
            subdevice: get32(b, 12),
            name,
            index: get32(b, 60),
        }
    }

    pub fn encode(&self) -> [u8; ELEM_ID_LEN] {
        let mut b = [0u8; ELEM_ID_LEN];
        put32(&mut b, 0, self.numid);
        put32(&mut b, 4, self.iface);
        put32(&mut b, 8, self.device);
        put32(&mut b, 12, self.subdevice);
        b[16..60].copy_from_slice(&self.name);
        put32(&mut b, 60, self.index);
        b
    }

    /// The name up to its terminating zero.
    pub fn name(&self) -> &[u8] {
        let n = self.name.iter().position(|&c| c == 0).unwrap_or(self.name.len());
        &self.name[..n]
    }
}

/// What an element holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Boolean,
    Integer { min: i64, max: i64, step: i64 },
}

/// One control.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Elem {
    pub name: &'static str,
    pub iface: u32,
    pub kind: Kind,
    pub values: [i64; CHANNELS],
}

impl Elem {
    fn id(&self, numid: u32) -> ElemId {
        let mut name = [0u8; 44];
        let n = self.name.len().min(43);
        name[..n].copy_from_slice(&self.name.as_bytes()[..n]);
        ElemId { numid, iface: self.iface, device: 0, subdevice: 0, name, index: 0 }
    }

    fn accepts(&self, v: i64) -> bool {
        match self.kind {
            Kind::Boolean => v == 0 || v == 1,
            Kind::Integer { min, max, .. } => (min..=max).contains(&v),
        }
    }
}

/// The most elements a card has.
pub const MAX_ELEMS: usize = 4;

const MASTER_VOLUME: &str = "Master Playback Volume";
const MASTER_SWITCH: &str = "Master Playback Switch";
const CAPTURE_VOLUME: &str = "Capture Volume";
const CAPTURE_SWITCH: &str = "Capture Switch";

/// The card's controls.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Controls {
    elems: [Option<Elem>; MAX_ELEMS],
    count: usize,
}

/// The result of a write: the bytes to hand back, and the element whose
/// value changed (for a `SNDRV_CTL_EVENT_MASK_VALUE` event).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Written {
    pub reply: [u8; ELEM_VALUE_LEN],
    pub changed: Option<ElemId>,
}

impl Controls {
    /// The controls of a card with playback and/or capture, at full volume
    /// and switched on.
    pub fn new(playback: bool, capture: bool) -> Controls {
        let volume = Kind::Integer { min: 0, max: VOLUME_MAX, step: 1 };
        let mut c = Controls { elems: [None; MAX_ELEMS], count: 0 };
        let mut add = |name, kind, v| {
            c.elems[c.count] = Some(Elem { name, iface: iface::MIXER, kind, values: [v; CHANNELS] });
            c.count += 1;
        };
        if playback {
            add(MASTER_VOLUME, volume, VOLUME_MAX);
            add(MASTER_SWITCH, Kind::Boolean, 1);
        }
        if capture {
            add(CAPTURE_VOLUME, volume, VOLUME_MAX);
            add(CAPTURE_SWITCH, Kind::Boolean, 1);
        }
        c
    }

    pub fn len(&self) -> usize {
        self.count
    }

    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    fn elems(&self) -> impl Iterator<Item = (usize, &Elem)> {
        self.elems[..self.count].iter().enumerate().filter_map(|(i, e)| e.as_ref().map(|e| (i, e)))
    }

    /// The element `id` names: by `numid` if it is set, else by interface,
    /// name and index.
    pub fn find(&self, id: &ElemId) -> Option<usize> {
        if id.numid != 0 {
            let i = id.numid as usize - 1;
            return (i < self.count).then_some(i);
        }
        self.elems()
            .find(|(_, e)| e.iface == id.iface && e.name.as_bytes() == id.name() && id.index == 0)
            .map(|(i, _)| i)
    }

    fn elem(&self, i: usize) -> &Elem {
        self.elems[i].as_ref().unwrap_or_else(|| unreachable!("element {i} exists"))
    }

    /// The id of element `i` (numbered from 0).
    pub fn id(&self, i: usize) -> Option<ElemId> {
        (i < self.count).then(|| self.elem(i).id(i as u32 + 1))
    }

    /// `SNDRV_CTL_IOCTL_ELEM_LIST`: fills `used` and `count` in `b`, and
    /// returns which elements' ids go to the caller's `pids` array
    /// (from `offset`, at most `space`).
    pub fn list(&self, b: &mut [u8; ELEM_LIST_LEN]) -> core::ops::Range<usize> {
        let offset = (get32(b, 0) as usize).min(self.count);
        let space = get32(b, 4) as usize;
        let used = space.min(self.count - offset);
        put32(b, 8, used as u32);
        put32(b, 12, self.count as u32);
        offset..offset + used
    }

    /// Where the caller's ids go: the `pids` pointer of `snd_ctl_elem_list`.
    pub fn list_pointer(b: &[u8; ELEM_LIST_LEN]) -> u64 {
        get64(b, 16)
    }

    /// `SNDRV_CTL_IOCTL_ELEM_INFO`.
    pub fn info(&self, req: &[u8; ELEM_INFO_LEN]) -> Result<[u8; ELEM_INFO_LEN], Errno> {
        let i = self.find(&ElemId::decode(req)).ok_or(Errno::ENOENT)?;
        let e = self.elem(i);
        let mut b = [0u8; ELEM_INFO_LEN];
        b[..ELEM_ID_LEN].copy_from_slice(&e.id(i as u32 + 1).encode());
        let (ty, min, max, step) = match e.kind {
            Kind::Boolean => (elem_type::BOOLEAN, 0, 1, 0),
            Kind::Integer { min, max, step } => (elem_type::INTEGER, min, max, step),
        };
        put32(&mut b, 64, ty);
        put32(&mut b, 68, access::READWRITE);
        put32(&mut b, 72, CHANNELS as u32);
        put32(&mut b, 76, 0);
        put64(&mut b, 80, min as u64);
        put64(&mut b, 88, max as u64);
        put64(&mut b, 96, step as u64);
        Ok(b)
    }

    /// `SNDRV_CTL_IOCTL_ELEM_READ`: the request with its id completed and
    /// the values filled in.
    pub fn read(&self, req: &[u8; ELEM_VALUE_LEN]) -> Result<[u8; ELEM_VALUE_LEN], Errno> {
        let i = self.find(&ElemId::decode(req)).ok_or(Errno::ENOENT)?;
        let e = self.elem(i);
        let mut b = *req;
        b[..ELEM_ID_LEN].copy_from_slice(&e.id(i as u32 + 1).encode());
        for (c, v) in e.values.iter().enumerate() {
            put64(&mut b, 72 + 8 * c, *v as u64);
        }
        Ok(b)
    }

    /// `SNDRV_CTL_IOCTL_ELEM_WRITE`: every value must be in range
    /// (`EINVAL` otherwise, and nothing changes).
    pub fn write(&mut self, req: &[u8; ELEM_VALUE_LEN]) -> Result<Written, Errno> {
        let i = self.find(&ElemId::decode(req)).ok_or(Errno::ENOENT)?;
        let mut values = [0i64; CHANNELS];
        for (c, v) in values.iter_mut().enumerate() {
            *v = get64(req, 72 + 8 * c) as i64;
        }
        let e = self.elems[i].as_mut().ok_or(Errno::ENOENT)?;
        if !values.iter().all(|&v| e.accepts(v)) {
            return Err(Errno::EINVAL);
        }
        let changed = e.values != values;
        e.values = values;
        let id = e.id(i as u32 + 1);
        let mut reply = *req;
        reply[..ELEM_ID_LEN].copy_from_slice(&id.encode());
        Ok(Written { reply, changed: changed.then_some(id) })
    }

    fn values_of(&self, name: &str) -> Option<[i64; CHANNELS]> {
        self.elems().find(|(_, e)| e.name == name).map(|(_, e)| e.values)
    }

    fn gains(&self, volume: &str, switch: &str) -> [u32; CHANNELS] {
        let v = self.values_of(volume).unwrap_or([VOLUME_MAX; CHANNELS]);
        let s = self.values_of(switch).unwrap_or([1; CHANNELS]);
        core::array::from_fn(|c| {
            if s[c] == 0 { 0 } else { ((v[c].clamp(0, VOLUME_MAX) as u64 * (1 << 16)) / VOLUME_MAX as u64) as u32 }
        })
    }

    /// The master gain per channel (16.16), volume and switch together.
    pub fn playback_gains(&self) -> [u32; CHANNELS] {
        self.gains(MASTER_VOLUME, MASTER_SWITCH)
    }

    /// The capture gain per channel (16.16).
    pub fn capture_gains(&self) -> [u32; CHANNELS] {
        self.gains(CAPTURE_VOLUME, CAPTURE_SWITCH)
    }
}

/// `struct snd_ctl_event` for a change of `mask` on `id`.
pub fn event(id: &ElemId, mask: u32) -> [u8; CTL_EVENT_LEN] {
    let mut b = [0u8; CTL_EVENT_LEN];
    put32(&mut b, 0, EVENT_ELEM);
    put32(&mut b, 4, mask);
    b[8..8 + ELEM_ID_LEN].copy_from_slice(&id.encode());
    b
}

/// An id naming element `name` of interface `iface` (as a caller would
/// ask for it).
pub fn id_by_name(iface: u32, name: &str) -> ElemId {
    let mut b = [0u8; ELEM_ID_LEN];
    put32(&mut b, 4, iface);
    put_str(&mut b, 16, 44, name);
    ElemId::decode(&b)
}
