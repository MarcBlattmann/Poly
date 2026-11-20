//! Host tests of `\Device\PolyAudio` (ADR 0244) through the services,
//! against the fake kernel of `crate::tests` with streams of its own: the
//! device opened by name, each request of [`poly_audio::nt`], and waits
//! that finish at once, later (the test plays as the mixer would), or end
//! with a reset or the handle.

extern crate std;

use super::*;
use crate::io::consts::*;
use crate::io::default_volumes;
use crate::services::Service;
use crate::tests::Fake;
use alloc::boxed::Box;
use poly_audio::nt::{Ring, SAMPLES};
use poly_audio::{Format, Sample, UNITY};
use poly_ntobj::OBJ_CASE_INSENSITIVE;
use poly_vfs::{Clock, Timespec, Tmpfs, Vfs};
use std::sync::Mutex;

#[derive(Debug)]
struct FixedClock;

impl Clock for FixedClock {
    fn now(&self) -> Timespec {
        Timespec::new(1_000_000_000, 0)
    }
}

/// A stream as the test's mixer: its ring, its bytes, its gains.
#[derive(Debug)]
struct TestStream {
    format: Format,
    state: Mutex<(Ring, Vec<u8>, [u32; MAX_CHANNELS])>,
}

impl TestStream {
    /// The mixer takes `frames` (as many as are queued): their bytes.
    fn play(&self, frames: u32) -> Vec<u8> {
        let mut g = self.state.lock().unwrap();
        let (ring, bytes, _) = &mut *g;
        let fb = self.format.frame_bytes();
        let s = ring.peek(frames);
        let mut out = Vec::new();
        for (at, n) in [s.first, s.second] {
            out.extend_from_slice(&bytes[at as usize * fb..(at + n) as usize * fb]);
        }
        // The test's device plays at once what the mixer took.
        ring.consume(s.frames());
        ring.heard(u64::from(s.frames()));
        out
    }
}

impl NtAudioStream for TestStream {
    fn write(&self, data: &[u8]) -> u32 {
        let mut g = self.state.lock().unwrap();
        let (ring, bytes, _) = &mut *g;
        let fb = self.format.frame_bytes();
        let s = ring.write((data.len() / fb) as u32);
        let mut from = 0;
        for (at, n) in [s.first, s.second] {
            let len = n as usize * fb;
            bytes[at as usize * fb..at as usize * fb + len].copy_from_slice(&data[from..from + len]);
            from += len;
        }
        s.frames()
    }
    fn start(&self) {
        self.state.lock().unwrap().0.start();
    }
    fn stop(&self) {
        self.state.lock().unwrap().0.stop();
    }
    fn reset(&self) {
        self.state.lock().unwrap().0.reset();
    }
    fn set_gains(&self, gains: [u32; MAX_CHANNELS]) {
        self.state.lock().unwrap().2 = gains;
    }
    fn info(&self) -> StreamInfo {
        self.state.lock().unwrap().0.info()
    }
    fn format(&self) -> Format {
        self.format
    }
}

const DEVICE_48K: DeviceInfo =
    DeviceInfo { format: Format::new(Sample::S16, 2, 48_000), period_frames: 480, periods: 4 };

struct T {
    f: Fake,
    streams: Arc<Mutex<Vec<Arc<TestStream>>>>,
}

fn setup(card: bool) -> T {
    let vfs: &'static Vfs = Box::leak(Box::new(Vfs::new(Arc::new(Tmpfs::new(Arc::new(FixedClock))))));
    let mut f = Fake::new();
    let ctx = vfs.root_context().unwrap();
    let vols = default_volumes(vfs, &ctx);
    f.sys.io.attach(&mut f.sys.objects, vfs, &vols).unwrap();
    let streams: Arc<Mutex<Vec<Arc<TestStream>>>> = Arc::default();
    let made = Arc::clone(&streams);
    if card {
        f.audio = Some((
            DEVICE_48K,
            Box::new(move |format: Format, frames: u32| {
                let bytes = alloc::vec![0u8; frames as usize * format.frame_bytes()];
                let s = Arc::new(TestStream {
                    format,
                    state: Mutex::new((Ring::new(frames), bytes, [UNITY; MAX_CHANNELS])),
                });
                made.lock().unwrap().push(Arc::clone(&s));
                Ok(s as Arc<dyn NtAudioStream>)
            }),
        ));
    }
    T { f, streams }
}

const RW: u32 = FILE_GENERIC_READ | FILE_GENERIC_WRITE;

impl T {
    fn open_name(&mut self, name: &str) -> (NtStatus, u64) {
        let out = self.f.alloc(8);
        let iosb = self.f.alloc(16);
        let oa = self.f.attrs(name, OBJ_CASE_INSENSITIVE);
        let s = self.f.call(Service::OpenFile, &[out, u64::from(RW | SYNCHRONIZE), oa, iosb, 3, 0]);
        (s, self.f.u64_at(out))
    }

    fn open(&mut self) -> u64 {
        let (s, h) = self.open_name(wire::DEVICE_NAME);
        assert_eq!(s, STATUS_SUCCESS);
        h
    }

    /// A handle with a stream of `format` and `frames`.
    fn stream(&mut self, format: Format, frames: u32) -> u64 {
        let h = self.open();
        let req = OpenReq { format, buffer_frames: frames }.encode();
        assert_eq!(self.ioctl(h, wire::OPEN, &req, 0).0, STATUS_SUCCESS);
        h
    }

    fn ioctl_ev(&mut self, h: u64, event: u64, code: u32, input: &[u8], outlen: usize) -> (NtStatus, u64, u64, u64) {
        let inp = if input.is_empty() { 0 } else { self.f.put(input) };
        let out = self.f.alloc(outlen.max(1));
        let iosb = self.f.alloc(16);
        let s = self.f.call(
            Service::DeviceIoControlFile,
            &[h, event, 0, 0, iosb, u64::from(code), inp, input.len() as u64, out, outlen as u64],
        );
        self.drain();
        (s, self.f.u64_at(iosb + 8), out, iosb)
    }

    fn ioctl(&mut self, h: u64, code: u32, input: &[u8], outlen: usize) -> (NtStatus, u64, Vec<u8>) {
        let (s, info, out, _) = self.ioctl_ev(h, 0, code, input, outlen);
        (s, info, self.f.bytes_at(out, outlen))
    }

    fn query(&mut self, h: u64) -> StreamInfo {
        let (s, info, out) = self.ioctl(h, wire::QUERY, &[], StreamInfo::SIZE);
        assert_eq!((s, info), (STATUS_SUCCESS, StreamInfo::SIZE as u64));
        StreamInfo::decode(&out).unwrap()
    }

    fn drain(&mut self) {
        while let Some(c) = self.f.sys.io.pop_completion() {
            if let Some((addr, data)) = &c.data {
                self.f.mem.write_bytes(*addr, data).unwrap();
            }
            if c.iosb != 0 {
                let w = c.iosb_write();
                self.f.mem.write_bytes(w.addr, w.bytes()).unwrap();
            }
            let _ = self.f.sys.io.finish(&mut self.f.sys.objects, &c);
        }
    }

    /// The kernel's worker: finish every wait whose stream played enough.
    fn worker(&mut self) {
        let waits = core::mem::take(&mut self.f.audio_waits);
        for w in waits {
            match w.reached() {
                Some(played) => finish_wait(&mut self.f.sys, &w, played),
                None if self.f.sys.io.irp(w.irp).is_some() => self.f.audio_waits.push(w),
                None => {}
            }
        }
        self.drain();
    }

    fn event(&mut self) -> u64 {
        let out = self.f.alloc(8);
        assert_eq!(self.f.call(Service::CreateEvent, &[out, 0x1F0003, 0, 0, 0]), STATUS_SUCCESS);
        self.f.u64_at(out)
    }

    fn is_set(&mut self, ev: u64) -> bool {
        let q = self.f.alloc(8);
        assert_eq!(self.f.call(Service::QueryEvent, &[ev, 0, q, 8, 0]), STATUS_SUCCESS);
        self.f.u32_at(q + 4) == 1
    }

    fn close(&mut self, h: u64) {
        assert_eq!(self.f.call(Service::Close, &[h]), STATUS_SUCCESS);
    }
}

fn s16_stereo() -> Format {
    Format::new(Sample::S16, 2, 44_100)
}

/// `frames` stereo S16 frames counting up from `from`.
fn frames(from: u16, n: u16) -> Vec<u8> {
    (from..from + n).flat_map(|i| [i.to_le_bytes(), i.to_le_bytes()].concat()).collect()
}

#[test]
fn the_device_opens_by_its_name_only() {
    let mut t = setup(true);
    let h = t.open();
    let (s, _) = t.open_name("\\Device\\PolyAudio\\Other");
    assert_eq!(s, STATUS_OBJECT_NAME_NOT_FOUND);
    t.close(h);
}

#[test]
fn without_a_card_there_is_no_device() {
    let mut t = setup(false);
    let h = t.open();
    assert_eq!(t.ioctl(h, wire::DEVICE, &[], DeviceInfo::SIZE).0, STATUS_NO_SUCH_DEVICE);
    let req = OpenReq { format: s16_stereo(), buffer_frames: 100 }.encode();
    assert_eq!(t.ioctl(h, wire::OPEN, &req, 0).0, STATUS_NO_SUCH_DEVICE);
    // Still no stream.
    assert_eq!(t.ioctl(h, wire::START, &[], 0).0, STATUS_INVALID_DEVICE_STATE);
}

#[test]
fn the_device_reports_the_cards_stream() {
    let mut t = setup(true);
    let h = t.open();
    let (s, info, out) = t.ioctl(h, wire::DEVICE, &[], DeviceInfo::SIZE);
    assert_eq!((s, info), (STATUS_SUCCESS, DeviceInfo::SIZE as u64));
    assert_eq!(DeviceInfo::decode(&out), Some(DEVICE_48K));
    // Too small an output buffer.
    assert_eq!(t.ioctl(h, wire::DEVICE, &[], 4).0, STATUS_BUFFER_TOO_SMALL);
}

#[test]
fn a_stream_needs_its_open_once() {
    let mut t = setup(true);
    let h = t.open();
    for code in [wire::WRITE, wire::START, wire::STOP, wire::RESET, wire::QUERY] {
        assert_eq!(t.ioctl(h, code, &[], 64).0, STATUS_INVALID_DEVICE_STATE, "{code:#x}");
    }
    // A request that is not a stream's.
    let bad = OpenReq { format: Format::new(Sample::S16, 0, 44_100), buffer_frames: 10 }.encode();
    assert_eq!(t.ioctl(h, wire::OPEN, &bad, 0).0, STATUS_INVALID_PARAMETER);
    assert_eq!(t.ioctl(h, wire::OPEN, &[0; 4], 0).0, STATUS_INVALID_PARAMETER);
    let req = OpenReq { format: s16_stereo(), buffer_frames: 100 }.encode();
    assert_eq!(t.ioctl(h, wire::OPEN, &req, 0).0, STATUS_SUCCESS);
    assert_eq!(t.ioctl(h, wire::OPEN, &req, 0).0, STATUS_INVALID_DEVICE_STATE);
    assert_eq!(t.streams.lock().unwrap().len(), 1);
    assert_eq!(t.ioctl(h, 0x001d_3fff, &[], 0).0, STATUS_INVALID_DEVICE_REQUEST);
    // Every sample format opens.
    for s in SAMPLES {
        let h = t.open();
        let req = OpenReq { format: Format::new(s, 1, 8000), buffer_frames: 16 }.encode();
        assert_eq!(t.ioctl(h, wire::OPEN, &req, 0).0, STATUS_SUCCESS);
    }
}

#[test]
fn writes_take_whole_frames_as_far_as_there_is_room() {
    let mut t = setup(true);
    let h = t.stream(s16_stereo(), 8);
    let (s, info, _) = t.ioctl(h, wire::WRITE, &frames(0, 5), 0);
    assert_eq!((s, info), (STATUS_SUCCESS, 5));
    let (s, info, _) = t.ioctl(h, wire::WRITE, &frames(5, 5), 0);
    assert_eq!((s, info), (STATUS_SUCCESS, 3));
    // Full: nothing is taken, and nothing waits.
    assert_eq!(t.ioctl(h, wire::WRITE, &frames(10, 1), 0).1, 0);
    // Not whole frames.
    assert_eq!(t.ioctl(h, wire::WRITE, &[0; 3], 0).0, STATUS_INVALID_PARAMETER);
    let i = t.query(h);
    assert_eq!((i.written, i.played, i.queued, i.buffer_frames, i.running), (8, 0, 8, 8, false));
    // The device's stream comes with it.
    assert_eq!((i.device_rate, i.device_period_frames, i.device_periods), (48_000, 480, 4));
    // The mixer gets the frames in order, round the end of the buffer too.
    let stream = Arc::clone(&t.streams.lock().unwrap()[0]);
    assert_eq!(stream.play(6), frames(0, 6));
    assert_eq!(t.ioctl(h, wire::WRITE, &frames(8, 4), 0).1, 4);
    assert_eq!(stream.play(100), frames(6, 6));
    assert_eq!(t.query(h).played, 12);
}

#[test]
fn start_stop_reset_and_gain_reach_the_stream() {
    let mut t = setup(true);
    let h = t.stream(s16_stereo(), 32);
    let stream = Arc::clone(&t.streams.lock().unwrap()[0]);
    assert_eq!(t.ioctl(h, wire::START, &[], 0).0, STATUS_SUCCESS);
    assert!(t.query(h).running);
    assert_eq!(t.ioctl(h, wire::STOP, &[], 0).0, STATUS_SUCCESS);
    assert!(!t.query(h).running);
    t.ioctl(h, wire::WRITE, &frames(0, 10), 0);
    stream.play(4);
    t.ioctl(h, wire::START, &[], 0);
    assert_eq!(t.ioctl(h, wire::RESET, &[], 0).0, STATUS_SUCCESS);
    let i = t.query(h);
    assert_eq!((i.written, i.played, i.queued, i.running), (0, 0, 0, false));
    // One gain for every channel, or one each.
    assert_eq!(t.ioctl(h, wire::SET_GAIN, &0x8000u32.to_le_bytes(), 0).0, STATUS_SUCCESS);
    assert_eq!(stream.state.lock().unwrap().2, [0x8000; MAX_CHANNELS]);
    let two = [1u32.to_le_bytes(), 2u32.to_le_bytes()].concat();
    assert_eq!(t.ioctl(h, wire::SET_GAIN, &two, 0).0, STATUS_SUCCESS);
    assert_eq!(stream.state.lock().unwrap().2[..3], [1, 2, 2]);
    assert_eq!(t.ioctl(h, wire::SET_GAIN, &[], 0).0, STATUS_INVALID_PARAMETER);
}

#[test]
fn a_wait_already_reached_finishes_at_once() {
    let mut t = setup(true);
    let h = t.stream(s16_stereo(), 32);
    let stream = Arc::clone(&t.streams.lock().unwrap()[0]);
    t.ioctl(h, wire::WRITE, &frames(0, 10), 0);
    stream.play(10);
    let ev = t.event();
    let (s, info, out, _) = t.ioctl_ev(h, ev, wire::WAIT, &7u64.to_le_bytes(), 8);
    assert_eq!((s, info, t.f.u64_at(out)), (STATUS_SUCCESS, 8, 10));
    assert!(t.is_set(ev));
    assert!(t.f.audio_waits.is_empty());
    // Its input and output.
    assert_eq!(t.ioctl(h, wire::WAIT, &[0; 4], 8).0, STATUS_INVALID_PARAMETER);
    assert_eq!(t.ioctl(h, wire::WAIT, &0u64.to_le_bytes(), 4).0, STATUS_BUFFER_TOO_SMALL);
}

#[test]
fn a_wait_finishes_when_the_mixer_took_enough() {
    let mut t = setup(true);
    let h = t.stream(s16_stereo(), 32);
    let stream = Arc::clone(&t.streams.lock().unwrap()[0]);
    t.ioctl(h, wire::WRITE, &frames(0, 20), 0);
    t.ioctl(h, wire::START, &[], 0);
    let ev = t.event();
    let (s, _, out, iosb) = t.ioctl_ev(h, ev, wire::WAIT, &12u64.to_le_bytes(), 8);
    assert_eq!(s, STATUS_PENDING);
    assert!(!t.is_set(ev));
    assert_eq!(t.f.audio_waits.len(), 1);
    // Not enough yet.
    stream.play(8);
    t.worker();
    assert!(!t.is_set(ev));
    // Enough: done, with the count the mixer reached.
    stream.play(8);
    t.worker();
    assert!(t.is_set(ev));
    assert_eq!(t.f.u32_at(iosb), STATUS_SUCCESS.0);
    assert_eq!((t.f.u64_at(iosb + 8), t.f.u64_at(out)), (8, 16));
    assert!(t.f.audio_waits.is_empty());
}

#[test]
fn a_reset_or_the_handle_ends_the_waits() {
    let mut t = setup(true);
    let h = t.stream(s16_stereo(), 32);
    let ev = t.event();
    let (s, _, _, iosb) = t.ioctl_ev(h, ev, wire::WAIT, &5u64.to_le_bytes(), 8);
    assert_eq!(s, STATUS_PENDING);
    assert_eq!(t.ioctl(h, wire::RESET, &[], 0).0, STATUS_SUCCESS);
    assert!(t.is_set(ev));
    assert_eq!(t.f.u32_at(iosb), STATUS_CANCELLED.0);
    t.worker();
    assert!(t.f.audio_waits.is_empty(), "the worker lets a cancelled wait go");
    // Closing the handle cancels a wait and lets the stream go.
    let ev = t.event();
    let (s, _, _, iosb) = t.ioctl_ev(h, ev, wire::WAIT, &5u64.to_le_bytes(), 8);
    assert_eq!(s, STATUS_PENDING);
    t.close(h);
    t.drain();
    assert_eq!(t.f.u32_at(iosb), STATUS_CANCELLED.0);
    t.worker();
    let stream = t.streams.lock().unwrap().pop().unwrap();
    assert_eq!(Arc::strong_count(&stream), 1, "only the test holds the stream");
}
