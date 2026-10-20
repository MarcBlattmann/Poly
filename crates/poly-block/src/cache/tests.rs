use super::*;
use crate::RamDisk;
use crate::sim::{Event, Rng, SimDisk, Tables};
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
