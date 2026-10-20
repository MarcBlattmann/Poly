extern crate std;

use super::*;
use core::alloc::GlobalAlloc;
use std::collections::BTreeMap;
use std::vec::Vec;

/// Page source backed by `std::alloc`. It checks that every free matches an
/// earlier allocation exactly, fills new pages with junk so the heap cannot
/// rely on zeroed memory, and can be limited or made to misalign.
#[derive(Debug)]
struct TestPages {
    /// Returned address → (page count, real base, std layout).
    live: BTreeMap<usize, (usize, *mut u8, std::alloc::Layout)>,
    held: usize,
    limit: Option<usize>,
    /// Ignore alignments above a page and return a run that is off by one.
    misalign: bool,
    alloc_calls: usize,
    free_calls: usize,
}

impl TestPages {
    const fn new() -> TestPages {
        TestPages { live: BTreeMap::new(), held: 0, limit: None, misalign: false, alloc_calls: 0, free_calls: 0 }
    }

    fn limited(pages: usize) -> TestPages {
        let mut p = TestPages::new();
        p.limit = Some(pages);
        p
    }

    fn misaligned() -> TestPages {
        let mut p = TestPages::new();
        p.misalign = true;
        p
    }
}

// SAFETY: the pointers are runs this source owns; nothing ties them to the
// thread that allocated them.
unsafe impl Send for TestPages {}

// SAFETY: runs come from `std::alloc` with room for `count` pages and are
// tracked until freed.
unsafe impl PageSource for TestPages {
    fn alloc_pages(&mut self, count: usize, align: usize) -> Option<NonNull<u8>> {
        assert!(count >= 1);
        assert!(align.is_power_of_two() && align >= PAGE_SIZE);
        self.alloc_calls += 1;
        if self.limit.is_some_and(|l| self.held + count > l) {
            return None;
        }
        let skew = if self.misalign && align > PAGE_SIZE { PAGE_SIZE } else { 0 };
        let layout = std::alloc::Layout::from_size_align(count * PAGE_SIZE + skew, align).ok()?;
        // SAFETY: the layout has a non-zero size.
        let base = unsafe { std::alloc::alloc(layout) };
        if base.is_null() {
            return None;
        }
        // SAFETY: the block is `count * PAGE_SIZE + skew` bytes.
        let run = unsafe {
            base.write_bytes(0xa5, layout.size());
            base.add(skew)
        };
        self.live.insert(run.addr(), (count, base, layout));
        self.held += count;
        NonNull::new(run)
    }

    unsafe fn free_pages(&mut self, ptr: NonNull<u8>, count: usize) {
        self.free_calls += 1;
        let (pages, base, layout) = self.live.remove(&ptr.addr().get()).expect("freeing a run we never handed out");
        assert_eq!(pages, count, "run freed with the wrong page count");
        self.held -= count;
        // SAFETY: `base` and `layout` are exactly what `alloc_pages` got from
        // std; poisoning first makes use-after-free visible.
        unsafe {
            base.write_bytes(0xdd, layout.size());
            std::alloc::dealloc(base, layout);
        }
    }
}

impl Drop for TestPages {
    fn drop(&mut self) {
        for &(_, base, layout) in self.live.values() {
            // SAFETY: still-live runs from `alloc_pages`.
            unsafe { std::alloc::dealloc(base, layout) };
        }
    }
}

fn layout(size: usize, align: usize) -> Layout {
    Layout::from_size_align(size, align).unwrap()
}

fn class_of(size: usize, align: usize) -> usize {
    match Kind::of(layout(size, align)) {
        Kind::Small(c) => c,
        k => panic!("{size}/{align} is not small: {k:?}"),
    }
}

/// xorshift64*: deterministic and dependency-free.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

/// The byte the allocation tagged `tag` must hold at offset `i`.
fn pattern(tag: u64, i: usize) -> u8 {
    (tag.wrapping_mul(0x9e37_79b9_7f4a_7c15) >> 56) as u8 ^ (i as u8).wrapping_mul(31)
}

fn fill(p: *mut u8, size: usize, tag: u64) {
    for i in 0..size {
        // SAFETY: the allocation is `size` bytes.
        unsafe { p.add(i).write(pattern(tag, i)) };
    }
}

fn check(p: *mut u8, size: usize, tag: u64) {
    for i in 0..size {
        // SAFETY: the allocation is `size` bytes.
        let b = unsafe { p.add(i).read() };
        assert_eq!(b, pattern(tag, i), "allocation {tag} at {p:p} overwritten at byte {i}");
    }
}

#[test]
fn class_table_is_consistent() {
    assert_eq!(MAX_SLAB_BYTES, 32 * 1024);
    for (i, c) in CLASSES.iter().enumerate() {
        assert_eq!(c.size, CLASS_SIZES[i]);
        assert_eq!(c.size % GRANULE, 0);
        assert!(i == 0 || CLASS_SIZES[i - 1] < c.size, "classes must be sorted");
        assert!(c.slab_bytes.is_power_of_two() && c.slab_bytes >= PAGE_SIZE);
        assert!(c.slab_bytes <= MAX_SLAB_BYTES);
        assert!(c.first >= size_of::<Slab>() && c.first % c.size == 0);
        assert!(c.first + c.objects * c.size <= c.slab_bytes);
        assert!(c.objects >= MIN_SLOTS - 2, "class {} has only {} slots", c.size, c.objects);
        assert!(c.size % c.align == 0 && (c.size / c.align) % 2 == 1);
    }
}

#[test]
fn class_choice_is_the_smallest_fit() {
    for align in (0..=12).map(|s| 1usize << s) {
        for size in 1..=MAX_SMALL + 1 {
            let want = CLASSES.iter().position(|c| c.size >= size && c.align >= align);
            assert_eq!(class_for(size, align), want, "size {size} align {align}");
        }
    }
    assert_eq!(CLASS_SIZES[class_of(1, 1)], 16);
    assert_eq!(CLASS_SIZES[class_of(40, 8)], 48);
    assert_eq!(CLASS_SIZES[class_of(40, 32)], 64);
    assert_eq!(CLASS_SIZES[class_of(8, 512)], 512);
    assert_eq!(CLASS_SIZES[class_of(1100, 8)], 1536);
    assert_eq!(Kind::of(layout(16, 4096)), Kind::Large { pages: 1, align: 4096 });
    assert_eq!(Kind::of(layout(2049, 8)), Kind::Large { pages: 1, align: 4096 });
    assert_eq!(Kind::of(layout(8193, 16384)), Kind::Large { pages: 3, align: 16384 });
}

#[test]
fn every_class_allocates_and_frees() {
    for (i, class) in CLASSES.iter().enumerate() {
        let mut heap = Heap::new(TestPages::new());
        let l = layout(class.size, 1);
        let n = class.objects * 2 + 1;
        let ptrs: Vec<*mut u8> = (0..n).map(|_| heap.alloc(l)).collect();

        let mut sorted: Vec<usize> = ptrs.iter().map(|p| p.addr()).collect();
        sorted.sort_unstable();
        for w in sorted.windows(2) {
            assert!(w[1] - w[0] >= class.size, "class {} objects overlap", class.size);
        }
        for (t, &p) in ptrs.iter().enumerate() {
            assert!(!p.is_null());
            assert_eq!(p.addr() % class.align, 0);
            fill(p, class.size, t as u64);
        }
        for (t, &p) in ptrs.iter().enumerate() {
            check(p, class.size, t as u64);
        }

        let s = heap.stats();
        assert_eq!(s.classes[i].slabs, 3);
        assert_eq!(s.classes[i].objects_in_use, n);
        assert_eq!(s.classes[i].objects_free, 3 * class.objects - n);
        assert_eq!(s.pages_held, 3 * class.slab_bytes / PAGE_SIZE);
        assert_eq!(heap.source().held, s.pages_held);

        for &p in &ptrs {
            // SAFETY: allocated above with `l`.
            unsafe { heap.dealloc(p, l) };
        }
        let s = heap.stats();
        assert_eq!(s.classes[i].slabs, 1, "one empty slab stays cached");
        assert_eq!(s.classes[i].objects_in_use, 0);
        assert_eq!(s.bytes_in_use, 0);
        assert_eq!(heap.trim(), class.slab_bytes / PAGE_SIZE);
        assert_eq!(heap.stats().pages_held, 0);
        assert!(heap.source().live.is_empty());
    }
}

#[test]
fn alignment_holds_for_many_layouts() {
    let sizes = [1, 3, 8, 15, 16, 17, 24, 33, 47, 48, 63, 100, 255, 256, 300, 513, 1000, 1500, 2047, 2048, 2049];
    let sizes = sizes.into_iter().chain([4095, 4096, 4097, 10_000, 70_000]);
    let mut heap = Heap::new(TestPages::new());
    let mut live = Vec::new();
    for size in sizes {
        for align in (0..=16).map(|s| 1usize << s) {
            let l = layout(size, align);
            let p = heap.alloc(l);
            assert!(!p.is_null(), "size {size} align {align}");
            assert_eq!(p.addr() % align, 0, "size {size} align {align}");
            fill(p, size, live.len() as u64);
            live.push((p, l));
        }
    }
    for (t, &(p, l)) in live.iter().enumerate() {
        check(p, l.size(), t as u64);
        // SAFETY: allocated above with `l`.
        unsafe { heap.dealloc(p, l) };
    }
    heap.trim();
    assert_eq!(heap.stats().pages_held, 0);
    assert!(heap.source().live.is_empty());
}

#[test]
fn large_allocations_are_page_runs() {
    let mut heap = Heap::new(TestPages::new());
    let a = layout(PAGE_SIZE + 1, 8);
    let b = layout(3 * PAGE_SIZE, 16 * PAGE_SIZE);
    let pa = heap.alloc(a);
    let pb = heap.alloc(b);
    assert_eq!(pa.addr() % PAGE_SIZE, 0);
    assert_eq!(pb.addr() % (16 * PAGE_SIZE), 0);
    let s = heap.stats();
    assert_eq!((s.large_allocations, s.large_pages, s.pages_held), (2, 5, 5));
    assert_eq!(s.bytes_in_use, a.size() + b.size());
    assert!(s.classes.iter().all(|c| c.slabs == 0));

    // SAFETY: allocated above with `a`; the test source checks the count.
    unsafe { heap.dealloc(pa, a) };
    assert_eq!(heap.source().held, 3);
    // SAFETY: allocated above with `b`.
    unsafe { heap.dealloc(pb, b) };
    let s = heap.stats();
    assert_eq!((s.large_allocations, s.large_pages, s.pages_held, s.bytes_in_use), (0, 0, 0, 0));
    assert_eq!(heap.source().free_calls, 2);
}

#[test]
fn empty_slabs_go_back_except_one() {
    let mut heap = Heap::new(TestPages::new());
    let l = layout(64, 8);
    let per_slab = CLASSES[class_of(64, 8)].objects;
    let ptrs: Vec<_> = (0..per_slab * 3).map(|_| heap.alloc(l)).collect();
    assert_eq!(heap.source().held, 3);

    // Free the middle slab's worth: it empties and becomes the cached slab.
    for &p in &ptrs[per_slab..2 * per_slab] {
        // SAFETY: allocated above with `l`.
        unsafe { heap.dealloc(p, l) };
    }
    assert_eq!(heap.source().held, 3);
    // Emptying a second slab hands it back, since one is already cached.
    for &p in &ptrs[..per_slab] {
        // SAFETY: allocated above with `l`.
        unsafe { heap.dealloc(p, l) };
    }
    assert_eq!(heap.source().held, 2);
    assert_eq!(heap.source().free_calls, 1);
    for &p in &ptrs[2 * per_slab..] {
        // SAFETY: allocated above with `l`.
        unsafe { heap.dealloc(p, l) };
    }
    assert_eq!(heap.source().held, 1);
    assert_eq!(heap.trim(), 1);
    assert_eq!(heap.source().held, 0);
    assert_eq!(heap.trim(), 0);
}

#[test]
fn cached_slab_absorbs_churn() {
    let mut heap = Heap::new(TestPages::new());
    let l = layout(100, 4);
    for _ in 0..1000 {
        let p = heap.alloc(l);
        assert!(!p.is_null());
        // SAFETY: allocated just above with `l`.
        unsafe { heap.dealloc(p, l) };
    }
    assert_eq!(heap.source().alloc_calls, 1);
    assert_eq!(heap.source().free_calls, 0);
}

#[test]
fn freed_slots_are_reused_first() {
    let mut heap = Heap::new(TestPages::new());
    let l = layout(32, 8);
    let a = heap.alloc(l);
    let b = heap.alloc(l);
    // SAFETY: allocated above with `l`.
    unsafe { heap.dealloc(a, l) };
    assert_eq!(heap.alloc(l), a);
    assert_ne!(heap.alloc(l), b);
    assert_eq!(heap.source().alloc_calls, 1);
}

#[test]
fn stats_track_classes_and_bytes() {
    let mut heap = Heap::new(TestPages::new());
    let small = [layout(10, 1), layout(10, 1), layout(200, 8), layout(1500, 8)];
    let ptrs: Vec<_> = small.iter().map(|&l| heap.alloc(l)).collect();
    let s = heap.stats();
    assert_eq!(s.bytes_in_use, 10 + 10 + 200 + 1500);
    for (i, c) in s.classes.iter().enumerate() {
        assert_eq!(c.object_size, CLASS_SIZES[i]);
        let want = match c.object_size {
            16 => 2,
            256 | 1536 => 1,
            _ => 0,
        };
        assert_eq!(c.objects_in_use, want, "class {}", c.object_size);
        assert_eq!(c.slabs, want.min(1));
        assert_eq!(c.objects_free + c.objects_in_use, c.slabs * CLASSES[i].objects);
    }
    assert_eq!(s.pages_held, 1 + 1 + 8);
    assert_eq!(s.large_allocations, 0);
    for (&p, &l) in ptrs.iter().zip(&small) {
        // SAFETY: allocated above with `l`.
        unsafe { heap.dealloc(p, l) };
    }
    assert_eq!(heap.stats().bytes_in_use, 0);
    assert_eq!(heap.stats().failed_allocations, 0);
}

#[test]
fn zero_size_uses_no_memory() {
    let mut heap = Heap::new(TestPages::new());
    for align in [1, 8, 4096, 1 << 20] {
        let l = layout(0, align);
        let p = heap.alloc(l);
        assert!(!p.is_null());
        assert_eq!(p.addr() % align, 0);
        // SAFETY: allocated just above with `l`.
        unsafe { heap.dealloc(p, l) };
    }
    assert_eq!(heap.source().alloc_calls, 0);
    assert_eq!(heap.stats(), Heap::new(TestPages::new()).stats());
}

#[test]
fn out_of_memory_returns_null() {
    let mut heap = Heap::new(TestPages::limited(0));
    assert!(heap.alloc(layout(8, 8)).is_null());
    assert!(heap.alloc(layout(1 << 20, 8)).is_null());
    assert_eq!(heap.stats().failed_allocations, 2);
    assert_eq!(heap.stats().bytes_in_use, 0);

    // Fill four pages of 16-byte objects, then run dry.
    let mut heap = Heap::new(TestPages::limited(4));
    let l = layout(16, 16);
    let mut ptrs = Vec::new();
    loop {
        let p = heap.alloc(l);
        if p.is_null() {
            break;
        }
        ptrs.push(p);
    }
    assert_eq!(ptrs.len(), 4 * CLASSES[0].objects);
    assert!(heap.alloc(layout(PAGE_SIZE, 8)).is_null());
    // SAFETY: allocated above with `l`.
    unsafe { heap.dealloc(ptrs.pop().unwrap(), l) };
    assert!(!heap.alloc(l).is_null(), "a freed slot is usable again");
    for p in ptrs {
        // SAFETY: allocated above with `l`.
        unsafe { heap.dealloc(p, l) };
    }
    heap.trim();
    assert!(!heap.alloc(layout(3 * PAGE_SIZE, 8)).is_null(), "freed pages are usable again");
}

#[test]
fn misaligned_runs_are_refused() {
    let mut heap = Heap::new(TestPages::misaligned());
    // Page-aligned requests are unaffected.
    let p = heap.alloc(layout(64, 8));
    assert!(!p.is_null());
    assert!(!heap.alloc(layout(PAGE_SIZE, PAGE_SIZE)).is_null());
    // A 512-byte class needs an 8 KiB-aligned slab, a large one its alignment.
    assert!(heap.alloc(layout(512, 8)).is_null());
    assert!(heap.alloc(layout(PAGE_SIZE, 4 * PAGE_SIZE)).is_null());
    let s = heap.stats();
    assert_eq!(s.failed_allocations, 2);
    assert_eq!(s.pages_held, 2);
    assert_eq!(heap.source().held, 2, "misaligned runs went back to the source");
}

#[test]
fn realloc_keeps_contents() {
    let mut heap = Heap::new(TestPages::new());
    let l = layout(20, 8);
    let p = heap.alloc(l);
    fill(p, 20, 7);
    // SAFETY: `p` was allocated with `l`; every later call passes the layout
    // the previous one produced.
    unsafe {
        // 20 → 30 stays in the 32-byte class.
        let q = heap.realloc(p, l, 30);
        assert_eq!(q, p);
        assert_eq!(heap.stats().bytes_in_use, 30);
        // 30 → 1000 moves to the 1024-byte class.
        let r = heap.realloc(q, layout(30, 8), 1000);
        assert_ne!(r, q);
        check(r, 20, 7);
        // Into a large allocation and back down.
        let s = heap.realloc(r, layout(1000, 8), 3 * PAGE_SIZE);
        check(s, 20, 7);
        let t = heap.realloc(s, layout(3 * PAGE_SIZE, 8), 2 * PAGE_SIZE + 1);
        assert_eq!(t, s, "same page count stays in place");
        let u = heap.realloc(t, layout(2 * PAGE_SIZE + 1, 8), 20);
        check(u, 20, 7);
        assert_eq!(heap.stats().large_allocations, 0);
        heap.dealloc(u, layout(20, 8));
    }
    heap.trim();
    assert_eq!(heap.stats().pages_held, 0);
    assert_eq!(heap.stats().bytes_in_use, 0);
}

#[test]
fn realloc_failure_keeps_the_old_block() {
    let mut heap = Heap::new(TestPages::limited(1));
    let l = layout(64, 8);
    let p = heap.alloc(l);
    fill(p, 64, 3);
    // SAFETY: allocated above with `l`.
    let q = unsafe { heap.realloc(p, l, 2 * PAGE_SIZE) };
    assert!(q.is_null());
    check(p, 64, 3);
    assert_eq!(heap.stats().bytes_in_use, 64);
}

/// Thousands of random allocations and frees. Every live block carries a
/// pattern derived from its tag, so any overlap or heap metadata written
/// into a live block shows up; the address ranges are checked for overlap
/// directly as well.
#[test]
fn stress_random_layouts() {
    let mut rng = Rng(0x5eed_1234_abcd_ef01);
    let mut heap = Heap::new(TestPages::new());
    let mut live: Vec<(*mut u8, Layout, u64)> = Vec::new();
    let mut ranges: BTreeMap<usize, usize> = BTreeMap::new();
    let mut expected_bytes = 0;

    for step in 0..30_000u64 {
        let grow = live.len() < 64 || (live.len() < 1500 && rng.below(100) < 55);
        if grow {
            let size = match rng.below(100) {
                0..=59 => 1 + rng.below(128) as usize,
                60..=89 => 1 + rng.below(MAX_SMALL as u64) as usize,
                90..=97 => 1 + rng.below(4 * PAGE_SIZE as u64) as usize,
                _ => 0,
            };
            let align = 1usize << [0, 0, 1, 2, 3, 3, 3, 4, 4, 5, 6, 8, 10, 12, 13][rng.below(15) as usize];
            let l = layout(size, align);
            let p = heap.alloc(l);
            assert!(!p.is_null(), "step {step}: {l:?}");
            assert_eq!(p.addr() % align, 0, "step {step}: {l:?}");
            if size > 0 {
                let start = p.addr();
                if let Some((&s, &e)) = ranges.range(..start + size).next_back() {
                    assert!(e <= start || s >= start + size, "step {step}: {start:#x}+{size} overlaps {s:#x}..{e:#x}");
                }
                ranges.insert(start, start + size);
            }
            fill(p, size, step);
            expected_bytes += size;
            live.push((p, l, step));
        } else {
            let (p, l, tag) = live.swap_remove(rng.below(live.len() as u64) as usize);
            check(p, l.size(), tag);
            if l.size() > 0 {
                ranges.remove(&p.addr());
            }
            // SAFETY: a live allocation made with `l`.
            unsafe { heap.dealloc(p, l) };
            expected_bytes -= l.size();
        }
        assert_eq!(heap.stats().bytes_in_use, expected_bytes);
        if step % 5000 == 0 {
            for &(p, l, tag) in &live {
                check(p, l.size(), tag);
            }
        }
    }

    let s = heap.stats();
    assert_eq!(heap.source().held, s.pages_held);
    for (p, l, tag) in live.drain(..) {
        check(p, l.size(), tag);
        // SAFETY: a live allocation made with `l`.
        unsafe { heap.dealloc(p, l) };
    }
    let s = heap.stats();
    assert_eq!(s.bytes_in_use, 0);
    assert_eq!(s.large_allocations, 0);
    assert!(s.classes.iter().all(|c| c.objects_in_use == 0 && c.slabs <= 1));
    heap.trim();
    assert_eq!(heap.stats().pages_held, 0);
    assert!(heap.source().live.is_empty(), "every page went back to the source");
    assert_eq!(heap.source().alloc_calls, heap.source().free_calls);
}

#[test]
fn locked_heap_serves_threads() {
    static HEAP: LockedHeap<TestPages> = LockedHeap::new(TestPages::new());

    let threads: Vec<_> = (0..4u64)
        .map(|t| {
            std::thread::spawn(move || {
                let mut rng = Rng(0x1234_5678 + t);
                let mut live = Vec::new();
                for step in 0..3000u64 {
                    if live.len() < 50 && rng.below(2) == 0 {
                        let l = layout(1 + rng.below(3000) as usize, 8);
                        // SAFETY: non-zero size.
                        let p = unsafe { HEAP.alloc(l) };
                        assert!(!p.is_null());
                        let tag = t << 32 | step;
                        fill(p, l.size(), tag);
                        live.push((p, l, tag));
                    } else if let Some((p, l, tag)) = live.pop() {
                        check(p, l.size(), tag);
                        // SAFETY: allocated by this thread with `l`.
                        unsafe { HEAP.dealloc(p, l) };
                    }
                }
                for (p, l, tag) in live {
                    check(p, l.size(), tag);
                    // SAFETY: allocated by this thread with `l`.
                    unsafe { HEAP.dealloc(p, l) };
                }
            })
        })
        .collect();
    for t in threads {
        t.join().unwrap();
    }

    let mut heap = HEAP.lock();
    assert_eq!(heap.stats().bytes_in_use, 0);
    heap.trim();
    assert_eq!(heap.stats().pages_held, 0);
    assert!(heap.source().live.is_empty());
}
