//! End-to-end tests against the simulated device in `sim`.

use super::*;
use crate::sim::{CommonRegs, DmaMemory, NotifyRegs, Seen, Sim};
use poly_block::{
    Abort, BlockDevice, DeviceSlot, IoBuf, IoError, Op, Part, QueueStorage, Request, RequestId, RequestQueue,
    RequestSlot, SubmitError, Watchdog,
};
use poly_virtio::{Buffer, DescState, Layout, PciTransport, Region};
extern crate std;
use std::{vec, vec::Vec};

const VERSION_1: u64 = poly_virtio::feature::VERSION_1;
const EVENT_IDX: u64 = poly_virtio::feature::EVENT_IDX;
const CONFIG_LEN: u32 = 60;

type Driver<'a, T = IdentityDma> = VirtioBlk<'a, CommonRegs, NotifyRegs, T>;

fn transport() -> PciTransport {
    PciTransport {
        common: Region { bar: 0, offset: 0, length: 0x38 },
        notify: Region { bar: 0, offset: 0x1000, length: 4 },
        notify_off_multiplier: 4,
        isr: Region { bar: 0, offset: 0x2000, length: 1 },
        device: Some(Region { bar: 0, offset: 0x3000, length: CONFIG_LEN }),
        pci_cfg: None,
    }
}

/// The driver's memory and tables, for a queue of up to `entries`.
struct Harness {
    sim: Sim,
    queue_mem: DmaMemory,
    area: DmaMemory,
    desc: Vec<DescState>,
    slots: Vec<ReqSlot>,
    buffers: Vec<Buffer>,
}

impl Harness {
    fn new(sim: &Sim, entries: u16) -> Harness {
        Harness {
            sim: sim.clone(),
            queue_mem: DmaMemory::new(Layout::new(entries).unwrap().total()),
            area: DmaMemory::new(request_area_len(entries)),
            desc: vec![DescState::default(); entries as usize],
            slots: vec![ReqSlot::EMPTY; entries as usize],
            buffers: vec![Buffer { addr: 0, len: 0 }; entries as usize],
        }
    }

    fn init_with<T: Dma>(&mut self, dma: T) -> Result<Driver<'_, T>, InitError> {
        let setup = Setup {
            common: CommonCfg::new(self.sim.common()),
            device_cfg: self.sim.config(),
            device_cfg_len: CONFIG_LEN,
            transport: transport(),
            notify: self.sim.notify(),
            dma,
            queue_mem: self.queue_mem.region(),
            area: self.area.region(),
            storage: Storage { desc_state: &mut self.desc, slots: &mut self.slots, buffers: &mut self.buffers },
            vector: None,
        };
        // SAFETY: the harness owns the memory and outlives the driver; only
        // the driver and the simulated device use it.
        unsafe { init(setup) }
    }

    fn init(&mut self) -> Driver<'_> {
        self.init_with(IdentityDma).unwrap()
    }
}

/// Storage for a request queue in front of the driver.
struct Tables {
    requests: Vec<RequestSlot>,
    device: Vec<DeviceSlot>,
    segments: Vec<IoBuf>,
    parts: Vec<Part>,
}

impl Tables {
    fn new(requests: usize, device: usize, segments_each: usize) -> Tables {
        Tables {
            requests: vec![RequestSlot::EMPTY; requests],
            device: vec![DeviceSlot::EMPTY; device],
            segments: vec![IoBuf::EMPTY; device * segments_each],
            parts: vec![Part::EMPTY; device * segments_each],
        }
    }

    fn storage(&mut self) -> QueueStorage<'_> {
        QueueStorage {
            requests: &mut self.requests,
            device: &mut self.device,
            segments: &mut self.segments,
            parts: &mut self.parts,
        }
    }
}

fn fill(buf: &mut [u8], tag: u8) {
    for (i, b) in buf.iter_mut().enumerate() {
        *b = tag ^ (i as u8).wrapping_mul(13) ^ (i >> 9) as u8;
    }
}

fn types(log: &[Seen]) -> Vec<u32> {
    log.iter().map(|s| s.ty).collect()
}

#[test]
fn feature_selection() {
    assert_eq!(select_features(u64::MAX), DRIVER_FEATURES);
    let unwanted = feature::MQ
        | feature::DISCARD
        | feature::WRITE_ZEROES
        | feature::CONFIG_WCE
        | feature::GEOMETRY
        | feature::TOPOLOGY
        | poly_virtio::feature::RING_PACKED;
    assert_eq!(select_features(unwanted), 0);
    assert_eq!(select_features(feature::FLUSH | feature::RO | VERSION_1), feature::FLUSH | feature::RO);
}

#[test]
fn reading_the_configuration() {
    let sim = Sim::new(8, VERSION_1, 8);
    sim.0.config.borrow_mut()[0..8].copy_from_slice(&0x2_0000_0005u64.to_le_bytes());
    sim.set_config32(8, 65536);
    sim.set_config32(12, 126);
    sim.set_config32(20, 4096);
    let common = CommonCfg::new(sim.common());
    let all = feature::SIZE_MAX | feature::SEG_MAX | feature::BLK_SIZE | feature::FLUSH;
    let c = BlkConfig::read(&common, &sim.config(), CONFIG_LEN, all).unwrap();
    assert_eq!(
        c,
        BlkConfig {
            capacity: 0x2_0000_0005,
            size_max: Some(65536),
            seg_max: Some(126),
            blk_size: Some(4096),
            read_only: false,
            flush: true
        }
    );
    // Fields of features not negotiated are not read.
    let c = BlkConfig::read(&common, &sim.config(), 8, feature::RO).unwrap();
    assert_eq!((c.size_max, c.seg_max, c.blk_size, c.read_only, c.flush), (None, None, None, true, false));
    assert_eq!(BlkConfig::read(&common, &sim.config(), 23, all), Err(ConfigError::TooShort));
    assert_eq!(BlkConfig::read(&common, &sim.config(), 15, feature::SEG_MAX), Err(ConfigError::TooShort));
    assert_eq!(BlkConfig::read(&common, &sim.config(), 7, 0), Err(ConfigError::TooShort));
    sim.0.churn.set(true);
    assert_eq!(BlkConfig::read(&common, &sim.config(), CONFIG_LEN, all), Err(ConfigError::Unstable));
}

#[test]
fn device_info_from_the_configuration() {
    let base =
        BlkConfig { capacity: 81, size_max: None, seg_max: None, blk_size: None, read_only: false, flush: false };
    let i = base.info(128).unwrap();
    assert_eq!((i.block_size, i.blocks, i.max_segments, i.max_segment_len), (512, 81, 126, u32::MAX));
    // A 4 KiB device: whole blocks only.
    let i = BlkConfig { blk_size: Some(4096), read_only: true, flush: true, ..base }.info(128).unwrap();
    assert_eq!((i.block_size, i.blocks, i.read_only, i.write_cache), (4096, 10, true, true));
    // Odd block sizes fall back to sectors.
    for bs in [0, 256, 1000, 1 << 17] {
        assert_eq!(BlkConfig { blk_size: Some(bs), ..base }.info(128).unwrap().block_size, 512);
    }
    // seg_max is taken to include header and status, and capped by the queue.
    assert_eq!(BlkConfig { seg_max: Some(126), ..base }.info(128).unwrap().max_segments, 124);
    assert_eq!(BlkConfig { seg_max: Some(1000), ..base }.info(16).unwrap().max_segments, 14);
    assert_eq!(BlkConfig { seg_max: Some(1), ..base }.info(16).unwrap().max_segments, 1);
    assert_eq!(BlkConfig { seg_max: Some(0), ..base }.info(16).unwrap().max_segments, 1);
    // size_max 0 means nothing; a block that does not fit the limits
    // falls back to sectors; a sector that does not is unusable.
    assert_eq!(BlkConfig { size_max: Some(0), ..base }.info(16).unwrap().max_segment_len, u32::MAX);
    let small = BlkConfig { size_max: Some(1024), seg_max: Some(3), blk_size: Some(4096), ..base };
    assert_eq!(small.info(16).unwrap().block_size, 512);
    let tiny = BlkConfig { size_max: Some(100), seg_max: Some(7), ..base };
    assert_eq!(tiny.info(16), Err(ConfigError::Unusable));
    assert_eq!(base.info(2), Err(ConfigError::Unusable));
    assert_eq!(base.info(4).unwrap().max_segments, 2);
}

#[test]
fn reads_and_writes_end_to_end() {
    let offered = VERSION_1
        | EVENT_IDX
        | feature::SIZE_MAX
        | feature::SEG_MAX
        | feature::BLK_SIZE
        | feature::FLUSH
        | feature::MQ
        | feature::DISCARD;
    let sim = Sim::new(2048, offered, 64);
    sim.set_config32(8, 4096);
    sim.set_config32(12, 6);
    sim.set_config32(20, 4096);
    let mut h = Harness::new(&sim, 32);
    let mut drv = h.init();
    assert_eq!(drv.queue_size(), 32);
    assert_eq!(
        drv.features(),
        VERSION_1 | EVENT_IDX | feature::SIZE_MAX | feature::SEG_MAX | feature::BLK_SIZE | feature::FLUSH
    );
    assert_eq!(*sim.0.status_writes.borrow(), [0, 1, 3, 11, 15]);
    let i = drv.info();
    assert_eq!((i.block_size, i.blocks, i.max_segments, i.max_segment_len, i.write_cache), (4096, 256, 4, 4096, true));
    assert_eq!(&drv.device_id().unwrap(), b"poly-sim-disk-0001\0\0");

    let mut t = Tables::new(8, 4, 8);
    let mut q = RequestQueue::new(&mut drv, t.storage()).unwrap();
    let mut data = vec![0u8; 10 * 4096];
    fill(&mut data, 0x42);
    q.write(3, &data).unwrap();
    let mut back = vec![0u8; 10 * 4096];
    q.read(3, &mut back).unwrap();
    assert_eq!(back, data);
    q.flush().unwrap();
    assert_eq!(&sim.dev().disk[3 * 4096..13 * 4096], &data[..]);
    let log = sim.dev().log.clone();
    // GET_ID, three writes of at most four segments, three reads, a flush.
    assert_eq!(types(&log), [8, 1, 1, 1, 0, 0, 0, 4]);
    let writes: Vec<(u64, u32, usize)> = log[1..4].iter().map(|s| (s.sector, s.len, s.descs)).collect();
    assert_eq!(writes, [(24, 16384, 6), (56, 16384, 6), (88, 8192, 4)]);
    assert!(log.iter().all(|s| s.status == req_status::OK));
    assert_eq!(drv.in_flight(), 0);
}

#[test]
fn flush_needs_the_feature() {
    let sim = Sim::new(64, VERSION_1, 16);
    let mut h = Harness::new(&sim, 16);
    let mut drv = h.init();
    assert!(!drv.info().write_cache);
    // SAFETY: a flush transfers no memory.
    let r = unsafe { drv.submit(&Request { id: RequestId(1), op: Op::Flush, lba: 0, segments: &[] }) };
    assert_eq!(r, Err(SubmitError::Rejected(IoError::Unsupported)));
    let mut t = Tables::new(4, 2, 4);
    let mut q = RequestQueue::new(&mut drv, t.storage()).unwrap();
    q.write(0, &[1u8; 512]).unwrap();
    q.flush().unwrap();
    assert_eq!(types(&sim.dev().log), [req_type::OUT]);
}

#[test]
fn error_statuses() {
    let sim = Sim::new(64, VERSION_1 | feature::FLUSH, 16);
    sim.dev().bad_sectors = vec![10];
    let mut h = Harness::new(&sim, 16);
    let mut drv = h.init();
    let mut t = Tables::new(4, 2, 4);
    let mut q = RequestQueue::new(&mut drv, t.storage()).unwrap();
    let mut buf = vec![0u8; 4 * 512];
    assert_eq!(q.read(8, &mut buf), Err(IoError::Media));
    assert_eq!(q.write(9, &buf[..512]), Ok(()));
    assert_eq!(q.write(10, &buf[..512]), Err(IoError::Media));
    sim.dev().status_override = Some(req_status::UNSUPP);
    assert_eq!(q.flush(), Err(IoError::Unsupported));
    // A status the specification does not define, or none at all.
    for st in [7, 0xff] {
        sim.dev().status_override = Some(st);
        assert_eq!(q.read(0, &mut buf), Err(IoError::Media));
    }
    sim.dev().status_override = None;
    assert_eq!(q.read(0, &mut buf), Ok(()));
    // Requests beyond the end never reach the device.
    let n = sim.dev().log.len();
    assert_eq!(q.read(62, &mut buf), Err(IoError::OutOfRange));
    assert_eq!(sim.dev().log.len(), n);
}

#[test]
fn read_only_devices_refuse_writes() {
    let sim = Sim::new(64, VERSION_1 | feature::RO, 16);
    let mut h = Harness::new(&sim, 16);
    let mut drv = h.init();
    assert!(drv.info().read_only);
    assert!(sim.negotiated(feature::RO));
    let buf = [0u8; 512];
    let seg = [IoBuf::from_ref(&buf)];
    // SAFETY: refused before anything is queued.
    let r = unsafe { drv.submit(&Request { id: RequestId(1), op: Op::Write, lba: 0, segments: &seg }) };
    assert_eq!(r, Err(SubmitError::Rejected(IoError::ReadOnly)));
    let mut t = Tables::new(4, 2, 4);
    let mut q = RequestQueue::new(&mut drv, t.storage()).unwrap();
    assert_eq!(q.write(0, &buf), Err(IoError::ReadOnly));
    let mut back = [1u8; 512];
    q.read(0, &mut back).unwrap();
    assert_eq!(back, buf);
    assert_eq!(types(&sim.dev().log), [req_type::IN]);
}

#[test]
fn a_full_queue_pushes_back() {
    let sim = Sim::new(256, VERSION_1 | EVENT_IDX, 4);
    let mut h = Harness::new(&sim, 16);
    let mut drv = h.init();
    assert_eq!((drv.queue_size(), drv.info().max_segments), (4, 2));
    sim.dev().hold = true;
    let mut a = [0u8; 512];
    let mut b = [0u8; 512];
    let (sa, sb) = ([IoBuf::from_mut(&mut a)], [IoBuf::from_mut(&mut b)]);
    // SAFETY: the buffers outlive the requests, which are collected below.
    unsafe {
        drv.submit(&Request { id: RequestId(1), op: Op::Read, lba: 0, segments: &sa }).unwrap();
        // Three descriptors taken, one left.
        let r = drv.submit(&Request { id: RequestId(2), op: Op::Read, lba: 1, segments: &sb });
        assert_eq!(r, Err(SubmitError::Busy));
    }
    drv.kick();
    assert_eq!(drv.poll(), None);
    sim.release_held();
    assert_eq!(drv.poll().map(|c| c.id), Some(RequestId(1)));
    // SAFETY: as above.
    unsafe { drv.submit(&Request { id: RequestId(2), op: Op::Read, lba: 1, segments: &sb }).unwrap() };
    drv.kick();
    sim.release_held();
    assert_eq!(drv.poll().map(|c| c.id), Some(RequestId(2)));
    sim.dev().hold = false;

    // Through a request queue, many more requests than fit at once.
    let mut t = Tables::new(32, 8, 4);
    let mut q = RequestQueue::new(&mut drv, t.storage()).unwrap();
    let mut mem = vec![0u8; 40 * 512];
    fill(&mut mem, 9);
    let mut tickets = Vec::new();
    for i in 0..20 {
        // Every third block, so nothing merges.
        let buf = IoBuf::from_ref(&mem[i * 1024..i * 1024 + 512]);
        // SAFETY: `mem` outlives the requests.
        tickets.push(unsafe { q.enqueue(Op::Write, i as u64 * 3, buf) }.unwrap());
    }
    for t in tickets {
        assert_eq!(q.wait(t), Ok(()));
    }
    let disk = &sim.dev().disk;
    for i in 0..20 {
        assert_eq!(&disk[i * 3 * 512..(i * 3 + 1) * 512], &mem[i * 1024..i * 1024 + 512]);
    }
}

#[test]
fn notifications_are_batched() {
    let sim = Sim::new(256, VERSION_1 | EVENT_IDX, 64);
    let mut h = Harness::new(&sim, 64);
    let mut drv = h.init();
    let mut t = Tables::new(16, 16, 2);
    let mut q = RequestQueue::new(&mut drv, t.storage()).unwrap();
    let mut mem = vec![0u8; 16 * 512];
    let mut tickets = Vec::new();
    for (i, chunk) in mem.chunks_mut(1024).enumerate() {
        // SAFETY: `mem` outlives the requests.
        tickets.push(unsafe { q.enqueue(Op::Read, i as u64 * 10, IoBuf::from_mut(&mut chunk[..512])) }.unwrap());
    }
    q.process();
    for t in tickets {
        assert_eq!(q.take(t), Some(Ok(())));
    }
    assert_eq!(sim.0.notifies.get(), 1);
    assert_eq!(sim.dev().log.len(), 8);
}

#[test]
fn out_of_order_completions_against_a_model() {
    let sim = Sim::new(128, VERSION_1 | EVENT_IDX | feature::FLUSH | feature::SEG_MAX, 16);
    sim.set_config32(12, 4);
    let mut h = Harness::new(&sim, 16);
    let mut drv = h.init();
    sim.dev().reverse = true;
    let mut t = Tables::new(12, 6, 4);
    let mut q = RequestQueue::new(&mut drv, t.storage()).unwrap();
    let mut model = vec![0u8; 128 * 512];
    let mut state = 0x1234_5678u64;
    let mut rand = |n: u64| {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state % n
    };
    let mut lent: Vec<(poly_block::Ticket, Vec<u8>, Option<Vec<u8>>)> = Vec::new();
    for step in 0..600 {
        let lba = rand(128);
        let len = (1 + rand(6)).min(128 - lba) as usize;
        let range = lba as usize * 512..(lba as usize + len) * 512;
        let (op, mut buf, expect) = match rand(8) {
            0 => (Op::Flush, Vec::new(), None),
            1..=3 => {
                let mut b = vec![0u8; len * 512];
                fill(&mut b, step as u8);
                (Op::Write, b, None)
            }
            _ => (Op::Read, vec![0u8; len * 512], Some(model[range.clone()].to_vec())),
        };
        let io = if op == Op::Flush { IoBuf::EMPTY } else { IoBuf::from_mut(&mut buf) };
        // SAFETY: the buffer is kept in `lent` until its result is collected.
        match unsafe { q.enqueue(op, lba, io) } {
            Ok(t) => {
                if op == Op::Write {
                    model[range].copy_from_slice(&buf);
                }
                lent.push((t, buf, expect));
            }
            Err(SubmitError::Busy) => {}
            Err(e) => panic!("{e:?}"),
        }
        if rand(3) == 0 || lent.len() >= 12 {
            q.process();
            lent.retain(|(t, buf, expect)| match q.take(*t) {
                None => true,
                Some(r) => {
                    assert_eq!(r, Ok(()));
                    if let Some(e) = expect {
                        assert_eq!(buf, e, "step {step}");
                    }
                    false
                }
            });
        }
    }
    for (t, buf, expect) in lent.drain(..) {
        assert_eq!(q.wait(t), Ok(()));
        if let Some(e) = expect {
            assert_eq!(buf, e);
        }
    }
    assert_eq!(sim.dev().disk, model);
}

#[test]
fn a_broken_device_is_reset_and_fails_everything() {
    let sim = Sim::new(64, VERSION_1, 16);
    let mut h = Harness::new(&sim, 16);
    let mut drv = h.init();
    sim.dev().hold = true;
    let mut a = [0u8; 1024];
    let (x, y) = a.split_at_mut(512);
    let (sx, sy) = ([IoBuf::from_mut(x)], [IoBuf::from_mut(y)]);
    // SAFETY: the buffers outlive the requests, which come back below.
    unsafe {
        drv.submit(&Request { id: RequestId(1), op: Op::Read, lba: 0, segments: &sx }).unwrap();
        drv.submit(&Request { id: RequestId(2), op: Op::Read, lba: 1, segments: &sy }).unwrap();
    }
    drv.kick();
    sim.dev().bogus_id = Some(9);
    // The device will not finish resetting at first: nothing comes back,
    // since it might still write the buffers.
    sim.0.stuck_reset.set(true);
    sim.release_held();
    assert_eq!(drv.poll(), None);
    assert!(drv.is_failed());
    assert_eq!(drv.poll(), None);
    sim.0.stuck_reset.set(false);
    let mut ids: Vec<u64> = core::iter::from_fn(|| drv.poll())
        .map(|c| {
            assert_eq!(c.result, Err(IoError::DeviceFailed));
            c.id.0
        })
        .collect();
    ids.sort();
    assert_eq!(ids, [1, 2]);
    assert_eq!(sim.0.status.get(), poly_virtio::status::FAILED);
    // SAFETY: refused at once.
    let r = unsafe { drv.submit(&Request { id: RequestId(3), op: Op::Read, lba: 0, segments: &sx }) };
    assert_eq!(r, Err(SubmitError::Rejected(IoError::DeviceFailed)));
    assert_eq!(drv.device_id(), Err(IoError::DeviceFailed));
}

#[test]
fn a_device_that_stops_answering_is_aborted() {
    let sim = Sim::new(64, VERSION_1, 16);
    let mut h = Harness::new(&sim, 16);
    let mut drv = h.init();
    // The device takes requests and never answers.
    sim.dev().hold = true;
    let mut w = Watchdog::new(&mut drv, 50);
    let mut t = Tables::new(4, 2, 4);
    let mut q = RequestQueue::new(&mut w, t.storage()).unwrap();
    let mut buf = vec![0u8; 1024];
    assert_eq!(q.read(0, &mut buf), Err(IoError::DeviceFailed));
    assert_eq!(q.read(4, &mut buf), Err(IoError::DeviceFailed));
    let _ = q.into_device();
    assert_eq!(w.aborts(), 1);
    assert!(drv.is_failed());
    assert_eq!(sim.0.status.get(), poly_virtio::status::FAILED);
    assert_eq!(drv.in_flight(), 0);
    // Aborting a device that is already reset changes nothing.
    let resets = sim.0.status_writes.borrow().iter().filter(|&&s| s == 0).count();
    drv.abort();
    assert_eq!(sim.0.status_writes.borrow().iter().filter(|&&s| s == 0).count(), resets);
}

#[test]
fn initialisation_failures() {
    // No VERSION_1.
    let sim = Sim::new(64, feature::FLUSH, 16);
    let mut h = Harness::new(&sim, 16);
    assert_eq!(h.init_with(IdentityDma).err(), Some(InitError::Virtio(poly_virtio::InitError::LegacyOnly)));
    // Storage for fewer than four entries.
    let sim = Sim::new(64, VERSION_1, 16);
    let mut h = Harness::new(&sim, 2);
    assert_eq!(h.init_with(IdentityDma).err(), Some(InitError::NoRoom));
    assert_eq!(sim.0.status.get() & poly_virtio::status::FAILED, poly_virtio::status::FAILED);
    // A device whose limits cannot carry a sector.
    let sim = Sim::new(64, VERSION_1 | feature::SIZE_MAX | feature::SEG_MAX, 16);
    sim.set_config32(8, 64);
    sim.set_config32(12, 3);
    let mut h = Harness::new(&sim, 16);
    assert_eq!(h.init_with(IdentityDma).err(), Some(InitError::Config(ConfigError::Unusable)));
    // Queue memory for a smaller queue than the device offers: it is used.
    let sim = Sim::new(64, VERSION_1, 256);
    let mut h = Harness::new(&sim, 8);
    assert_eq!(h.init().queue_size(), 8);
}

#[test]
fn memory_the_device_cannot_reach() {
    struct NoDma;
    impl Dma for NoDma {
        fn bus_address(&self, _: IoBuf) -> Option<u64> {
            None
        }
    }
    let sim = Sim::new(64, VERSION_1, 16);
    let mut h = Harness::new(&sim, 16);
    let mut drv = h.init_with(NoDma).unwrap();
    let mut t = Tables::new(4, 2, 4);
    let mut q = RequestQueue::new(&mut drv, t.storage()).unwrap();
    let mut buf = [0u8; 512];
    assert_eq!(q.read(0, &mut buf), Err(IoError::BadBuffer));
    // Requests that carry no data still work.
    let _ = q.into_device();
    assert_eq!(&drv.device_id().unwrap()[..4], b"poly");
    assert!(sim.dev().log.iter().all(|s| s.ty == req_type::GET_ID));
}
