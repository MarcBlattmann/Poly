//! Input from the Linux event devices, `/dev/input/event*` (ADR 0103,
//! 0107): each device opened, described with `EVIOCGBIT`/`EVIOCGABS`,
//! grabbed (`EVIOCGRAB`, so the kernel console does not read the keys as
//! well), and its events turned into `poly-wayland`'s
//! [`InputEvent`]s: the [`InputSource`] of the seat.
//!
//! The translation ([`Translator`]) is plain logic, tested on the host:
//! events build up until `SYN_REPORT` ends a frame; then relative motion
//! (`REL_X`/`REL_Y`) becomes one `PointerMotion`, an absolute position
//! (`ABS_X`/`ABS_Y`, scaled from the axis' range onto the output) one
//! `PointerMotionAbsolute`, and buttons (`BTN_*`), keys (`KEY_*`, presses
//! and releases; the device's own repeats are dropped, clients repeat by
//! `wl_keyboard.repeat_info`) and wheels (`REL_WHEEL` positive away from
//! the user, which scrolls up: negative `axis` values; `REL_HWHEEL`)
//! follow in their order. `SYN_DROPPED` throws the frame away.
//!
//! Sources: linux/input.h and linux/input-event-codes.h (ABI facts: the
//! event layout, the codes, the `EVIOC*` request numbers) and the kernel's
//! input documentation (`input.rst`, `event-codes.rst`).

use poly_wayland::seat::{Capabilities, InputEvent, InputSource};
use poly_wayland::wire::Fixed;
use std::collections::VecDeque;
use std::ffi::CString;
use std::os::fd::RawFd;

const EV_SYN: u16 = 0;
const EV_KEY: u16 = 1;
const EV_REL: u16 = 2;
const EV_ABS: u16 = 3;
const SYN_REPORT: u16 = 0;
const SYN_DROPPED: u16 = 3;
const REL_X: u16 = 0;
const REL_Y: u16 = 1;
const REL_HWHEEL: u16 = 6;
const REL_WHEEL: u16 = 8;
const ABS_X: u16 = 0;
const ABS_Y: u16 = 1;
/// `BTN_MISC` to `BTN_GEAR_UP`: the button codes; keys are below.
const BUTTONS: std::ops::Range<u16> = 0x100..0x160;
const KEY_A: u16 = 30;
/// How far one wheel step scrolls, logical pixels.
pub const WHEEL_STEP: i32 = 15;
/// Bytes of `struct input_event` on x86-64.
const EVENT_SIZE: usize = 24;

/// What a device is to the seat.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Keyboard,
    /// Relative motion (a mouse).
    Mouse,
    /// Absolute positions (a tablet, a VM's pointer), with the axes'
    /// ranges.
    Tablet {
        x: (i32, i32),
        y: (i32, i32),
    },
}

/// One device's frame being built.
#[derive(Debug)]
pub struct Translator {
    pub kind: Kind,
    dx: i32,
    dy: i32,
    abs: (Option<i32>, Option<i32>),
    /// The last absolute position, for a frame that moves one axis.
    last_abs: (i32, i32),
    /// Buttons, keys and wheels of the frame, in order (time filled in at
    /// the end).
    rest: Vec<InputEvent>,
}

impl Translator {
    pub fn new(kind: Kind) -> Translator {
        Translator { kind, dx: 0, dy: 0, abs: (None, None), last_abs: (0, 0), rest: Vec::new() }
    }

    /// Scale `v` from the axis range onto `0..size` logical pixels.
    fn scale(v: i32, (min, max): (i32, i32), size: i32) -> Fixed {
        let span = i64::from(max) - i64::from(min);
        if span <= 0 {
            return Fixed::from_int(0);
        }
        let pos = (i64::from(v) - i64::from(min)).clamp(0, span);
        Fixed(((pos * i64::from(size - 1).max(0) * 256) / span) as i32)
    }

    /// One event; at `SYN_REPORT` what the frame means is appended to
    /// `out`. `size` is the output's logical size, `time` the time in ms.
    pub fn feed(&mut self, ty: u16, code: u16, value: i32, time: u32, size: (i32, i32), out: &mut Vec<InputEvent>) {
        match (ty, code) {
            (EV_SYN, SYN_REPORT) => self.flush(time, size, out),
            (EV_SYN, SYN_DROPPED) => {
                (self.dx, self.dy, self.abs) = (0, 0, (None, None));
                self.rest.clear();
            }
            (EV_REL, REL_X) => self.dx += value,
            (EV_REL, REL_Y) => self.dy += value,
            (EV_REL, REL_WHEEL) => self.rest.push(InputEvent::PointerAxis {
                time,
                horizontal: false,
                value: Fixed::from_int(-value * WHEEL_STEP),
            }),
            (EV_REL, REL_HWHEEL) => self.rest.push(InputEvent::PointerAxis {
                time,
                horizontal: true,
                value: Fixed::from_int(value * WHEEL_STEP),
            }),
            (EV_ABS, ABS_X) => self.abs.0 = Some(value),
            (EV_ABS, ABS_Y) => self.abs.1 = Some(value),
            (EV_KEY, c) if value == 2 => {
                let _ = c; // the device's own repeat
            }
            (EV_KEY, c) if BUTTONS.contains(&c) => {
                self.rest.push(InputEvent::PointerButton { time, button: u32::from(c), pressed: value != 0 })
            }
            (EV_KEY, c) => self.rest.push(InputEvent::Key { time, key: u32::from(c), pressed: value != 0 }),
            _ => {}
        }
    }

    fn flush(&mut self, time: u32, size: (i32, i32), out: &mut Vec<InputEvent>) {
        if self.dx != 0 || self.dy != 0 {
            out.push(InputEvent::PointerMotion { time, dx: Fixed::from_int(self.dx), dy: Fixed::from_int(self.dy) });
            (self.dx, self.dy) = (0, 0);
        }
        if let Kind::Tablet { x, y } = self.kind
            && (self.abs.0.is_some() || self.abs.1.is_some())
        {
            let ax = self.abs.0.unwrap_or(self.last_abs.0);
            let ay = self.abs.1.unwrap_or(self.last_abs.1);
            self.last_abs = (ax, ay);
            self.abs = (None, None);
            out.push(InputEvent::PointerMotionAbsolute {
                time,
                x: Self::scale(ax, x, size.0),
                y: Self::scale(ay, y, size.1),
            });
        }
        out.append(&mut self.rest);
    }
}

/// `_IOC(dir, 'E', nr, size)` (asm-generic/ioctl.h): read is 2, write 1.
const fn evioc(dir: u32, nr: u32, size: u32) -> u32 {
    dir << 30 | size << 16 | (b'E' as u32) << 8 | nr
}

/// `EVIOCGBIT(ev, len)`.
const fn eviocgbit(ev: u32, len: u32) -> u32 {
    evioc(2, 0x20 + ev, len)
}

/// `EVIOCGABS(abs)`: `struct input_absinfo`, six `__s32`s.
const fn eviocgabs(abs: u32) -> u32 {
    evioc(2, 0x40 + abs, 24)
}

/// `EVIOCGRAB`: an `int` by value.
const EVIOCGRAB: u32 = evioc(1, 0x90, 4);

/// `EVIOCGNAME(len)`.
const fn eviocgname(len: u32) -> u32 {
    evioc(2, 0x06, len)
}

fn bit(bits: &[u8], n: u16) -> bool {
    bits.get(usize::from(n / 8)).is_some_and(|b| b & (1 << (n % 8)) != 0)
}

/// What the device's bitmaps say it is, if it is anything to the seat.
pub fn classify(types: &[u8], keys: &[u8], rels: &[u8], abs: &[u8]) -> Option<bool> {
    // Some(true): pointer-like; Some(false): keyboard.
    if bit(types, EV_ABS) && bit(abs, ABS_X) && bit(abs, ABS_Y) {
        return Some(true);
    }
    if bit(types, EV_REL) && bit(rels, REL_X) && bit(rels, REL_Y) {
        return Some(true);
    }
    if bit(types, EV_KEY) && bit(keys, KEY_A) {
        return Some(false);
    }
    None
}

/// An opened event device.
#[derive(Debug)]
pub struct Device {
    pub fd: RawFd,
    pub name: String,
    pub translator: Translator,
}

fn ioctl_buf(fd: RawFd, request: u32, buf: &mut [u8]) -> bool {
    // SAFETY: `buf` is as long as the size `request` encodes (each caller
    // builds the request from `buf.len()`).
    unsafe { libc::ioctl(fd, request as libc::c_ulong, buf.as_mut_ptr()) >= 0 }
}

impl Device {
    /// Open and describe `path`; `None` if it is nothing the seat uses.
    pub fn open(path: &str) -> Option<Device> {
        let c = CString::new(path).ok()?;
        // SAFETY: a NUL-terminated path.
        let fd = unsafe { libc::open(c.as_ptr(), libc::O_RDONLY | libc::O_NONBLOCK | libc::O_CLOEXEC) };
        if fd < 0 {
            return None;
        }
        let mut types = [0u8; 4];
        let mut keys = [0u8; 96];
        let mut rels = [0u8; 4];
        let mut abs = [0u8; 8];
        let mut name = [0u8; 64];
        let ok = ioctl_buf(fd, eviocgbit(0, types.len() as u32), &mut types);
        ioctl_buf(fd, eviocgbit(u32::from(EV_KEY), keys.len() as u32), &mut keys);
        ioctl_buf(fd, eviocgbit(u32::from(EV_REL), rels.len() as u32), &mut rels);
        ioctl_buf(fd, eviocgbit(u32::from(EV_ABS), abs.len() as u32), &mut abs);
        ioctl_buf(fd, eviocgname(name.len() as u32), &mut name);
        let name =
            String::from_utf8_lossy(&name[..name.iter().position(|&b| b == 0).unwrap_or(name.len())]).into_owned();
        let kind = match (ok, classify(&types, &keys, &rels, &abs)) {
            (true, Some(true)) if bit(&types, EV_ABS) && bit(&abs, ABS_X) => {
                let range = |axis: u16| {
                    let mut info = [0u8; 24];
                    if !ioctl_buf(fd, eviocgabs(u32::from(axis)), &mut info) {
                        return (0, 0x7fff);
                    }
                    let w = |i: usize| i32::from_le_bytes([info[i], info[i + 1], info[i + 2], info[i + 3]]);
                    (w(4), w(8))
                };
                Kind::Tablet { x: range(ABS_X), y: range(ABS_Y) }
            }
            (true, Some(true)) => Kind::Mouse,
            (true, Some(false)) => Kind::Keyboard,
            _ => {
                crate::unix::close(fd);
                return None;
            }
        };
        // Our events alone (the kernel console would read keys too).
        // SAFETY: EVIOCGRAB takes its argument by value.
        unsafe { libc::ioctl(fd, EVIOCGRAB as libc::c_ulong, 1 as libc::c_ulong) };
        Some(Device { fd, name, translator: Translator::new(kind) })
    }
}

impl Drop for Device {
    fn drop(&mut self) {
        crate::unix::close(self.fd);
    }
}

/// Every input device, as the seat's [`InputSource`].
#[derive(Debug, Default)]
pub struct Evdev {
    pub devices: Vec<Device>,
    queue: VecDeque<InputEvent>,
}

impl Evdev {
    /// Open `/dev/input/event0` to `event31`.
    pub fn open_all() -> Evdev {
        let devices = (0..32).filter_map(|n| Device::open(&format!("/dev/input/event{n}"))).collect();
        Evdev { devices, queue: VecDeque::new() }
    }

    /// Read what device `index` has (it was readable); `size` is the
    /// output's logical size, `time` now in ms.
    pub fn read(&mut self, index: usize, size: (i32, i32), time: u32) {
        let Some(dev) = self.devices.get_mut(index) else { return };
        let mut buf = [0u8; EVENT_SIZE * 64];
        let mut out = Vec::new();
        loop {
            // SAFETY: `buf` is a live buffer of the length given.
            let n = unsafe { libc::read(dev.fd, buf.as_mut_ptr().cast(), buf.len()) };
            if n <= 0 {
                break;
            }
            for e in buf[..n as usize].chunks_exact(EVENT_SIZE) {
                let ty = u16::from_le_bytes([e[16], e[17]]);
                let code = u16::from_le_bytes([e[18], e[19]]);
                let value = i32::from_le_bytes([e[20], e[21], e[22], e[23]]);
                dev.translator.feed(ty, code, value, time, size, &mut out);
            }
            if (n as usize) < buf.len() {
                break;
            }
        }
        self.queue.extend(out);
    }
}

impl InputSource for Evdev {
    fn capabilities(&self) -> Capabilities {
        self.devices.iter().fold(Capabilities::NONE, |c, d| {
            c.union(match d.translator.kind {
                Kind::Keyboard => Capabilities::KEYBOARD,
                Kind::Mouse | Kind::Tablet { .. } => Capabilities::POINTER,
            })
        })
    }

    fn next_event(&mut self) -> Option<InputEvent> {
        self.queue.pop_front()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SIZE: (i32, i32) = (1280, 800);

    fn frame(t: &mut Translator, events: &[(u16, u16, i32)]) -> Vec<InputEvent> {
        let mut out = Vec::new();
        for &(ty, code, value) in events {
            t.feed(ty, code, value, 5, SIZE, &mut out);
        }
        t.feed(EV_SYN, SYN_REPORT, 0, 5, SIZE, &mut out);
        out
    }

    #[test]
    fn request_numbers() {
        // linux/input.h, as a C program prints them on x86-64.
        assert_eq!(eviocgbit(0, 4), 0x8004_4520);
        assert_eq!(eviocgabs(0), 0x8018_4540);
        assert_eq!(EVIOCGRAB, 0x4004_4590);
        assert_eq!(eviocgname(64), 0x8040_4506);
    }

    #[test]
    fn mouse_frames() {
        let mut t = Translator::new(Kind::Mouse);
        let out = frame(&mut t, &[(EV_REL, REL_X, 3), (EV_REL, REL_Y, -2), (EV_REL, REL_X, 1)]);
        assert_eq!(out, [InputEvent::PointerMotion { time: 5, dx: Fixed::from_int(4), dy: Fixed::from_int(-2) }]);
        // A click with motion: the motion first.
        let out = frame(&mut t, &[(EV_KEY, 0x110, 1), (EV_REL, REL_X, 1)]);
        assert_eq!(out.len(), 2);
        assert!(matches!(out[0], InputEvent::PointerMotion { .. }));
        assert_eq!(out[1], InputEvent::PointerButton { time: 5, button: 0x110, pressed: true });
        // The wheel up: scroll up.
        let out = frame(&mut t, &[(EV_REL, REL_WHEEL, 1)]);
        assert_eq!(out, [InputEvent::PointerAxis { time: 5, horizontal: false, value: Fixed::from_int(-15) }]);
        // A dropped frame is forgotten.
        let mut out = Vec::new();
        t.feed(EV_REL, REL_X, 9, 5, SIZE, &mut out);
        t.feed(EV_SYN, SYN_DROPPED, 0, 5, SIZE, &mut out);
        t.feed(EV_SYN, SYN_REPORT, 0, 5, SIZE, &mut out);
        assert!(out.is_empty());
        // An empty frame says nothing.
        assert!(frame(&mut t, &[]).is_empty());
    }

    #[test]
    fn tablet_positions_are_scaled_to_the_output() {
        let mut t = Translator::new(Kind::Tablet { x: (0, 0x7fff), y: (0, 0x7fff) });
        let out = frame(&mut t, &[(EV_ABS, ABS_X, 0x7fff), (EV_ABS, ABS_Y, 0)]);
        assert_eq!(out, [InputEvent::PointerMotionAbsolute { time: 5, x: Fixed::from_int(1279), y: Fixed(0) }]);
        // One axis moves: the other keeps its value.
        let out = frame(&mut t, &[(EV_ABS, ABS_Y, 0x7fff)]);
        assert_eq!(
            out,
            [InputEvent::PointerMotionAbsolute { time: 5, x: Fixed::from_int(1279), y: Fixed::from_int(799) }]
        );
        let out = frame(&mut t, &[(EV_ABS, ABS_X, 0x4000)]);
        let InputEvent::PointerMotionAbsolute { x, .. } = out[0] else { panic!() };
        assert_eq!(x.floor(), 639);
    }

    #[test]
    fn keys_without_the_devices_repeat() {
        let mut t = Translator::new(Kind::Keyboard);
        let out = frame(&mut t, &[(EV_KEY, 30, 1), (EV_KEY, 30, 2), (EV_KEY, 30, 0)]);
        assert_eq!(
            out,
            [InputEvent::Key { time: 5, key: 30, pressed: true }, InputEvent::Key { time: 5, key: 30, pressed: false }]
        );
    }

    #[test]
    fn classification() {
        let set = |bits: &[u16]| {
            let mut b = [0u8; 96];
            for &n in bits {
                b[usize::from(n / 8)] |= 1 << (n % 8);
            }
            b
        };
        let none = [0u8; 96];
        assert_eq!(classify(&set(&[EV_KEY]), &set(&[KEY_A]), &none, &none), Some(false));
        assert_eq!(classify(&set(&[EV_KEY, EV_REL]), &set(&[0x110]), &set(&[REL_X, REL_Y]), &none), Some(true));
        assert_eq!(classify(&set(&[EV_KEY, EV_ABS]), &set(&[0x110]), &none, &set(&[ABS_X, ABS_Y])), Some(true));
        assert_eq!(classify(&set(&[EV_KEY]), &set(&[0x110]), &none, &none), None, "buttons alone");
    }
}
