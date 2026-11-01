//! The sound card's master volume through ALSA's control device
//! (`/dev/snd/controlC0`, ADR 0109's mixer controls), for the ridge and
//! the settings window (ADR 0117).
//!
//! The structures and request numbers are from the public uapi header
//! `sound/asound.h` (`struct snd_ctl_card_info`, `snd_ctl_elem_id`,
//! `snd_ctl_elem_info`, `snd_ctl_elem_value`, `snd_ctl_event`, the
//! `SNDRV_CTL_IOCTL_*` numbers), with the offsets of x86-64. The controls
//! are found by name, as ALSA's control-name conventions call them:
//! "Master Playback Volume" and "Master Playback Switch".

use std::ffi::CString;
use std::io;
use std::os::fd::RawFd;

/// `SNDRV_CTL_IOCTL_*`.
mod ioctl {
    pub const CARD_INFO: u64 = 0x8178_5501;
    pub const ELEM_INFO: u64 = 0xc110_5511;
    pub const ELEM_READ: u64 = 0xc4c8_5512;
    pub const ELEM_WRITE: u64 = 0xc4c8_5513;
    pub const SUBSCRIBE_EVENTS: u64 = 0xc004_5516;
}

/// The structures' sizes.
const CARD_INFO_LEN: usize = 376;
const ELEM_ID_LEN: usize = 64;
const ELEM_INFO_LEN: usize = 272;
const ELEM_VALUE_LEN: usize = 1224;
const EVENT_LEN: usize = 72;
/// `SNDRV_CTL_ELEM_IFACE_MIXER`.
const IFACE_MIXER: i32 = 2;
/// `SNDRV_CTL_ELEM_TYPE_BOOLEAN`, `_INTEGER`.
const TYPE_BOOLEAN: i32 = 1;
const TYPE_INTEGER: i32 = 2;

pub const VOLUME: &str = "Master Playback Volume";
pub const SWITCH: &str = "Master Playback Switch";

/// A `struct snd_ctl_elem_id` naming a mixer control (numid 0: by name).
pub fn elem_id(numid: u32, name: &str) -> [u8; ELEM_ID_LEN] {
    let mut b = [0u8; ELEM_ID_LEN];
    b[0..4].copy_from_slice(&numid.to_ne_bytes());
    b[4..8].copy_from_slice(&IFACE_MIXER.to_ne_bytes());
    let n = name.len().min(43);
    b[16..16 + n].copy_from_slice(&name.as_bytes()[..n]);
    b
}

/// What `ELEM_INFO` says about a control: its numid, type, value count
/// and an integer's range.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ElemInfo {
    pub numid: u32,
    pub kind: i32,
    pub count: u32,
    pub min: i64,
    pub max: i64,
}

pub fn parse_info(b: &[u8; ELEM_INFO_LEN]) -> ElemInfo {
    let u32_at = |o: usize| u32::from_ne_bytes(b[o..o + 4].try_into().unwrap_or([0; 4]));
    let i64_at = |o: usize| i64::from_ne_bytes(b[o..o + 8].try_into().unwrap_or([0; 8]));
    ElemInfo { numid: u32_at(0), kind: u32_at(64) as i32, count: u32_at(72), min: i64_at(80), max: i64_at(88) }
}

/// A `struct snd_ctl_elem_value` for control `numid` with these values.
pub fn elem_value(numid: u32, name: &str, values: &[i64]) -> [u8; ELEM_VALUE_LEN] {
    let mut b = [0u8; ELEM_VALUE_LEN];
    b[..ELEM_ID_LEN].copy_from_slice(&elem_id(numid, name));
    for (i, v) in values.iter().enumerate().take(128) {
        b[72 + 8 * i..80 + 8 * i].copy_from_slice(&v.to_ne_bytes());
    }
    b
}

/// The first values of an `ELEM_READ` result.
pub fn value_at(b: &[u8; ELEM_VALUE_LEN], i: usize) -> i64 {
    i64::from_ne_bytes(b[72 + 8 * i..80 + 8 * i].try_into().unwrap_or([0; 8]))
}

/// The card's short name from `struct snd_ctl_card_info`.
pub fn card_name(b: &[u8; CARD_INFO_LEN]) -> String {
    let name = &b[40..72];
    let end = name.iter().position(|&c| c == 0).unwrap_or(name.len());
    String::from_utf8_lossy(&name[..end]).into_owned()
}

/// The master volume and mute of the first sound card.
#[derive(Debug)]
pub struct Mixer {
    fd: RawFd,
    pub card: String,
    volume: ElemInfo,
    switch: Option<ElemInfo>,
}

impl Drop for Mixer {
    fn drop(&mut self) {
        crate::unix::close(self.fd);
    }
}

impl Mixer {
    /// Open the control device and find the master controls.
    pub fn open(path: &str) -> io::Result<Mixer> {
        let c = CString::new(path).map_err(|_| io::ErrorKind::InvalidInput)?;
        // SAFETY: a NUL-terminated path.
        let fd = unsafe { libc::open(c.as_ptr(), libc::O_RDWR | libc::O_CLOEXEC | libc::O_NONBLOCK) };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        let mut m = Mixer {
            fd,
            card: String::new(),
            volume: ElemInfo { numid: 0, kind: 0, count: 0, min: 0, max: 0 },
            switch: None,
        };
        let mut info = [0u8; CARD_INFO_LEN];
        if m.ioctl(ioctl::CARD_INFO, info.as_mut_ptr()).is_ok() {
            m.card = card_name(&info);
        }
        m.volume = m.info(VOLUME)?;
        if m.volume.kind != TYPE_INTEGER || m.volume.max <= m.volume.min {
            return Err(io::Error::other("the master volume is not an integer control"));
        }
        m.switch = m.info(SWITCH).ok().filter(|i| i.kind == TYPE_BOOLEAN);
        // Changes made by others come as events on the descriptor.
        let mut on: i32 = 1;
        let _ = m.ioctl(ioctl::SUBSCRIBE_EVENTS, (&mut on as *mut i32).cast());
        Ok(m)
    }

    /// The descriptor to poll for change events.
    pub fn fd(&self) -> RawFd {
        self.fd
    }

    fn ioctl(&self, request: u64, arg: *mut u8) -> io::Result<()> {
        // SAFETY: `arg` points at a buffer of the size `request` encodes,
        // owned by the caller for the call.
        let r = unsafe { libc::ioctl(self.fd, request as _, arg) };
        if r < 0 { Err(io::Error::last_os_error()) } else { Ok(()) }
    }

    fn info(&self, name: &str) -> io::Result<ElemInfo> {
        let mut b = [0u8; ELEM_INFO_LEN];
        b[..ELEM_ID_LEN].copy_from_slice(&elem_id(0, name));
        self.ioctl(ioctl::ELEM_INFO, b.as_mut_ptr())?;
        Ok(parse_info(&b))
    }

    fn read_values(&self, e: &ElemInfo, name: &str) -> io::Result<[u8; ELEM_VALUE_LEN]> {
        let mut b = elem_value(e.numid, name, &[]);
        self.ioctl(ioctl::ELEM_READ, b.as_mut_ptr())?;
        Ok(b)
    }

    fn write_values(&self, e: &ElemInfo, name: &str, v: i64) -> io::Result<()> {
        let values = vec![v; e.count.clamp(1, 128) as usize];
        let mut b = elem_value(e.numid, name, &values);
        self.ioctl(ioctl::ELEM_WRITE, b.as_mut_ptr())
    }

    /// The volume in percent (the first channel) and whether it is muted.
    pub fn read(&self) -> io::Result<(u32, bool)> {
        let v = value_at(&self.read_values(&self.volume, VOLUME)?, 0);
        let pct = percent(v, self.volume.min, self.volume.max);
        let muted = match &self.switch {
            Some(s) => value_at(&self.read_values(s, SWITCH)?, 0) == 0,
            None => false,
        };
        Ok((pct, muted))
    }

    /// Set every channel to `pct` percent.
    pub fn set_volume(&self, pct: u32) -> io::Result<()> {
        let (min, max) = (self.volume.min, self.volume.max);
        self.write_values(&self.volume, VOLUME, min + (max - min) * i64::from(pct.min(100)) / 100)
    }

    /// Mute or unmute (the switch is "on" when sound plays).
    pub fn set_muted(&self, muted: bool) -> io::Result<()> {
        match &self.switch {
            Some(s) => self.write_values(s, SWITCH, i64::from(!muted)),
            None => Err(io::Error::other("no master switch")),
        }
    }

    /// Read the pending change events; true if there were any.
    pub fn drain_events(&self) -> bool {
        let mut any = false;
        let mut ev = [0u8; EVENT_LEN];
        // SAFETY: reading into a buffer of the length given.
        while unsafe { libc::read(self.fd, ev.as_mut_ptr().cast(), EVENT_LEN) } > 0 {
            any = true;
        }
        any
    }
}

/// `v` in `min..=max` as a percentage, rounded.
pub fn percent(v: i64, min: i64, max: i64) -> u32 {
    if max <= min {
        return 0;
    }
    (((v - min) * 100 + (max - min) / 2) / (max - min)).clamp(0, 100) as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layouts() {
        let id = elem_id(0, VOLUME);
        assert_eq!(&id[4..8], &2i32.to_ne_bytes(), "the mixer interface");
        assert_eq!(&id[16..16 + VOLUME.len()], VOLUME.as_bytes());
        assert_eq!(id[16 + VOLUME.len()], 0);
        let v = elem_value(7, VOLUME, &[50, 60]);
        assert_eq!(&v[0..4], &7u32.to_ne_bytes());
        assert_eq!((value_at(&v, 0), value_at(&v, 1), value_at(&v, 2)), (50, 60, 0));
        let mut info = [0u8; ELEM_INFO_LEN];
        info[0..4].copy_from_slice(&3u32.to_ne_bytes());
        info[64..68].copy_from_slice(&2i32.to_ne_bytes());
        info[72..76].copy_from_slice(&2u32.to_ne_bytes());
        info[88..96].copy_from_slice(&100i64.to_ne_bytes());
        assert_eq!(parse_info(&info), ElemInfo { numid: 3, kind: 2, count: 2, min: 0, max: 100 });
        let mut card = [0u8; CARD_INFO_LEN];
        card[40..52].copy_from_slice(b"virtio-sound");
        assert_eq!(card_name(&card), "virtio-sound");
        assert_eq!((percent(50, 0, 100), percent(32, 0, 64), percent(0, 0, 0)), (50, 50, 0));
    }

    #[test]
    fn no_card_here_is_an_error() {
        assert!(Mixer::open("/nonexistent/controlC0").is_err());
    }
}
