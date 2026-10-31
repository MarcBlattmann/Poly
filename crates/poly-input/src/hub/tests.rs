use super::*;
use crate::codes::{bus, rel};
use std::vec::Vec;

const MS: u64 = 1_000_000;

fn keyboard(hub: &mut Hub, soft: bool) -> usize {
    let info = DeviceInfo::keyboard("kbd", InputId { bustype: bus::I8042, ..InputId::default() })
        .with_repeat(DEFAULT_REPEAT, soft);
    hub.add(info)
}

fn mouse(hub: &mut Hub) -> usize {
    hub.add(DeviceInfo::mouse("mouse", InputId::default(), &[key::BTN_LEFT, key::BTN_RIGHT], true, false))
}

fn open(hub: &mut Hub, dev: usize) -> ClientId {
    hub.open(dev, Queue::new(QUEUE_EVENTS)).unwrap()
}

fn drain(hub: &mut Hub, dev: usize, c: ClientId) -> Vec<(u16, u16, i32)> {
    let mut out = [Event::default(); 512];
    match hub.read(dev, c, &mut out) {
        Ok(n) => out[..n].iter().map(|e| (e.kind, e.code, e.value)).collect(),
        Err(Error::Again) => Vec::new(),
        Err(e) => panic!("read: {e:?}"),
    }
}

fn console(hub: &mut Hub) -> Vec<(Key, i32)> {
    core::iter::from_fn(|| hub.console_pop()).map(|k| (k.key, k.value)).collect()
}

const SYN: (u16, u16, i32) = (ev::SYN, syn::REPORT, 0);

fn press(hub: &mut Hub, dev: usize, t: u64, code: u16, value: i32) -> Delivered {
    hub.report_all(dev, t, &[(ev::KEY, code, value), SYN])
}

#[test]
fn frames_reach_every_client_at_syn_report() {
    let mut hub = Hub::new();
    let m = mouse(&mut hub);
    let (a, b) = (open(&mut hub, m), open(&mut hub, m));
    assert_eq!(hub.report(m, 10, ev::REL, rel::X, 5), Delivered::default(), "nothing until the frame ends");
    hub.report(m, 10, ev::REL, rel::Y, -3);
    assert_eq!(drain(&mut hub, m, a), []);
    let d = hub.report(m, 10, ev::SYN, syn::REPORT, 0);
    assert_eq!(d, Delivered { devices: 1 << m, console: false });
    let want = [(ev::REL, rel::X, 5), (ev::REL, rel::Y, -3), SYN];
    assert_eq!(drain(&mut hub, m, a), want);
    assert_eq!(drain(&mut hub, m, b), want);
    assert_eq!(hub.read(m, a, &mut [Event::default()]), Err(Error::Again));
    // Times are kept.
    hub.report_all(m, 99, &[(ev::REL, rel::WHEEL, 1), SYN]);
    let mut e = [Event::default(); 2];
    assert_eq!(hub.read(m, a, &mut e), Ok(2));
    assert_eq!(e[0], Event::new(99, ev::REL, rel::WHEEL, 1));
}

#[test]
fn undeclared_and_unchanged_events_are_dropped() {
    let mut hub = Hub::new();
    let m = mouse(&mut hub);
    let c = open(&mut hub, m);
    // Not declared: horizontal wheel, a key, an absolute axis.
    hub.report_all(m, 1, &[(ev::REL, rel::HWHEEL, 1), (ev::KEY, key::A, 1), (ev::ABS, abs::X, 4), SYN]);
    // No motion, a release of a button that is up.
    hub.report_all(m, 2, &[(ev::REL, rel::X, 0), (ev::KEY, key::BTN_LEFT, 0), SYN]);
    assert_eq!(drain(&mut hub, m, c), [], "empty frames are not delivered");
    hub.report_all(m, 3, &[(ev::KEY, key::BTN_LEFT, 1), SYN]);
    hub.report_all(m, 4, &[(ev::KEY, key::BTN_LEFT, 1), SYN]);
    hub.report_all(m, 5, &[(ev::KEY, key::BTN_LEFT, 0), SYN]);
    // A mouse has no repeat: the second press of a held button is a
    // repeat from the device, which buttons get as value 2.
    assert_eq!(
        drain(&mut hub, m, c),
        [(ev::KEY, key::BTN_LEFT, 1), SYN, (ev::KEY, key::BTN_LEFT, 2), SYN, (ev::KEY, key::BTN_LEFT, 0), SYN]
    );
    // Absolute axes report changes only.
    let t = hub.add(
        DeviceInfo::new("tablet", InputId::default())
            .with_abs(abs::X, AbsInfo { minimum: 0, maximum: 32767, ..AbsInfo::default() }),
    );
    let tc = open(&mut hub, t);
    hub.report_all(t, 1, &[(ev::ABS, abs::X, 100), SYN, (ev::ABS, abs::X, 100), SYN, (ev::ABS, abs::X, 7), SYN]);
    assert_eq!(drain(&mut hub, t, tc), [(ev::ABS, abs::X, 100), SYN, (ev::ABS, abs::X, 7), SYN]);
    assert_eq!(hub.info(t).unwrap().abs[0].value, 7);
}

#[test]
fn keyboards_feed_the_console_and_track_keys() {
    let mut hub = Hub::new();
    let k = keyboard(&mut hub, false);
    let c = open(&mut hub, k);
    assert_eq!(press(&mut hub, k, 1, key::A, 1), Delivered { devices: 1 << k, console: true });
    assert!(hub.is_key_down(k, key::A));
    // The keyboard's own repeat (a press of a held key) is value 2.
    press(&mut hub, k, 2, key::A, 1);
    press(&mut hub, k, 3, key::A, 0);
    assert!(!hub.is_key_down(k, key::A));
    assert_eq!(console(&mut hub), [(Key::A, 1), (Key::A, 2), (Key::A, 0)]);
    assert_eq!(
        drain(&mut hub, k, c),
        [(ev::KEY, key::A, 1), SYN, (ev::KEY, key::A, 2), SYN, (ev::KEY, key::A, 0), SYN]
    );
    // Without readers, keys still reach the console.
    hub.close(k, c);
    assert_eq!(press(&mut hub, k, 4, key::ENTER, 1), Delivered { devices: 0, console: true });
    assert_eq!(console(&mut hub), [(Key::Enter, 1)]);
    // Mouse buttons never do.
    let m = mouse(&mut hub);
    assert_eq!(press(&mut hub, m, 5, key::BTN_LEFT, 1), Delivered::default());
    // Keys injected without a device.
    assert!(hub.console_push(Key::B, 1, 6));
    assert_eq!(console(&mut hub), [(Key::B, 1)]);
}

#[test]
fn a_grab_takes_the_events_from_everyone_else() {
    let mut hub = Hub::new();
    let k = keyboard(&mut hub, false);
    let (a, b) = (open(&mut hub, k), open(&mut hub, k));
    assert_eq!(hub.grab(k, a, true), Ok(()));
    assert_eq!(hub.grab(k, b, true), Err(Error::Busy));
    assert_eq!(hub.grab(k, a, true), Err(Error::Busy), "a second grab");
    assert_eq!(hub.grab(k, b, false), Err(Error::Invalid), "b holds no grab");
    press(&mut hub, k, 1, key::Q, 1);
    assert_eq!(drain(&mut hub, k, a), [(ev::KEY, key::Q, 1), SYN]);
    assert_eq!(drain(&mut hub, k, b), []);
    assert_eq!(console(&mut hub), [], "nor the console");
    assert_eq!(hub.grab(k, a, false), Ok(()));
    press(&mut hub, k, 2, key::Q, 0);
    assert_eq!(drain(&mut hub, k, b), [(ev::KEY, key::Q, 0), SYN]);
    assert_eq!(console(&mut hub), [(Key::Q, 0)]);
    // Closing the grabbing client lets go.
    hub.grab(k, b, true).unwrap();
    hub.close(k, b);
    assert_eq!(hub.grabbed_by(k), None);
}

#[test]
fn a_full_queue_drops_and_says_so() {
    let mut hub = Hub::new();
    let m = mouse(&mut hub);
    let c = hub.open(m, Queue::new(8)).unwrap();
    for i in 1..=3 {
        hub.report_all(m, i, &[(ev::REL, rel::X, i as i32), (ev::REL, rel::Y, 1), SYN]);
    }
    // 9 events for 8 places: the queue was emptied and SYN_DROPPED put
    // before the frame that did not fit.
    assert_eq!(drain(&mut hub, m, c), [(ev::SYN, syn::DROPPED, 0), (ev::REL, rel::X, 3), (ev::REL, rel::Y, 1), SYN]);
    assert_eq!(hub.overruns(m, c), 1);
    assert_eq!(hub.stats(m).unwrap().overruns, 1);
}

#[test]
fn reads_take_whole_events_up_to_the_buffer() {
    let mut hub = Hub::new();
    let m = mouse(&mut hub);
    let c = open(&mut hub, m);
    hub.report_all(m, 1, &[(ev::REL, rel::X, 1), (ev::REL, rel::Y, 2), SYN]);
    let mut one = [Event::default(); 1];
    assert_eq!(hub.read(m, c, &mut one), Ok(1));
    assert_eq!(one[0].code, rel::X);
    assert_eq!(hub.pending(m, c), Ok(2));
    assert_eq!(drain(&mut hub, m, c), [(ev::REL, rel::Y, 2), SYN]);
}

#[test]
fn soft_repeat_repeats_the_last_key_held() {
    let mut hub = Hub::new();
    let k = keyboard(&mut hub, true);
    let c = open(&mut hub, k);
    assert_eq!(hub.next_deadline(), None);
    press(&mut hub, k, 0, key::A, 1);
    assert_eq!(hub.next_deadline(), Some(250 * MS));
    assert_eq!(hub.tick(249 * MS), Delivered::default());
    assert_eq!(hub.tick(250 * MS), Delivered { devices: 1 << k, console: true });
    assert_eq!(hub.next_deadline(), Some(283 * MS));
    hub.tick(283 * MS);
    // The keyboard's own repeats are dropped: the hub repeats.
    press(&mut hub, k, 290 * MS, key::A, 1);
    // A second key takes over the repeat.
    press(&mut hub, k, 300 * MS, key::B, 1);
    assert_eq!(hub.next_deadline(), Some(550 * MS));
    press(&mut hub, k, 310 * MS, key::B, 0);
    assert_eq!(hub.next_deadline(), None, "releasing the repeating key stops it");
    press(&mut hub, k, 320 * MS, key::A, 0);
    assert_eq!(
        drain(&mut hub, k, c),
        [
            (ev::KEY, key::A, 1),
            SYN,
            (ev::KEY, key::A, 2),
            SYN,
            (ev::KEY, key::A, 2),
            SYN,
            (ev::KEY, key::B, 1),
            SYN,
            (ev::KEY, key::B, 0),
            SYN,
            (ev::KEY, key::A, 0),
            SYN
        ]
    );
    assert_eq!(console(&mut hub), [(Key::A, 1), (Key::A, 2), (Key::A, 2), (Key::B, 1), (Key::B, 0), (Key::A, 0)]);
    assert_eq!(hub.stats(k).unwrap().repeats, 2);
    // A late tick makes one repeat and schedules the next a period later.
    press(&mut hub, k, 1000 * MS, key::C, 1);
    hub.tick(5000 * MS);
    assert_eq!(hub.next_deadline(), Some(5033 * MS));
    // A period of 0 turns repeating off.
    hub.set_repeat(k, 250, 0).unwrap();
    assert_eq!(hub.next_deadline(), None);
    assert_eq!(hub.set_repeat(k, -1, 10), Err(Error::Invalid));
    let m = mouse(&mut hub);
    assert_eq!(hub.set_repeat(m, 250, 33), Err(Error::Invalid), "a mouse has no EV_REP");
    assert_eq!(hub.set_soft_repeat(m, true), Err(Error::Invalid));
    // Switching to the device's own repeats stops the hub's.
    hub.set_repeat(k, 250, 33).unwrap();
    press(&mut hub, k, 6000 * MS, key::D, 1);
    assert!(hub.next_deadline().is_some());
    hub.set_soft_repeat(k, false).unwrap();
    assert_eq!(hub.next_deadline(), None);
    let c2 = open(&mut hub, k);
    press(&mut hub, k, 6001 * MS, key::D, 1);
    assert_eq!(drain(&mut hub, k, c2), [(ev::KEY, key::D, 2), SYN]);
}

#[test]
fn leds_are_state_and_events() {
    let mut hub = Hub::new();
    let k = keyboard(&mut hub, false);
    let c = open(&mut hub, k);
    let num = 1 << led::NUML;
    let caps = 1 << led::CAPSL;
    assert_eq!(hub.set_leds(k, num | caps, 5), (true, Delivered { devices: 1 << k, console: false }));
    assert_eq!(hub.leds(k), num | caps);
    assert!(!hub.set_leds(k, num | caps, 6).0, "no change");
    // LEDs the device does not have are ignored.
    assert!(hub.set_leds(k, num | (1 << led::KANA), 7).0);
    assert_eq!(hub.leds(k), num);
    assert_eq!(
        drain(&mut hub, k, c),
        [(ev::LED, led::NUML, 1), (ev::LED, led::CAPSL, 1), SYN, (ev::LED, led::CAPSL, 0), SYN]
    );
    // A client's write goes through the device.
    let d = hub.inject(k, c, 8, ev::LED, led::SCROLLL, 1).unwrap();
    assert_eq!(d, Delivered::default());
    assert_eq!(hub.inject(k, c, 8, ev::SYN, syn::REPORT, 0), Ok(Delivered { devices: 1 << k, console: false }));
    assert_eq!(hub.leds(k), num | (1 << led::SCROLLL));
    assert_eq!(hub.console_keyboards().collect::<Vec<_>>(), [k]);
}

#[test]
fn revoked_clients_get_nothing() {
    let mut hub = Hub::new();
    let k = keyboard(&mut hub, false);
    let c = open(&mut hub, k);
    hub.grab(k, c, true).unwrap();
    press(&mut hub, k, 1, key::A, 1);
    assert_eq!(hub.revoke(k, c), Ok(()));
    assert_eq!(hub.grabbed_by(k), None);
    assert_eq!(hub.read(k, c, &mut [Event::default()]), Err(Error::NoDevice));
    assert_eq!(hub.pending(k, c), Err(Error::NoDevice));
    assert_eq!(hub.revoke(k, c), Err(Error::NoDevice));
    assert_eq!(press(&mut hub, k, 2, key::A, 0), Delivered { devices: 0, console: true });
    assert!(hub.close(k, c).is_some());
    assert!(hub.close(k, c).is_none());
    assert!(hub.open(7, Queue::new(8)).is_none());
}

#[test]
fn long_frames_are_cut() {
    let mut hub = Hub::new();
    let k = keyboard(&mut hub, false);
    let c = hub.open(k, Queue::new(1024)).unwrap();
    // Every key down in one frame: more than a frame holds.
    let codes: Vec<u16> = Key::ALL.iter().map(|&k| keycode::code(k)).collect();
    for &code in &codes {
        hub.report(k, 1, ev::KEY, code, 1);
    }
    hub.report(k, 1, ev::SYN, syn::REPORT, 0);
    let got = drain(&mut hub, k, c);
    assert_eq!(got.iter().filter(|e| e.0 == ev::KEY).count(), codes.len());
    assert_eq!(got.iter().filter(|e| **e == SYN).count(), codes.len().div_ceil(FRAME_EVENTS - 1));
    assert_eq!(console(&mut hub).len(), codes.len());
}

#[test]
fn capabilities_and_bitmaps() {
    let info = DeviceInfo::keyboard("k", InputId::default());
    assert!(info.has_letter_keys());
    assert!(info.caps.has(ev::KEY, key::A));
    assert!(!info.caps.has(ev::KEY, key::BTN_LEFT));
    assert!(info.caps.has(ev::LED, led::CAPSL));
    assert!(info.caps.has(ev::SYN, syn::REPORT));
    let types = info.caps.bitmap(0).unwrap();
    assert_eq!(types.len(), 8);
    assert_eq!(types[0], 1 << ev::SYN | 1 << ev::KEY);
    assert_eq!(types[2], 1 << (ev::LED - 16) | 1 << (ev::REP - 16));
    let keys = info.caps.bitmap(ev::KEY).unwrap();
    assert_eq!(keys.len(), 96);
    assert_eq!(keys[usize::from(key::A / 8)] & (1 << (key::A % 8)), 1 << (key::A % 8));
    assert_eq!(info.caps.bitmap(ev::REP).unwrap(), [0; 8]);
    assert_eq!(info.caps.bitmap(0x1e), None);
    let m = DeviceInfo::mouse("m", InputId::default(), &[key::BTN_LEFT], true, true);
    assert!(!m.has_letter_keys());
    assert_eq!(m.caps.rel.0[0], 1 << rel::X | 1 << rel::Y | 1 << rel::WHEEL | 1 << rel::HWHEEL);
    assert!(m.caps.prop.get(codes::prop::POINTER));
    let mut b = Bits::<2>::new();
    b.set(70, true);
    b.set(500, true);
    assert_eq!(b.iter().collect::<Vec<_>>(), [70]);
}
