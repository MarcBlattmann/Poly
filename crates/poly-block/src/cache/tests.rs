use super::*;
use crate::sim::{Event, Rng, SimDisk, Tables};
use crate::{DeviceInfo, RamDisk, RequestQueue};
extern crate std;
use std::{vec, vec::Vec};

const BS: usize = 512;

/// A buffer cache's storage.
struct Store {
    slots: Vec<CacheSlot>,
    data: Vec<u8>,
}

impl Store {
    fn new(buffers: usize) -> Store {
        Store { slots: vec![CacheSlot::EMPTY; buffers], data: vec![0; buffers * BS] }
    }

    fn cache(&mut self) -> BufferCache<'_> {
        BufferCache::new(&mut self.slots, &mut self.data, BS).unwrap()
    }
}

fn pattern(tag: u64) -> Vec<u8> {
    (0..BS).map(|i| (tag.wrapping_mul(0x9e37_79b9_7f4a_7c15) >> 56) as u8 ^ (i as u8).wrapping_mul(31)).collect()
}

/// Device requests of kind `op` in the log, as (first block, blocks).
fn requests(log: &[Event], op: Op) -> Vec<(u64, u32)> {
    log.iter()
        .filter_map(|e| match *e {
            Event::Submit { op: o, lba, blocks, .. } if o == op => Some((lba, blocks)),
            _ => None,
        })
        .collect()
}

#[test]
fn setup_checks() {
    let mut slots = vec![CacheSlot::EMPTY; 4];
    let mut data = vec![0u8; 4 * BS];
    assert!(BufferCache::new(&mut [], &mut data, BS).is_none());
    assert!(BufferCache::new(&mut slots, &mut data, 1000).is_none());
    assert!(BufferCache::new(&mut slots, &mut data[..3 * BS + 511], BS).is_none());
    let c = BufferCache::new(&mut slots, &mut data, BS).unwrap();
    assert_eq!((c.capacity(), c.block_size()), (4, BS));
    // Five slots get four hash chains.
    let mut slots = vec![CacheSlot::EMPTY; 5];
    let mut data = vec![0u8; 5 * BS];
    assert_eq!(BufferCache::new(&mut slots, &mut data, BS).unwrap().mask, 3);
}

#[test]
fn hits_misses_and_lru_eviction() {
    let mut mem = vec![0u8; 16 * BS];
    for b in 0..16 {
        mem[b * BS..(b + 1) * BS].copy_from_slice(&pattern(b as u64));
    }
    let mut t = Tables::new(4, 2, 4);
    let mut q = RequestQueue::new(RamDisk::new(&mut mem, BS as u32).unwrap(), t.storage()).unwrap();
    let mut s = Store::new(3);
    let mut c = s.cache();
    for b in [0, 1, 2, 0] {
        let buf = c.get(&mut q, b).unwrap();
        assert_eq!(c.data(&buf), &pattern(b)[..]);
        c.release(buf);
    }
    assert_eq!((c.stats().hits, c.stats().misses), (1, 3));
    // Block 1 is now the least recently used.
    let b3 = c.get(&mut q, 3).unwrap();
    assert!(!c.is_cached(1));
    assert!(c.is_cached(0) && c.is_cached(2) && c.is_cached(3));
    c.release(b3);
    // Taking the same block twice pins it twice.
    let a = c.get(&mut q, 2).unwrap();
    let b = c.get(&mut q, 2).unwrap();
    assert_eq!(c.pinned_count(), 1);
    c.release(a);
    c.get(&mut q, 4).map(|x| c.release(x)).unwrap();
    c.get(&mut q, 5).map(|x| c.release(x)).unwrap();
    assert!(c.is_cached(2), "still pinned once");
    c.release(b);
    assert_eq!(c.pinned_count(), 0);
    assert_eq!(c.stats().evictions, 3);
}

#[test]
fn pinned_buffers_are_never_evicted() {
    let mut mem = vec![0u8; 16 * BS];
    let mut t = Tables::new(4, 2, 4);
    let mut q = RequestQueue::new(RamDisk::new(&mut mem, BS as u32).unwrap(), t.storage()).unwrap();
    let mut s = Store::new(2);
    let mut c = s.cache();
    let a = c.get(&mut q, 0).unwrap();
    let b = c.get(&mut q, 1).unwrap();
    assert_eq!(c.get(&mut q, 2), Err(CacheError::AllPinned));
    assert_eq!(c.get_zeroed(&mut q, 2), Err(CacheError::AllPinned));
    // Cached blocks can still be had.
    let a2 = c.get(&mut q, 0).unwrap();
    c.release(a2);
    c.release(a);
    let x = c.get(&mut q, 2).unwrap();
    assert!(!c.is_cached(0) && c.is_cached(1));
    c.release(x);
    c.release(b);
}

#[test]
fn dirty_blocks_are_written_back_on_eviction() {
    let mut mem = vec![0u8; 16 * BS];
    {
        let mut t = Tables::new(4, 2, 4);
        let mut q = RequestQueue::new(RamDisk::new(&mut mem, BS as u32).unwrap(), t.storage()).unwrap();
        let mut s = Store::new(2);
        let mut c = s.cache();
        let b = c.get(&mut q, 7).unwrap();
        c.data_mut(&b).copy_from_slice(&pattern(70));
        c.release(b);
        assert_eq!(c.dirty_count(), 1);
        assert!(q.device().contents()[7 * BS..8 * BS].iter().all(|&x| x == 0));
        for blk in [1, 2] {
            c.get(&mut q, blk).map(|x| c.release(x)).unwrap();
        }
        assert_eq!((c.dirty_count(), c.stats().write_backs), (0, 1));
        // An explicit write-back of a clean block does nothing.
        let b = c.get(&mut q, 1).unwrap();
        c.write_back(&mut q, &b).unwrap();
        assert_eq!(c.stats().write_backs, 1);
        c.data_mut(&b)[0] = 0xee;
        c.write_back(&mut q, &b).unwrap();
        assert_eq!(c.stats().write_backs, 2);
        c.release(b);
    }
    assert_eq!(&mem[7 * BS..8 * BS], &pattern(70)[..]);
    assert_eq!(mem[BS], 0xee);
}

#[test]
fn invalidate_keeps_dirty_and_pinned_blocks() {
    let mut mem = vec![0u8; 16 * BS];
    let mut t = Tables::new(4, 2, 4);
    let mut q = RequestQueue::new(RamDisk::new(&mut mem, BS as u32).unwrap(), t.storage()).unwrap();
    let mut s = Store::new(4);
    let mut c = s.cache();
    for b in [0, 1] {
        c.get(&mut q, b).map(|x| c.release(x)).unwrap();
    }
    let dirty = c.get(&mut q, 2).unwrap();
    c.data_mut(&dirty).fill(0x22);
    c.release(dirty);
    let pinned = c.get(&mut q, 3).unwrap();
    // The disk changes behind the cache's back.
    q.device_mut().contents_mut()[..BS].fill(0x11);
    assert_eq!(c.invalidate(), 2);
    assert!(!c.is_cached(0) && !c.is_cached(1) && c.is_cached(2) && c.is_cached(3));
    assert_eq!((c.dirty_count(), c.pinned_count()), (1, 1));
    let misses = c.stats().misses;
    let b = c.get(&mut q, 0).unwrap();
    assert_eq!(c.data(&b), &[0x11; BS][..]);
    assert_eq!(c.stats().misses, misses + 1);
    c.release(b);
    // Freed slots are reused before the blocks that are still cached.
    c.get(&mut q, 5).map(|x| c.release(x)).unwrap();
    assert!(c.is_cached(0) && c.is_cached(2) && c.is_cached(3));
    assert_eq!(c.dirty_count(), 1);
    c.release(pinned);
    c.sync(&mut q).unwrap();
    assert_eq!(c.invalidate(), 4);
    assert_eq!(&q.device().contents()[2 * BS..3 * BS], &[0x22; BS][..]);
}

#[test]
fn zeroed_buffers_are_not_read() {
    let mut t = Tables::new(4, 2, 4);
    let mut disk = SimDisk::new(16, BS as u32, false, 1);
    disk.durable.fill(0xab);
    let mut q = RequestQueue::new(disk, t.storage()).unwrap();
    let mut s = Store::new(4);
    let mut c = s.cache();
    let b = c.get_zeroed(&mut q, 3).unwrap();
    assert!(c.data(&b).iter().all(|&x| x == 0));
    c.release(b);
    assert!(q.device().log.is_empty());
    // A cached block is zeroed too.
    let b = c.get(&mut q, 4).unwrap();
    assert_eq!(c.data(&b)[0], 0xab);
    c.release(b);
    let b = c.get_zeroed(&mut q, 4).unwrap();
    assert_eq!(c.data(&b)[0], 0);
    c.release(b);
    c.sync(&mut q).unwrap();
    assert!(q.device().block(3).iter().all(|&x| x == 0));
    assert!(q.device().block(4).iter().all(|&x| x == 0));
    assert_eq!(c.get_zeroed(&mut q, 16), Err(CacheError::Io(IoError::OutOfRange)));
    q.device_mut().info.read_only = true;
    let mut t = Tables::new(4, 2, 4);
    let mut q = RequestQueue::new(q.into_device(), t.storage()).unwrap();
    assert_eq!(c.get_zeroed(&mut q, 5), Err(CacheError::Io(IoError::ReadOnly)));
}

#[test]
fn sync_batches_writes_and_flushes() {
    let mut t = Tables::new(8, 4, 8);
    let mut disk = SimDisk::new(64, BS as u32, true, 2);
    disk.max_pending = 2;
    let mut q = RequestQueue::new(disk, t.storage()).unwrap();
    let mut s = Store::new(16);
    let mut c = s.cache();
    // Blocks 10-14 land in slots 0-4, next to each other in memory too.
    for b in 10..15 {
        let x = c.get_zeroed(&mut q, b).unwrap();
        c.data_mut(&x).copy_from_slice(&pattern(b));
        c.release(x);
    }
    let pinned = c.get_zeroed(&mut q, 40).unwrap();
    c.data_mut(&pinned).copy_from_slice(&pattern(40));
    // More dirty blocks than the queue has request slots.
    for b in 20..30 {
        let x = c.get_zeroed(&mut q, b).unwrap();
        c.data_mut(&x).copy_from_slice(&pattern(b));
        c.release(x);
    }
    c.sync(&mut q).unwrap();
    assert_eq!(c.dirty_count(), 0);
    let d = q.device();
    assert!(d.volatile.is_empty(), "flushed");
    for b in (10..15).chain(20..30).chain([40]) {
        assert_eq!(&d.durable[b as usize * BS..(b as usize + 1) * BS], &pattern(b)[..]);
    }
    let writes = requests(&d.log, Op::Write);
    assert!(writes.contains(&(10, 5)), "{writes:?}");
    assert_eq!(writes.iter().map(|w| w.1).sum::<u32>(), 16);
    assert_eq!(requests(&d.log, Op::Flush).len(), 1);
    c.release(pinned);
    // Nothing dirty: only the flush goes out.
    let n = q.device().log.len();
    c.sync(&mut q).unwrap();
    assert_eq!(requests(&q.device().log[n..], Op::Write), []);
}

#[test]
fn io_errors() {
    let mut t = Tables::new(4, 2, 4);
    let mut disk = SimDisk::new(16, BS as u32, true, 3);
    disk.bad = vec![5];
    let mut q = RequestQueue::new(disk, t.storage()).unwrap();
    let mut s = Store::new(2);
    let mut c = s.cache();
    // A failed read caches nothing and frees its slot.
    assert_eq!(c.get(&mut q, 5), Err(CacheError::Io(IoError::Media)));
    assert!(!c.is_cached(5));
    assert_eq!(c.get(&mut q, 16), Err(CacheError::Io(IoError::OutOfRange)));
    // A failed write-back keeps the block dirty and cached, and the
    // operation that wanted its slot fails.
    let b = c.get_zeroed(&mut q, 5).unwrap();
    c.release(b);
    c.get(&mut q, 0).map(|x| c.release(x)).unwrap();
    assert_eq!(c.get(&mut q, 1), Err(CacheError::Io(IoError::Media)));
    assert!(c.is_cached(5) && c.dirty_count() == 1);
    assert_eq!(c.sync(&mut q), Err(CacheError::Io(IoError::Media)));
    assert_eq!(requests(&q.device().log, Op::Flush), []);
    // Once the block is freed, everything works again.
    assert!(c.discard(5));
    assert_eq!(c.dirty_count(), 0);
    c.sync(&mut q).unwrap();
    c.get(&mut q, 1).map(|x| c.release(x)).unwrap();
    // Pinned blocks cannot be discarded.
    let x = c.get(&mut q, 2).unwrap();
    assert!(!c.discard(2));
    c.release(x);
    assert!(c.discard(2) && !c.is_cached(2));
    assert!(c.discard(9));
}

#[test]
fn read_ahead() {
    let mut t = Tables::new(8, 2, 8);
    let mut disk = SimDisk::new(32, BS as u32, false, 4);
    for b in 0..32u64 {
        disk.durable[b as usize * BS..(b as usize + 1) * BS].copy_from_slice(&pattern(b));
    }
    let mut q = RequestQueue::new(disk, t.storage()).unwrap();
    let mut s = Store::new(6);
    let mut c = s.cache();
    let dirty = c.get_zeroed(&mut q, 0).unwrap();
    c.release(dirty);
    let held = c.get(&mut q, 9).unwrap();
    // Four free slots; the dirty block is never evicted for read-ahead.
    assert_eq!(c.prefetch(&mut q, 10, 8), 4);
    assert_eq!(c.dirty_count(), 1);
    assert_eq!(requests(&q.device().log, Op::Read), [(9, 1), (10, 4)]);
    for b in 10..14 {
        let x = c.get(&mut q, b).unwrap();
        assert_eq!(c.data(&x), &pattern(b)[..]);
        c.release(x);
    }
    assert_eq!(c.stats().read_ahead, 4);
    assert_eq!(c.stats().hits, 4);
    // Cached blocks are skipped; the end of the disk stops it.
    assert_eq!(c.prefetch(&mut q, 12, 100), 4);
    assert!((14..18).all(|b| c.is_cached(b)));
    assert_eq!(c.prefetch(&mut q, 31, 5), 1);
    c.release(held);
}

#[test]
#[should_panic(expected = "does not belong")]
fn buffers_from_another_cache_are_caught() {
    let mut mem = vec![0u8; 4 * BS];
    let mut t = Tables::new(4, 2, 4);
    let mut q = RequestQueue::new(RamDisk::new(&mut mem, BS as u32).unwrap(), t.storage()).unwrap();
    let (mut s1, mut s2) = (Store::new(2), Store::new(2));
    let (mut c1, mut c2) = (s1.cache(), s2.cache());
    let b = c1.get(&mut q, 1).unwrap();
    c2.release(b);
}

/// Random use of a small cache over a disk with a volatile write cache, with
/// power failures. A read must always return the latest write; after a
/// crash each block must hold either what the last sync made durable or
/// something written after it, never anything else.
#[test]
fn randomized_with_crashes() {
    const BLOCKS: u64 = 40;
    for seed in 1..=25u64 {
        let mut rng = Rng(seed.wrapping_mul(0x2545_f491));
        let mut disk = SimDisk::new(BLOCKS, BS as u32, true, seed);
        disk.max_pending = 1 + rng.below(4) as usize;
        disk.info.max_blocks = 1 + rng.below(6) as u32;
        let mut t = Tables::new(3 + rng.below(6) as usize, 1 + rng.below(3) as usize, 4);
        let mut q = RequestQueue::new(disk, t.storage()).unwrap();
        let mut store = Store::new(4 + rng.below(6) as usize);
        let mut latest: Vec<Vec<u8>> = vec![vec![0; BS]; BLOCKS as usize];
        let mut durable = latest.clone();
        // Everything written to each block since the last sync.
        let mut since: Vec<Vec<Vec<u8>>> = vec![Vec::new(); BLOCKS as usize];
        let mut crashes = 0;
        let mut cache = Some(store.cache());
        let mut held: Vec<Buf> = Vec::new();
        for step in 0..1500u64 {
            let c = cache.as_mut().unwrap();
            let b = rng.below(BLOCKS);
            match rng.below(100) {
                0..40 => {
                    let x = c.get(&mut q, b).unwrap();
                    assert_eq!(c.data(&x), &latest[b as usize][..], "seed {seed} step {step} block {b}");
                    if held.len() < 3 && rng.one_in(3) {
                        held.push(x);
                    } else {
                        c.release(x);
                    }
                }
                40..70 => {
                    let x = if rng.one_in(2) { c.get(&mut q, b) } else { c.get_zeroed(&mut q, b) }.unwrap();
                    let data = pattern(seed << 32 | step);
                    c.data_mut(&x).copy_from_slice(&data);
                    latest[b as usize] = data.clone();
                    since[b as usize].push(data);
                    c.release(x);
                }
                70..80 => {
                    if !held.is_empty() {
                        let i = rng.below(held.len() as u64) as usize;
                        c.release(held.swap_remove(i));
                    }
                }
                80..86 => {
                    c.sync(&mut q).unwrap();
                    assert_eq!(c.dirty_count(), 0);
                    durable.clone_from(&latest);
                    since.iter_mut().for_each(Vec::clear);
                }
                86..90 => {
                    if let Some(x) = held.first() {
                        c.write_back(&mut q, x).unwrap();
                    }
                }
                90..95 => {
                    c.prefetch(&mut q, b, rng.below(5));
                }
                _ => {
                    // Power failure: the cache's memory and the disk's write
                    // cache are gone.
                    held.clear();
                    let _ = cache.take();
                    assert!(q.is_idle());
                    q.device_mut().crash();
                    for blk in 0..BLOCKS as usize {
                        let now = q.device().block(blk as u64);
                        assert!(
                            now == durable[blk] || since[blk].contains(&now),
                            "seed {seed} step {step}: block {blk} holds something never written"
                        );
                        latest[blk] = now;
                    }
                    durable.clone_from(&latest);
                    since.iter_mut().for_each(Vec::clear);
                    cache = Some(store.cache());
                    crashes += 1;
                }
            }
        }
        let c = cache.as_mut().unwrap();
        for x in held.drain(..) {
            c.release(x);
        }
        c.sync(&mut q).unwrap();
        assert_eq!(c.pinned_count(), 0);
        assert!(crashes > 0);
        let d = q.device();
        assert!(d.volatile.is_empty());
        for b in 0..BLOCKS {
            assert_eq!(d.block(b), latest[b as usize], "seed {seed} block {b}");
        }
    }
}

/// An [`IoQueue`] that waits the way a kernel with disk interrupts does:
/// enqueueing never runs the queue, and waiting starts it once, then
/// "sleeps" until an "interrupt" (the device finishing what it holds, and
/// the handler running the queue) has finished something.
struct Sleeping<'q, 'a> {
    q: &'q mut RequestQueue<'a, SimDisk>,
    sleeps: usize,
    interrupts: usize,
}

impl IoQueue for Sleeping<'_, '_> {
    fn info(&self) -> DeviceInfo {
        *self.q.info()
    }

    unsafe fn enqueue(&mut self, op: Op, lba: u64, buf: IoBuf) -> Result<Ticket, SubmitError> {
        // SAFETY: our caller's promise, passed on.
        unsafe { self.q.enqueue(op, lba, buf) }
    }

    fn wait(&mut self, t: Ticket) -> Result<(), IoError> {
        self.q.process();
        loop {
            if let Some(r) = self.q.take(t) {
                return r;
            }
            self.sleeps += 1;
            let finished = self.q.stats().finished;
            while self.q.stats().finished == finished {
                // The device finishes what it has and interrupts; the
                // handler runs the queue (which may start more).
                self.interrupts += 1;
                self.q.device_mut().hold = false;
                self.q.process();
                self.q.device_mut().hold = true;
            }
        }
    }
}

#[test]
fn a_cache_that_sleeps_through_a_wait_hook() {
    let mut t = Tables::new(16, 2, 8);
    let mut disk = SimDisk::new(64, BS as u32, true, 5);
    for b in 0..64u64 {
        disk.durable[b as usize * BS..(b as usize + 1) * BS].copy_from_slice(&pattern(b));
    }
    disk.hold = true;
    let mut q = RequestQueue::new(disk, t.storage()).unwrap();
    // Another user's read sits in the same queue, not waited for.
    let mut other = vec![0u8; BS];
    // SAFETY: `other` outlives the request, collected below.
    let theirs = unsafe { q.enqueue(Op::Read, 50, IoBuf::from_mut(&mut other)) }.unwrap();
    let mut s = Store::new(16);
    let mut c = s.cache();
    let mut io = Sleeping { q: &mut q, sleeps: 0, interrupts: 0 };

    // A miss sleeps once, until its read is back.
    let x = c.get(&mut io, 3).unwrap();
    assert_eq!(c.data(&x), &pattern(3)[..]);
    c.release(x);
    assert_eq!((io.sleeps, io.interrupts), (1, 1));
    // A hit does not wait at all.
    let x = c.get(&mut io, 3).unwrap();
    c.release(x);
    assert_eq!(io.sleeps, 1);
    // Read-ahead enqueues everything before the first wait, so the queue
    // merges it into one device request.
    assert_eq!(c.prefetch(&mut io, 10, 8), 8);
    assert!(requests(&io.q.device().log, Op::Read).contains(&(10, 8)));
    // Dirty blocks go out together on sync, then the flush.
    for b in 20..24 {
        let x = c.get_zeroed(&mut io, b).unwrap();
        c.data_mut(&x).copy_from_slice(&pattern(b + 100));
        c.release(x);
    }
    let before = io.sleeps;
    c.sync(&mut io).unwrap();
    assert!(io.sleeps > before);
    assert!(requests(&io.q.device().log, Op::Write).contains(&(20, 4)));
    let d = io.q.device();
    assert!(d.volatile.is_empty(), "flushed");
    for b in 20..24 {
        assert_eq!(&d.durable[b as usize * BS..(b as usize + 1) * BS], &pattern(b + 100)[..]);
    }
    // The helpers the trait provides wait the same way.
    let mut buf = vec![0u8; 2 * BS];
    io.read(30, &mut buf).unwrap();
    assert_eq!(&buf[BS..], &pattern(31)[..]);
    io.write(40, &buf).unwrap();
    io.flush().unwrap();
    assert_eq!(&io.q.device().durable[41 * BS..42 * BS], &pattern(31)[..]);
    assert_eq!(io.read(63, &mut buf), Err(IoError::OutOfRange));
    // The other user's request finished on the way, and its result waited
    // for its owner.
    assert_eq!(q.take(theirs), Some(Ok(())));
    assert_eq!(other, pattern(50));
    assert_eq!(q.stats().finished, q.stats().requests);
}

#[test]
fn read_blocks_reads_each_uncached_run_in_one_request() {
    let mut t = Tables::new(8, 2, 8);
    let mut disk = SimDisk::new(400, BS as u32, false, 5);
    for b in 0..400u64 {
        disk.durable[b as usize * BS..(b as usize + 1) * BS].copy_from_slice(&pattern(b));
    }
    let mut q = RequestQueue::new(disk, t.storage()).unwrap();
    let mut s = Store::new(KEEP_RUN_BLOCKS * 2);
    let mut c = s.cache();
    // Block 105 is cached (and dirty: the device must not overwrite it).
    let x = c.get_zeroed(&mut q, 105).unwrap();
    c.data_mut(&x).copy_from_slice(&pattern(1000));
    c.release(x);
    let mut buf = vec![0u8; 10 * BS];
    c.read_blocks(&mut q, 100, &mut buf).unwrap();
    for k in 0..10u64 {
        let want = if k == 5 { pattern(1000) } else { pattern(100 + k) };
        assert_eq!(&buf[k as usize * BS..(k as usize + 1) * BS], &want[..], "block {}", 100 + k);
    }
    // One request for the run before the cached block, one for the run after.
    assert_eq!(requests(&q.device().log, Op::Read), [(100, 5), (106, 4)]);
    assert_eq!(c.stats().hits, 1);
    // A short run was kept: reading it again asks the device for nothing.
    let n = q.device().log.len();
    c.read_blocks(&mut q, 100, &mut buf).unwrap();
    assert_eq!(requests(&q.device().log[n..], Op::Read), []);
    assert!(c.is_cached(103) && c.is_cached(109));
}

#[test]
fn a_long_read_leaves_the_cache_alone() {
    let mut t = Tables::new(8, 2, 8);
    let mut disk = SimDisk::new(400, BS as u32, false, 6);
    for b in 0..400u64 {
        disk.durable[b as usize * BS..(b as usize + 1) * BS].copy_from_slice(&pattern(b));
    }
    let mut q = RequestQueue::new(disk, t.storage()).unwrap();
    let mut s = Store::new(KEEP_RUN_BLOCKS * 2);
    let mut c = s.cache();
    for b in 0..4 {
        c.get(&mut q, b).map(|x| c.release(x)).unwrap();
    }
    let mut buf = vec![0u8; (KEEP_RUN_BLOCKS + 1) * BS];
    c.read_blocks(&mut q, 200, &mut buf).unwrap();
    for k in 0..=KEEP_RUN_BLOCKS as u64 {
        assert_eq!(&buf[k as usize * BS..(k as usize + 1) * BS], &pattern(200 + k)[..]);
    }
    assert_eq!(requests(&q.device().log, Op::Read).last(), Some(&(200, KEEP_RUN_BLOCKS as u32 + 1)));
    // The long run is not cached; what was cached before still is.
    assert!(!c.is_cached(200) && (0..4).all(|b| c.is_cached(b)));
    // Errors.
    assert_eq!(c.read_blocks(&mut q, 399, &mut buf), Err(CacheError::Io(IoError::OutOfRange)));
    assert_eq!(c.read_blocks(&mut q, 0, &mut buf[..BS - 1]), Err(CacheError::Io(IoError::BadRequest)));
}

#[test]
fn staged_sync_writes_runs_of_neighbours_in_single_requests() {
    let mut t = Tables::new(8, 2, 8);
    let mut q = RequestQueue::new(SimDisk::new(64, BS as u32, true, 7), t.storage()).unwrap();
    let mut s = Store::new(32);
    let mut stage = vec![0u8; 8 * BS];
    let mut c = s.cache();
    c.set_staging(&mut stage);
    // 10-24 in a scrambled order (so slots are not in block order), 40-41, 50.
    let blocks: Vec<u64> = (10..25).rev().chain([40, 41, 50]).collect();
    for &b in &blocks {
        let x = c.get_zeroed(&mut q, b).unwrap();
        c.data_mut(&x).copy_from_slice(&pattern(b));
        c.release(x);
    }
    c.sync(&mut q).unwrap();
    assert_eq!(c.dirty_count(), 0);
    let d = q.device();
    for &b in &blocks {
        assert_eq!(&d.durable[b as usize * BS..(b as usize + 1) * BS], &pattern(b)[..], "block {b}");
    }
    let writes = requests(&d.log, Op::Write);
    // No run is longer than the staging memory (8 blocks), nothing is
    // written twice, and 18 blocks go out in a handful of requests.
    assert!(writes.iter().all(|w| w.1 <= 8), "{writes:?}");
    assert_eq!(writes.iter().map(|w| w.1).sum::<u32>(), 18);
    assert!(writes.len() <= 6, "{writes:?}");
    assert_eq!(requests(&d.log, Op::Flush).len(), 1);
}

#[test]
fn a_dirty_victim_goes_out_with_its_neighbours() {
    let mut t = Tables::new(8, 2, 8);
    let mut q = RequestQueue::new(SimDisk::new(64, BS as u32, false, 8), t.storage()).unwrap();
    let mut s = Store::new(6);
    let mut stage = vec![0u8; 4 * BS];
    let mut c = s.cache();
    c.set_staging(&mut stage);
    for b in 0..6 {
        let x = c.get_zeroed(&mut q, b).unwrap();
        c.data_mut(&x).copy_from_slice(&pattern(b));
        c.release(x);
    }
    // Block 0 is the victim: blocks 0-3 (what the staging memory holds) go out together.
    c.get(&mut q, 30).map(|x| c.release(x)).unwrap();
    assert_eq!(requests(&q.device().log, Op::Write), [(0, 4)]);
    assert_eq!(c.dirty_count(), 2);
    let d = q.device();
    for b in 0..4u64 {
        assert_eq!(&d.durable[b as usize * BS..(b as usize + 1) * BS], &pattern(b)[..]);
    }
}

#[test]
fn a_cache_is_a_32nd_of_the_disk_within_the_memory_and_the_limits() {
    const KIB: u64 = 1024;
    const MIB: u64 = 1024 * KIB;
    let size = |disk, ram| cache_size(disk, ram, 64 * KIB as usize, 16 * MIB as usize) as u64;
    // A test disk and a RAM disk get the least.
    assert_eq!(size(MIB, 4096 * MIB), 64 * KIB);
    assert_eq!(size(256 * KIB, 4096 * MIB), 64 * KIB);
    // In between: a 32nd of the disk.
    assert_eq!(size(256 * MIB, 4096 * MIB), 8 * MIB);
    // The most, however big the disk.
    assert_eq!(size(1228 * MIB, 4096 * MIB), 16 * MIB);
    assert_eq!(size(u64::MAX, u64::MAX), 16 * MIB);
    // Short of memory: a 16th of what is free, but never below the least.
    assert_eq!(size(1228 * MIB, 64 * MIB), 4 * MIB);
    assert_eq!(size(1228 * MIB, MIB), 64 * KIB);
    assert_eq!(size(0, 0), 64 * KIB);
    // Whole pages.
    assert_eq!(cache_size(100_000_000, u64::MAX, 4096, usize::MAX) % 4096, 0);
}

#[test]
fn randomized_reads_and_staged_writes_against_a_model() {
    const BLOCKS: u64 = 300;
    for seed in 1..=15u64 {
        let mut rng = Rng(seed.wrapping_mul(0x9e37_79b9) | 1);
        let mut disk = SimDisk::new(BLOCKS, BS as u32, true, seed);
        disk.max_pending = 1 + rng.below(4) as usize;
        disk.info.max_blocks = 1 + rng.below(40) as u32;
        let mut latest: Vec<Vec<u8>> = (0..BLOCKS).map(pattern).collect();
        for (b, data) in latest.iter().enumerate() {
            disk.durable[b * BS..(b + 1) * BS].copy_from_slice(data);
        }
        let mut t = Tables::new(8, 3, 8);
        let mut q = RequestQueue::new(disk, t.storage()).unwrap();
        let mut store = Store::new(6 + rng.below(100) as usize);
        let mut stage = vec![0u8; (rng.below(10) as usize) * BS];
        let mut c = store.cache();
        c.set_staging(&mut stage);
        for step in 0..600u64 {
            match rng.below(100) {
                0..45 => {
                    let n = 1 + rng.below(90);
                    let first = rng.below(BLOCKS - n + 1);
                    let mut buf = vec![0xeeu8; n as usize * BS];
                    c.read_blocks(&mut q, first, &mut buf).unwrap();
                    for k in 0..n {
                        let got = &buf[k as usize * BS..(k as usize + 1) * BS];
                        assert_eq!(
                            got,
                            &latest[(first + k) as usize][..],
                            "seed {seed} step {step} block {}",
                            first + k
                        );
                    }
                }
                45..85 => {
                    let b = rng.below(BLOCKS);
                    let x = if rng.one_in(2) { c.get(&mut q, b) } else { c.get_zeroed(&mut q, b) }.unwrap();
                    let data = pattern(seed << 32 | step);
                    c.data_mut(&x).copy_from_slice(&data);
                    latest[b as usize] = data;
                    c.release(x);
                }
                85..95 => {
                    let b = rng.below(BLOCKS);
                    let x = c.get(&mut q, b).unwrap();
                    assert_eq!(c.data(&x), &latest[b as usize][..], "seed {seed} step {step} get {b}");
                    c.release(x);
                }
                _ => {
                    c.sync(&mut q).unwrap();
                    assert_eq!(c.dirty_count(), 0);
                    for (b, want) in latest.iter().enumerate() {
                        assert_eq!(
                            &q.device().durable[b * BS..(b + 1) * BS],
                            &want[..],
                            "seed {seed} step {step} durable {b}"
                        );
                    }
                }
            }
        }
    }
}
