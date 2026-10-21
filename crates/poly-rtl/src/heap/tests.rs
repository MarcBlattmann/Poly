extern crate std;

use super::*;
use std::alloc::{Layout, alloc_zeroed, dealloc};
use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::vec::Vec;

/// A test event: signalled flag and its condition variable.
type Event = Arc<(Mutex<bool>, Condvar)>;

/// Virtual memory from the host allocator (64 KiB aligned, zeroed), every
/// allocation tracked; events as in the critical-section tests.
#[derive(Default)]
struct Host {
    live: Mutex<BTreeMap<usize, usize>>,
    allocations: AtomicUsize,
    fail_after: Mutex<Option<usize>>,
    events: Mutex<HashMap<u64, Event>>,
    next: AtomicU64,
}

std::thread_local!(static TID: u64 = {
    static NEXT: AtomicU64 = AtomicU64::new(8);
    NEXT.fetch_add(4, Ordering::Relaxed)
});

impl Waiter for Host {
    fn current_thread(&self) -> u64 {
        TID.with(|t| *t)
    }
    fn create_event(&self) -> u64 {
        let h = self.next.fetch_add(4, Ordering::Relaxed) + 4;
        self.events.lock().unwrap().insert(h, Arc::new((Mutex::new(false), Condvar::new())));
        h
    }
    fn wait(&self, e: u64) {
        let ev = self.events.lock().unwrap()[&e].clone();
        let mut s = ev.0.lock().unwrap();
        while !*s {
            s = ev.1.wait(s).unwrap();
        }
        *s = false;
    }
    fn set(&self, e: u64) {
        let ev = self.events.lock().unwrap()[&e].clone();
        *ev.0.lock().unwrap() = true;
        ev.1.notify_one();
    }
    fn close(&self, e: u64) {
        self.events.lock().unwrap().remove(&e);
    }
}

impl Platform for Host {
    fn allocate(&self, size: usize, _execute: bool) -> *mut u8 {
        assert!(size.is_multiple_of(GRANULE), "whole granules only");
        {
            let mut f = self.fail_after.lock().unwrap();
            if let Some(n) = f.as_mut() {
                if *n == 0 {
                    return null_mut();
                }
                *n -= 1;
            }
        }
        // SAFETY: a non-zero size with a valid alignment.
        let p = unsafe { alloc_zeroed(Layout::from_size_align(size, GRANULE).unwrap()) };
        assert!(!p.is_null());
        self.live.lock().unwrap().insert(p as usize, size);
        self.allocations.fetch_add(1, Ordering::Relaxed);
        p
    }
    fn release(&self, base: *mut u8, size: usize) {
        let had = self.live.lock().unwrap().remove(&(base as usize));
        assert_eq!(had, Some(size), "released what was allocated, with its size");
        // SAFETY: allocated above with this layout.
        unsafe { dealloc(base, Layout::from_size_align(size, GRANULE).unwrap()) };
    }
}

impl Host {
    fn live(&self) -> usize {
        self.live.lock().unwrap().len()
    }
}

fn growable(h: &Host) -> Heap {
    // SAFETY: platform memory.
    unsafe { Heap::create(h, flags::GROWABLE, None, 0) }.expect("a heap")
}

/// A small deterministic generator (xorshift64*).
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

#[test]
fn bins_are_monotonic() {
    let mut last = 0;
    for units in MIN_UNITS..100_000 {
        let b = bin_of(units);
        assert!(b >= last && b < NBINS, "{units}");
        last = b;
    }
    assert_eq!(bin_of(2), 0);
    assert_eq!(bin_of(65), 63);
    assert_eq!(bin_of(66), 64);
    assert_eq!(bin_of(u32::MAX), NBINS - 1);
}

#[test]
fn alloc_size_free() {
    let h = Host::default();
    let heap = growable(&h);
    // SAFETY: a valid heap; pointers from it.
    unsafe {
        assert!(heap.is_valid());
        let a = heap.allocate(&h, flags::ZERO_MEMORY, 100).unwrap();
        assert_eq!(a as usize % 16, 0);
        assert!(core::slice::from_raw_parts(a, 100).iter().all(|&b| b == 0));
        assert_eq!(heap.size(&h, 0, a), Ok(100));
        let z = heap.allocate(&h, 0, 0).unwrap();
        let z2 = heap.allocate(&h, 0, 0).unwrap();
        assert_ne!(z, z2, "zero-byte blocks are unique");
        assert_eq!(heap.size(&h, 0, z), Ok(0));
        assert!(heap.validate(&h, 0, ptr::null()));
        assert!(heap.validate(&h, 0, a));
        assert!(heap.free(&h, 0, a));
        assert!(!heap.free(&h, 0, a), "a second free fails");
        assert!(!heap.validate(&h, 0, a));
        assert_eq!(heap.size(&h, 0, a), Err(HeapError::InvalidPointer));
        assert!(heap.free(&h, 0, ptr::null_mut()));
        assert!(heap.free(&h, 0, z) && heap.free(&h, 0, z2));
        assert!(heap.validate(&h, 0, ptr::null()));
        heap.destroy(&h);
    }
    assert_eq!(h.live(), 0, "everything given back");
}

#[test]
fn foreign_and_interior_pointers_are_refused() {
    let h = Host::default();
    let heap = growable(&h);
    let other = growable(&h);
    // SAFETY: valid heaps.
    unsafe {
        let a = heap.allocate(&h, 0, 64).unwrap();
        let b = other.allocate(&h, 0, 64).unwrap();
        assert!(!heap.free(&h, 0, b), "another heap's block");
        assert!(!heap.free(&h, 0, a.add(16)), "inside a block");
        assert!(!heap.free(&h, 0, a.add(1)), "misaligned");
        let mut local = [0u8; 64];
        assert!(!heap.free(&h, 0, local.as_mut_ptr().add(16)));
        assert!(!heap.free(&h, 0, 0x10 as *mut u8));
        assert!(heap.validate(&h, 0, ptr::null()));
        assert!(heap.free(&h, 0, a) && other.free(&h, 0, b));
        heap.destroy(&h);
        other.destroy(&h);
    }
    assert_eq!(h.live(), 0);
}

#[test]
fn reallocation_in_place_and_moving() {
    let h = Host::default();
    let heap = growable(&h);
    // SAFETY: a valid heap.
    unsafe {
        let a = heap.allocate(&h, 0, 40).unwrap();
        for i in 0..40 {
            *a.add(i) = i as u8;
        }
        // Shrinking stays in place.
        let s = heap.reallocate(&h, 0, a, 20).unwrap();
        assert_eq!(s, a);
        assert_eq!(heap.size(&h, 0, s), Ok(20));
        // Growing into the free space after it stays in place.
        let g = heap.reallocate(&h, flags::ZERO_MEMORY, s, 300).unwrap();
        assert_eq!(g, a);
        assert_eq!(heap.size(&h, 0, g), Ok(300));
        assert!((0..20).all(|i| *g.add(i) == i as u8));
        assert!((20..300).all(|i| *g.add(i) == 0), "the new bytes are zeroed");
        // A busy neighbour forces a move, which IN_PLACE_ONLY refuses.
        let n = heap.allocate(&h, 0, 16).unwrap();
        assert_eq!(heap.reallocate(&h, flags::REALLOC_IN_PLACE_ONLY, g, 100_000), Err(HeapError::WouldMove));
        assert_eq!(heap.size(&h, 0, g), Ok(300), "unchanged");
        let m = heap.reallocate(&h, flags::ZERO_MEMORY, g, 100_000).unwrap();
        assert_ne!(m, g);
        assert!((0..20).all(|i| *m.add(i) == i as u8));
        assert!((300..100_000).all(|i| *m.add(i) == 0));
        assert!(!heap.validate(&h, 0, g), "the old block is free");
        assert_eq!(heap.reallocate(&h, 0, g, 10), Err(HeapError::InvalidPointer));
        assert!(heap.validate(&h, 0, ptr::null()));
        assert!(heap.free(&h, 0, m) && heap.free(&h, 0, n));
        heap.destroy(&h);
    }
    assert_eq!(h.live(), 0);
}

#[test]
fn big_blocks_get_their_own_memory() {
    let h = Host::default();
    let heap = growable(&h);
    let before = h.live();
    // SAFETY: a valid heap.
    unsafe {
        let big = heap.allocate(&h, flags::ZERO_MEMORY, 3 << 20).unwrap();
        assert_eq!(h.live(), before + 1);
        assert_eq!(heap.size(&h, 0, big), Ok(3 << 20));
        *big.add((3 << 20) - 1) = 7;
        let r = heap.reallocate(&h, 0, big, (3 << 20) + 100).unwrap();
        assert_eq!(r, big, "room in the last granule: in place");
        let r2 = heap.reallocate(&h, 0, r, 8 << 20).unwrap();
        assert_eq!(*r2.add((3 << 20) - 1), 7);
        assert_eq!(h.live(), before + 1, "the old big block went back");
        assert!(heap.free(&h, 0, r2));
        assert_eq!(h.live(), before);
        let keep = heap.allocate(&h, 0, 1 << 20).unwrap();
        heap.destroy(&h);
        let _ = keep;
    }
    assert_eq!(h.live(), 0, "destroy frees big blocks too");
}

#[test]
fn fixed_heaps_do_not_grow() {
    let h = Host::default();
    // SAFETY: platform memory.
    let heap = unsafe { Heap::create(&h, 0, None, 0x1_0000) }.unwrap();
    let mut got = Vec::new();
    // SAFETY: a valid heap.
    unsafe {
        while let Ok(p) = heap.allocate(&h, 0, 1000) {
            got.push(p);
        }
        assert!(got.len() >= 50 && got.len() <= 65, "{}", got.len());
        assert_eq!(heap.allocate(&h, 0, 1 << 20), Err(HeapError::NoMemory));
        assert!(raises(heap.flags(), flags::GENERATE_EXCEPTIONS));
        assert!(!raises(heap.flags(), 0));
        assert_eq!(h.live(), 1);
        for p in got {
            assert!(heap.free(&h, 0, p));
        }
        assert!(heap.validate(&h, 0, ptr::null()));
        // Everything coalesced back: a large block fits again.
        let p = heap.allocate(&h, 0, 60_000).unwrap();
        assert!(heap.free(&h, 0, p));
        heap.destroy(&h);
    }
    assert_eq!(h.live(), 0);
}

#[test]
fn growth_adds_segments_and_gives_them_back() {
    let h = Host::default();
    // SAFETY: platform memory.
    let heap = unsafe { Heap::create(&h, flags::GROWABLE, None, 0x1_0000) }.unwrap();
    let mut got = Vec::new();
    // SAFETY: a valid heap.
    unsafe {
        for _ in 0..3000 {
            got.push(heap.allocate(&h, 0, 2000).unwrap());
        }
        assert!(h.live() > 2, "segments were added");
        assert!(heap.validate(&h, 0, ptr::null()));
        for p in got {
            assert!(heap.free(&h, 0, p));
        }
        assert_eq!(h.live(), 1, "empty extra segments are released");
        assert!(heap.validate(&h, 0, ptr::null()));
        heap.destroy(&h);
    }
}

#[test]
fn out_of_memory_is_reported() {
    let h = Host::default();
    let heap = growable(&h);
    *h.fail_after.lock().unwrap() = Some(0);
    // SAFETY: a valid heap.
    unsafe {
        assert_eq!(heap.allocate(&h, 0, 10 << 20), Err(HeapError::NoMemory), "no big block");
        let small = heap.allocate(&h, 0, 16).unwrap();
        // The first 1 MiB segment holds two of these; a third needs a new
        // segment, which the platform refuses.
        let first = heap.allocate(&h, 0, 500_000).unwrap();
        let second = heap.allocate(&h, 0, 500_000).unwrap();
        assert_eq!(heap.allocate(&h, 0, 500_000), Err(HeapError::NoMemory), "no new segment");
        assert!(heap.free(&h, 0, first) && heap.free(&h, 0, second) && heap.free(&h, 0, small));
        assert!(heap.validate(&h, 0, ptr::null()));
        *h.fail_after.lock().unwrap() = None;
        heap.destroy(&h);
    }
    *h.fail_after.lock().unwrap() = Some(0);
    // SAFETY: creating with no memory fails cleanly.
    assert!(unsafe { Heap::create(&h, flags::GROWABLE, None, 0) }.is_none());
}

#[test]
fn caller_memory_is_used_and_kept() {
    let h = Host::default();
    let layout = Layout::from_size_align(0x8000, 16).unwrap();
    // SAFETY: a valid layout.
    let mem = unsafe { alloc_zeroed(layout) };
    // SAFETY: `mem` is 0x8000 writable bytes for the heap's life.
    unsafe {
        let heap = Heap::create(&h, flags::GROWABLE, Some(mem), 0x8000).unwrap();
        assert_eq!(heap.0, mem);
        assert_eq!(heap.flags() & flags::GROWABLE, 0, "caller memory never grows");
        let p = heap.allocate(&h, 0, 1000).unwrap();
        assert!(p as usize > mem as usize && (p as usize) < mem as usize + 0x8000);
        assert_eq!(heap.allocate(&h, 0, 0x8000), Err(HeapError::NoMemory));
        heap.destroy(&h);
        assert_eq!(h.live(), 0);
        dealloc(mem, layout);
        assert!(Heap::create(&h, 0, Some(mem.add(1)), 0x1000).is_none(), "misaligned");
    }
}

#[test]
fn disable_coalesce_still_reuses_blocks() {
    let h = Host::default();
    // SAFETY: platform memory.
    let heap = unsafe { Heap::create(&h, flags::GROWABLE | flags::DISABLE_COALESCE_ON_FREE, None, 0) }.unwrap();
    // SAFETY: a valid heap.
    unsafe {
        let a = heap.allocate(&h, 0, 32).unwrap();
        let b = heap.allocate(&h, 0, 32).unwrap();
        assert!(heap.free(&h, 0, a) && heap.free(&h, 0, b));
        let c = heap.allocate(&h, 0, 32).unwrap();
        assert!(c == a || c == b);
        assert!(heap.validate(&h, 0, ptr::null()));
        heap.destroy(&h);
    }
}

/// Random allocations, frees and reallocations against a model: contents
/// survive, blocks never overlap, the heap stays consistent.
#[test]
fn random_operations_against_a_model() {
    for seed in [1u64, 7, 99, 12345] {
        let h = Host::default();
        let heap = growable(&h);
        let mut rng = Rng(seed);
        let mut live: Vec<(*mut u8, usize, u8)> = Vec::new();
        for step in 0..4000 {
            // SAFETY: a valid heap and its own pointers.
            unsafe {
                match rng.below(10) {
                    0..=4 => {
                        let size = match rng.below(20) {
                            0 => rng.below(700_000) as usize,
                            1..=4 => rng.below(8000) as usize,
                            _ => rng.below(300) as usize,
                        };
                        let fill = (step % 251) as u8;
                        let p = heap.allocate(&h, 0, size).unwrap();
                        ptr::write_bytes(p, fill, size);
                        live.push((p, size, fill));
                    }
                    5..=7 if !live.is_empty() => {
                        let i = rng.below(live.len() as u64) as usize;
                        let (p, size, fill) = live.swap_remove(i);
                        assert!(core::slice::from_raw_parts(p, size).iter().all(|&b| b == fill), "contents survive");
                        assert!(heap.free(&h, 0, p));
                    }
                    8..=9 if !live.is_empty() => {
                        let i = rng.below(live.len() as u64) as usize;
                        let (p, size, fill) = live[i];
                        let new = rng.below(5000) as usize;
                        let q = heap.reallocate(&h, 0, p, new).unwrap();
                        assert!(core::slice::from_raw_parts(q, size.min(new)).iter().all(|&b| b == fill));
                        ptr::write_bytes(q, fill, new);
                        live[i] = (q, new, fill);
                    }
                    _ => {}
                }
                if step % 97 == 0 {
                    assert!(heap.validate(&h, 0, ptr::null()), "seed {seed} step {step}");
                    let mut spans: Vec<(usize, usize)> =
                        live.iter().map(|&(p, s, _)| (p as usize, p as usize + s.max(1))).collect();
                    spans.sort_unstable();
                    assert!(spans.windows(2).all(|w| w[0].1 <= w[1].0), "blocks overlap");
                    for &(p, s, _) in &live {
                        assert_eq!(heap.size(&h, 0, p), Ok(s));
                    }
                }
            }
        }
        // SAFETY: as above.
        unsafe {
            for (p, _, _) in live {
                assert!(heap.free(&h, 0, p));
            }
            assert!(heap.validate(&h, 0, ptr::null()));
            heap.destroy(&h);
        }
        assert_eq!(h.live(), 0);
    }
}

#[test]
fn threads_share_a_serialized_heap() {
    let h = Arc::new(Host::default());
    let heap = growable(&h);
    let addr = heap.0 as usize;
    let threads: Vec<_> = (0..4)
        .map(|t| {
            let h = h.clone();
            std::thread::spawn(move || {
                let heap = Heap(addr as *mut u8);
                let mut mine = Vec::new();
                for i in 0..3000usize {
                    // SAFETY: a valid heap shared under its lock.
                    unsafe {
                        let p = heap.allocate(&*h, 0, 8 + (i * 37 + t) % 900).unwrap();
                        *p = t as u8;
                        mine.push(p);
                        if i % 3 == 0 {
                            let q = mine.swap_remove((i * 7) % mine.len());
                            assert_eq!(*q, t as u8);
                            assert!(heap.free(&*h, 0, q));
                        }
                    }
                }
                for p in mine {
                    // SAFETY: as above.
                    unsafe { assert!(heap.free(&*h, 0, p)) };
                }
            })
        })
        .collect();
    for t in threads {
        t.join().unwrap();
    }
    // SAFETY: no other user is left.
    unsafe {
        assert!(heap.validate(&*h, 0, ptr::null()));
        heap.destroy(&*h);
    }
    assert_eq!(h.live(), 0);
}

#[test]
fn no_serialize_and_explicit_locking() {
    let h = Host::default();
    // SAFETY: platform memory.
    let heap = unsafe { Heap::create(&h, flags::GROWABLE | flags::NO_SERIALIZE, None, 0) }.unwrap();
    // SAFETY: a valid heap used by one thread.
    unsafe {
        heap.lock(&h);
        let p = heap.allocate(&h, 0, 10).unwrap();
        heap.unlock(&h);
        assert!(heap.free(&h, flags::NO_SERIALIZE, p));
        heap.destroy(&h);
    }
    // SAFETY: a null handle is never read.
    assert!(!unsafe { Heap(ptr::null_mut()).is_valid() });
}
