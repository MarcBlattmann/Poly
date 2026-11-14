extern crate std;

use super::*;
use std::vec;

const P: u64 = PAGE_SIZE;
/// `min_addr` of the small test layout.
const MIN: u64 = 0x10_0000;
/// `max_addr` of the small test layout: 1024 pages above `MIN`.
const MAX: u64 = MIN + 1024 * P;

type Map = VmMap<u32>;

/// A 4 MiB address space: stack at the top (up to 64 pages), a 4-page
/// guard gap, top-down search from 256 pages below the top.
fn small() -> Layout {
    Layout {
        min_addr: MIN,
        max_addr: MAX,
        stack_top: MAX,
        stack_max: 64 * P,
        stack_guard_gap: 4 * P,
        mmap_base: MAX - 256 * P,
        search: Search::TopDown,
        max_map_count: 1000,
        heap_max: u64::MAX,
    }
}

fn map_with(layout: Layout) -> Map {
    VmMap::new(layout).unwrap()
}

fn anon(len: u64, prot: Prot, at: Placement) -> MapRequest<u32> {
    MapRequest::anonymous(len, prot).at(at)
}

fn file(len: u64, prot: Prot, f: u32, offset: u64, shared: bool, at: Placement) -> MapRequest<u32> {
    MapRequest::file(len, prot, f, offset, shared).at(at)
}

fn fixed(m: &mut Map, addr: u64, pages: u64, prot: Prot) -> Vec<Vma<u32>> {
    let r = m.map(anon(pages * P, prot, Placement::Fixed(addr))).unwrap();
    assert_eq!(r.addr, addr);
    r.replaced
}

/// The regions as `(start, end, prot)`.
fn spans(m: &Map) -> Vec<(u64, u64, Prot)> {
    m.regions().map(|v| (v.start, v.end, v.prot)).collect()
}

fn region(start: u64, end: u64, prot: Prot, kind: Kind, backing: Backing<u32>) -> Vma<u32> {
    Vma {
        start,
        end,
        prot,
        max_prot: Prot::ALL,
        kind,
        backing,
        wipe_on_fork: false,
        locked: false,
        sealed: false,
        uffd: 0,
        name: None,
    }
}

fn anon_vma(start: u64, end: u64, prot: Prot) -> Vma<u32> {
    region(start, end, prot, Kind::Normal, Backing::Anonymous)
}

fn file_vma(start: u64, end: u64, prot: Prot, f: u32, offset: u64, shared: bool) -> Vma<u32> {
    region(start, end, prot, Kind::Normal, Backing::File { file: f, offset, shared })
}

// ---------------------------------------------------------------- layout --

#[test]
fn default_layout() {
    let l = Layout::default();
    assert_eq!(l.min_addr, 0x1_0000);
    assert_eq!(l.max_addr, 0x7fff_ffff_f000);
    assert_eq!(l.stack_top, USER_END);
    assert_eq!(l.mmap_base, USER_END - (128 << 20) - (1 << 20));
    assert_eq!(l.stack_floor(), USER_END - (8 << 20));
    assert_eq!(l.search, Search::TopDown);
    assert_eq!(l.max_map_count, 65_530);
    assert!(Map::new(l).is_ok());
    // A stack limit above 128 MiB pushes the mmap area down with it.
    let big = Layout::new(USER_END, 1 << 30);
    assert_eq!(big.mmap_base, USER_END - (1 << 30) - (1 << 20));
    // A tiny space clamps mmap_base to the bottom.
    assert_eq!(Layout::new(USER_MIN_ADDR + P, P).mmap_base, USER_MIN_ADDR);
}

#[test]
fn invalid_layouts_are_rejected() {
    let bad = [
        Layout { min_addr: 0, ..small() },
        Layout { min_addr: MIN + 1, ..small() },
        Layout { max_addr: MIN, ..small() },
        Layout { max_addr: USER_END + P, stack_top: USER_END, ..small() },
        Layout { stack_top: MAX + P, ..small() },
        Layout { stack_top: MIN, ..small() },
        Layout { stack_max: 3, ..small() },
        Layout { stack_guard_gap: 5, ..small() },
        Layout { mmap_base: MIN - P, ..small() },
        Layout { mmap_base: MAX + P, ..small() },
        Layout { max_map_count: 0, ..small() },
    ];
    for l in bad {
        assert_eq!(Map::new(l).err(), Some(Error::Invalid), "{l:?}");
    }
}

#[test]
fn page_helpers() {
    assert_eq!(page_down(0x1fff), 0x1000);
    assert_eq!(page_up(0x1001), Some(0x2000));
    assert_eq!(page_up(0x2000), Some(0x2000));
    assert_eq!(page_up(u64::MAX - 10), None);
    assert!(is_page_aligned(0) && !is_page_aligned(1));
}

#[test]
fn prot_and_codes() {
    assert_eq!(Prot::from_bits(0x7), Some(Prot::ALL));
    assert_eq!(Prot::from_bits(0x8), None);
    assert_eq!(Prot::from_bits(0), Some(Prot::NONE));
    assert_eq!(Prot::READ | Prot::WRITE, Prot::RW);
    assert_eq!(Prot::ALL.without(Prot::WRITE), Prot::RX);
    assert_eq!(std::format!("{:?} {:?} {:?}", Prot::RX, Prot::NONE, Prot::ALL), "r-x --- rwx");
    // Any permission implies read; write and execute need their bit.
    assert!(Prot::WRITE.allows(Access::Read) && Prot::EXEC.allows(Access::Read));
    assert!(!Prot::NONE.allows(Access::Read));
    assert!(!Prot::RX.allows(Access::Write) && !Prot::RW.allows(Access::Execute));
    assert_eq!(write_protected(Prot::WRITE), Prot::READ);
    assert_eq!(write_protected(Prot::ALL), Prot::RX);
    assert_eq!(write_protected(Prot::EXEC), Prot::EXEC);
    let errnos: Vec<u64> =
        [Error::Permission, Error::NoMemory, Error::Access, Error::Exists, Error::Invalid, Error::Overflow]
            .iter()
            .map(|e| e.linux_errno())
            .collect();
    assert_eq!(errnos, [1, 12, 13, 17, 22, 75]);
    assert_eq!((SegvCode::MapErr.si_code(), SegvCode::AccErr.si_code()), (1, 2));
}

// ------------------------------------------------------------------- map --

#[test]
fn search_is_top_down_from_mmap_base_and_merges() {
    let mut m = map_with(small());
    let base = small().mmap_base;
    let a = m.map(anon(2 * P, Prot::RW, Placement::Hint(0))).unwrap();
    assert_eq!(a, Mapped { addr: base - 2 * P, replaced: vec![] });
    // Lengths round up to whole pages.
    let b = m.map(anon(1, Prot::RW, Placement::Hint(0))).unwrap();
    assert_eq!(b.addr, base - 3 * P);
    // Compatible neighbours are one region.
    assert_eq!(spans(&m), [(base - 3 * P, base, Prot::RW)]);
    let c = m.map(anon(P, Prot::READ, Placement::Hint(0))).unwrap();
    assert_eq!(c.addr, base - 4 * P);
    assert_eq!(m.region_count(), 2);
    assert_eq!(m.find(base - 4 * P).unwrap().prot, Prot::READ);
    assert_eq!(m.find(base - 1).unwrap().start, base - 3 * P);
    assert!(m.find(base).is_none());
}

#[test]
fn hints() {
    let mut m = map_with(small());
    // A free hint is used, rounded up to a page.
    assert_eq!(m.map(anon(P, Prot::RW, Placement::Hint(MIN + 5 * P + 1))).unwrap().addr, MIN + 6 * P);
    // An occupied hint falls back to the search.
    assert_eq!(m.map(anon(P, Prot::RW, Placement::Hint(MIN + 6 * P))).unwrap().addr, small().mmap_base - P);
    // A hint whose range leaves the space, and one that overflows when
    // rounded, are ignored.
    assert_eq!(m.map(anon(2 * P, Prot::RW, Placement::Hint(MAX - P))).unwrap().addr, small().mmap_base - 3 * P);
    assert_eq!(m.map(anon(P, Prot::RW, Placement::Hint(u64::MAX))).unwrap().addr, small().mmap_base - 4 * P);
    // A hint inside the stack guard gap is not free.
    m.setup_stack(P, Prot::RW).unwrap();
    assert_eq!(m.map(anon(P, Prot::RW, Placement::Hint(MAX - 3 * P))).unwrap().addr, small().mmap_base - 5 * P);
    assert_eq!(m.map(anon(P, Prot::RW, Placement::Hint(MAX - 6 * P))).unwrap().addr, MAX - 6 * P);

    // A hint below min_addr moves up to it ("always above or equal to the
    // value specified in /proc/sys/vm/mmap_min_addr", mmap(2)).
    let mut low = map_with(small());
    assert_eq!(low.map(anon(P, Prot::RW, Placement::Hint(P))).unwrap().addr, MIN);
    assert_eq!(low.map(anon(P, Prot::RW, Placement::Hint(1))).unwrap().addr, small().mmap_base - P);
}

#[test]
fn length_errors() {
    let mut m = map_with(small());
    assert_eq!(m.map(anon(0, Prot::RW, Placement::Hint(0))), Err(Error::Invalid));
    assert_eq!(m.map(anon(u64::MAX, Prot::RW, Placement::Hint(0))), Err(Error::NoMemory));
    assert_eq!(m.map(anon(u64::MAX - P - 1, Prot::RW, Placement::Hint(0))), Err(Error::NoMemory));
    assert_eq!(m.map(anon(MAX - MIN + 1, Prot::RW, Placement::Hint(0))), Err(Error::NoMemory));
    assert_eq!(m.map(anon(MAX - MIN + 1, Prot::RW, Placement::Fixed(MIN))), Err(Error::NoMemory));
    // The whole space is one mapping.
    assert_eq!(m.map(anon(MAX - MIN, Prot::RW, Placement::Hint(0))).unwrap().addr, MIN);
    assert_eq!(m.map(anon(P, Prot::RW, Placement::Hint(0))), Err(Error::NoMemory));
    assert_eq!(spans(&m), [(MIN, MAX, Prot::RW)]);
}

#[test]
fn argument_errors() {
    let mut m = map_with(small());
    let fx = Placement::Fixed(MIN);
    assert_eq!(m.map(anon(P, Prot::RW, fx).with_max_prot(Prot::READ)), Err(Error::Access));
    assert_eq!(m.map(file(P, Prot::READ, 1, 100, false, fx)), Err(Error::Invalid));
    assert_eq!(m.map(file(2 * P, Prot::READ, 1, u64::MAX - P - P + 1, false, fx)), Err(Error::Overflow));
    assert!(m.map(file(P, Prot::READ, 1, u64::MAX - P - P + 1, false, fx)).is_ok());
    assert_eq!(m.map(anon(P, Prot::RW, Placement::Fixed(MIN + 1))), Err(Error::Invalid));
    assert_eq!(m.map(anon(P, Prot::RW, Placement::FixedNoReplace(MIN + 1))), Err(Error::Invalid));
    assert_eq!(m.map(anon(P, Prot::RW, Placement::Fixed(MIN - P))), Err(Error::Permission));
    assert_eq!(m.map(anon(P, Prot::RW, Placement::Fixed(0))), Err(Error::Permission));
    assert_eq!(m.map(anon(P, Prot::RW, Placement::FixedNoReplace(0))), Err(Error::Permission));
    assert_eq!(m.map(anon(P, Prot::RW, Placement::Fixed(MAX))), Err(Error::NoMemory));
    assert_eq!(m.map(anon(2 * P, Prot::RW, Placement::Fixed(MAX - P))), Err(Error::NoMemory));
    assert_eq!(m.map(anon(2 * P, Prot::RW, Placement::Fixed(page_down(u64::MAX)))), Err(Error::NoMemory));
    // The last page of the space is fine.
    assert_eq!(fixed(&mut m, MAX - P, 1, Prot::RW), []);
    assert_eq!(m.region_count(), 2);
}

#[test]
fn map_fixed_reports_what_it_replaced() {
    let mut m = map_with(small());
    let a = MIN + 10 * P;
    fixed(&mut m, a, 4, Prot::RW); //                         [a, a+4)   anon rw
    m.map(file(4 * P, Prot::READ, 7, 8 * P, false, Placement::Fixed(a + 4 * P))).unwrap(); // [a+4, a+8) file 7 @ 8
    fixed(&mut m, a + 8 * P, 4, Prot::RX); //                 [a+8, a+12) anon rx
    assert_eq!(m.region_count(), 3);

    // Covers the end of the first, all of the second, the start of the third.
    let r = m.map(file(6 * P, Prot::RW, 9, 0, true, Placement::Fixed(a + 3 * P))).unwrap();
    assert_eq!(r.addr, a + 3 * P);
    assert_eq!(
        r.replaced,
        [
            anon_vma(a + 3 * P, a + 4 * P, Prot::RW),
            file_vma(a + 4 * P, a + 8 * P, Prot::READ, 7, 8 * P, false),
            anon_vma(a + 8 * P, a + 9 * P, Prot::RX),
        ]
    );
    let all: Vec<_> = m.regions().cloned().collect();
    assert_eq!(
        all,
        [
            anon_vma(a, a + 3 * P, Prot::RW),
            file_vma(a + 3 * P, a + 9 * P, Prot::RW, 9, 0, true),
            anon_vma(a + 9 * P, a + 12 * P, Prot::RX),
        ]
    );

    // Inside one file region: the replaced part has the right file offset,
    // the pieces left on both sides keep theirs.
    let r = m.map(anon(P, Prot::NONE, Placement::Fixed(a + 5 * P))).unwrap();
    assert_eq!(r.replaced, [file_vma(a + 5 * P, a + 6 * P, Prot::RW, 9, 2 * P, true)]);
    assert_eq!(m.find(a + 6 * P).unwrap().backing, Backing::File { file: 9, offset: 3 * P, shared: true });
    assert_eq!(m.find(a + 4 * P).unwrap().backing, Backing::File { file: 9, offset: 0, shared: true });

    // Putting the same file page back merges the three again.
    let r = m.map(file(P, Prot::RW, 9, 2 * P, true, Placement::Fixed(a + 5 * P))).unwrap();
    assert_eq!(r.replaced, [anon_vma(a + 5 * P, a + 6 * P, Prot::NONE)]);
    assert_eq!(m.find(a + 5 * P).unwrap(), &file_vma(a + 3 * P, a + 9 * P, Prot::RW, 9, 0, true));
    assert_eq!(m.region_count(), 3);

    // Fixed over a hole replaces nothing.
    assert_eq!(fixed(&mut m, MIN + 100 * P, 3, Prot::RW), []);
}

#[test]
fn fixed_noreplace() {
    let mut m = map_with(small());
    fixed(&mut m, MIN + 10 * P, 2, Prot::RW);
    for addr in [MIN + 9 * P, MIN + 10 * P, MIN + 11 * P] {
        assert_eq!(m.map(anon(2 * P, Prot::RW, Placement::FixedNoReplace(addr))), Err(Error::Exists), "{addr:#x}");
    }
    // Adjacent on either side is fine (and merges).
    assert_eq!(m.map(anon(P, Prot::RW, Placement::FixedNoReplace(MIN + 12 * P))).unwrap().addr, MIN + 12 * P);
    assert_eq!(m.map(anon(2 * P, Prot::RW, Placement::FixedNoReplace(MIN + 8 * P))).unwrap().addr, MIN + 8 * P);
    assert_eq!(spans(&m), [(MIN + 8 * P, MIN + 13 * P, Prot::RW)]);
}

#[test]
fn file_regions_merge_only_when_continuous() {
    let mut m = map_with(small());
    let a = MIN + 10 * P;
    let at = |n: u64| Placement::Fixed(a + n * P);
    m.map(file(2 * P, Prot::READ, 1, 0, false, at(0))).unwrap();
    m.map(file(2 * P, Prot::READ, 1, 2 * P, false, at(2))).unwrap(); // continues: merges
    assert_eq!(m.region_count(), 1);
    m.map(file(P, Prot::READ, 1, 5 * P, false, at(4))).unwrap(); // offset jumps
    m.map(file(P, Prot::READ, 2, 6 * P, false, at(5))).unwrap(); // other file
    m.map(file(P, Prot::READ, 2, 7 * P, true, at(6))).unwrap(); // shared vs private
    m.map(anon(P, Prot::READ, at(7))).unwrap(); // anonymous after a file
    m.map(file(P, Prot::READ, 2, 8 * P, true, at(8)).with_max_prot(Prot::READ)).unwrap(); // other ceiling
    assert_eq!(m.region_count(), 6);
}

#[test]
fn replace_files_changes_only_the_handles_asked_for() {
    let mut m = map_with(small());
    let a = MIN + 10 * P;
    let at = |n: u64| Placement::Fixed(a + n * P);
    m.map(file(2 * P, Prot::READ, 1, 0, true, at(0))).unwrap();
    m.map(anon(P, Prot::READ, at(2))).unwrap();
    m.map(file(P, Prot::READ, 2, 0, true, at(3))).unwrap();
    m.protect(a, P, Prot::NONE).unwrap(); // splits file 1 in two
    let before = spans(&m);
    m.replace_files(|f| (*f == 1).then_some(7));
    assert_eq!(spans(&m), before);
    let files: Vec<Option<u32>> = m
        .regions()
        .map(|v| match v.backing {
            Backing::File { file, .. } => Some(file),
            Backing::Anonymous => None,
        })
        .collect();
    assert_eq!(files, vec![Some(7), Some(7), None, Some(2)]);
}

#[test]
fn heap_and_stack_never_merge_with_ordinary_regions() {
    let mut m = map_with(Layout { stack_guard_gap: 0, ..small() });
    m.init_brk(MIN + 10 * P).unwrap();
    assert_eq!(m.brk(MIN + 12 * P).brk, MIN + 12 * P);
    fixed(&mut m, MIN + 12 * P, 1, Prot::RW);
    fixed(&mut m, MIN + 9 * P, 1, Prot::RW);
    let s = m.setup_stack(2 * P, Prot::RW).unwrap();
    fixed(&mut m, s - P, 1, Prot::RW);
    let kinds: Vec<Kind> = m.regions().map(|v| v.kind).collect();
    assert_eq!(kinds, [Kind::Normal, Kind::Heap, Kind::Normal, Kind::Normal, Kind::Stack]);
}

// ---------------------------------------------------------------- search --

#[test]
fn top_down_search_with_fragmentation() {
    let l = small();
    let mut m = map_with(l);
    let base = l.mmap_base;
    // Holes below mmap_base of 1, 3 and 2 pages (from the top down), with
    // everything else between MIN and base mapped.
    let holes = [(base - P, 1), (base - 10 * P, 3), (base - 100 * P, 2)];
    let mut at = MIN;
    for (h, n) in holes.iter().rev() {
        fixed(&mut m, at, (h - at) / P, Prot::READ);
        at = h + n * P;
    }
    assert_eq!(at, base);
    assert_eq!(m.region_count(), 3);
    // Highest hole that fits.
    assert_eq!(m.map(anon(2 * P, Prot::RW, Placement::Hint(0))).unwrap().addr, base - 9 * P);
    assert_eq!(m.map(anon(P, Prot::RW, Placement::Hint(0))).unwrap().addr, base - P);
    assert_eq!(m.map(anon(2 * P, Prot::RW, Placement::Hint(0))).unwrap().addr, base - 100 * P);
    assert_eq!(m.map(anon(P, Prot::RW, Placement::Hint(0))).unwrap().addr, base - 10 * P);
    // Nothing left below base: bottom-up above it.
    assert_eq!(m.map(anon(3 * P, Prot::RW, Placement::Hint(0))).unwrap().addr, base);
    // Fill up to the stack's guard gap: then nothing fits.
    m.setup_stack(8 * P, Prot::RW).unwrap();
    let left = (MAX - 8 * P - 4 * P) - (base + 3 * P);
    assert_eq!(m.map(anon(left + P, Prot::RW, Placement::Hint(0))), Err(Error::NoMemory));
    assert_eq!(m.map(anon(left, Prot::RW, Placement::Hint(0))).unwrap().addr, base + 3 * P);
    assert_eq!(m.map(anon(P, Prot::RW, Placement::Hint(0))), Err(Error::NoMemory));
    // MAP_FIXED may still go into the guard gap.
    assert_eq!(fixed(&mut m, MAX - 9 * P, 1, Prot::RW), []);
}

#[test]
fn top_down_skips_a_region_straddling_mmap_base() {
    let l = small();
    let mut m = map_with(l);
    fixed(&mut m, l.mmap_base - 2 * P, 4, Prot::READ);
    assert_eq!(m.map(anon(P, Prot::RW, Placement::Hint(0))).unwrap().addr, l.mmap_base - 3 * P);
}

#[test]
fn top_down_respects_a_stack_guard_gap_reaching_below_mmap_base() {
    // A stack just above mmap_base: its gap covers the top of the mmap area.
    let l = Layout { stack_top: small().mmap_base + 2 * P, ..small() };
    let mut m = map_with(l);
    m.setup_stack(2 * P, Prot::RW).unwrap();
    assert_eq!(m.map(anon(P, Prot::RW, Placement::Hint(0))).unwrap().addr, l.mmap_base - 5 * P);
}

#[test]
fn bottom_up_search() {
    let l = Layout { search: Search::BottomUp, mmap_base: MIN + 100 * P, ..small() };
    let mut m = map_with(l);
    assert_eq!(m.map(anon(2 * P, Prot::RW, Placement::Hint(0))).unwrap().addr, MIN + 100 * P);
    assert_eq!(m.map(anon(P, Prot::READ, Placement::Hint(0))).unwrap().addr, MIN + 102 * P);
    m.unmap(MIN + 100 * P, P).unwrap();
    assert_eq!(m.map(anon(P, Prot::READ, Placement::Hint(0))).unwrap().addr, MIN + 100 * P);
    assert_eq!(m.map(anon(2 * P, Prot::READ, Placement::Hint(0))).unwrap().addr, MIN + 103 * P);
    // A region straddling the base is skipped.
    m.unmap(MIN + 99 * P, 2 * P).unwrap();
    fixed(&mut m, MIN + 99 * P, 2, Prot::RW);
    assert_eq!(m.map(anon(P, Prot::RW, Placement::Hint(0))).unwrap().addr, MIN + 105 * P);
    // When nothing fits above the base, top-down below it.
    let rest = MAX - (MIN + 106 * P);
    assert_eq!(m.map(anon(rest, Prot::RW, Placement::Hint(0))).unwrap().addr, MIN + 106 * P);
    assert_eq!(m.map(anon(P, Prot::RW, Placement::Hint(0))).unwrap().addr, MIN + 98 * P);
}

// ----------------------------------------------------------------- unmap --

#[test]
fn unmap_splits_spans_and_ignores_holes() {
    let mut m = map_with(small());
    let a = MIN + 10 * P;
    m.map(file(10 * P, Prot::RW, 3, 0, false, Placement::Fixed(a))).unwrap();
    // Middle: one region becomes two, offsets kept.
    let r = m.unmap(a + 4 * P, 2 * P).unwrap();
    assert_eq!(r, [file_vma(a + 4 * P, a + 6 * P, Prot::RW, 3, 4 * P, false)]);
    assert_eq!(spans(&m), [(a, a + 4 * P, Prot::RW), (a + 6 * P, a + 10 * P, Prot::RW)]);
    assert_eq!(m.find(a + 6 * P).unwrap().file_offset(a + 7 * P), Some(7 * P));
    // Across both and the hole between, with an unaligned length.
    let r = m.unmap(a + 3 * P, 4 * P - 10).unwrap();
    assert_eq!(
        r,
        [
            file_vma(a + 3 * P, a + 4 * P, Prot::RW, 3, 3 * P, false),
            file_vma(a + 6 * P, a + 7 * P, Prot::RW, 3, 6 * P, false),
        ]
    );
    // A hole: nothing happens.
    assert_eq!(m.unmap(a + 4 * P, 2 * P).unwrap(), []);
    assert_eq!(m.unmap(0, P).unwrap(), []);
    // Everything.
    assert_eq!(m.unmap(MIN, MAX - MIN).unwrap().len(), 2);
    assert_eq!(m.region_count(), 0);
}

#[test]
fn unmap_errors() {
    let mut m = map_with(small());
    fixed(&mut m, MIN, 4, Prot::RW);
    assert_eq!(m.unmap(MIN + 1, P), Err(Error::Invalid));
    assert_eq!(m.unmap(MIN, 0), Err(Error::Invalid));
    assert_eq!(m.unmap(MIN, u64::MAX), Err(Error::Invalid));
    assert_eq!(m.unmap(MAX - P, 2 * P), Err(Error::Invalid));
    assert_eq!(m.unmap(page_down(u64::MAX), P), Err(Error::Invalid));
    assert_eq!(m.region_count(), 1);
    assert_eq!(m.unmap(MAX - P, P).unwrap(), []);
}

// --------------------------------------------------------------- protect --

#[test]
fn protect_splits_and_merges_back() {
    let mut m = map_with(small());
    let a = MIN + 10 * P;
    fixed(&mut m, a, 6, Prot::RW);
    m.protect(a + 2 * P, 2 * P, Prot::READ).unwrap();
    assert_eq!(
        spans(&m),
        [(a, a + 2 * P, Prot::RW), (a + 2 * P, a + 4 * P, Prot::READ), (a + 4 * P, a + 6 * P, Prot::RW)]
    );
    // Unaligned length rounds up; changing the edge piece merges with the middle.
    m.protect(a + 4 * P, 1, Prot::READ).unwrap();
    assert_eq!(
        spans(&m),
        [(a, a + 2 * P, Prot::RW), (a + 2 * P, a + 5 * P, Prot::READ), (a + 5 * P, a + 6 * P, Prot::RW)]
    );
    m.protect(a, 6 * P, Prot::RW).unwrap();
    assert_eq!(spans(&m), [(a, a + 6 * P, Prot::RW)]);
    // Several regions at once, and the ones beside are untouched.
    fixed(&mut m, a + 6 * P, 2, Prot::RX);
    m.protect(a + 5 * P, 2 * P, Prot::NONE).unwrap();
    assert_eq!(
        spans(&m),
        [(a, a + 5 * P, Prot::RW), (a + 5 * P, a + 7 * P, Prot::NONE), (a + 7 * P, a + 8 * P, Prot::RX)]
    );
    // Zero length does nothing, even unmapped.
    assert_eq!(m.protect(MIN, 0, Prot::ALL), Ok(()));
}

#[test]
fn protect_errors_change_nothing() {
    let mut m = map_with(small());
    let a = MIN + 10 * P;
    fixed(&mut m, a, 2, Prot::RW);
    fixed(&mut m, a + 3 * P, 2, Prot::RW);
    m.map(file(P, Prot::READ, 1, 0, true, Placement::Fixed(a + 5 * P)).with_max_prot(Prot::RX)).unwrap();
    let before = spans(&m);
    assert_eq!(m.protect(a + 1, P, Prot::READ), Err(Error::Invalid));
    // A hole anywhere in the range.
    assert_eq!(m.protect(a, 5 * P, Prot::READ), Err(Error::NoMemory));
    assert_eq!(m.protect(a - P, 2 * P, Prot::READ), Err(Error::NoMemory));
    assert_eq!(m.protect(a + 3 * P, 4 * P, Prot::READ), Err(Error::NoMemory));
    assert_eq!(m.protect(a, u64::MAX, Prot::READ), Err(Error::NoMemory));
    assert_eq!(m.protect(MAX - P, 2 * P, Prot::READ), Err(Error::NoMemory));
    // Beyond the shared read-only file's ceiling; a hole wins over that.
    assert_eq!(m.protect(a + 3 * P, 3 * P, Prot::RW), Err(Error::Access));
    assert_eq!(m.protect(a + 2 * P, 4 * P, Prot::RW), Err(Error::NoMemory));
    assert_eq!(spans(&m), before);
    assert_eq!(m.protect(a + 3 * P, 3 * P, Prot::RX), Ok(()));
}

// ------------------------------------------------------------- map count --

#[test]
fn map_count_limit() {
    let mut m = map_with(Layout { max_map_count: 3, ..small() });
    let a = MIN + 10 * P;
    fixed(&mut m, a, 9, Prot::RW);
    fixed(&mut m, a + 20 * P, 1, Prot::RW);
    // Changing or replacing the middle of a region makes three of it: four.
    assert_eq!(m.protect(a + 3 * P, P, Prot::READ), Err(Error::NoMemory));
    assert_eq!(m.map(anon(P, Prot::READ, Placement::Fixed(a + 3 * P))), Err(Error::NoMemory));
    assert_eq!(spans(&m), [(a, a + 9 * P, Prot::RW), (a + 20 * P, a + 21 * P, Prot::RW)]);
    // Cutting a hole makes two of it: three.
    assert_eq!(m.unmap(a + 3 * P, P).unwrap().len(), 1);
    assert_eq!(m.region_count(), 3);
    // At the limit: no further split, no new region; growing one is fine.
    assert_eq!(m.unmap(a + P, P), Err(Error::NoMemory));
    assert_eq!(m.protect(a, P, Prot::READ), Err(Error::NoMemory));
    assert_eq!(m.map(anon(P, Prot::RW, Placement::Hint(0))), Err(Error::NoMemory));
    assert_eq!(fixed(&mut m, a + 21 * P, 1, Prot::RW), []);
    // Filling the hole joins two regions.
    assert_eq!(fixed(&mut m, a + 3 * P, 1, Prot::RW), []);
    assert_eq!(m.region_count(), 2);
    m.protect(a, P, Prot::READ).unwrap();
    assert_eq!(m.region_count(), 3);
    // Unmapping a whole region always works.
    let mut full = map_with(Layout { max_map_count: 1, ..small() });
    fixed(&mut full, a, 2, Prot::RW);
    assert_eq!(full.unmap(a + P, P).unwrap().len(), 1);
    assert_eq!(full.unmap(a, P).unwrap().len(), 1);
}

// ----------------------------------------------------------------- stack --

#[test]
fn stack_setup() {
    let mut m = map_with(small());
    assert_eq!(m.setup_stack(0, Prot::RW), Err(Error::Invalid));
    assert_eq!(m.setup_stack(P + 1, Prot::RW), Err(Error::Invalid));
    assert_eq!(m.setup_stack(65 * P, Prot::RW), Err(Error::Invalid));
    fixed(&mut m, MAX - P, 1, Prot::READ);
    assert_eq!(m.setup_stack(2 * P, Prot::RW), Err(Error::Exists));
    m.unmap(MAX - P, P).unwrap();
    assert_eq!(m.setup_stack(2 * P, Prot::RW), Ok(MAX - 2 * P));
    assert_eq!(m.regions().next().unwrap(), &region(MAX - 2 * P, MAX, Prot::RW, Kind::Stack, Backing::Anonymous));
    // A stack bigger than the space below it.
    let mut tiny = map_with(Layout { stack_top: MIN + P, stack_max: 4 * P, ..small() });
    assert_eq!(tiny.setup_stack(2 * P, Prot::RW), Err(Error::Invalid));
}

#[test]
fn stack_grows_down_on_faults_within_its_limit() {
    let mut m = map_with(small());
    let bottom = m.setup_stack(4 * P, Prot::RW).unwrap();
    // A fault just below grows the region to the faulting page.
    let page = bottom - 3 * P;
    assert_eq!(m.fault(page + 8, Access::Write, PageState::Absent), FaultAction::ZeroFill { page, prot: Prot::RW });
    assert_eq!(spans(&m), [(page, MAX, Prot::RW)]);
    // Down to the limit, and no further.
    let floor = small().stack_floor();
    assert_eq!(floor, MAX - 64 * P);
    assert_eq!(m.fault(floor, Access::Read, PageState::Absent), FaultAction::ZeroFill { page: floor, prot: Prot::RW });
    assert_eq!(m.fault(floor - 1, Access::Read, PageState::Absent), FaultAction::Segv(SegvCode::MapErr));
    assert_eq!(spans(&m), [(floor, MAX, Prot::RW)]);
}

#[test]
fn stack_growth_keeps_the_guard_gap() {
    let mut m = map_with(small());
    let bottom = m.setup_stack(4 * P, Prot::RW).unwrap();
    let below_end = bottom - 10 * P;
    fixed(&mut m, below_end - P, 1, Prot::RW); // the gap is 4 pages
    let ok = below_end + 4 * P;
    assert_eq!(m.fault(ok - 1, Access::Read, PageState::Absent), FaultAction::Segv(SegvCode::MapErr));
    assert_eq!(m.find(bottom).unwrap().start, bottom);
    assert_eq!(m.fault(ok, Access::Read, PageState::Absent), FaultAction::ZeroFill { page: ok, prot: Prot::RW });
    assert_eq!(m.find(ok).unwrap().start, ok);
    // The gap itself and the region below fault normally.
    assert_eq!(m.fault(below_end, Access::Read, PageState::Absent), FaultAction::Segv(SegvCode::MapErr));
    assert_eq!(
        m.fault(below_end - 1, Access::Read, PageState::Absent),
        FaultAction::ZeroFill { page: below_end - P, prot: Prot::RW }
    );
}

#[test]
fn stack_growth_without_a_gap_merges_with_a_compatible_stack_piece() {
    let mut m = map_with(Layout { stack_guard_gap: 0, ..small() });
    let bottom = m.setup_stack(8 * P, Prot::RW).unwrap();
    // Punch a hole into the stack: the upper piece grows back into it.
    m.unmap(bottom + 2 * P, 2 * P).unwrap();
    assert_eq!(m.region_count(), 2);
    assert_eq!(
        m.fault(bottom + 3 * P, Access::Write, PageState::Absent),
        FaultAction::ZeroFill { page: bottom + 3 * P, prot: Prot::RW }
    );
    assert_eq!(m.region_count(), 2);
    assert_eq!(
        m.fault(bottom + 2 * P, Access::Write, PageState::Absent),
        FaultAction::ZeroFill { page: bottom + 2 * P, prot: Prot::RW }
    );
    assert_eq!(spans(&m), [(bottom, MAX, Prot::RW)]);
}

#[test]
fn stack_growth_needs_an_allowed_access() {
    let mut m = map_with(small());
    let bottom = m.setup_stack(2 * P, Prot::RW).unwrap();
    assert_eq!(m.fault(bottom - 1, Access::Execute, PageState::Absent), FaultAction::Segv(SegvCode::AccErr));
    assert_eq!(m.find(bottom - 1), None);
    // Only a stack region grows: below an ordinary one is a map error.
    fixed(&mut m, MIN + 10 * P, 1, Prot::RW);
    assert_eq!(m.fault(MIN + 9 * P, Access::Read, PageState::Absent), FaultAction::Segv(SegvCode::MapErr));
}

// ------------------------------------------------------------------- brk --

#[test]
fn brk_grows_and_shrinks_the_heap() {
    let mut m = map_with(small());
    assert_eq!(m.brk(MIN + 100 * P), Brk { brk: 0, removed: vec![] });
    assert_eq!(m.brk_start(), None);
    assert_eq!(m.init_brk(MIN - 1), Err(Error::Invalid));
    assert_eq!(m.init_brk(MAX), Err(Error::Invalid));
    // The program's data ends mid-page; that page is already mapped.
    let start = MIN + 10 * P + 0x234;
    fixed(&mut m, MIN, 11, Prot::RW);
    m.init_brk(start).unwrap();
    assert_eq!(m.brk_start(), Some(start));
    assert_eq!(m.brk(0).brk, start);
    assert_eq!(m.brk(start - 1).brk, start);
    // Within the same page: no new region.
    assert_eq!(m.brk(start + 0x100).brk, start + 0x100);
    assert_eq!(m.region_count(), 1);
    // Grow by two and a bit pages.
    let heap = MIN + 11 * P;
    assert_eq!(m.brk(heap + 2 * P + 1), Brk { brk: heap + 2 * P + 1, removed: vec![] });
    assert_eq!(m.find(heap).unwrap(), &region(heap, heap + 3 * P, Prot::RW, Kind::Heap, Backing::Anonymous));
    assert_eq!(m.region_count(), 2);
    // Grow again: one heap region.
    assert_eq!(m.brk(heap + 5 * P).brk, heap + 5 * P);
    assert_eq!(m.region_count(), 2);
    // Shrink: the whole pages above the new break go.
    let r = m.brk(heap + P + 5);
    assert_eq!(r.brk, heap + P + 5);
    assert_eq!(r.removed, [region(heap + 2 * P, heap + 5 * P, Prot::RW, Kind::Heap, Backing::Anonymous)]);
    // Back to the start removes the heap region.
    let r = m.brk(start);
    assert_eq!(r.removed, [region(heap, heap + 2 * P, Prot::RW, Kind::Heap, Backing::Anonymous)]);
    assert_eq!(m.region_count(), 1);
}

#[test]
fn brk_refuses_collisions_and_limits() {
    let mut m = map_with(Layout { heap_max: 20 * P, ..small() });
    let start = MIN + 10 * P;
    m.init_brk(start).unwrap();
    fixed(&mut m, start + 8 * P, 1, Prot::READ);
    assert_eq!(m.brk(start + 9 * P).brk, start);
    assert_eq!(m.brk(start + 8 * P + 1).brk, start);
    assert_eq!(m.brk(start + 8 * P).brk, start + 8 * P);
    m.unmap(start + 8 * P, P).unwrap();
    // Above heap_max.
    assert_eq!(m.brk(start + 20 * P + 1).brk, start + 8 * P);
    assert_eq!(m.brk(start + 20 * P).brk, start + 20 * P);
    // Overflow and the end of the address space.
    assert_eq!(m.brk(u64::MAX).brk, start + 20 * P);
    let mut top = map_with(small());
    top.init_brk(MAX - 2 * P).unwrap();
    assert_eq!(top.brk(MAX + 1).brk, MAX - 2 * P);
    assert_eq!(top.brk(MAX).brk, MAX);
}

#[test]
fn brk_stays_out_of_the_stack_guard_gap() {
    let mut m = map_with(small());
    let bottom = m.setup_stack(4 * P, Prot::RW).unwrap();
    let start = bottom - 10 * P;
    m.init_brk(start).unwrap();
    assert_eq!(m.brk(bottom - 3 * P).brk, start);
    assert_eq!(m.brk(bottom - 4 * P).brk, bottom - 4 * P);
}

#[test]
fn brk_fails_when_shrinking_would_split_past_the_limit() {
    let mut m = map_with(Layout { max_map_count: 1, ..small() });
    let start = MIN + 10 * P;
    m.init_brk(start).unwrap();
    assert_eq!(m.brk(start + 4 * P).brk, start + 4 * P);
    // The program maps one region over the whole heap and beyond: shrinking
    // the break would cut a hole into it, making two regions.
    assert_eq!(fixed(&mut m, start, 5, Prot::RW).len(), 1);
    assert_eq!(m.brk(start + P).brk, start + 4 * P);
    assert_eq!(spans(&m), [(start, start + 5 * P, Prot::RW)]);
    // Growing into the region is a collision.
    assert_eq!(m.brk(start + 6 * P).brk, start + 4 * P);
    // Shrinking all the way removes the region's lower part, which keeps
    // one region.
    assert_eq!(m.brk(start).brk, start);
    assert_eq!(spans(&m), [(start + 4 * P, start + 5 * P, Prot::RW)]);
}

// ----------------------------------------------------------------- fault --

#[test]
fn faults_outside_any_region() {
    let mut m = map_with(small());
    fixed(&mut m, MIN + 10 * P, 1, Prot::RW);
    for addr in [0, MIN - 1, MIN + 9 * P, MIN + 11 * P, MAX, u64::MAX, 0xffff_8000_0000_0000] {
        assert_eq!(m.fault(addr, Access::Read, PageState::Absent), FaultAction::Segv(SegvCode::MapErr), "{addr:#x}");
    }
}

#[test]
fn access_errors() {
    let mut m = map_with(small());
    let a = MIN + 10 * P;
    fixed(&mut m, a, 1, Prot::READ);
    fixed(&mut m, a + P, 1, Prot::NONE);
    fixed(&mut m, a + 2 * P, 1, Prot::RW);
    let acc = FaultAction::Segv(SegvCode::AccErr);
    for state in [PageState::Absent, PageState::Present { refs: 1 }] {
        assert_eq!(m.fault(a, Access::Write, state), acc);
        assert_eq!(m.fault(a, Access::Execute, state), acc);
        assert_eq!(m.fault(a + P, Access::Read, state), acc);
        assert_eq!(m.fault(a + 2 * P, Access::Execute, state), acc);
    }
}

#[test]
fn anonymous_faults_zero_fill() {
    let mut m = map_with(small());
    let a = MIN + 10 * P;
    fixed(&mut m, a, 2, Prot::ALL);
    fixed(&mut m, a + 2 * P, 1, Prot::WRITE);
    for access in [Access::Read, Access::Write, Access::Execute] {
        assert_eq!(
            m.fault(a + P + 0x123, access, PageState::Absent),
            FaultAction::ZeroFill { page: a + P, prot: Prot::ALL }
        );
    }
    // Write-only is readable on x86-64.
    assert_eq!(
        m.fault(a + 2 * P, Access::Read, PageState::Absent),
        FaultAction::ZeroFill { page: a + 2 * P, prot: Prot::WRITE }
    );
}

#[test]
fn file_faults_read_the_right_offset() {
    let mut m = map_with(small());
    let a = MIN + 10 * P;
    m.map(file(4 * P, Prot::RW, 5, 16 * P, false, Placement::Fixed(a))).unwrap();
    m.map(file(4 * P, Prot::RW, 6, 32 * P, true, Placement::Fixed(a + 4 * P))).unwrap();
    m.map(file(P, Prot::RX, 7, 0, false, Placement::Fixed(a + 8 * P))).unwrap();
    let read =
        |page, file, offset, prot, private_copy| FaultAction::ReadFile { page, file, offset, prot, private_copy };
    // Private, read: the page cache's frame may be mapped, but not writable.
    assert_eq!(m.fault(a + 2 * P + 7, Access::Read, PageState::Absent), read(a + 2 * P, 5, 18 * P, Prot::READ, false));
    // Private, write: a private copy, writable.
    assert_eq!(m.fault(a + 3 * P, Access::Write, PageState::Absent), read(a + 3 * P, 5, 19 * P, Prot::RW, true));
    // Shared: the file's page, with the region's permissions.
    assert_eq!(m.fault(a + 5 * P, Access::Read, PageState::Absent), read(a + 5 * P, 6, 33 * P, Prot::RW, false));
    assert_eq!(m.fault(a + 7 * P, Access::Write, PageState::Absent), read(a + 7 * P, 6, 35 * P, Prot::RW, false));
    // Executable text.
    assert_eq!(m.fault(a + 8 * P, Access::Execute, PageState::Absent), read(a + 8 * P, 7, 0, Prot::RX, false));
}

#[test]
fn present_pages_copy_on_write_or_update_permissions() {
    let mut m = map_with(small());
    let a = MIN + 10 * P;
    fixed(&mut m, a, 1, Prot::RW);
    m.map(file(P, Prot::RW, 1, 0, true, Placement::Fixed(a + P))).unwrap();
    m.map(file(P, Prot::WRITE, 2, 0, false, Placement::Fixed(a + 2 * P))).unwrap();
    fixed(&mut m, a + 3 * P, 1, Prot::RX);
    let present = |refs| PageState::Present { refs };
    let set = |page, prot| FaultAction::SetPermissions { page, prot };
    let cow = |page, prot| FaultAction::CopyOnWrite { page, prot };

    // Private page shared with another address space (after fork) or the
    // page cache: a write copies it; reads keep sharing it.
    assert_eq!(m.fault(a, Access::Write, present(2)), cow(a, Prot::RW));
    assert_eq!(m.fault(a, Access::Read, present(2)), set(a, Prot::READ));
    // The last user of a frame just gets write permission back.
    assert_eq!(m.fault(a, Access::Write, present(1)), set(a, Prot::RW));
    // Shared pages are never copied.
    assert_eq!(m.fault(a + P, Access::Write, present(3)), set(a + P, Prot::RW));
    // A write-only private page stays readable while write-protected: no
    // fault loop on reads.
    assert_eq!(
        m.fault(a + 2 * P, Access::Read, PageState::Absent),
        FaultAction::ReadFile { page: a + 2 * P, file: 2, offset: 0, prot: Prot::READ, private_copy: false }
    );
    assert_eq!(m.fault(a + 2 * P, Access::Read, present(2)), set(a + 2 * P, Prot::READ));
    assert_eq!(m.fault(a + 2 * P, Access::Write, present(2)), cow(a + 2 * P, Prot::WRITE));
    // After mprotect added exec: the entry is brought up to date.
    assert_eq!(m.fault(a + 3 * P, Access::Execute, present(1)), set(a + 3 * P, Prot::RX));
}

#[test]
fn page_prot_for_the_kernel() {
    let private = anon_vma(MIN, MIN + P, Prot::ALL);
    assert_eq!(private.page_prot(1), Prot::ALL);
    assert_eq!(private.page_prot(2), Prot::RX);
    let shared = file_vma(MIN, MIN + P, Prot::RW, 1, 0, true);
    assert!(shared.is_shared() && !private.is_shared());
    assert_eq!(shared.page_prot(5), Prot::RW);
    let none = anon_vma(MIN, MIN + P, Prot::NONE);
    assert_eq!(none.page_prot(2), Prot::NONE);
    assert_eq!(shared.file_offset(MIN + 5), Some(5));
    assert_eq!(shared.file_offset(MIN + P), None);
    assert_eq!(private.file_offset(MIN), None);
    assert_eq!(private.size(), P);
}

#[test]
fn regions_in_lists_overlaps() {
    let mut m = map_with(small());
    fixed(&mut m, MIN, 2, Prot::RW);
    fixed(&mut m, MIN + 3 * P, 2, Prot::READ);
    fixed(&mut m, MIN + 6 * P, 1, Prot::RW);
    let starts = |s, e| m.regions_in(s, e).map(|v| v.start).collect::<Vec<_>>();
    assert_eq!(starts(MIN + P, MIN + 4 * P), [MIN, MIN + 3 * P]);
    assert_eq!(starts(MIN + 2 * P, MIN + 3 * P), [] as [u64; 0]);
    assert_eq!(starts(MIN + 5 * P, MIN + 5 * P), [] as [u64; 0]);
    assert_eq!(starts(MIN + 4 * P, MIN + 4 * P), [] as [u64; 0]);
    assert_eq!(starts(0, u64::MAX), [MIN, MIN + 3 * P, MIN + 6 * P]);
}

#[test]
fn wipe_on_fork_splits_and_merges_back() {
    let mut m = map_with(small());
    fixed(&mut m, MIN, 4, Prot::RW);
    m.set_wipe_on_fork(MIN + P, P, true).unwrap();
    let wiped: Vec<(u64, bool)> = m.regions().map(|v| (v.start, v.wipe_on_fork)).collect();
    assert_eq!(wiped, [(MIN, false), (MIN + P, true), (MIN + 2 * P, false)]);
    m.set_wipe_on_fork(MIN, 4 * P, false).unwrap();
    assert_eq!(m.region_count(), 1, "keep-on-fork merges the pieces again");
    // Only private anonymous memory may be wiped; a hole is ENOMEM after
    // the mapped part got it.
    m.map(file(P, Prot::RW, 7, 0, true, Placement::Fixed(MIN + 8 * P))).unwrap();
    assert_eq!(m.set_wipe_on_fork(MIN + 8 * P, P, true), Err(Error::Invalid));
    assert_eq!(m.set_wipe_on_fork(MIN + 3 * P, 2 * P, true), Err(Error::NoMemory));
    assert!(m.find(MIN + 3 * P).unwrap().wipe_on_fork);
    assert_eq!(m.set_wipe_on_fork(MIN + 8 * P, P, false), Ok(()), "keeping is fine for any mapping");
}

#[test]
fn a_cloned_map_is_independent() {
    let mut m = map_with(small());
    fixed(&mut m, MIN, 2, Prot::RW);
    let mut child = m.clone();
    child.unmap(MIN, P).unwrap();
    assert_eq!(m.region_count(), 1);
    assert_eq!(m.find(MIN).unwrap().start, MIN);
    assert_eq!(child.find(MIN), None);
}

// ----------------------------------------------------------------- remap --

#[test]
fn remap_shrinks_and_grows_in_place() {
    let mut m = map_with(small());
    let a = MIN + 100 * P;
    fixed(&mut m, a, 4, Prot::RW);
    // Shrinking removes the end.
    let r = m.remap(a, 4 * P, 2 * P + 1, RemapPlacement::InPlace).unwrap();
    assert_eq!((r.addr, r.moved), (a, None));
    assert_eq!(r.removed, [anon_vma(a + 3 * P, a + 4 * P, Prot::RW)], "2 pages and a byte keep 3 pages");
    // Growing into the free space after it, even without MayMove.
    let r = m.remap(a, 3 * P, 8 * P, RemapPlacement::InPlace).unwrap();
    assert_eq!((r.addr, r.moved, r.removed.len()), (a, None, 0));
    assert_eq!(spans(&m), [(a, a + 8 * P, Prot::RW)]);
    // A file mapping that grows continues the file.
    let f = MIN + 300 * P;
    m.map(file(2 * P, Prot::READ, 7, 4 * P, false, Placement::Fixed(f))).unwrap();
    m.remap(f, 2 * P, 3 * P, RemapPlacement::MayMove).unwrap();
    assert_eq!(m.find(f).unwrap(), &file_vma(f, f + 3 * P, Prot::READ, 7, 4 * P, false));
    assert_eq!(m.find(f + 2 * P).unwrap().file_offset(f + 2 * P), Some(6 * P));
}

#[test]
fn remap_moves_when_it_cannot_grow_and_may() {
    let mut m = map_with(small());
    let a = MIN + 100 * P;
    fixed(&mut m, a, 2, Prot::RW);
    fixed(&mut m, a + 2 * P, 1, Prot::READ); // in the way
    assert_eq!(m.remap(a, 2 * P, 4 * P, RemapPlacement::InPlace), Err(Error::NoMemory));
    let r = m.remap(a, 2 * P, 4 * P, RemapPlacement::MayMove).unwrap();
    let to = r.addr;
    assert_ne!(to, a);
    assert_eq!(r.moved, Some(Move { from: a, to, len: 2 * P }));
    assert!(r.removed.is_empty());
    assert_eq!(m.find(a), None, "the old place is free");
    assert_eq!(m.find(to).unwrap(), &anon_vma(to, to + 4 * P, Prot::RW));
    // Part of a region moves; the rest stays.
    let b = MIN + 400 * P;
    fixed(&mut m, b, 6, Prot::RW);
    let r = m.remap(b + 2 * P, 2 * P, 5 * P, RemapPlacement::MayMove).unwrap();
    assert_eq!(r.moved.map(|mv| (mv.from, mv.len)), Some((b + 2 * P, 2 * P)));
    assert_eq!(m.find(b).unwrap().end, b + 2 * P);
    assert_eq!(m.find(b + 4 * P).unwrap().start, b + 4 * P);
    assert_eq!(m.find(b + 2 * P), None);
}

#[test]
fn remap_to_a_fixed_address_replaces_what_is_there() {
    let mut m = map_with(small());
    let (a, to) = (MIN + 100 * P, MIN + 200 * P);
    m.map(file(3 * P, Prot::RW, 9, 0, true, Placement::Fixed(a))).unwrap();
    fixed(&mut m, to, 1, Prot::READ);
    let r = m.remap(a, 3 * P, 2 * P, RemapPlacement::Fixed(to)).unwrap();
    assert_eq!(r.addr, to);
    assert_eq!(r.moved, Some(Move { from: a, to, len: 2 * P }));
    // The old end (shrunk away) and the page at the destination.
    assert_eq!(r.removed, [file_vma(a + 2 * P, a + 3 * P, Prot::RW, 9, 2 * P, true), anon_vma(to, to + P, Prot::READ)]);
    assert_eq!(spans(&m), [(to, to + 2 * P, Prot::RW)]);
    assert!(m.find(to).unwrap().is_shared());
}

#[test]
fn remap_errors_change_nothing() {
    let mut m = map_with(small());
    let a = MIN + 100 * P;
    fixed(&mut m, a, 4, Prot::RW);
    fixed(&mut m, a + 4 * P, 1, Prot::READ);
    let before = spans(&m);
    let e = |m: &mut Map, old, ol, nl, how| m.remap(old, ol, nl, how).err();
    assert_eq!(e(&mut m, a + 1, P, P, RemapPlacement::MayMove), Some(Error::Invalid), "misaligned");
    assert_eq!(e(&mut m, a, P, 0, RemapPlacement::MayMove), Some(Error::Invalid), "zero new length");
    assert_eq!(e(&mut m, a, 0, P, RemapPlacement::MayMove), Some(Error::Invalid), "zero old length");
    assert_eq!(e(&mut m, a, 5 * P, 6 * P, RemapPlacement::MayMove), Some(Error::Fault), "two regions");
    assert_eq!(e(&mut m, MIN + 900 * P, P, 2 * P, RemapPlacement::MayMove), Some(Error::Fault), "nothing there");
    assert_eq!(e(&mut m, a, 2 * P, 2 * P, RemapPlacement::Fixed(a + P)), Some(Error::Invalid), "overlap");
    assert_eq!(e(&mut m, a, P, P, RemapPlacement::Fixed(a + 10 * P + 1)), Some(Error::Invalid), "misaligned target");
    assert_eq!(e(&mut m, a, P, 2 * P, RemapPlacement::Fixed(MAX - P)), Some(Error::NoMemory), "past the end");
    assert_eq!(e(&mut m, a, P, 2000 * P, RemapPlacement::MayMove), Some(Error::NoMemory), "no hole");
    assert_eq!(spans(&m), before);
    // The stack is not remapped.
    let stack = m.setup_stack(4 * P, Prot::RW).unwrap();
    assert_eq!(e(&mut m, stack, P, 2 * P, RemapPlacement::MayMove), Some(Error::Fault));
}

#[test]
fn locks_split_merge_and_count() {
    let mut m = map_with(small());
    fixed(&mut m, MIN, 8, Prot::RW);
    m.set_locked(MIN + P, 2 * P, true).unwrap();
    assert_eq!(m.region_count(), 3);
    assert_eq!(m.locked_bytes(), 2 * P);
    assert_eq!(m.locked_bytes_in(MIN, MIN + 2 * P), P);
    m.set_locked(MIN + 2 * P, 2 * P, true).unwrap();
    assert_eq!(m.locked_bytes(), 3 * P, "locked pages count once");
    m.set_locked(MIN, 8 * P, false).unwrap();
    assert_eq!((m.region_count(), m.locked_bytes()), (1, 0), "merged back");
    assert_eq!(m.set_locked(MIN + P, P + 1, true), Err(Error::Invalid));
}

#[test]
fn lock_stops_at_a_hole() {
    let mut m = map_with(small());
    fixed(&mut m, MIN, 2, Prot::RW);
    fixed(&mut m, MIN + 3 * P, 2, Prot::READ);
    assert_eq!(m.set_locked(MIN, 5 * P, true), Err(Error::NoMemory));
    assert_eq!(m.locked_bytes(), 2 * P, "before the hole only");
    m.set_all_locked(true);
    assert_eq!(m.locked_bytes(), 4 * P);
    m.set_all_locked(false);
    assert_eq!(m.locked_bytes(), 0);
    assert_eq!(m.mapped_bytes(), 4 * P);
}

#[test]
fn future_locks_apply_to_new_mappings() {
    let mut m = map_with(small());
    m.set_lock_future(Some(FutureLock::OnFault));
    fixed(&mut m, MIN, 2, Prot::RW);
    assert_eq!(m.locked_bytes(), 2 * P);
    m.set_lock_future(None);
    fixed(&mut m, MIN + 4 * P, 1, Prot::RW);
    assert_eq!(m.locked_bytes(), 2 * P);
    assert_eq!(m.lock_future(), None);
}

// ------------------------------------------------------- mseal, userfault --

#[test]
fn sealed_regions_refuse_every_change() {
    let mut m = map_with(small());
    fixed(&mut m, MIN, 4, Prot::RW);
    assert_eq!(m.seal(MIN + 1, P), Err(Error::Invalid), "misaligned");
    assert_eq!(m.seal(MIN, u64::MAX - P), Err(Error::Invalid), "wraps around");
    assert_eq!(m.seal(MIN + 3 * P, 2 * P), Err(Error::NoMemory), "a hole");
    assert!(!m.is_sealed(MIN, MIN + 4 * P), "a failed seal seals nothing");
    assert_eq!(m.seal(MIN, 0), Ok(()));
    m.seal(MIN, 1).unwrap();
    assert_eq!(m.region_count(), 2, "the sealed page is a region of its own");
    assert_eq!(m.seal(MIN, P), Ok(()), "sealing again is fine");
    assert_eq!(m.protect(MIN, P, Prot::READ), Err(Error::Permission));
    assert_eq!(m.unmap(MIN, P), Err(Error::Permission));
    assert_eq!(m.unmap(MIN, 4 * P), Err(Error::Permission));
    assert_eq!(m.region_count(), 2, "a refused partly sealed unmap changes nothing");
    assert_eq!(m.remap(MIN, P, 2 * P, RemapPlacement::MayMove).map(|_| ()), Err(Error::Permission));
    assert_eq!(m.map(anon(P, Prot::READ, Placement::Fixed(MIN))).map(|_| ()), Err(Error::Permission));
    assert_eq!(m.remap(MIN + 2 * P, P, P, RemapPlacement::Fixed(MIN)).map(|_| ()), Err(Error::Permission));
    // The neighbours stay changeable.
    m.protect(MIN + P, P, Prot::READ).unwrap();
    m.unmap(MIN + 2 * P, 2 * P).unwrap();
    // A copy (fork) keeps the seal.
    let mut child = m.clone();
    assert_eq!(child.unmap(MIN, P), Err(Error::Permission));
}

#[test]
fn sealed_read_only_private_memory_refuses_discards() {
    let mut m = map_with(small());
    fixed(&mut m, MIN, 1, Prot::RW);
    fixed(&mut m, MIN + 2 * P, 1, Prot::READ);
    m.map(file(P, Prot::READ, 7, 0, true, Placement::Fixed(MIN + 4 * P))).unwrap();
    assert!(!m.discard_refused(MIN, MIN + 5 * P), "nothing sealed yet");
    for at in [MIN, MIN + 2 * P, MIN + 4 * P] {
        m.seal(at, P).unwrap();
    }
    assert!(!m.discard_refused(MIN, MIN + P), "writable private memory");
    assert!(m.discard_refused(MIN + 2 * P, MIN + 3 * P), "read-only private memory");
    assert!(!m.discard_refused(MIN + 4 * P, MIN + 5 * P), "a shared mapping");
}

#[test]
fn userfault_registration_turns_missing_pages_into_user_faults() {
    let mut m = map_with(small());
    fixed(&mut m, MIN, 4, Prot::RW);
    assert_eq!(m.set_userfault(MIN + 1, P, 3), Err(Error::Invalid));
    assert_eq!(m.set_userfault(MIN, 0, 3), Err(Error::Invalid));
    assert_eq!(m.set_userfault(MIN + 2 * P, 4 * P, 3), Err(Error::Invalid), "partly unmapped");
    m.set_userfault(MIN, 2 * P, 3).unwrap();
    m.set_userfault(MIN, 2 * P, 3).unwrap();
    assert!(m.userfault_registered(MIN + P, 3));
    assert_eq!(
        m.fault(MIN + P + 5, Access::Write, PageState::Absent),
        FaultAction::UserFault { page: MIN + P, uffd: 3 }
    );
    assert_eq!(
        m.fault(MIN, Access::Write, PageState::Present { refs: 1 }),
        FaultAction::SetPermissions { page: MIN, prot: Prot::RW },
        "present pages fault as usual"
    );
    assert_eq!(
        m.fault(MIN + 2 * P, Access::Read, PageState::Absent),
        FaultAction::ZeroFill { page: MIN + 2 * P, prot: Prot::RW }
    );
    m.clear_userfault(MIN, P, 3).unwrap();
    assert_eq!(m.fault(MIN, Access::Read, PageState::Absent), FaultAction::ZeroFill { page: MIN, prot: Prot::RW });
    let mut child = m.clone();
    child.forget_userfaults(None);
    assert_eq!(child.region_count(), 1, "a fork child's copy has no registrations");
    m.forget_userfaults(Some(4));
    assert!(m.userfault_registered(MIN + P, 3), "another object's registration stays");
    m.forget_userfaults(Some(3));
    assert_eq!(m.region_count(), 1);
}

#[test]
fn forced_writes_copy_private_pages_and_keep_their_protection() {
    let mut m = map_with(small());
    let a = MIN + 10 * P;
    m.map(file(P, Prot::READ, 5, 16 * P, false, Placement::Fixed(a))).unwrap();
    m.map(file(P, Prot::READ, 6, 0, true, Placement::Fixed(a + P))).unwrap();
    fixed(&mut m, a + 2 * P, 1, Prot::READ);
    let read =
        |page, file, offset, prot, private_copy| FaultAction::ReadFile { page, file, offset, prot, private_copy };
    // A read-only private file page: a private copy, still read-only.
    assert_eq!(m.fault_forced(a + 8, PageState::Absent), read(a, 5, 16 * P, Prot::READ, true));
    assert_eq!(
        m.fault_forced(a, PageState::Present { refs: 2 }),
        FaultAction::CopyOnWrite { page: a, prot: Prot::READ }
    );
    // A read-only shared mapping is not written.
    assert_eq!(m.fault_forced(a + P, PageState::Absent), FaultAction::Segv(SegvCode::AccErr));
    // Anonymous read-only memory.
    assert_eq!(
        m.fault_forced(a + 2 * P, PageState::Absent),
        FaultAction::ZeroFill { page: a + 2 * P, prot: Prot::READ }
    );
    // Nothing mapped.
    assert_eq!(m.fault_forced(a + 9 * P, PageState::Absent), FaultAction::Segv(SegvCode::MapErr));
}

// ------------------------------------------------------------ anon names --

#[test]
fn anonymous_regions_take_names_and_split_for_them() {
    let mut m = map_with(small());
    fixed(&mut m, MIN, 3, Prot::RW);
    let names = |m: &Map| {
        m.regions().map(|v| (v.start, v.end, v.name.as_deref().map(std::string::String::from))).collect::<Vec<_>>()
    };
    let name = |s: &str| Some(Arc::<str>::from(s));
    m.set_anon_name(MIN, 3 * P, name("region")).unwrap();
    assert_eq!(names(&m), vec![(MIN, MIN + 3 * P, Some("region".into()))]);
    m.set_anon_name(MIN + P, 1, name("mid")).unwrap();
    assert_eq!(
        names(&m),
        vec![
            (MIN, MIN + P, Some("region".into())),
            (MIN + P, MIN + 2 * P, Some("mid".into())),
            (MIN + 2 * P, MIN + 3 * P, Some("region".into()))
        ],
        "the length is rounded up to a page"
    );
    m.set_anon_name(MIN, 3 * P, None).unwrap();
    assert_eq!(names(&m), vec![(MIN, MIN + 3 * P, None)], "unnamed again, merged");
    assert_eq!(m.set_anon_name(MIN + 1, P, name("x")), Err(Error::Invalid));
    assert_eq!(m.set_anon_name(MIN, 4 * P, name("x")), Err(Error::NoMemory), "a hole");
    assert_eq!(names(&m), vec![(MIN, MIN + 3 * P, None)], "nothing changed");
    assert_eq!(m.set_anon_name(MIN, 0, name("x")), Ok(()));
    m.map(file(P, Prot::READ, 7, 0, false, Placement::Fixed(MIN + 10 * P))).unwrap();
    assert_eq!(m.set_anon_name(MIN + 10 * P, P, name("x")), Err(Error::NotAnonymous));
    assert_eq!(Error::NotAnonymous.linux_errno(), 9);
    // A fork child's copy keeps the names.
    m.set_anon_name(MIN, P, name("kept")).unwrap();
    assert_eq!(m.clone().find(MIN).and_then(|v| v.name.clone()), name("kept"));
}

// ------------------------------------------------- low hints (V8's pattern) --

/// A process-like address space: the default layout, a program at the PIE
/// base, a heap, a few libraries and the stack.
fn process_like() -> Map {
    let mut m = map_with(Layout::default());
    let pie = 0x5555_5555_4000;
    m.map(file(0x20 * P, Prot::RX, 1, 0, false, Placement::FixedNoReplace(pie))).unwrap();
    m.map(anon(8 * P, Prot::RW, Placement::FixedNoReplace(pie + 0x20 * P))).unwrap();
    m.init_brk(pie + 0x28 * P).unwrap();
    m.setup_stack(34 * P, Prot::RW).unwrap();
    for i in 0..5 {
        m.map(file(0x10 * P, Prot::RX, 10 + i, 0, false, Placement::Hint(0))).unwrap();
    }
    m
}

/// V8 reserves the low 4 GiB: `mmap(hint, 0x1_0000_1000 - hint, PROT_NONE,
/// MAP_PRIVATE | MAP_ANONYMOUS | MAP_NORESERVE)`. The hint is honoured when
/// the range is free, for every hint from the lowest allowed address up.
#[test]
fn a_low_hint_for_four_gib_is_honoured_when_free() {
    let mut m = process_like();
    let end = 0x1_0000_1000;
    let mut hint = USER_MIN_ADDR;
    while hint < USER_MIN_ADDR + 64 * P {
        let r = m.map(anon(end - hint, Prot::NONE, Placement::Hint(hint))).unwrap();
        assert_eq!(r.addr, hint, "hint {hint:#x}");
        assert!(r.replaced.is_empty());
        m.unmap(r.addr, end - hint).unwrap();
        hint += P;
    }
}

/// The end of the 4 GiB reservation V8 asks for (one page past 4 GiB).
const V8_END: u64 = 0x1_0000_1000;

fn reservation(m: &mut Map, hint: u64) -> Result<Mapped<u32>, Error> {
    m.map(anon(V8_END - hint, Prot::NONE, Placement::Hint(hint)))
}

#[test]
fn a_hint_is_not_honoured_when_the_range_is_taken() {
    let m = process_like();
    let top = Layout::default().mmap_base;
    let len = V8_END - 0x1a000;
    // A mapping anywhere inside the range: the hint is not used and the
    // search puts the mapping high, as for no hint.
    for taken in [USER_MIN_ADDR + 0x40 * P, 0x1a000, 0x40_0000, 0x1_0000_0000, V8_END - P] {
        let mut m = m.clone();
        m.map(anon(P, Prot::RW, Placement::Fixed(taken))).unwrap();
        let r = reservation(&mut m, 0x1a000).unwrap();
        assert_ne!(r.addr, 0x1a000, "taken {taken:#x}");
        assert!(r.addr >= 0x1_0000_0000 && r.addr + len <= top, "taken {taken:#x}: {:#x}", r.addr);
        assert_eq!(m.find(taken).unwrap().prot, Prot::RW, "the other mapping stays");
    }
    // Only a page inside the range matters: the pages just before and just
    // after it are not in the way.
    let mut m = m;
    m.map(anon(P, Prot::RW, Placement::Fixed(0x19000))).unwrap();
    m.map(anon(P, Prot::RW, Placement::Fixed(V8_END))).unwrap();
    assert_eq!(reservation(&mut m, 0x1a000).unwrap().addr, 0x1a000);
}

#[test]
fn a_hint_below_the_minimum_address_moves_up_to_it() {
    // "A nearby page boundary, but always above or equal to mmap_min_addr"
    // (mmap(2)): nothing is ever placed below the minimum.
    let mut m = process_like();
    for hint in [1, P, USER_MIN_ADDR - P, USER_MIN_ADDR - 1] {
        let r = m.map(anon(P, Prot::NONE, Placement::Hint(hint))).unwrap();
        assert_eq!(r.addr, USER_MIN_ADDR, "hint {hint:#x}");
        m.unmap(r.addr, P).unwrap();
    }
    // Hint 0 is no hint at all: the search puts it high.
    let r = m.map(anon(P, Prot::NONE, Placement::Hint(0))).unwrap();
    assert!(r.addr > 0x7000_0000_0000);
    m.unmap(r.addr, P).unwrap();
    // The raised range is what must be free: a taken minimum page sends the
    // search elsewhere, and a range that starts below and ends beyond what
    // the space allows is not used.
    assert_eq!(
        m.map(anon(P, Prot::NONE, Placement::Fixed(USER_MIN_ADDR + 2 * P))).unwrap().addr,
        USER_MIN_ADDR + 2 * P
    );
    let r = m.map(anon(4 * P, Prot::NONE, Placement::Hint(P))).unwrap();
    assert_ne!(r.addr, USER_MIN_ADDR, "the raised range overlaps the page at min + 2 pages");
    let r = m.map(anon(USER_END, Prot::NONE, Placement::Hint(P)));
    assert_eq!(r, Err(Error::NoMemory));
}

#[test]
fn a_hint_is_rounded_up_to_a_page() {
    let mut m = process_like();
    assert_eq!(m.map(anon(P, Prot::NONE, Placement::Hint(0x1a001))).unwrap().addr, 0x1b000);
    assert_eq!(m.map(anon(P, Prot::NONE, Placement::Hint(0x1c000 - 1))).unwrap().addr, 0x1c000);
    assert_eq!(m.map(anon(2 * P, Prot::NONE, Placement::Hint(0x30fff))).unwrap().addr, 0x31000);
    // The rounded range is what must be free.
    m.map(anon(P, Prot::RW, Placement::Fixed(0x42000))).unwrap();
    let r = m.map(anon(2 * P, Prot::NONE, Placement::Hint(0x41001))).unwrap();
    assert_ne!(r.addr, 0x42000, "0x42000..0x44000 overlaps the mapping at 0x42000");
    assert!(r.addr >= 0x1_0000_0000);
}

#[test]
fn a_hint_whose_range_leaves_the_address_space_is_not_honoured() {
    let mut m = process_like();
    let l = Layout::default();
    // Ends past USER_END (its last page stays unmapped), or overflows.
    let len = 0x10 * P;
    for hint in [USER_END - len + P, USER_END - P, USER_END, u64::MAX - P, u64::MAX] {
        let r = m.map(anon(len, Prot::NONE, Placement::Hint(hint))).unwrap();
        assert!(r.addr + len <= l.mmap_base, "hint {hint:#x} gave {:#x}", r.addr);
        m.unmap(r.addr, len).unwrap();
    }
    // The stack and its guard gap are taken space.
    let stack = m.find(USER_END - P).unwrap().start;
    let in_gap = stack - DEFAULT_STACK_GUARD_GAP;
    let r = m.map(anon(P, Prot::NONE, Placement::Hint(in_gap))).unwrap();
    assert_ne!(r.addr, in_gap);
    // Ending exactly at USER_END is fine where nothing is in the way.
    let mut free = map_with(Layout::default());
    assert_eq!(free.map(anon(len, Prot::NONE, Placement::Hint(USER_END - len))).unwrap().addr, USER_END - len);
}

#[test]
fn a_fixed_mapping_without_replace_fails_with_eexist_on_overlap() {
    let mut m = process_like();
    let r = reservation(&mut m, 0x1a000).unwrap();
    assert_eq!(r.addr, 0x1a000);
    let at = |a: u64, len: u64| anon(len, Prot::RW, Placement::FixedNoReplace(a));
    // Every kind of overlap.
    for (a, len) in [(0x1a000, P), (0x50_0000, P), (V8_END - P, P), (0x19000, 2 * P), (V8_END - P, 2 * P)] {
        let err = m.map(at(a, len)).unwrap_err();
        assert_eq!(err, Error::Exists, "{a:#x}+{len:#x}");
        assert_eq!(err.linux_errno(), 17);
    }
    // Touching is not overlapping.
    assert_eq!(m.map(at(0x19000, P)).unwrap().addr, 0x19000);
    assert_eq!(m.map(at(V8_END, P)).unwrap().addr, V8_END);
    // Below the minimum: EPERM (no overlap involved).
    assert_eq!(m.map(at(USER_MIN_ADDR - P, P)), Err(Error::Permission));
    // Nothing changed by the failures.
    assert_eq!(m.find(0x1a000).unwrap().end, V8_END);
    // A plain MAP_FIXED over it replaces just that page.
    let replaced = m.map(anon(P, Prot::RW, Placement::Fixed(0x50_0000))).unwrap().replaced;
    assert_eq!(replaced.len(), 1);
    assert_eq!((replaced[0].start, replaced[0].end), (0x50_0000, 0x50_0000 + P));
}

#[test]
fn a_program_at_the_low_exec_base_is_not_overwritten_by_a_hint() {
    // A non-PIE program: 0x400000 with its heap right after.
    let mut m = map_with(Layout::default());
    m.map(file(0x100 * P, Prot::RX, 1, 0, false, Placement::FixedNoReplace(0x40_0000))).unwrap();
    m.map(anon(0x10 * P, Prot::RW, Placement::FixedNoReplace(0x40_0000 + 0x100 * P))).unwrap();
    m.init_brk(0x40_0000 + 0x110 * P).unwrap();
    assert_eq!(m.brk(0x40_0000 + 0x120 * P).brk, 0x40_0000 + 0x120 * P);
    // V8's 4 GiB cannot go at 0x1a000 with the program in the way.
    assert_ne!(reservation(&mut m, 0x1a000).unwrap().addr, 0x1a000);
    // A range that ends before the program fits, and one on it does not.
    assert_eq!(m.map(anon(0x100 * P, Prot::NONE, Placement::Hint(0x1a000))).unwrap().addr, 0x1a000);
    assert_ne!(m.map(anon(P, Prot::NONE, Placement::Hint(0x40_0000))).unwrap().addr, 0x40_0000);
    // Above the heap it is honoured, and the heap stops at it.
    let above = 0x40_0000 + 0x200 * P;
    assert_eq!(m.map(anon(P, Prot::NONE, Placement::Hint(above))).unwrap().addr, above);
    assert_eq!(m.brk(above).brk, above);
    assert_eq!(m.brk(above + P).brk, above, "the heap stops at the mapping");
}

#[test]
fn a_big_reservation_costs_one_region_and_commits_in_pieces() {
    let mut m = process_like();
    let before = m.region_count();
    let r = reservation(&mut m, 0x1a000).unwrap();
    assert_eq!(r.addr, 0x1a000);
    // One region for 4 GiB, whatever the page count.
    assert_eq!(m.region_count(), before + 1);
    // Commit a middle part, then the piece next to it, then one apart.
    m.protect(0x1000_0000, 0x40_0000, Prot::RW).unwrap();
    assert_eq!(m.region_count(), before + 3);
    m.protect(0x1040_0000, 0x40_0000, Prot::RW).unwrap();
    assert_eq!(m.region_count(), before + 3, "the neighbours merge");
    m.protect(0x8000_0000, P, Prot::RW).unwrap();
    assert_eq!(m.region_count(), before + 5);
    let piece = m.find(0x1000_0000).unwrap();
    assert_eq!((piece.start, piece.end, piece.prot), (0x1000_0000, 0x1080_0000, Prot::RW));
    assert_eq!(m.find(0x2000_0000).unwrap().prot, Prot::NONE);
    // A fault on a committed page maps a zeroed page; the rest is PROT_NONE.
    assert!(matches!(m.fault(0x1000_0000 + 5 * P, Access::Write, PageState::Absent), FaultAction::ZeroFill { .. }));
    assert_eq!(m.fault(0x2000_0000, Access::Read, PageState::Absent), FaultAction::Segv(SegvCode::AccErr));
    // Unmap it in pieces.
    m.unmap(0x8000_0000, P).unwrap();
    assert_eq!(m.region_count(), before + 4);
    let removed = m.unmap(0x1a000, 0x2000_0000 - 0x1a000).unwrap();
    assert_eq!(removed.iter().map(Vma::size).sum::<u64>(), 0x2000_0000 - 0x1a000);
    let removed = m.unmap(0x2000_0000, V8_END - 0x2000_0000).unwrap();
    assert_eq!(removed.len(), 2, "the reserved parts either side of the page that went");
    assert_eq!(m.region_count(), before);
}

#[test]
fn a_big_reservation_shrinks_and_moves_by_remap() {
    let mut m = process_like();
    let r = reservation(&mut m, 0x1a000).unwrap();
    let len = V8_END - r.addr;
    let half = page_down(len / 2);
    // Shrinking in place gives the end back.
    let s = m.remap(r.addr, len, half, RemapPlacement::InPlace).unwrap();
    assert_eq!(s.addr, r.addr);
    assert_eq!(s.removed.iter().map(Vma::size).sum::<u64>(), len - half);
    assert_eq!(m.find(r.addr).unwrap().size(), half);
    // Moving it (fixed) keeps it one region.
    let to = 0x2_0000_0000;
    let moved = m.remap(r.addr, half, half, RemapPlacement::Fixed(to)).unwrap();
    assert_eq!(moved.addr, to);
    assert_eq!(m.find(to).unwrap().size(), half);
    assert!(m.find(r.addr).is_none());
}

#[test]
fn map_32bit_places_below_2_gib() {
    let mut m = map_with(Layout::default());
    let low = |len: u64, hint: u64| anon(len, Prot::RW, Placement::Hint(hint));
    // No hint: the lowest hole from 1 GiB.
    assert_eq!(m.map_below(low(4 * P, 0), LOW_2G).unwrap().addr, LOW_SEARCH_BASE);
    assert_eq!(m.map_below(low(P, 0), LOW_2G).unwrap().addr, LOW_SEARCH_BASE + 4 * P);
    // A free hint below 2 GiB is honoured, one that does not fit is not.
    assert_eq!(m.map_below(low(P, 0x20_0000), LOW_2G).unwrap().addr, 0x20_0000);
    assert_eq!(m.map_below(low(P, LOW_2G - P), LOW_2G).unwrap().addr, LOW_2G - P);
    let r = m.map_below(low(2 * P, LOW_2G - P), LOW_2G).unwrap();
    assert_eq!(r.addr, LOW_SEARCH_BASE + 5 * P, "the hint would end above 2 GiB");
    let r = m.map_below(low(P, 0x1_0000_0000), LOW_2G).unwrap();
    assert!(r.addr + P <= LOW_2G, "a hint above 2 GiB is ignored: {:#x}", r.addr);
    // Fixed placements ignore the limit (mmap(2): MAP_32BIT is ignored with MAP_FIXED).
    let fixed = anon(P, Prot::RW, Placement::Fixed(0x3_0000_0000));
    assert_eq!(m.map_below(fixed, LOW_2G).unwrap().addr, 0x3_0000_0000);
    // Without room from 1 GiB it takes the holes below it; with none, ENOMEM.
    let mut full = map_with(Layout::default());
    full.map(anon(LOW_2G - LOW_SEARCH_BASE, Prot::NONE, Placement::Fixed(LOW_SEARCH_BASE))).unwrap();
    assert_eq!(full.map_below(low(P, 0), LOW_2G).unwrap().addr, USER_MIN_ADDR);
    full.map(anon(LOW_SEARCH_BASE - USER_MIN_ADDR - P, Prot::NONE, Placement::Fixed(USER_MIN_ADDR + P))).unwrap();
    assert_eq!(full.map_below(low(2 * P, 0), LOW_2G), Err(Error::NoMemory));
    // A mapping too long for the area never lands above it.
    assert_eq!(map_with(Layout::default()).map_below(low(LOW_2G, 0), LOW_2G), Err(Error::NoMemory));
}
