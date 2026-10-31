use super::*;
use crate::codes::{abs, bus, ev, key, led, rel, syn};
use crate::hub::{DeviceInfo, InputId, QUEUE_EVENTS, Queue};

fn setup() -> (Hub, usize, ClientId) {
    let mut hub = Hub::new();
    let id = InputId { bustype: bus::I8042, vendor: 1, product: 2, version: 0xab41 };
    let dev = hub.add(DeviceInfo::keyboard("AT keyboard", id));
    let c = hub.open(dev, Queue::new(QUEUE_EVENTS)).unwrap();
    (hub, dev, c)
}

#[test]
fn request_numbers_are_the_uapi_ones() {
    // As C computes them from linux/input.h on x86-64.
    assert_eq!(EVIOCGVERSION, 0x8004_4501);
    assert_eq!(EVIOCGID, 0x8008_4502);
    assert_eq!(EVIOCGREP, 0x8008_4503);
    assert_eq!(EVIOCSREP, 0x4008_4503);
    assert_eq!(eviocgname(256), 0x8100_4506);
    assert_eq!(eviocgbit(0, 4), 0x8004_4520);
    assert_eq!(eviocgbit(ev::KEY as u8, 96), 0x8060_4521);
    assert_eq!(eviocgabs(abs::X as u8), 0x8018_4540);
    assert_eq!(eviocsabs(abs::Y as u8), 0x4018_45c1);
    assert_eq!(EVIOCGRAB, 0x4004_4590);
    assert_eq!(EVIOCREVOKE, 0x4004_4591);
    assert_eq!(EVIOCSCLOCKID, 0x4004_45a0);
    assert_eq!(Request::decode(eviocgkey(96)), Some(Request::Keys(96)));
    assert_eq!(Request::decode(eviocsabs(3)), Some(Request::SetAbs(3)));
    assert_eq!(Request::decode(EVIOCSKEYCODE), Some(Request::Unsupported));
    // A terminal request is not ours.
    assert_eq!(Request::decode(0x5401), None);
    assert_eq!(Request::decode(ioc(0, b'E', 0x01, 0)), None);
}

#[test]
fn events_encode_as_struct_input_event() {
    let e = Event::new(3_000_456_789, ev::KEY, key::A, 1);
    let b = encode(&e, Clock::Monotonic, 0);
    assert_eq!(&b[0..8], &3i64.to_le_bytes());
    assert_eq!(&b[8..16], &456i64.to_le_bytes());
    assert_eq!(&b[16..], &[1, 0, 30, 0, 1, 0, 0, 0]);
    assert_eq!(decode(&b), (ev::KEY, key::A, 1));
    // The realtime clock is the monotonic one plus the offset.
    let r = encode(&e, Clock::Realtime, 10_000_000_000);
    assert_eq!(&r[0..8], &13i64.to_le_bytes());
    let negative = encode(&Event::new(0, ev::REL, rel::X, -2), Clock::Boottime, 0);
    assert_eq!(decode(&negative), (ev::REL, rel::X, -2));
}

#[test]
fn reads_return_whole_events() {
    let (mut hub, dev, c) = setup();
    let mut buf = [0u8; 100];
    assert_eq!(read(&mut hub, dev, c, &mut buf, 0), Err(Error::Again));
    assert_eq!(read(&mut hub, dev, c, &mut buf[..23], 0), Err(Error::Invalid));
    hub.report_all(dev, 7, &[(ev::KEY, key::B, 1), (ev::SYN, syn::REPORT, 0), (ev::KEY, key::B, 0), (ev::SYN, 0, 0)]);
    // 100 bytes hold four events.
    assert_eq!(read(&mut hub, dev, c, &mut buf, 0), Ok(96));
    assert_eq!(decode(buf[48..72].try_into().unwrap()), (ev::KEY, key::B, 0));
    assert_eq!(read(&mut hub, dev, c, &mut buf, 0), Err(Error::Again));
    // More than the batch read internally.
    for i in 0..40 {
        hub.report_all(dev, i, &[(ev::KEY, key::C, (i % 2 == 0) as i32), (ev::SYN, 0, 0)]);
    }
    let mut big = std::vec![0u8; 200 * EVENT_SIZE];
    assert_eq!(read(&mut hub, dev, c, &mut big, 0), Ok(80 * EVENT_SIZE));
}

#[test]
fn writes_inject_events() {
    let (mut hub, dev, c) = setup();
    let other = hub.open(dev, Queue::new(QUEUE_EVENTS)).unwrap();
    let mut buf = std::vec::Vec::new();
    buf.extend_from_slice(&encode(&Event::new(0, ev::LED, led::CAPSL, 1), Clock::Monotonic, 0));
    buf.extend_from_slice(&encode(&Event::syn(0), Clock::Monotonic, 0));
    buf.extend_from_slice(&[0; 5]);
    let (n, d) = write(&mut hub, dev, c, &buf, 50).unwrap();
    assert_eq!(n, 48, "the partial event is left");
    assert_eq!(d.devices, 1 << dev);
    assert_eq!(hub.leds(dev), 1 << led::CAPSL);
    let mut got = [0u8; 48];
    assert_eq!(read(&mut hub, dev, other, &mut got, 0), Ok(48));
    assert_eq!(decode(got[..24].try_into().unwrap()), (ev::LED, led::CAPSL, 1));
    assert_eq!(write(&mut hub, dev, c, &[0; 10], 0), Err(Error::Invalid));
}

#[test]
fn identity_and_capability_requests() {
    let (mut hub, dev, c) = setup();
    let r = ioctl_raw(&mut hub, dev, c, EVIOCGVERSION, 0, &[]).unwrap();
    assert_eq!(r, Reply { value: 0, out: 0x01_0001u32.to_le_bytes().to_vec() });
    let r = ioctl_raw(&mut hub, dev, c, EVIOCGID, 0, &[]).unwrap();
    assert_eq!(r.out, [0x11, 0, 1, 0, 2, 0, 0x41, 0xab]);
    // Names come with their NUL, cut to the length asked for.
    let r = ioctl_raw(&mut hub, dev, c, eviocgname(256), 0, &[]).unwrap();
    assert_eq!((r.value, r.out.as_slice()), (12, &b"AT keyboard\0"[..]));
    let r = ioctl_raw(&mut hub, dev, c, eviocgname(4), 0, &[]).unwrap();
    assert_eq!((r.value, r.out.as_slice()), (4, &b"AT k"[..]));
    assert_eq!(ioctl_raw(&mut hub, dev, c, eviocgphys(64), 0, &[]), Err(Error::NoEntry));
    assert_eq!(ioctl_raw(&mut hub, dev, c, eviocguniq(64), 0, &[]), Err(Error::NoEntry));
    // Bitmaps: the event types, then the keys.
    let r = ioctl_raw(&mut hub, dev, c, eviocgbit(0, 32), 0, &[]).unwrap();
    assert_eq!(r.value, 8);
    assert_eq!(r.out[0], 0b11, "EV_SYN and EV_KEY");
    let r = ioctl_raw(&mut hub, dev, c, eviocgbit(ev::KEY as u8, 256), 0, &[]).unwrap();
    assert_eq!(r.value, 96);
    assert_ne!(r.out[3] & 1 << 6, 0, "KEY_A is bit 30");
    let r = ioctl_raw(&mut hub, dev, c, eviocgbit(ev::REL as u8, 8), 0, &[]).unwrap();
    assert_eq!(r.out, [0; 8], "a keyboard has no axes");
    assert_eq!(ioctl_raw(&mut hub, dev, c, eviocgbit(0x1d, 8), 0, &[]), Err(Error::Invalid));
    let r = ioctl_raw(&mut hub, dev, c, eviocgprop(4), 0, &[]).unwrap();
    assert_eq!(r.out, [0; 4]);
    assert_eq!(ioctl_raw(&mut hub, dev, c, 0x5401, 0, &[]), Err(Error::NotTty));
    assert_eq!(ioctl_raw(&mut hub, dev, c, EVIOCGKEYCODE, 0, &[]), Err(Error::Invalid));
    let r = ioctl_raw(&mut hub, dev, c, EVIOCGEFFECTS, 0, &[]).unwrap();
    assert_eq!(r.out, [0; 4]);
}

#[test]
fn state_requests() {
    let (mut hub, dev, c) = setup();
    hub.report_all(dev, 1, &[(ev::KEY, key::LEFTSHIFT, 1), (ev::SYN, 0, 0)]);
    let _ = hub.set_leds(dev, 1 << led::NUML, 2);
    let r = ioctl_raw(&mut hub, dev, c, eviocgkey(96), 0, &[]).unwrap();
    assert_eq!(r.out[5], 1 << 2, "KEY_LEFTSHIFT (42) is down");
    let r = ioctl_raw(&mut hub, dev, c, eviocgled(8), 0, &[]).unwrap();
    assert_eq!(r.out[0], 1);
    // Repeat parameters.
    let r = ioctl_raw(&mut hub, dev, c, EVIOCGREP, 0, &[]).unwrap();
    assert_eq!(r.out, [250, 0, 0, 0, 33, 0, 0, 0]);
    let mut set = [0u8; 8];
    set[..4].copy_from_slice(&500i32.to_le_bytes());
    set[4..].copy_from_slice(&100i32.to_le_bytes());
    assert_eq!(ioctl_raw(&mut hub, dev, c, EVIOCSREP, 0, &set).map(|r| r.value), Ok(0));
    assert_eq!(hub.info(dev).unwrap().rep, [500, 100]);
    assert_eq!(ioctl_raw(&mut hub, dev, c, EVIOCSREP, 0, &set[..4]), Err(Error::Invalid));
}

#[test]
fn absolute_axes() {
    let mut hub = Hub::new();
    let range = AbsInfo { minimum: 0, maximum: 32767, ..AbsInfo::default() };
    let dev = hub.add(DeviceInfo::new("tablet", InputId::default()).with_abs(abs::X, range).with_abs(abs::Y, range));
    let c = hub.open(dev, Queue::new(QUEUE_EVENTS)).unwrap();
    hub.report_all(dev, 1, &[(ev::ABS, abs::X, 1234), (ev::SYN, 0, 0)]);
    let r = ioctl_raw(&mut hub, dev, c, eviocgabs(abs::X as u8), 0, &[]).unwrap();
    let words: std::vec::Vec<i32> = r.out.chunks(4).map(|w| i32::from_le_bytes(w.try_into().unwrap())).collect();
    assert_eq!(words, [1234, 0, 32767, 0, 0, 0]);
    assert_eq!(ioctl_raw(&mut hub, dev, c, eviocgabs(abs::Z as u8), 0, &[]), Err(Error::Invalid));
    let new: std::vec::Vec<u8> = [0i32, -100, 100, 2, 3, 4].iter().flat_map(|v| v.to_le_bytes()).collect();
    assert_eq!(ioctl_raw(&mut hub, dev, c, eviocsabs(abs::Y as u8), 0, &new).map(|r| r.value), Ok(0));
    assert_eq!(hub.info(dev).unwrap().abs[usize::from(abs::Y)].minimum, -100);
    assert_eq!(ioctl_raw(&mut hub, dev, c, eviocsabs(abs::Z as u8), 0, &new), Err(Error::Invalid));
    // No repeat on a tablet.
    assert_eq!(ioctl_raw(&mut hub, dev, c, EVIOCGREP, 0, &[]), Err(Error::Invalid));
}

#[test]
fn grab_revoke_and_clock() {
    let (mut hub, dev, c) = setup();
    let other = hub.open(dev, Queue::new(QUEUE_EVENTS)).unwrap();
    assert_eq!(ioctl_raw(&mut hub, dev, c, EVIOCGRAB, 1, &[]).map(|r| r.value), Ok(0));
    assert_eq!(ioctl_raw(&mut hub, dev, other, EVIOCGRAB, 1, &[]), Err(Error::Busy));
    assert_eq!(ioctl_raw(&mut hub, dev, other, EVIOCGRAB, 0, &[]), Err(Error::Invalid));
    assert_eq!(ioctl_raw(&mut hub, dev, c, EVIOCGRAB, 0, &[]).map(|r| r.value), Ok(0));
    // The clock.
    assert_eq!(hub.clock(dev, c), Clock::Realtime);
    let r = ioctl_raw(&mut hub, dev, c, EVIOCSCLOCKID, 0, &CLOCK_MONOTONIC.to_le_bytes());
    assert_eq!(r.map(|r| r.value), Ok(0));
    assert_eq!(hub.clock(dev, c), Clock::Monotonic);
    assert_eq!(ioctl_raw(&mut hub, dev, c, EVIOCSCLOCKID, 0, &5i32.to_le_bytes()), Err(Error::Invalid));
    hub.report_all(dev, 2_000_000_000, &[(ev::KEY, key::A, 1), (ev::SYN, 0, 0)]);
    let mut buf = [0u8; 24];
    read(&mut hub, dev, c, &mut buf, 1_000_000_000_000).unwrap();
    assert_eq!(&buf[0..8], &2i64.to_le_bytes(), "monotonic, not realtime");
    read(&mut hub, dev, other, &mut buf, 1_000_000_000_000).unwrap();
    assert_eq!(&buf[0..8], &1002i64.to_le_bytes(), "realtime by default");
    // Revoking.
    assert_eq!(ioctl_raw(&mut hub, dev, c, EVIOCREVOKE, 1, &[]), Err(Error::Invalid));
    assert_eq!(ioctl_raw(&mut hub, dev, c, EVIOCREVOKE, 0, &[]).map(|r| r.value), Ok(0));
    assert_eq!(ioctl_raw(&mut hub, dev, c, EVIOCGVERSION, 0, &[]), Err(Error::NoDevice));
    assert_eq!(read(&mut hub, dev, c, &mut buf, 0), Err(Error::NoDevice));
    assert_eq!(write(&mut hub, dev, c, &buf, 0), Err(Error::NoDevice));
}
