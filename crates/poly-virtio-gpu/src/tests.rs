//! Tests: the wire format against offsets from the spec, EDID parsing,
//! scanout planning, and the driver against the simulated device in `sim`.

use super::*;
use crate::edid::{Edid, EdidError};
use crate::modes::{Plan, Request, SizeSource};
use crate::protocol::{Command, ResponseError, ctrl, decode_response};
use crate::sim::{
    CommonRegs, ConfigRegs, DmaMemory, ERR_INVALID_PARAMETER, ERR_INVALID_RESOURCE_ID, ERR_INVALID_SCANOUT_ID,
    NotifyRegs, Sim,
};
use core::sync::atomic::{AtomicU64, Ordering};
use poly_virtio::{DescState, Layout, PciTransport, Region, status};
extern crate std;
use std::{vec, vec::Vec};

const VERSION_1: u64 = poly_virtio::feature::VERSION_1;
const EVENT_IDX: u64 = poly_virtio::feature::EVENT_IDX;
/// Roughly what QEMU's virtio-gpu-gl-pci offers: EDID, event index, indirect
/// descriptors (which the driver must not take), 3D and blobs.
const OFFERED: u64 = VERSION_1
    | EVENT_IDX
    | poly_virtio::feature::INDIRECT_DESC
    | feature::EDID
    | feature::VIRGL
    | feature::RESOURCE_BLOB;

type Driver<'a> = VirtioGpu<'a, CommonRegs, ConfigRegs, NotifyRegs>;

fn transport() -> PciTransport {
    PciTransport {
        common: Region { bar: 0, offset: 0, length: 0x38 },
        notify: Region { bar: 0, offset: 0x1000, length: 16 },
        notify_off_multiplier: 4,
        isr: Region { bar: 0, offset: 0x2000, length: 1 },
        device: Some(Region { bar: 0, offset: 0x3000, length: cfg::LEN }),
        pci_cfg: None,
    }
}

/// A clock that moves 1 ms each time it is read.
fn fake_now() -> u64 {
    static NOW: AtomicU64 = AtomicU64::new(0);
    NOW.fetch_add(1_000_000, Ordering::Relaxed)
}

const CLOCK: Clock = Clock { now_ns: fake_now, timeout_ns: 50_000_000 };

/// The driver's memory and tables.
struct Harness {
    sim: Sim,
    ring: DmaMemory,
    commands: DmaMemory,
    desc: Vec<DescState>,
    vector: Option<u16>,
}

impl Harness {
    fn new(sim: &Sim) -> Harness {
        Harness {
            sim: sim.clone(),
            ring: DmaMemory::new(Layout::new(16).unwrap().total()),
            commands: DmaMemory::new(COMMAND_MEMORY_LEN),
            desc: vec![DescState::default(); 16],
            vector: None,
        }
    }

    fn try_init(&mut self) -> Result<Driver<'_>, InitError> {
        let setup = Setup {
            common: poly_virtio::CommonCfg::new(self.sim.common()),
            device_cfg: self.sim.config(),
            device_cfg_len: cfg::LEN,
            transport: transport(),
            notify: self.sim.notify(),
            ring: self.ring.region(),
            commands: self.commands.region(),
            desc: &mut self.desc,
            clock: CLOCK,
            queue_vector: self.vector,
        };
        // SAFETY: the harness owns the memory and outlives the driver.
        unsafe { init(setup) }
    }

    fn init(&mut self) -> Driver<'_> {
        self.try_init().expect("driver init")
    }
}

/// One monitor of 640x480 on a single-scanout device.
fn one_monitor() -> Sim {
    Sim::new(OFFERED, 1, &[(0, 640, 480)], 64)
}

const REQ: Request = Request { size: None, fallback: (800, 600), max_pixels: 4096 * 4096 };

/// Guest memory for a framebuffer, and the scatter list for it.
fn framebuffer_memory(width: u32, height: u32) -> (DmaMemory, Vec<MemEntry>) {
    let len = (width * height * 4) as usize;
    let mem = DmaMemory::new(len);
    let entries = vec![MemEntry { addr: mem.ptr.as_ptr() as u64, length: len as u32 }];
    (mem, entries)
}

fn px(fb: &mut [u8], width: u32, x: u32, y: u32, bgrx: [u8; 4]) {
    let at = ((y * width + x) * 4) as usize;
    fb[at..at + 4].copy_from_slice(&bgrx);
}

/// A recognisable colour for each pixel.
fn pattern(x: u32, y: u32) -> [u8; 4] {
    [x as u8, y as u8, (x ^ y) as u8, 0]
}

// --- The wire format ------------------------------------------------------------

fn le32(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().unwrap())
}

fn le64(b: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(b[at..at + 8].try_into().unwrap())
}

#[test]
fn request_layouts_follow_the_spec() {
    let mut b = [0xaau8; 256];
    // Header: type, flags (FENCE), fence_id, ctx_id, ring_idx, padding.
    let n = Command::GetDisplayInfo.encode(Some(7), &mut b).unwrap();
    assert_eq!(n, 24);
    assert_eq!((le32(&b, 0), le32(&b, 4), le64(&b, 8), le32(&b, 16)), (0x100, 1, 7, 0));
    assert_eq!(&b[20..24], &[0, 0, 0, 0]);
    let n = Command::GetDisplayInfo.encode(None, &mut b).unwrap();
    assert_eq!((n, le32(&b, 4), le64(&b, 8)), (24, 0, 0));

    let n = Command::GetEdid { scanout: 3 }.encode(None, &mut b).unwrap();
    assert_eq!((n, le32(&b, 0), le32(&b, 24), le32(&b, 28)), (32, 0x10a, 3, 0));

    let c = Command::ResourceCreate2d { resource_id: 5, format: 2, width: 1024, height: 768 };
    assert_eq!(c.encode(None, &mut b), Some(40));
    assert_eq!((le32(&b, 0), le32(&b, 24), le32(&b, 28), le32(&b, 32), le32(&b, 36)), (0x101, 5, 2, 1024, 768));

    assert_eq!(Command::ResourceUnref { resource_id: 9 }.encode(None, &mut b), Some(32));
    assert_eq!((le32(&b, 0), le32(&b, 24), le32(&b, 28)), (0x102, 9, 0));

    let c = Command::SetScanout { scanout_id: 1, resource_id: 5, rect: Rect::new(1, 2, 3, 4) };
    assert_eq!(c.encode(None, &mut b), Some(48));
    assert_eq!(le32(&b, 0), 0x103);
    assert_eq!([le32(&b, 24), le32(&b, 28), le32(&b, 32), le32(&b, 36)], [1, 2, 3, 4]);
    assert_eq!((le32(&b, 40), le32(&b, 44)), (1, 5));

    let c = Command::ResourceFlush { resource_id: 5, rect: Rect::new(10, 20, 30, 40) };
    assert_eq!(c.encode(None, &mut b), Some(48));
    assert_eq!((le32(&b, 0), le32(&b, 24), le32(&b, 36), le32(&b, 40), le32(&b, 44)), (0x104, 10, 40, 5, 0));

    let c = Command::TransferToHost2d { resource_id: 5, rect: Rect::new(10, 20, 30, 40), offset: 0x0012_3456_789a };
    assert_eq!(c.encode(None, &mut b), Some(56));
    assert_eq!(
        (le32(&b, 0), le32(&b, 28), le64(&b, 40), le32(&b, 48), le32(&b, 52)),
        (0x105, 20, 0x0012_3456_789a, 5, 0)
    );

    let entries = [MemEntry { addr: 0x1000, length: 4096 }, MemEntry { addr: 0x8000_0000_0000, length: 12 }];
    let c = Command::ResourceAttachBacking { resource_id: 5, entries: &entries };
    assert_eq!(c.encode(None, &mut b), Some(64));
    assert_eq!((le32(&b, 0), le32(&b, 24), le32(&b, 28)), (0x106, 5, 2));
    assert_eq!((le64(&b, 32), le32(&b, 40), le32(&b, 44)), (0x1000, 4096, 0));
    assert_eq!((le64(&b, 48), le32(&b, 56), le32(&b, 60)), (0x8000_0000_0000, 12, 0));

    assert_eq!(Command::ResourceDetachBacking { resource_id: 5 }.encode(None, &mut b), Some(32));
    assert_eq!(le32(&b, 0), 0x107);

    // Too short a buffer.
    assert_eq!(c.encode(None, &mut b[..63]), None);
}

#[test]
fn response_lengths_and_types() {
    assert_eq!(protocol::DISPLAY_INFO_LEN, 408);
    assert_eq!(protocol::EDID_RESP_LEN, 1056);
    assert_eq!(Command::GetDisplayInfo.ok_type(), ctrl::RESP_OK_DISPLAY_INFO);
    assert_eq!(Command::GetEdid { scanout: 0 }.ok_type(), ctrl::RESP_OK_EDID);
    assert_eq!(Command::ResourceUnref { resource_id: 1 }.ok_type(), ctrl::RESP_OK_NODATA);
    assert_eq!(protocol::ctrl_name(0x1203), "ERR_INVALID_RESOURCE_ID");
}

fn header(ty: u32, flags: u32, fence: u64) -> Vec<u8> {
    let mut b = vec![0u8; 24];
    b[0..4].copy_from_slice(&ty.to_le_bytes());
    b[4..8].copy_from_slice(&flags.to_le_bytes());
    b[8..16].copy_from_slice(&fence.to_le_bytes());
    b
}

#[test]
fn responses_are_checked() {
    let unref = Command::ResourceUnref { resource_id: 1 };
    let ok = header(ctrl::RESP_OK_NODATA, 1, 9);
    assert_eq!(decode_response(&unref, Some(9), &ok, 24), Ok(protocol::Response::NoData));
    // Errors from the device.
    let err = header(ctrl::RESP_ERR_INVALID_RESOURCE_ID, 1, 9);
    assert_eq!(
        decode_response(&unref, Some(9), &err, 24),
        Err(ResponseError::Device { request: ctrl::RESOURCE_UNREF, response: ctrl::RESP_ERR_INVALID_RESOURCE_ID })
    );
    // The fence must come back.
    assert_eq!(decode_response(&unref, Some(8), &ok, 24), Err(ResponseError::Fence { want: 8, got: Some(9) }));
    let unfenced = header(ctrl::RESP_OK_NODATA, 0, 0);
    assert_eq!(decode_response(&unref, Some(8), &unfenced, 24), Err(ResponseError::Fence { want: 8, got: None }));
    assert!(decode_response(&unref, None, &unfenced, 24).is_ok());
    // Too short.
    assert_eq!(decode_response(&unref, Some(9), &ok, 23), Err(ResponseError::TooShort { len: 23 }));
    let mut info = header(ctrl::RESP_OK_DISPLAY_INFO, 0, 0);
    info.resize(408, 0);
    assert_eq!(decode_response(&Command::GetDisplayInfo, None, &info, 407), Err(ResponseError::TooShort { len: 407 }));
    info[24 + 24 + 8..24 + 24 + 12].copy_from_slice(&1920u32.to_le_bytes());
    info[24 + 24 + 12..24 + 24 + 16].copy_from_slice(&1080u32.to_le_bytes());
    info[24 + 24 + 16..24 + 24 + 20].copy_from_slice(&1u32.to_le_bytes());
    let Ok(protocol::Response::DisplayInfo(d)) = decode_response(&Command::GetDisplayInfo, None, &info, 408) else {
        panic!("display info not parsed")
    };
    assert_eq!(d.scanouts[1], ScanoutInfo { rect: Rect::sized(1920, 1080), enabled: true, flags: 0 });
    assert_eq!(d.enabled(16).map(|(i, _)| i).collect::<Vec<_>>(), [1]);
    assert_eq!(d.enabled(1).count(), 0);
    // EDID sizes are checked against the limit and what was written.
    let edid = Command::GetEdid { scanout: 0 };
    let mut e = header(ctrl::RESP_OK_EDID, 0, 0);
    e.extend_from_slice(&2000u32.to_le_bytes());
    e.resize(1056, 0);
    assert_eq!(decode_response(&edid, None, &e, 1056), Err(ResponseError::BadEdidSize(2000)));
    e[24..28].copy_from_slice(&128u32.to_le_bytes());
    assert_eq!(decode_response(&edid, None, &e, 100), Err(ResponseError::TooShort { len: 100 }));
    let Ok(protocol::Response::Edid(blob)) = decode_response(&edid, None, &e, 160) else { panic!("EDID not parsed") };
    assert_eq!(blob.bytes().len(), 128);
}

#[test]
fn rect_clipping() {
    let r = Rect::new(10, 20, 100, 100);
    assert!(r.fits(110, 120));
    assert!(!r.fits(109, 120));
    assert_eq!(r.clip(50, 50), Rect::new(10, 20, 40, 30));
    assert!(r.clip(5, 5).is_empty());
    let huge = Rect::new(u32::MAX, u32::MAX, u32::MAX, u32::MAX);
    assert!(!huge.fits(u32::MAX, u32::MAX));
    assert!(huge.clip(640, 480).is_empty());
}

// --- EDID ---------------------------------------------------------------------

/// An EDID 1.4 base block from vendor "RHT", product 0x1234, preferring
/// `width` × `height`, named "QEMU Monitor".
fn make_edid(width: u32, height: u32) -> Vec<u8> {
    let mut b = vec![0u8; 128];
    b[..8].copy_from_slice(&[0, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0]);
    // R = 18, H = 8, T = 20, five bits each, big-endian.
    let id: u16 = (18 << 10) | (8 << 5) | 20;
    b[8..10].copy_from_slice(&id.to_be_bytes());
    b[10..12].copy_from_slice(&0x1234u16.to_le_bytes());
    (b[18], b[19]) = (1, 4);
    let d = &mut b[54..72];
    d[0..2].copy_from_slice(&6500u16.to_le_bytes());
    d[2] = width as u8;
    d[3] = 0x40;
    d[4] = (((width >> 8) & 0xf) << 4) as u8 | 0x1;
    d[5] = height as u8;
    d[6] = 0x26;
    d[7] = (((height >> 8) & 0xf) << 4) as u8;
    let n = &mut b[72..90];
    n[3] = 0xfc;
    n[5..18].copy_from_slice(b"QEMU Monitor\n");
    let sum = b.iter().fold(0u8, |s, &x| s.wrapping_add(x));
    b[127] = 0u8.wrapping_sub(sum);
    b
}

#[test]
fn edid_base_block() {
    let b = make_edid(1920, 1080);
    let e = Edid::parse(&b).unwrap();
    assert_eq!(e.vendor(), "RHT");
    assert_eq!(e.product, 0x1234);
    assert_eq!(e.version, (1, 4));
    assert_eq!(e.preferred, Some((1920, 1080)));
    assert_eq!(e.name(), Some("QEMU Monitor"));

    assert_eq!(Edid::parse(&b[..127]), Err(EdidError::TooShort(127)));
    let mut bad = b.clone();
    bad[0] = 1;
    assert_eq!(Edid::parse(&bad), Err(EdidError::BadHeader));
    let mut bad = b.clone();
    bad[100] ^= 1;
    assert_eq!(Edid::parse(&bad), Err(EdidError::BadChecksum));
    // No timing in the first descriptor, no name.
    let mut plain = b.clone();
    plain[54..90].fill(0);
    let sum = plain[..127].iter().fold(0u8, |s, &x| s.wrapping_add(x));
    plain[127] = 0u8.wrapping_sub(sum);
    let e = Edid::parse(&plain).unwrap();
    assert_eq!((e.preferred, e.name()), (None, None));
}

// --- Planning -------------------------------------------------------------------

fn info(scanouts: &[(usize, u32, u32, bool)]) -> DisplayInfo {
    let mut d = DisplayInfo::default();
    for &(i, w, h, enabled) in scanouts {
        d.scanouts[i] = ScanoutInfo { rect: Rect::sized(w, h), enabled, flags: 0 };
    }
    d
}

#[test]
fn plan_takes_the_preferred_mode_of_the_first_enabled_scanout() {
    let d = info(&[(0, 1024, 768, false), (1, 1280, 800, true), (2, 640, 480, true)]);
    let p = Plan::new(&d, 4, &REQ);
    assert_eq!((p.width, p.height, p.primary, p.source), (1280, 800, 1, SizeSource::Preferred));
    // Scanout 2 mirrors the corner that fits it; 0 is off.
    let active: Vec<_> = p.active().collect();
    assert_eq!(active, [(1, Rect::sized(1280, 800)), (2, Rect::sized(640, 480))]);
    // Only the first `num_scanouts` count.
    let p = Plan::new(&d, 1, &REQ);
    assert_eq!((p.width, p.height, p.primary, p.source), (800, 600, 0, SizeSource::Fallback));
    assert_eq!(p.active().collect::<Vec<_>>(), [(0, Rect::sized(800, 600))]);
}

#[test]
fn plan_honours_the_command_line_and_limits() {
    let d = info(&[(0, 1920, 1080, true), (1, 3840, 2160, true)]);
    let p = Plan::new(&d, 2, &Request { size: Some((1024, 768)), ..REQ });
    assert_eq!((p.width, p.height, p.source), (1024, 768, SizeSource::Requested));
    // The bigger monitor shows the whole (smaller) framebuffer.
    assert_eq!(p.scanouts[1], Some(Rect::sized(1024, 768)));
    // Too large for memory: the fallback.
    let small = Request { max_pixels: 1024 * 768, ..REQ };
    let p = Plan::new(&d, 2, &small);
    assert_eq!((p.width, p.height, p.source), (800, 600, SizeSource::Fallback));
    let p = Plan::new(&d, 2, &Request { size: Some((4000, 4000)), ..small });
    assert_eq!(p.source, SizeSource::Fallback);
    // A requested size of zero is ignored.
    let p = Plan::new(&d, 2, &Request { size: Some((0, 768)), ..REQ });
    assert_eq!((p.width, p.height, p.source), (1920, 1080, SizeSource::Preferred));
}

#[test]
fn plan_changes() {
    let before = Plan::new(&info(&[(0, 1024, 768, true), (1, 800, 600, true)]), 2, &REQ);
    let after = Plan::new(&info(&[(0, 1280, 720, true)]), 2, &REQ);
    assert!(after.resized(&before));
    assert_eq!(after.scanouts_to_disable(&before).collect::<Vec<_>>(), [1]);
    let same = Plan::new(&info(&[(0, 1024, 768, true)]), 2, &REQ);
    assert!(!same.resized(&before));
}

// --- The driver -----------------------------------------------------------------

#[test]
fn init_negotiates_and_sets_up_the_control_queue() {
    let sim = one_monitor();
    let mut h = Harness::new(&sim);
    let gpu = h.init();
    assert_eq!(gpu.features(), VERSION_1 | EVENT_IDX | feature::EDID | feature::VIRGL | feature::RESOURCE_BLOB);
    assert!(gpu.has_edid() && gpu.has_virgl() && gpu.has_blob() && !gpu.has_context_init());
    assert!(!gpu.interrupts());
    assert_eq!(gpu.num_scanouts(), 1);
    // As large as the ring and the table allow (16 here, at most 64).
    assert_eq!(gpu.queue_size(), 16);
    assert_eq!(sim.0.status.get(), status::ACKNOWLEDGE | status::DRIVER | status::FEATURES_OK | status::DRIVER_OK);
    let (control, cursor) = (sim.queue(0), sim.queue(1));
    assert_eq!((control.enable, control.size, control.msix), (1, 16, 0xffff));
    assert_eq!(cursor.enable, 0);
    assert!(!sim.negotiated(poly_virtio::feature::INDIRECT_DESC) && !sim.negotiated(feature::RESOURCE_UUID));
}

#[test]
fn init_refuses_bad_devices() {
    let sim = Sim::new(OFFERED, 0, &[], 64);
    let mut h = Harness::new(&sim);
    assert_eq!(h.try_init().err(), Some(InitError::Config(ConfigError::BadScanoutCount(0))));
    assert_ne!(sim.0.status.get() & status::FAILED, 0);

    let sim = Sim::new(OFFERED, 17, &[], 64);
    let mut h = Harness::new(&sim);
    assert_eq!(h.try_init().err(), Some(InitError::Config(ConfigError::BadScanoutCount(17))));

    let sim = one_monitor();
    let mut h = Harness::new(&sim);
    h.commands = DmaMemory::new(COMMAND_MEMORY_LEN - 1);
    assert_eq!(h.try_init().err(), Some(InitError::NoRoom));

    // A queue of 2 entries cannot hold a transfer and its flush.
    let sim = Sim::new(OFFERED, 1, &[(0, 640, 480)], 2);
    let mut h = Harness::new(&sim);
    assert_eq!(h.try_init().err(), Some(InitError::NoRoom));

    let legacy = Sim::new(OFFERED & !VERSION_1, 1, &[], 64);
    let mut h = Harness::new(&legacy);
    assert_eq!(h.try_init().err(), Some(InitError::Virtio(poly_virtio::InitError::LegacyOnly)));
}

#[test]
fn display_info_and_edid() {
    let sim = Sim::new(OFFERED, 2, &[(0, 1024, 768), (1, 1920, 1080)], 64);
    sim.dev().scanouts[1].edid = make_edid(1920, 1080);
    let mut h = Harness::new(&sim);
    let mut gpu = h.init();
    let d = gpu.display_info().unwrap();
    assert_eq!(d.enabled(gpu.num_scanouts()).collect::<Vec<_>>().len(), 2);
    assert_eq!(d.scanouts[1].rect, Rect::sized(1920, 1080));
    let blob = gpu.edid(1).unwrap();
    assert_eq!(Edid::parse(blob.bytes()).unwrap().preferred, Some((1920, 1080)));
    // An empty EDID parses as too short.
    assert_eq!(Edid::parse(gpu.edid(0).unwrap().bytes()), Err(EdidError::TooShort(0)));
    assert_eq!(
        gpu.edid(5),
        Err(Error::Response(ResponseError::Device { request: ctrl::GET_EDID, response: ERR_INVALID_PARAMETER }))
    );
    assert_eq!(gpu.stats().errors, 1);
    // Fence IDs count up, and each request had one.
    let log = sim.dev().log.clone();
    assert_eq!(log.len(), 4);
}

#[test]
fn edid_needs_the_feature() {
    let sim = Sim::new(OFFERED & !feature::EDID, 1, &[(0, 640, 480)], 64);
    let mut h = Harness::new(&sim);
    let mut gpu = h.init();
    assert!(!gpu.has_edid());
    assert_eq!(gpu.edid(0), Err(Error::NoEdid));
    assert!(sim.types().is_empty());
}

/// Show a framebuffer per the device's display info; the pieces.
fn bring_up<'a>(gpu: &mut Driver<'a>, req: &Request) -> (Plan, Framebuffer, DmaMemory) {
    let d = gpu.display_info().unwrap();
    let plan = Plan::new(&d, gpu.num_scanouts(), req);
    let (mem, entries) = framebuffer_memory(plan.width, plan.height);
    let fb = gpu.show(1, &plan, None, &entries).unwrap();
    (plan, fb, mem)
}

#[test]
fn modeset_draw_and_flush_damage() {
    let sim = one_monitor();
    let mut h = Harness::new(&sim);
    let mut gpu = h.init();
    let (plan, fb, mut mem) = bring_up(&mut gpu, &REQ);
    assert_eq!((fb.width, fb.height, fb.stride, fb.resource_id), (640, 480, 2560, 1));
    assert_eq!(
        sim.types(),
        [ctrl::GET_DISPLAY_INFO, ctrl::RESOURCE_CREATE_2D, ctrl::RESOURCE_ATTACH_BACKING, ctrl::SET_SCANOUT]
    );
    {
        let d = sim.dev();
        let r = &d.resources[&1];
        assert_eq!((r.format, r.width, r.height), (protocol::format::B8G8R8X8_UNORM, 640, 480));
        assert_eq!(r.backing, [(mem.ptr.as_ptr() as u64, 640 * 480 * 4)]);
        assert_eq!((d.scanouts[0].resource, d.scanouts[0].rect), (1, (0, 0, 640, 480)));
    }
    assert_eq!(plan.source, SizeSource::Preferred);

    // Draw everywhere, flush only a damaged rectangle away from the origin.
    let fbm = mem.bytes();
    for y in 0..480 {
        for x in 0..640 {
            px(fbm, 640, x, y, pattern(x, y));
        }
    }
    let notifies = gpu.stats().notifies;
    let damage = Rect::new(100, 50, 20, 10);
    gpu.update(&fb, damage).unwrap();
    // Transfer and flush in one notification.
    assert_eq!(gpu.stats().notifies, notifies + 1);
    assert_eq!(gpu.stats().updates, 1);
    let log = sim.dev().log.clone();
    let [.., transfer, flush] = log.as_slice() else { panic!("no update") };
    assert_eq!((transfer.ty, flush.ty), (ctrl::TRANSFER_TO_HOST_2D, ctrl::RESOURCE_FLUSH));
    assert_eq!(le64(&transfer.body, 16), 50 * 2560 + 100 * 4);
    for y in 0..480 {
        for x in 0..640 {
            let want = if (damage.x..120).contains(&x) && (50..60).contains(&y) { pattern(x, y) } else { [0; 4] };
            assert_eq!(sim.screen_pixel(0, x, y), want, "pixel ({x}, {y})");
        }
    }
    // A rectangle sticking out is clipped; one wholly outside sends nothing.
    gpu.update(&fb, Rect::new(600, 470, 100, 100)).unwrap();
    assert_eq!(sim.screen_pixel(0, 639, 479), pattern(639, 479));
    let sent = sim.types().len();
    gpu.update(&fb, Rect::new(640, 0, 10, 10)).unwrap();
    assert_eq!(sim.types().len(), sent);
    assert_eq!(gpu.stats().errors, 0);
}

#[test]
fn backing_may_be_a_scatter_list() {
    let sim = one_monitor();
    let mut h = Harness::new(&sim);
    let mut gpu = h.init();
    let plan = Plan::new(&gpu.display_info().unwrap(), 1, &REQ);
    // Three pieces of odd sizes, not on line boundaries.
    let total = (640 * 480 * 4) as usize;
    let sizes = [1000usize, 500_000, total - 501_000];
    let mut pieces: Vec<DmaMemory> = sizes.iter().map(|&n| DmaMemory::new(n)).collect();
    let entries: Vec<MemEntry> =
        pieces.iter().map(|p| MemEntry { addr: p.ptr.as_ptr() as u64, length: p.len as u32 }).collect();
    let fb = gpu.show(1, &plan, None, &entries).unwrap();
    // Write the pattern through the pieces.
    let mut at = 0usize;
    for p in pieces.iter_mut() {
        for b in p.bytes().iter_mut() {
            let pixel = (at / 4) as u32;
            *b = pattern(pixel % 640, pixel / 640)[at % 4];
            at += 1;
        }
    }
    gpu.update(&fb, Rect::sized(640, 480)).unwrap();
    for (x, y) in [(0, 0), (249, 0), (250, 0), (639, 195), (0, 196), (639, 479)] {
        assert_eq!(sim.screen_pixel(0, x, y), pattern(x, y), "pixel ({x}, {y})");
    }
}

#[test]
fn several_scanouts_mirror_one_framebuffer() {
    let sim = Sim::new(OFFERED, 3, &[(0, 800, 600), (2, 640, 480)], 64);
    let mut h = Harness::new(&sim);
    let mut gpu = h.init();
    let (_, fb, mut mem) = bring_up(&mut gpu, &REQ);
    {
        let d = sim.dev();
        assert_eq!((d.scanouts[0].resource, d.scanouts[0].rect), (1, (0, 0, 800, 600)));
        assert_eq!((d.scanouts[1].resource, d.scanouts[2].resource), (0, 1));
        assert_eq!(d.scanouts[2].rect, (0, 0, 640, 480));
    }
    px(mem.bytes(), 800, 639, 479, [1, 2, 3, 0]);
    px(mem.bytes(), 800, 700, 500, [4, 5, 6, 0]);
    gpu.update(&fb, Rect::sized(800, 600)).unwrap();
    assert_eq!(sim.screen_pixel(0, 639, 479), [1, 2, 3, 0]);
    assert_eq!(sim.screen_pixel(2, 639, 479), [1, 2, 3, 0]);
    assert_eq!(sim.screen_pixel(0, 700, 500), [4, 5, 6, 0]);
}

#[test]
fn display_change_event_and_resize() {
    let sim = Sim::new(OFFERED, 2, &[(0, 640, 480), (1, 320, 200)], 64);
    let mut h = Harness::new(&sim);
    let mut gpu = h.init();
    let (plan, fb, _mem) = bring_up(&mut gpu, &REQ);
    assert_eq!(gpu.take_events(), 0);

    // The monitor on 0 grows, the one on 1 goes away.
    sim.replug(0, Some((1024, 768)));
    sim.replug(1, None);
    assert_eq!(gpu.take_events(), event::DISPLAY);
    assert_eq!(gpu.take_events(), 0, "events_clear acknowledged the event");
    assert_eq!(gpu.stats().display_events, 1);

    let new_plan = Plan::new(&gpu.display_info().unwrap(), gpu.num_scanouts(), &REQ);
    assert!(new_plan.resized(&plan));
    let (mem2, entries) = framebuffer_memory(new_plan.width, new_plan.height);
    let fb2 = gpu.show(2, &new_plan, Some(&plan), &entries).unwrap();
    gpu.destroy(&fb).unwrap();
    {
        let d = sim.dev();
        assert!(!d.resources.contains_key(&1));
        assert_eq!((d.scanouts[0].resource, d.scanouts[0].rect), (2, (0, 0, 1024, 768)));
        assert_eq!(d.scanouts[1].resource, 0, "the unplugged scanout is off");
        assert_eq!((d.unref_while_shown, d.detach_while_shown), (0, 0));
    }
    assert_eq!((fb2.width, fb2.height), (1024, 768));
    drop(mem2);
}

#[test]
fn device_errors_are_reported_and_survivable() {
    let sim = one_monitor();
    let mut h = Harness::new(&sim);
    let mut gpu = h.init();
    let (_, fb, _mem) = bring_up(&mut gpu, &REQ);
    let device = |request, response| Err(Error::Response(ResponseError::Device { request, response }));
    assert_eq!(gpu.set_scanout(3, 1, Rect::sized(10, 10)), device(ctrl::SET_SCANOUT, ERR_INVALID_SCANOUT_ID));
    assert_eq!(gpu.set_scanout(0, 1, Rect::sized(641, 10)), device(ctrl::SET_SCANOUT, ERR_INVALID_PARAMETER));
    assert_eq!(gpu.unref(42), device(ctrl::RESOURCE_UNREF, ERR_INVALID_RESOURCE_ID));
    assert_eq!(gpu.create_2d(1, 2, 10, 10), device(ctrl::RESOURCE_CREATE_2D, ERR_INVALID_RESOURCE_ID));
    assert_eq!(gpu.stats().errors, 4);
    // An update whose transfer fails reports it; the driver carries on.
    sim.dev().fail_next = Some(sim::ERR_UNSPEC);
    assert_eq!(gpu.update(&fb, Rect::sized(8, 8)), device(ctrl::TRANSFER_TO_HOST_2D, sim::ERR_UNSPEC));
    assert!(!gpu.is_broken());
    gpu.update(&fb, Rect::sized(8, 8)).unwrap();
    assert_eq!(gpu.attach_backing(1, &[]), Err(Error::BadBacking));
    let too_many = vec![MemEntry::default(); MAX_BACKING_ENTRIES + 1];
    assert_eq!(gpu.attach_backing(1, &too_many), Err(Error::BadBacking));
}

#[test]
fn show_checks_and_cleans_up() {
    let sim = one_monitor();
    let mut h = Harness::new(&sim);
    let mut gpu = h.init();
    let plan = Plan::new(&gpu.display_info().unwrap(), 1, &REQ);
    let (_mem, entries) = framebuffer_memory(640, 479);
    assert_eq!(
        gpu.show(1, &plan, None, &entries),
        Err(Error::BackingTooSmall { have: 640 * 479 * 4, need: 640 * 480 * 4 })
    );
    assert!(sim.dev().resources.is_empty());
    // Setting the scanouts fails: the resource is destroyed again.
    let (_mem, entries) = framebuffer_memory(640, 480);
    let before = sim.types().len();
    let mut plan_bad = plan;
    plan_bad.scanouts[0] = Some(Rect::sized(641, 480));
    assert_eq!(gpu.show(1, &plan_bad, None, &entries), Err(Error::BadRect));
    assert!(sim.dev().resources.is_empty(), "the resource was destroyed after the failure");
    assert_eq!(
        sim.types()[before..],
        [ctrl::RESOURCE_CREATE_2D, ctrl::RESOURCE_ATTACH_BACKING, ctrl::RESOURCE_DETACH_BACKING, ctrl::RESOURCE_UNREF]
    );
}

#[test]
fn a_hung_device_breaks_the_driver() {
    let sim = one_monitor();
    let mut h = Harness::new(&sim);
    let mut gpu = h.init();
    sim.dev().hold = true;
    assert_eq!(gpu.display_info(), Err(Error::Timeout { request: ctrl::GET_DISPLAY_INFO }));
    assert!(gpu.is_broken());
    sim.dev().hold = false;
    assert_eq!(gpu.display_info(), Err(Error::Broken));
    assert!(gpu.reset());
    assert_eq!(sim.0.status.get(), 0);
}

#[test]
fn a_wrong_fence_or_used_id_is_caught() {
    let sim = one_monitor();
    let mut h = Harness::new(&sim);
    let mut gpu = h.init();
    sim.dev().bad_fence = true;
    assert!(matches!(gpu.display_info(), Err(Error::Response(ResponseError::Fence { .. }))));
    sim.dev().bad_fence = false;
    assert!(gpu.display_info().is_ok());
    sim.dev().bogus_id = Some(3);
    assert!(matches!(gpu.display_info(), Err(Error::Queue(_) | Error::Protocol)));
    assert!(gpu.is_broken());
}

#[test]
fn many_updates_wrap_the_queue() {
    let sim = one_monitor();
    let mut h = Harness::new(&sim);
    let mut gpu = h.init();
    let (_, fb, mut mem) = bring_up(&mut gpu, &REQ);
    for i in 0..300u32 {
        let (x, y) = (i % 640, (i * 7) % 480);
        px(mem.bytes(), 640, x, y, pattern(x, y));
        gpu.update(&fb, Rect::new(x, y, 1, 1)).unwrap();
        assert_eq!(sim.screen_pixel(0, x, y), pattern(x, y));
    }
    assert_eq!(gpu.stats().updates, 300);
    assert!(!gpu.is_broken());
}

// --- 3D: capsets, contexts, resources, submission, blobs, fences --------------------

/// A 3D device: virgl and virgl2 capsets, context init, blobs, a 1 MiB
/// host-visible region.
fn gpu_3d() -> Sim {
    let sim = Sim::new(OFFERED | feature::CONTEXT_INIT, 1, &[(0, 640, 480)], 64);
    {
        let mut d = sim.dev();
        d.capsets = vec![(1, 1, vec![0x11; 300]), (2, 2, (0..900u32).map(|i| i as u8).collect()), (4, 0, vec![4; 64])];
        d.shm_len = 1 << 20;
    }
    sim
}

/// A toy `FILL resource byte` command.
fn fill(resource: u32, byte: u8) -> Vec<u8> {
    [sim::TOY_FILL, resource, byte as u32].iter().flat_map(|v| v.to_le_bytes()).collect()
}

fn buffer_of(mem: &DmaMemory, len: usize) -> Buffer {
    Buffer { addr: mem.ptr.as_ptr() as u64, len: len as u32 }
}

#[test]
fn encodes_3d_requests_at_the_spec_offsets() {
    let mut b = [0u8; 256];
    let c = Create3d {
        resource_id: 7,
        target: 2,
        format: 1,
        bind: 0x8,
        width: 64,
        height: 32,
        depth: 1,
        array_size: 1,
        last_level: 0,
        nr_samples: 0,
        flags: 0,
    };
    assert_eq!(Command::ResourceCreate3d(c).encode(Some(9), &mut b), Some(24 + 48));
    assert_eq!((le32(&b, 0), le32(&b, 4), le64(&b, 8)), (ctrl::RESOURCE_CREATE_3D, 1, 9));
    assert_eq!((le32(&b, 24), le32(&b, 28), le32(&b, 32), le32(&b, 36)), (7, 2, 1, 8));
    assert_eq!((le32(&b, 40), le32(&b, 44), le32(&b, 48), le32(&b, 52)), (64, 32, 1, 1));

    let t = Transfer3d {
        resource_id: 3,
        region: Box3d { x: 1, y: 2, z: 0, w: 4, h: 5, d: 1 },
        offset: 0x1234,
        level: 2,
        stride: 256,
        layer_stride: 4096,
    };
    let n = Command::TransferFromHost3d { ctx_id: 5, transfer: t }.encode_on_ring(Some(3), Some(2), &mut b).unwrap();
    assert_eq!(n, 24 + 48);
    // FENCE | INFO_RING_IDX, the context and the ring in the header.
    assert_eq!((le32(&b, 0), le32(&b, 4), le32(&b, 16), b[20]), (ctrl::TRANSFER_FROM_HOST_3D, 3, 5, 2));
    assert_eq!((le32(&b, 24), le32(&b, 28), le32(&b, 36), le32(&b, 40)), (1, 2, 4, 5));
    assert_eq!((le64(&b, 48), le32(&b, 56), le32(&b, 60), le32(&b, 64), le32(&b, 68)), (0x1234, 3, 2, 256, 4096));

    assert_eq!(Command::CtxCreate { ctx_id: 4, context_init: 4, name: b"kmscube" }.encode(None, &mut b), Some(96));
    assert_eq!((le32(&b, 4), le32(&b, 16), le32(&b, 24), le32(&b, 28)), (0, 4, 7, 4));
    assert_eq!(&b[32..39], b"kmscube");
    assert_eq!(Command::Submit3d { ctx_id: 4, size: 96 }.encode(Some(1), &mut b), Some(32));
    assert_eq!((le32(&b, 16), le32(&b, 24)), (4, 96));

    let blob = CreateBlob { resource_id: 9, blob_mem: 2, blob_flags: 1, blob_id: 0xabcd, size: 0x10000 };
    let e = [MemEntry { addr: 0x5000, length: 4096 }];
    assert_eq!(Command::ResourceCreateBlob { ctx_id: 4, blob, entries: &e }.encode(None, &mut b), Some(56 + 16));
    assert_eq!((le32(&b, 24), le32(&b, 28), le32(&b, 32), le32(&b, 36)), (9, 2, 1, 1));
    assert_eq!((le64(&b, 40), le64(&b, 48), le64(&b, 56), le32(&b, 64)), (0xabcd, 0x10000, 0x5000, 4096));
    assert_eq!(Command::ResourceMapBlob { resource_id: 9, offset: 0x2000 }.encode(None, &mut b), Some(40));
    assert_eq!((le32(&b, 24), le64(&b, 32)), (9, 0x2000));
    assert_eq!(Command::GetCapset { id: 2, version: 2, size: 900 }.response_len(), 924);
}

#[test]
fn capsets_are_listed_and_fetched() {
    let sim = gpu_3d();
    let mut h = Harness::new(&sim);
    let mut gpu = h.init();
    assert_eq!(gpu.config().num_capsets, 3);
    let infos: Vec<CapsetInfo> = (0..3).map(|i| gpu.capset_info(i).unwrap()).collect();
    assert_eq!(infos[1], CapsetInfo { id: 2, max_version: 2, max_size: 900 });
    assert_eq!(infos[2].id, 4);
    assert_eq!(gpu.capset_info(3), Err(Error::Unsupported));
    let mut out = DmaMemory::new(24 + 900);
    // SAFETY: the memory outlives the call and only the device writes it.
    let n = unsafe { gpu.capset(2, 2, out.region()) }.unwrap();
    assert_eq!(n, 900);
    assert_eq!(&out.bytes()[24..30], &[0, 1, 2, 3, 4, 5]);
    // A version above the set's newest is refused by the device.
    // SAFETY: as above.
    let r = unsafe { gpu.capset(2, 3, out.region()) };
    assert!(matches!(r, Err(Error::Response(ResponseError::Device { request: ctrl::GET_CAPSET, .. }))));
}

#[test]
fn contexts_resources_transfers_and_submission() {
    let sim = gpu_3d();
    let mut h = Harness::new(&sim);
    let mut gpu = h.init();
    gpu.ctx_create(1, 0, b"virgl test").unwrap();
    gpu.ctx_create(2, crate::protocol::capset::VENUS, b"venus").unwrap();
    assert_eq!(sim.dev().contexts[&2].capset, 4);
    assert_eq!(&sim.dev().contexts[&1].name, b"virgl test");
    // A 2D texture of 16x8 4-byte texels with 16x8x4 bytes of guest memory.
    let c = Create3d {
        resource_id: 0x100,
        target: 2,
        width: 16,
        height: 8,
        depth: 1,
        array_size: 1,
        ..Create3d::default()
    };
    gpu.create_3d(&c).unwrap();
    let mut backing = DmaMemory::new(16 * 8 * 4);
    gpu.attach_backing(0x100, &[MemEntry { addr: backing.ptr.as_ptr() as u64, length: 512 }]).unwrap();
    let all = Transfer3d {
        resource_id: 0x100,
        region: Box3d { w: 16, h: 8, d: 1, ..Box3d::default() },
        ..Transfer3d::default()
    };
    // Not attached to the context yet: refused.
    assert!(gpu.transfer_3d(true, 1, all).is_err());
    gpu.ctx_attach(1, 0x100).unwrap();
    backing.bytes()[..4].copy_from_slice(&[1, 2, 3, 4]);
    gpu.transfer_3d(true, 1, all).unwrap();
    assert_eq!(&sim.dev().resources_3d[&0x100].host[..4], &[1, 2, 3, 4]);

    // A submission fills the resource on the host; a transfer back shows it.
    let cmds = fill(0x100, 0x5a);
    let mut stream = DmaMemory::new(cmds.len());
    stream.bytes().copy_from_slice(&cmds);
    // SAFETY: the stream outlives the ticket, which is waited for below.
    let t = unsafe { gpu.submit_3d(1, None, cmds.len() as u32, &[buffer_of(&stream, cmds.len())]) }.unwrap();
    while gpu.poll(t).is_none() {}
    assert_eq!(gpu.stats().submits, 1);
    // A box of 2x2 texels at (1, 1) into a packed 8-byte-stride copy.
    let part = Transfer3d {
        resource_id: 0x100,
        region: Box3d { x: 1, y: 1, w: 2, h: 2, d: 1, ..Box3d::default() },
        offset: 64,
        stride: 8,
        ..Transfer3d::default()
    };
    gpu.transfer_3d(false, 1, part).unwrap();
    assert_eq!(&backing.bytes()[64..80], &[0x5a; 16]);
    assert_eq!(backing.bytes()[80], 0);
    // A stream that names an unattached resource: the device refuses it.
    let bad = fill(0x999, 1);
    stream = DmaMemory::new(bad.len());
    stream.bytes().copy_from_slice(&bad);
    // SAFETY: as above.
    let t = unsafe { gpu.submit_3d(1, None, bad.len() as u32, &[buffer_of(&stream, bad.len())]) }.unwrap();
    let r = loop {
        if let Some(r) = gpu.poll(t) {
            break r;
        }
    };
    assert!(matches!(r, Err(Error::Response(ResponseError::Device { request: ctrl::SUBMIT_3D, .. }))));
    // The data must add up to the size.
    // SAFETY: nothing is sent.
    assert_eq!(unsafe { gpu.submit_3d(1, None, 99, &[buffer_of(&stream, 12)]) }, Err(Error::BadBacking));
    // Clean-up: detach, unref (also takes it off the context), destroy.
    gpu.ctx_detach(1, 0x100).unwrap();
    gpu.detach_backing(0x100).unwrap();
    gpu.unref(0x100).unwrap();
    assert!(sim.dev().resources_3d.is_empty());
    gpu.ctx_destroy(2).unwrap();
    gpu.ctx_destroy(1).unwrap();
    assert!(sim.dev().contexts.is_empty());
    assert!(!gpu.is_broken());
}

#[test]
fn fences_complete_out_of_order_while_synchronous_commands_go_on() {
    let sim = gpu_3d();
    let mut h = Harness::new(&sim);
    let mut gpu = h.init();
    gpu.ctx_create(1, crate::protocol::capset::VIRGL2, b"t").unwrap();
    let c =
        Create3d { resource_id: 5, target: 0, width: 4096, height: 1, depth: 1, array_size: 1, ..Create3d::default() };
    gpu.create_3d(&c).unwrap();
    gpu.ctx_attach(1, 5).unwrap();
    sim.dev().hold_3d = true;
    let mut streams = Vec::new();
    let mut tickets = Vec::new();
    for (i, ring) in [None, Some(0), Some(1)].into_iter().enumerate() {
        let cmds = fill(5, i as u8 + 1);
        let mut m = DmaMemory::new(cmds.len());
        m.bytes().copy_from_slice(&cmds);
        // SAFETY: the streams live until the end of the test.
        tickets.push(unsafe { gpu.submit_3d(1, ring, 12, &[buffer_of(&m, 12)]) }.unwrap());
        streams.push(m);
    }
    assert_eq!(gpu.in_flight(), 3);
    assert_eq!(sim.held(), 3);
    // Ring fences carry INFO_RING_IDX and the ring; the context is in the header.
    let log: Vec<_> =
        sim.dev().log.iter().filter(|l| l.ty == ctrl::SUBMIT_3D).map(|l| (l.flags, l.ctx_id, l.ring_idx)).collect();
    assert_eq!(log, [(1, 1, 0), (3, 1, 0), (3, 1, 1)]);
    // Synchronous commands are answered meanwhile.
    assert!(gpu.display_info().is_ok());
    assert!(tickets.iter().all(|&t| gpu.poll(t).is_none()));
    // The host finishes the last one first, then the others.
    sim.retire(&[2]);
    assert_eq!(gpu.poll(tickets[2]), Some(Ok(())));
    assert_eq!(gpu.poll(tickets[0]), Some(Ok(())));
    assert_eq!(gpu.poll(tickets[1]), Some(Ok(())));
    // Spent tickets are refused.
    assert_eq!(gpu.poll(tickets[1]), Some(Err(Error::BadTicket)));
    assert_eq!(gpu.in_flight(), 0);
    // A forgotten ticket frees its slot when the device is done.
    let t = gpu.send_transfer_3d(false, 1, None, Transfer3d::default()).unwrap();
    gpu.forget(t);
    assert_eq!(gpu.in_flight(), 1);
    sim.retire(&[]);
    gpu.reap().unwrap();
    assert_eq!(gpu.in_flight(), 0);
    assert!(!gpu.is_broken());
}

#[test]
fn every_slot_busy_is_reported_and_recovers() {
    let sim = Sim::new(OFFERED, 1, &[(0, 640, 480)], 64);
    let mut h = Harness::new(&sim);
    h.ring = DmaMemory::new(Layout::new(64).unwrap().total());
    h.desc = vec![DescState::default(); 64];
    let mut gpu = h.init();
    assert_eq!(gpu.queue_size(), 64);
    gpu.ctx_create(1, 0, b"").unwrap();
    sim.dev().hold_3d = true;
    let tickets: Vec<Ticket> =
        (0..SLOTS).map(|_| gpu.send_transfer_3d(true, 1, None, Transfer3d::default()).unwrap()).collect();
    assert_eq!(gpu.send_transfer_3d(true, 1, None, Transfer3d::default()), Err(Error::Busy));
    // A synchronous command waits for a slot, and gives up within its
    // timeout if none comes back.
    assert_eq!(gpu.display_info(), Err(Error::Timeout { request: ctrl::GET_DISPLAY_INFO }));
    assert!(gpu.is_broken());
    drop(tickets);
}

#[test]
fn blobs_are_created_mapped_and_unmapped() {
    let sim = gpu_3d();
    let mut h = Harness::new(&sim);
    let mut gpu = h.init();
    gpu.ctx_create(3, crate::protocol::capset::VENUS, b"venus").unwrap();
    use crate::protocol::{blob_flags, blob_mem};
    let host = CreateBlob {
        resource_id: 0x20,
        blob_mem: blob_mem::HOST3D,
        blob_flags: blob_flags::USE_MAPPABLE,
        blob_id: 77,
        size: 0x4000,
    };
    gpu.create_blob(3, &host, &[]).unwrap();
    assert_eq!(sim.dev().resources_3d[&0x20].blob, Some((2, 1, 77)));
    assert_eq!(gpu.map_blob(0x20, 0x8000), Ok(crate::protocol::map_cache::CACHED));
    assert_eq!(sim.dev().resources_3d[&0x20].mapped, Some(0x8000));
    // Mapped twice, or past the region: refused.
    assert!(gpu.map_blob(0x20, 0).is_err());
    gpu.unmap_blob(0x20).unwrap();
    assert!(gpu.map_blob(0x20, (1 << 20) - 0x2000).is_err());
    // A guest blob carries its scatter list; host blobs need a context.
    let mem = DmaMemory::new(8192);
    let e = [MemEntry { addr: mem.ptr.as_ptr() as u64, length: 8192 }];
    let guest = CreateBlob { resource_id: 0x21, blob_mem: blob_mem::GUEST, blob_flags: 0, blob_id: 0, size: 8192 };
    gpu.create_blob(0, &guest, &e).unwrap();
    assert_eq!(sim.dev().resources_3d[&0x21].backing, [(mem.ptr.as_ptr() as u64, 8192)]);
    let orphan = CreateBlob { resource_id: 0x22, ..host };
    assert!(gpu.create_blob(9, &orphan, &[]).is_err());
    gpu.unref(0x20).unwrap();
    gpu.unref(0x21).unwrap();
}

#[test]
fn three_d_needs_its_features() {
    let sim = Sim::new(OFFERED & !feature::VIRGL & !feature::RESOURCE_BLOB, 1, &[(0, 640, 480)], 64);
    let mut h = Harness::new(&sim);
    let mut gpu = h.init();
    assert!(!gpu.has_virgl());
    assert_eq!(gpu.ctx_create(1, 0, b""), Err(Error::Unsupported));
    assert_eq!(gpu.create_3d(&Create3d { resource_id: 1, ..Create3d::default() }), Err(Error::Unsupported));
    assert_eq!(gpu.map_blob(1, 0), Err(Error::Unsupported));
    // Without context init, a context of another capset is refused here.
    let sim = gpu_3d();
    let mut h = Harness::new(&Sim::new(OFFERED, 1, &[(0, 640, 480)], 64));
    let mut gpu2 = h.init();
    assert_eq!(gpu2.ctx_create(1, 4, b""), Err(Error::Unsupported));
    gpu2.ctx_create(1, 0, b"").unwrap();
    drop(sim);
}

#[test]
fn the_control_queue_can_interrupt() {
    let sim = gpu_3d();
    sim.0.msix_entries.set(true);
    let mut h = Harness::new(&sim);
    h.vector = Some(0);
    let gpu = h.init();
    assert!(gpu.interrupts());
    assert_eq!(sim.queue(0).msix, 0);
}
