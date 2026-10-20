extern crate std;

use super::*;
use std::boxed::Box;
use std::collections::{HashMap, HashSet};
use std::vec::Vec;

use PageSize::*;

const K4: u64 = 1 << 12;
const M2: u64 = 1 << 21;
const G1: u64 = 1 << 30;
/// Start of the higher half, where the kernel's direct map will live.
const HIGH: u64 = 0xffff_8000_0000_0000;
const RW: PageFlags = PageFlags::WRITABLE;

/// Simulated physical memory that only contains page tables. Touching a
/// frame that was never set up as a table panics, which catches walks that
/// follow a leaf or garbage as if it were a table.
#[derive(Debug, Default)]
struct Sim {
    tables: HashMap<u64, Box<[u64; ENTRIES]>>,
}

impl Sim {
    fn table(&self, table: u64) -> &[u64; ENTRIES] {
        self.tables.get(&table).unwrap_or_else(|| panic!("{table:#x} is not a page table"))
    }
}

impl PhysMemory for Sim {
    fn read_entry(&self, table: u64, index: usize) -> u64 {
        self.table(table)[index]
    }

    fn write_entry(&mut self, table: u64, index: usize, value: u64) {
        self.tables.get_mut(&table).unwrap_or_else(|| panic!("{table:#x} is not a page table"))[index] = value;
    }

    fn zero_table(&mut self, table: u64) {
        assert_eq!(table % FRAME_SIZE, 0);
        self.tables.insert(table, Box::new([0; ENTRIES]));
    }
}

/// Frame allocator that tracks which frames are out, so leaks and double
/// frees show up, and can be told to run dry.
#[derive(Debug)]
struct Frames {
    next: u64,
    free: Vec<u64>,
    live: HashSet<u64>,
    budget: usize,
}

impl Frames {
    fn new() -> Frames {
        Frames::with_budget(usize::MAX)
    }

    fn with_budget(budget: usize) -> Frames {
        Frames { next: 0x7000_0000_0000, free: Vec::new(), live: HashSet::new(), budget }
    }

    fn give_back(&mut self, freed: FreedTables) {
        for &f in freed.as_slice() {
            self.deallocate_frame(f);
        }
    }
}

impl FrameAllocator for Frames {
    fn allocate_frame(&mut self) -> Option<u64> {
        if self.budget == 0 {
            return None;
        }
        self.budget -= 1;
        let f = self.free.pop().unwrap_or_else(|| {
            self.next += FRAME_SIZE;
            self.next
        });
        assert!(self.live.insert(f));
        Some(f)
    }

    fn deallocate_frame(&mut self, frame: u64) {
        assert!(self.live.remove(&frame), "double free of {frame:#x}");
        self.budget = self.budget.saturating_add(1);
        self.free.push(frame);
    }
}

fn setup() -> (PageTables<Sim>, Frames) {
    let mut frames = Frames::new();
    let pt = PageTables::new(Sim::default(), &mut frames).unwrap();
    (pt, frames)
}

/// The raw entries on the walk to `virt`, from the PML4 down to the leaf.
fn walk(pt: &PageTables<Sim>, virt: u64) -> Vec<u64> {
    let mut out = Vec::new();
    let mut table = pt.root();
    for level in (1..=4).rev() {
        let e = pt.memory().read_entry(table, index(virt, level));
        out.push(e);
        if e & PageFlags::PRESENT.bits() == 0 || level == 1 || (level < 4 && e & PageFlags::HUGE.bits() != 0) {
            break;
        }
        table = e & ADDR_MASK;
    }
    out
}

fn tr(pt: &PageTables<Sim>, virt: u64) -> Translation {
    pt.translate(virt).unwrap()
}

#[test]
fn flag_bits_match_the_sdm() {
    assert_eq!(PageFlags::PRESENT.bits(), 1);
    assert_eq!(PageFlags::WRITABLE.bits(), 2);
    assert_eq!(PageFlags::USER.bits(), 4);
    assert_eq!(PageFlags::WRITE_THROUGH.bits(), 8);
    assert_eq!(PageFlags::CACHE_DISABLE.bits(), 0x10);
    assert_eq!(PageFlags::ACCESSED.bits(), 0x20);
    assert_eq!(PageFlags::DIRTY.bits(), 0x40);
    assert_eq!(PageFlags::HUGE.bits(), 0x80);
    assert_eq!(PageFlags::GLOBAL.bits(), 0x100);
    assert_eq!(PageFlags::NO_EXECUTE.bits(), 1 << 63);
    assert_eq!(ADDR_MASK & 0xfff, 0);
    assert_eq!(ADDR_MASK.count_ones(), 40);
}

#[test]
fn flag_operations() {
    let f = PageFlags::WRITABLE | PageFlags::USER;
    assert!(f.contains(PageFlags::USER));
    assert!(!f.contains(PageFlags::USER | PageFlags::GLOBAL));
    assert!(f.intersects(PageFlags::USER | PageFlags::GLOBAL));
    assert_eq!(f & PageFlags::USER, PageFlags::USER);
    assert_eq!(f.difference(PageFlags::USER), PageFlags::WRITABLE);
    assert_eq!((!f).bits() & ADDR_MASK, 0);
    assert!(!(!f).contains(PageFlags::WRITABLE));
    assert_eq!(PageFlags::from_bits(0x1234_5678_9fff), PageFlags::from_bits(0xfff));
    let mut g = PageFlags::empty();
    g |= PageFlags::NO_EXECUTE;
    assert_eq!(std::format!("{:?}", g | RW), "PageFlags(WRITABLE | NO_EXECUTE)");
    assert_eq!(std::format!("{:?}", PageFlags::from_bits(1 << 52)), "PageFlags(0x10000000000000)");
}

#[test]
fn canonical_addresses() {
    assert!(is_canonical(0));
    assert!(is_canonical(0x0000_7fff_ffff_ffff));
    assert!(!is_canonical(0x0000_8000_0000_0000));
    assert!(!is_canonical(0xffff_7fff_ffff_ffff));
    assert!(is_canonical(HIGH));
    assert!(is_canonical(u64::MAX));
}

#[test]
fn map_and_translate_every_size() {
    let (mut pt, mut frames) = setup();
    pt.map(0x40_0000, 0x1234_5000, Size4K, RW, &mut frames).unwrap();
    pt.map(HIGH + 0x20_0000, 0x4000_0000 + M2, Size2M, RW, &mut frames).unwrap();
    pt.map(HIGH + 4 * G1, 7 * G1, Size1G, RW | PageFlags::GLOBAL, &mut frames).unwrap();

    let t = tr(&pt, 0x40_0abc);
    assert_eq!(t, Translation { phys: 0x1234_5abc, frame: 0x1234_5000, size: Size4K, flags: PageFlags::PRESENT | RW });

    let t = tr(&pt, HIGH + 0x20_0000 + 0x1_2345);
    assert_eq!((t.phys, t.frame, t.size), (0x4000_0000 + M2 + 0x1_2345, 0x4000_0000 + M2, Size2M));
    assert!(!t.flags.contains(PageFlags::HUGE));

    let t = tr(&pt, HIGH + 4 * G1 + 0x3abc_def0);
    assert_eq!((t.phys, t.frame, t.size), (7 * G1 + 0x3abc_def0, 7 * G1, Size1G));
    assert_eq!(t.flags, PageFlags::PRESENT | RW | PageFlags::GLOBAL);

    // PS is set exactly on the large leaves.
    assert_eq!(walk(&pt, 0x40_0000).len(), 4);
    assert_eq!(walk(&pt, 0x40_0000)[3] & PageFlags::HUGE.bits(), 0);
    let w = walk(&pt, HIGH + 0x20_0000);
    assert_eq!(w.len(), 3);
    assert_ne!(w[2] & PageFlags::HUGE.bits(), 0);
    let w = walk(&pt, HIGH + 4 * G1);
    assert_eq!(w.len(), 2);
    assert_ne!(w[1] & PageFlags::HUGE.bits(), 0);

    assert_eq!(pt.translate(0x40_1000), Err(Error::NotMapped));
    assert_eq!(pt.translate(HIGH + 0x40_0000), Err(Error::NotMapped));
    assert_eq!(pt.translate(HIGH + 5 * G1), Err(Error::NotMapped));
    // Root + PDPT/PD/PT for the low page + PDPT/PD for the high 2 MiB page.
    assert_eq!(frames.live.len(), 6);
}

#[test]
fn caller_cannot_forge_huge_or_address_bits() {
    let (mut pt, mut frames) = setup();
    pt.map(0x1000, 0x5000, Size4K, PageFlags::HUGE | PageFlags::from_bits(u64::MAX), &mut frames).unwrap();
    let leaf = walk(&pt, 0x1000)[3];
    assert_eq!(leaf & ADDR_MASK, 0x5000);
    assert_eq!(leaf & PageFlags::HUGE.bits(), 0);
    pt.map(M2, 0, Size2M, PageFlags::empty(), &mut frames).unwrap();
    assert_eq!(tr(&pt, M2).flags, PageFlags::PRESENT);
}

#[test]
fn intermediate_entries_are_permissive() {
    let (mut pt, mut frames) = setup();
    let leaf = PageFlags::NO_EXECUTE | PageFlags::CACHE_DISABLE | PageFlags::GLOBAL;
    pt.map(HIGH + 0x1000, 0x9000, Size4K, leaf, &mut frames).unwrap();
    let w = walk(&pt, HIGH + 0x1000);
    for &e in &w[..3] {
        assert_eq!(PageFlags::from_bits(e), PageFlags::PRESENT | RW, "{e:#x}");
    }
    assert_eq!(PageFlags::from_bits(w[3]), PageFlags::PRESENT | leaf);
}

#[test]
fn user_bit_set_along_the_whole_path() {
    let (mut pt, mut frames) = setup();
    pt.map(0x40_0000, 0x1000, Size4K, RW, &mut frames).unwrap();
    for &e in &walk(&pt, 0x40_0000) {
        assert_eq!(e & PageFlags::USER.bits(), 0);
    }
    // A user page next to it reuses the same tables and turns U/S on for them.
    pt.map(0x40_1000, 0x2000, Size4K, RW | PageFlags::USER, &mut frames).unwrap();
    for &e in &walk(&pt, 0x40_1000) {
        assert_ne!(e & PageFlags::USER.bits(), 0, "{e:#x}");
    }
    // The kernel page keeps a supervisor-only leaf.
    let w = walk(&pt, 0x40_0000);
    assert_eq!(w[3] & PageFlags::USER.bits(), 0);
    assert!(!tr(&pt, 0x40_0000).flags.contains(PageFlags::USER));

    // Large user pages, too.
    pt.map(HIGH, 0, Size1G, PageFlags::USER, &mut frames).unwrap();
    assert!(walk(&pt, HIGH).iter().all(|e| e & PageFlags::USER.bits() != 0));
}

#[test]
fn existing_restrictive_intermediates_are_relaxed() {
    let (mut pt, mut frames) = setup();
    pt.map(0x1000, 0x1000, Size4K, PageFlags::empty(), &mut frames).unwrap();
    // Someone else made the PML4 entry read-only and no-execute.
    let e = pt.memory().read_entry(pt.root(), 0);
    let root = pt.root();
    pt.memory_mut().write_entry(root, 0, (e & !RW.bits()) | PageFlags::NO_EXECUTE.bits());
    pt.map(0x2000, 0x2000, Size4K, RW, &mut frames).unwrap();
    assert_eq!(PageFlags::from_bits(pt.memory().read_entry(root, 0)), PageFlags::PRESENT | RW);
}

#[test]
fn misaligned_addresses_are_rejected() {
    let (mut pt, mut frames) = setup();
    assert_eq!(pt.map(0x1001, 0, Size4K, RW, &mut frames), Err(Error::Misaligned));
    assert_eq!(pt.map(0x1000, 0x10, Size4K, RW, &mut frames), Err(Error::Misaligned));
    assert_eq!(pt.map(K4, 0, Size2M, RW, &mut frames), Err(Error::Misaligned));
    assert_eq!(pt.map(M2, K4, Size2M, RW, &mut frames), Err(Error::Misaligned));
    assert_eq!(pt.map(M2, 0, Size1G, RW, &mut frames), Err(Error::Misaligned));
    assert_eq!(pt.map(G1, M2, Size1G, RW, &mut frames), Err(Error::Misaligned));
    assert_eq!(pt.unmap(0x1234), Err(Error::Misaligned));
    assert_eq!(pt.update_flags(0x1234, RW), Err(Error::Misaligned));
    assert_eq!(pt.map_range(0, 0, 0x1800, RW, Size1G, &mut frames), Err(Error::Misaligned));
    assert_eq!(pt.map_range(0x800, 0, K4, RW, Size1G, &mut frames), Err(Error::Misaligned));
    assert_eq!(pt.map_range(0, 0x800, K4, RW, Size1G, &mut frames), Err(Error::Misaligned));
    assert_eq!(frames.live.len(), 1);
}

#[test]
fn physical_addresses_beyond_52_bits_are_rejected() {
    let (mut pt, mut frames) = setup();
    assert_eq!(pt.map(0, PHYS_LIMIT, Size4K, RW, &mut frames), Err(Error::InvalidPhysical));
    assert_eq!(pt.map(0, 1 << 60, Size1G, RW, &mut frames), Err(Error::InvalidPhysical));
    assert_eq!(pt.map_range(0, PHYS_LIMIT - K4, 2 * K4, RW, Size1G, &mut frames), Err(Error::InvalidPhysical));
    assert_eq!(pt.map_range(0, u64::MAX - 0xfff, K4, RW, Size1G, &mut frames), Err(Error::InvalidPhysical));
    pt.map(0, PHYS_LIMIT - K4, Size4K, RW, &mut frames).unwrap();
    assert_eq!(tr(&pt, 0).frame, PHYS_LIMIT - K4);
}

#[test]
fn non_canonical_addresses_are_rejected() {
    let (mut pt, mut frames) = setup();
    let bad = 0x0000_8000_0000_0000;
    assert_eq!(pt.map(bad, 0, Size4K, RW, &mut frames), Err(Error::NonCanonical));
    assert_eq!(pt.map(bad, 0, Size1G, RW, &mut frames), Err(Error::NonCanonical));
    assert_eq!(pt.translate(bad), Err(Error::NonCanonical));
    assert_eq!(pt.unmap(bad), Err(Error::NonCanonical));
    assert_eq!(pt.unmap_and_prune(bad).map(|_| ()), Err(Error::NonCanonical));
    assert_eq!(pt.update_flags(bad, RW), Err(Error::NonCanonical));
    assert_eq!(pt.map_range(bad, 0, K4, RW, Size1G, &mut frames), Err(Error::NonCanonical));
    // Ranges must not run into or across the hole.
    assert_eq!(pt.map_range(0x7fff_ffff_f000, 0, 2 * K4, RW, Size1G, &mut frames), Err(Error::NonCanonical));
    assert_eq!(pt.map_range(HIGH - K4, 0, 2 * K4, RW, Size1G, &mut frames), Err(Error::NonCanonical));
    assert_eq!(pt.map_range(u64::MAX - 0xfff, 0, 2 * K4, RW, Size1G, &mut frames), Err(Error::NonCanonical));
    assert_eq!(frames.live.len(), 1);
    // The very top and bottom pages of each half are fine.
    pt.map_range(0x7fff_ffff_f000, 0, K4, RW, Size1G, &mut frames).unwrap();
    pt.map_range(u64::MAX - 0xfff, K4, K4, RW, Size1G, &mut frames).unwrap();
    assert_eq!(tr(&pt, u64::MAX).phys, 2 * K4 - 1);
}

#[test]
fn already_mapped_and_huge_page_conflicts() {
    let (mut pt, mut frames) = setup();
    pt.map(0, 0, Size4K, RW, &mut frames).unwrap();
    pt.map(M2, 0, Size2M, RW, &mut frames).unwrap();
    pt.map(G1, 0, Size1G, RW, &mut frames).unwrap();
    let before = frames.live.len();

    // Same slot, same size.
    assert_eq!(pt.map(0, K4, Size4K, RW, &mut frames), Err(Error::AlreadyMapped));
    assert_eq!(pt.map(M2, 0, Size2M, RW, &mut frames), Err(Error::AlreadyMapped));
    assert_eq!(pt.map(G1, 0, Size1G, RW, &mut frames), Err(Error::AlreadyMapped));
    // Smaller page inside a larger one.
    assert_eq!(pt.map(M2 + K4, 0, Size4K, RW, &mut frames), Err(Error::HugePageConflict));
    assert_eq!(pt.map(G1 + 5 * K4, 0, Size4K, RW, &mut frames), Err(Error::HugePageConflict));
    assert_eq!(pt.map(G1 + 3 * M2, 0, Size2M, RW, &mut frames), Err(Error::HugePageConflict));
    // Larger page over smaller ones (a table sits in its slot).
    assert_eq!(pt.map(0, 0, Size2M, RW, &mut frames), Err(Error::AlreadyMapped));
    assert_eq!(pt.map(0, 0, Size1G, RW, &mut frames), Err(Error::AlreadyMapped));
    assert_eq!(pt.map_range(0, 0, G1, RW, Size1G, &mut frames), Err(Error::AlreadyMapped));

    // Nothing changed, nothing leaked.
    assert_eq!(frames.live.len(), before);
    assert_eq!(tr(&pt, 0).frame, 0);
    assert_eq!(tr(&pt, M2 + K4).size, Size2M);
    assert_eq!(tr(&pt, G1 + 3 * M2).size, Size1G);
}

#[test]
fn unmap_returns_the_old_page() {
    let (mut pt, mut frames) = setup();
    let user = RW | PageFlags::USER;
    pt.map(0x5000, 0xabc000, Size4K, user, &mut frames).unwrap();
    pt.map(HIGH + M2, 4 * M2, Size2M, PageFlags::GLOBAL, &mut frames).unwrap();
    pt.map(HIGH + G1, 3 * G1, Size1G, PageFlags::NO_EXECUTE, &mut frames).unwrap();

    let u = pt.unmap(0x5000).unwrap();
    assert_eq!(u, Unmapped { frame: 0xabc000, size: Size4K, flags: PageFlags::PRESENT | user });
    assert_eq!(pt.translate(0x5000), Err(Error::NotMapped));

    // Only the whole large page can be unmapped.
    assert_eq!(pt.unmap(HIGH + M2 + K4), Err(Error::HugePageConflict));
    assert_eq!(pt.unmap(HIGH + G1 + M2), Err(Error::HugePageConflict));
    let u = pt.unmap(HIGH + M2).unwrap();
    assert_eq!(u, Unmapped { frame: 4 * M2, size: Size2M, flags: PageFlags::PRESENT | PageFlags::GLOBAL });
    let u = pt.unmap(HIGH + G1).unwrap();
    assert_eq!(u, Unmapped { frame: 3 * G1, size: Size1G, flags: PageFlags::PRESENT | PageFlags::NO_EXECUTE });

    assert_eq!(pt.unmap(0x5000), Err(Error::NotMapped));
    assert_eq!(pt.unmap(0x6000), Err(Error::NotMapped));
    assert_eq!(pt.unmap(HIGH + G1), Err(Error::NotMapped));

    // Plain unmap keeps the empty tables, so a large page cannot go where
    // a 4 KiB one was...
    assert_eq!(pt.map(0, 0, Size2M, RW, &mut frames), Err(Error::AlreadyMapped));
    // ...but the same slot is free again.
    pt.map(0x5000, 0x1000, Size4K, RW, &mut frames).unwrap();
}

#[test]
fn prune_frees_empty_tables_only() {
    let (mut pt, mut frames) = setup();
    pt.map(0x1000, 0x1000, Size4K, RW, &mut frames).unwrap();
    pt.map(0x2000, 0x2000, Size4K, RW, &mut frames).unwrap();
    pt.map(G1, 0, Size2M, RW, &mut frames).unwrap();
    // Root, PDPT, PD for 0..1G, PT, PD for 1G..2G.
    assert_eq!(frames.live.len(), 5);

    // The PT still holds 0x2000.
    let (u, freed) = pt.unmap_and_prune(0x1000).unwrap();
    assert_eq!(u.frame, 0x1000);
    assert!(freed.is_empty());

    // Now the PT empties, and so does the low PD; the PDPT still holds 1G.
    let pt_frame = walk(&pt, 0x2000)[2] & ADDR_MASK;
    let pd_frame = walk(&pt, 0x2000)[1] & ADDR_MASK;
    let (_, freed) = pt.unmap_and_prune(0x2000).unwrap();
    assert_eq!(freed.as_slice(), &[pt_frame, pd_frame]);
    assert_eq!(walk(&pt, 0).len(), 2);
    frames.give_back(freed);

    // Last mapping: PD and PDPT go, the root stays.
    let (u, freed) = pt.unmap_and_prune(G1).unwrap();
    assert_eq!(u.size, Size2M);
    assert_eq!(freed.len(), 2);
    frames.give_back(freed);
    assert_eq!(frames.live.len(), 1);
    assert!(frames.live.contains(&pt.root()));
    assert!((0..ENTRIES).all(|i| pt.memory().read_entry(pt.root(), i) == 0));

    // A large page fits where the 4 KiB pages were.
    pt.map(0, 0, Size1G, RW, &mut frames).unwrap();
}

#[test]
fn prune_keeps_tables_with_non_present_data() {
    let (mut pt, mut frames) = setup();
    pt.map(0x1000, 0x1000, Size4K, RW, &mut frames).unwrap();
    pt.map(0x2000, 0x2000, Size4K, RW, &mut frames).unwrap();
    // Turn 0x2000 into a non-present entry carrying software data (as a
    // swapped-out page might be).
    let table = walk(&pt, 0x2000)[2] & ADDR_MASK;
    pt.memory_mut().write_entry(table, 2, 0xdead_0000);
    let (_, freed) = pt.unmap_and_prune(0x1000).unwrap();
    assert!(freed.is_empty());
    assert_eq!(pt.memory().read_entry(table, 2), 0xdead_0000);
}

#[test]
fn software_bits_are_kept_and_reported() {
    let (mut pt, mut frames) = setup();
    let sw = PageFlags::SOFTWARE_9 | PageFlags::SOFTWARE_11;
    assert_eq!(sw.bits(), (1 << 9) | (1 << 11));
    pt.map(0x7000, 0x9000, Size4K, RW | PageFlags::USER | sw, &mut frames).unwrap();
    assert_eq!(tr(&pt, 0x7000).flags, PageFlags::PRESENT | RW | PageFlags::USER | sw);
    // Intermediate entries never get them.
    assert!(walk(&pt, 0x7000)[..3].iter().all(|e| e & sw.bits() == 0));
    let old = pt.update_flags(0x7000, PageFlags::USER | PageFlags::SOFTWARE_10).unwrap();
    assert!(old.contains(sw));
    assert_eq!(pt.unmap(0x7000).unwrap().flags, PageFlags::PRESENT | PageFlags::USER | PageFlags::SOFTWARE_10);
    assert_eq!(std::format!("{:?}", PageFlags::SOFTWARE_10), "PageFlags(SOFTWARE_10)");
}

#[test]
fn clear_empties_a_range_of_slots_and_reports_everything() {
    let (mut pt, mut frames) = setup();
    let user = RW | PageFlags::USER;
    // Lower half: 4 KiB pages in two PTs, a 2 MiB page, a 1 GiB page, and
    // a page in the last slot of the lower half.
    pt.map(0x1000, 0xa000, Size4K, user, &mut frames).unwrap();
    pt.map(0x3000, 0xb000, Size4K, user, &mut frames).unwrap();
    pt.map(M2 + 0x5000, 0xc000, Size4K, user, &mut frames).unwrap();
    pt.map(4 * M2, 8 * M2, Size2M, user, &mut frames).unwrap();
    pt.map(3 * G1, 5 * G1, Size1G, user, &mut frames).unwrap();
    let top = 0x7fff_ffff_f000;
    pt.map(top, 0xd000, Size4K, user, &mut frames).unwrap();
    // Higher half: must survive.
    pt.map(HIGH + 0x1000, 0xe000, Size4K, RW, &mut frames).unwrap();
    pt.map(0xffff_ffff_8000_0000, 0, Size2M, RW, &mut frames).unwrap();
    // A non-present entry with software data is dropped without a report.
    let table = walk(&pt, 0x1000)[2] & ADDR_MASK;
    pt.memory_mut().write_entry(table, 7, 0xdead_0000);
    let tables_before = frames.live.len();

    let mut pages = Vec::new();
    let mut tables = Vec::new();
    pt.clear(0..ENTRIES / 2, |c| match c {
        Cleared::Page { virt, page } => pages.push((virt, page.frame, page.size)),
        Cleared::Table(frame) => {
            // Children before parents: nothing reported later lives in it.
            assert!(!tables.contains(&frame));
            tables.push(frame);
        }
    });
    pages.sort();
    assert_eq!(
        pages,
        [
            (0x1000, 0xa000, Size4K),
            (0x3000, 0xb000, Size4K),
            (M2 + 0x5000, 0xc000, Size4K),
            (4 * M2, 8 * M2, Size2M),
            (3 * G1, 5 * G1, Size1G),
            (top, 0xd000, Size4K),
        ]
    );
    // Two PTs and a PD for the low GiB, the low PDPT, and a PDPT, PD and
    // PT for the top page.
    assert_eq!(tables.len(), 7);
    assert!((0..ENTRIES / 2).all(|i| pt.memory().read_entry(pt.root(), i) == 0));
    for f in &tables {
        frames.deallocate_frame(*f);
    }
    assert_eq!(frames.live.len(), tables_before - 7);
    assert_eq!(tr(&pt, HIGH + 0x1000).phys, 0xe000);
    assert_eq!(tr(&pt, 0xffff_ffff_8000_0000).size, Size2M);

    // The higher half reports its own (sign-extended) addresses.
    let mut high = Vec::new();
    pt.clear(ENTRIES / 2..ENTRIES, |c| {
        if let Cleared::Page { virt, .. } = c {
            high.push(virt);
        }
    });
    high.sort();
    assert_eq!(high, [HIGH + 0x1000, 0xffff_ffff_8000_0000]);
    assert!((0..ENTRIES).all(|i| pt.memory().read_entry(pt.root(), i) == 0));

    // Clearing an empty range reports nothing.
    pt.clear(0..ENTRIES, |_| panic!("nothing left to clear"));
}

#[test]
fn update_flags_changes_protection() {
    let (mut pt, mut frames) = setup();
    pt.map(0x3000, 0x8000, Size4K, RW, &mut frames).unwrap();
    pt.map(M2, 6 * M2, Size2M, RW, &mut frames).unwrap();
    pt.map(HIGH, 2 * G1, Size1G, RW, &mut frames).unwrap();

    let old = pt.update_flags(0x3000, PageFlags::NO_EXECUTE).unwrap();
    assert_eq!(old, PageFlags::PRESENT | RW);
    assert_eq!(
        tr(&pt, 0x3000),
        Translation { phys: 0x8000, frame: 0x8000, size: Size4K, flags: PageFlags::PRESENT | PageFlags::NO_EXECUTE }
    );

    // Large pages keep their size and frame.
    pt.update_flags(M2, PageFlags::GLOBAL).unwrap();
    let t = tr(&pt, M2 + 0x123);
    assert_eq!((t.phys, t.size, t.flags), (6 * M2 + 0x123, Size2M, PageFlags::PRESENT | PageFlags::GLOBAL));
    pt.update_flags(HIGH, PageFlags::empty()).unwrap();
    let t = tr(&pt, HIGH + G1 - 1);
    assert_eq!((t.phys, t.size, t.flags), (3 * G1 - 1, Size1G, PageFlags::PRESENT));

    // Making a page user-accessible opens the path, too.
    assert!(walk(&pt, 0x3000).iter().all(|e| e & PageFlags::USER.bits() == 0));
    pt.update_flags(0x3000, PageFlags::USER).unwrap();
    assert!(walk(&pt, 0x3000).iter().all(|e| e & PageFlags::USER.bits() != 0));

    assert_eq!(pt.update_flags(0x4000, RW), Err(Error::NotMapped));
    assert_eq!(pt.update_flags(M2 + K4, RW), Err(Error::HugePageConflict));
    assert_eq!(pt.update_flags(HIGH + M2, RW), Err(Error::HugePageConflict));
}

#[test]
fn map_range_picks_the_largest_pages() {
    let (mut pt, mut frames) = setup();
    // 4 KiB pages up to the next 2 MiB boundary, 2 MiB pages up to 1 GiB,
    // one 1 GiB page, then 2 MiB and 4 KiB pages for the tail.
    let start = G1 - M2 - 3 * K4;
    let len = 3 * K4 + M2 + G1 + M2 + 2 * K4;
    pt.map_range(HIGH + start, start, len, RW, Size1G, &mut frames).unwrap();

    let mut sizes = Vec::new();
    let mut off = 0;
    while off < len {
        let t = tr(&pt, HIGH + start + off);
        assert_eq!(t.phys, start + off);
        assert_eq!(t.frame, start + off, "page does not start at {off:#x}");
        sizes.push(t.size);
        off += t.size.bytes();
    }
    assert_eq!(sizes, [Size4K, Size4K, Size4K, Size2M, Size1G, Size2M, Size4K, Size4K]);
    assert_eq!(pt.translate(HIGH + start + len), Err(Error::NotMapped));
    assert_eq!(pt.translate(HIGH + start - K4), Err(Error::NotMapped));
}

#[test]
fn map_range_respects_largest_and_relative_alignment() {
    let (mut pt, mut frames) = setup();
    pt.map_range(0, 0, 2 * G1, RW, Size2M, &mut frames).unwrap();
    assert_eq!(tr(&pt, 0).size, Size2M);
    assert_eq!(tr(&pt, G1 + 0x1234).size, Size2M);
    // 1024 2 MiB pages: root + PDPT + 2 PDs.
    assert_eq!(frames.live.len(), 4);

    // Virtual and physical addresses differently aligned: only 4 KiB pages.
    pt.map_range(HIGH, K4, 4 * M2, RW, Size1G, &mut frames).unwrap();
    assert_eq!(tr(&pt, HIGH).size, Size4K);
    assert_eq!(tr(&pt, HIGH + 4 * M2 - 1).phys, K4 + 4 * M2 - 1);

    // Zero length is a no-op.
    pt.map_range(HIGH + G1, 0, 0, RW, Size1G, &mut frames).unwrap();
    assert_eq!(pt.translate(HIGH + G1), Err(Error::NotMapped));
}

#[test]
fn direct_map_of_64_gib_is_cheap() {
    let (mut pt, mut frames) = setup();
    pt.map_range(HIGH, 0, 64 * G1, RW | PageFlags::GLOBAL | PageFlags::NO_EXECUTE, Size1G, &mut frames).unwrap();
    // Root + one PDPT is all it takes.
    assert_eq!(frames.live.len(), 2);
    let t = tr(&pt, HIGH + 37 * G1 + 0x1234_5678);
    assert_eq!((t.phys, t.size), (37 * G1 + 0x1234_5678, Size1G));
}

#[test]
fn map_range_is_all_or_nothing_on_conflict() {
    let (mut pt, mut frames) = setup();
    pt.map(10 * K4, 0, Size4K, RW, &mut frames).unwrap();
    let before = frames.live.len();
    assert_eq!(pt.map_range(0, 0, 16 * K4, RW, Size1G, &mut frames), Err(Error::AlreadyMapped));
    for i in 0..16 {
        assert_eq!(pt.translate(i * K4).is_ok(), i == 10);
    }
    assert_eq!(frames.live.len(), before);

    pt.map(HIGH + 3 * M2, 0, Size2M, RW, &mut frames).unwrap();
    assert_eq!(pt.map_range(HIGH, 0, 4 * M2, RW, Size1G, &mut frames), Err(Error::AlreadyMapped));
    assert_eq!(pt.map_range(HIGH + 3 * M2 + K4, 0, K4, RW, Size1G, &mut frames), Err(Error::HugePageConflict));
    assert_eq!(pt.translate(HIGH), Err(Error::NotMapped));
}

#[test]
fn running_out_of_frames_leaks_nothing() {
    // A 4 KiB page in an empty hierarchy needs three tables below the root.
    for budget in 0..3 {
        let mut frames = Frames::with_budget(1 + budget);
        let mut pt = PageTables::new(Sim::default(), &mut frames).unwrap();
        assert_eq!(pt.map(0x1000, 0, Size4K, RW, &mut frames), Err(Error::OutOfFrames));
        assert_eq!(frames.live.len(), 1, "budget {budget}");
        assert!((0..ENTRIES).all(|i| pt.memory().read_entry(pt.root(), i) == 0));
        assert_eq!(pt.translate(0x1000), Err(Error::NotMapped));
    }
    let mut frames = Frames::with_budget(4);
    let mut pt = PageTables::new(Sim::default(), &mut frames).unwrap();
    pt.map(0x1000, 0, Size4K, RW, &mut frames).unwrap();
    // Same PT: no new frame needed.
    pt.map(0x2000, 0, Size4K, RW, &mut frames).unwrap();
    assert!(PageTables::new(Sim::default(), &mut frames).is_err());
}

#[test]
fn map_range_rolls_back_when_frames_run_out() {
    // Root, PDPT, PD, then one PT: the second 2 MiB stretch of 4 KiB pages
    // needs another PT that is not there.
    let mut frames = Frames::with_budget(4);
    let mut pt = PageTables::new(Sim::default(), &mut frames).unwrap();
    assert_eq!(pt.map_range(0, K4, 2 * M2, RW, Size1G, &mut frames), Err(Error::OutOfFrames));
    for i in 0..1024 {
        assert_eq!(pt.translate(i * K4), Err(Error::NotMapped));
    }
    // The tables created on the way stay linked and get reused.
    frames.budget = 1;
    pt.map_range(0, K4, 2 * M2, RW, Size1G, &mut frames).unwrap();
    assert_eq!(tr(&pt, 2 * M2 - 1).phys, 2 * M2 + K4 - 1);
}

#[test]
fn from_root_sees_existing_tables() {
    let (mut pt, mut frames) = setup();
    pt.map(HIGH, 0x7000, Size4K, RW, &mut frames).unwrap();
    let root = pt.root();
    let pt2 = PageTables::from_root(root, pt.into_memory());
    assert_eq!(tr(&pt2, HIGH + 1).phys, 0x7001);
}

/// Small xorshift so the randomized test needs no dependencies and is
/// reproducible.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

#[derive(Clone, Copy, Debug)]
struct Model {
    phys: u64,
    size: PageSize,
    flags: PageFlags,
}

/// The mapping in `model` that covers `virt`.
fn covering(model: &HashMap<u64, Model>, virt: u64) -> Option<(u64, Model)> {
    model.iter().map(|(&v, &m)| (v, m)).find(|&(v, m)| virt >= v && virt - v < m.size.bytes())
}

#[test]
fn randomized_against_a_reference_model() {
    let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
    let (mut pt, mut frames) = setup();
    let mut model: HashMap<u64, Model> = HashMap::new();
    let sizes = [Size4K, Size2M, Size1G];
    let flag_choices = [
        RW,
        PageFlags::empty(),
        RW | PageFlags::USER,
        PageFlags::NO_EXECUTE | PageFlags::GLOBAL,
        RW | PageFlags::USER | PageFlags::NO_EXECUTE,
    ];
    // A small window of addresses in both halves, so that sizes collide often.
    let bases = [0, 0x7f80_0000_0000, HIGH, 0xffff_ff80_0000_0000];
    let random_virt = |rng: &mut Rng, size: PageSize| -> u64 {
        let base = bases[rng.below(4) as usize] + rng.below(2) * G1;
        match size {
            Size1G => base,
            Size2M => base + rng.below(4) * M2,
            Size4K => base + rng.below(4) * M2 + rng.below(8) * K4,
        }
    };

    // How often each outcome happened, so the test cannot silently stop
    // exercising a path.
    let mut seen: HashMap<&str, usize> = HashMap::new();
    let mut note = |what| *seen.entry(what).or_default() += 1;

    for step in 0..20_000 {
        match rng.below(10) {
            0..=4 => {
                let size = sizes[rng.below(3) as usize];
                let virt = random_virt(&mut rng, size);
                let phys = (rng.below(1 << 40) * K4) & !(size.bytes() - 1);
                let flags = flag_choices[rng.below(5) as usize];
                let expect = match model.iter().find(|&(&v, m)| v < virt + size.bytes() && virt < v + m.size.bytes()) {
                    Some((_, m)) if m.size > size => Err(Error::HugePageConflict),
                    Some(_) => Err(Error::AlreadyMapped),
                    None => Ok(()),
                };
                assert_eq!(pt.map(virt, phys, size, flags, &mut frames), expect, "step {step}: map {virt:#x} {size:?}");
                match expect {
                    Ok(()) => {
                        note(["map 4K", "map 2M", "map 1G"][size.level() - 1]);
                        model.insert(virt, Model { phys, size, flags });
                    }
                    Err(Error::AlreadyMapped) => note("map: already mapped"),
                    Err(_) => note("map: huge page conflict"),
                }
            }
            5..=7 => {
                // Mostly unmap something that exists, or the hierarchy fills
                // up with small pages and large ones never fit.
                let virt = if !model.is_empty() && rng.below(3) != 0 {
                    let mut keys: Vec<u64> = model.keys().copied().collect();
                    keys.sort_unstable();
                    keys[rng.below(keys.len() as u64) as usize]
                } else {
                    let size = sizes[rng.below(3) as usize];
                    random_virt(&mut rng, size)
                };
                let res = pt.unmap_and_prune(virt);
                match covering(&model, virt) {
                    None => assert_eq!(res.map(|_| ()), Err(Error::NotMapped), "step {step}"),
                    Some((v, _)) if v != virt => {
                        assert_eq!(res.map(|_| ()), Err(Error::HugePageConflict));
                        note("unmap: inside a larger page");
                    }
                    Some((v, m)) => {
                        let (u, freed) = res.unwrap();
                        assert_eq!(u, Unmapped { frame: m.phys, size: m.size, flags: m.flags | PageFlags::PRESENT });
                        note("unmap");
                        if !freed.is_empty() {
                            note("unmap: tables freed");
                        }
                        frames.give_back(freed);
                        model.remove(&v);
                    }
                }
            }
            8 => {
                let size = sizes[rng.below(3) as usize];
                let virt = random_virt(&mut rng, size);
                let flags = flag_choices[rng.below(5) as usize];
                let res = pt.update_flags(virt, flags);
                match covering(&model, virt) {
                    None => assert_eq!(res, Err(Error::NotMapped)),
                    Some((v, _)) if v != virt => assert_eq!(res, Err(Error::HugePageConflict)),
                    Some((v, m)) => {
                        assert_eq!(res, Ok(m.flags | PageFlags::PRESENT));
                        note("update flags");
                        model.get_mut(&v).unwrap().flags = flags;
                    }
                }
            }
            _ => {
                let virt = random_virt(&mut rng, Size4K) + rng.below(K4);
                match covering(&model, virt) {
                    None => assert_eq!(pt.translate(virt), Err(Error::NotMapped)),
                    Some((v, m)) => {
                        let t = tr(&pt, virt);
                        assert_eq!(t.phys, m.phys + (virt - v));
                        assert_eq!((t.frame, t.size, t.flags), (m.phys, m.size, m.flags | PageFlags::PRESENT));
                        note("translate: hit");
                    }
                }
            }
        }
    }

    for what in [
        "map 4K",
        "map 2M",
        "map 1G",
        "map: already mapped",
        "map: huge page conflict",
        "unmap",
        "unmap: inside a larger page",
        "unmap: tables freed",
        "update flags",
        "translate: hit",
    ] {
        assert!(seen.get(what).copied().unwrap_or(0) >= 50, "{what}: {seen:?}");
    }

    // Every user leaf has U/S along its whole path.
    for (&v, m) in &model {
        if m.flags.contains(PageFlags::USER) {
            assert!(walk(&pt, v).iter().all(|e| e & PageFlags::USER.bits() != 0));
        }
    }

    // Unmapping everything returns every table but the root.
    assert!(!model.is_empty());
    let all: Vec<u64> = model.keys().copied().collect();
    for v in all {
        let (_, freed) = pt.unmap_and_prune(v).unwrap();
        frames.give_back(freed);
    }
    assert_eq!(frames.live.len(), 1);
    assert_eq!(pt.memory().tables.values().filter(|t| t.iter().any(|&e| e != 0)).count(), 0);
}

#[test]
fn mapped_walks_present_leaves_in_order() {
    let (mut pt, mut frames) = setup();
    let user = PageFlags::USER | RW;
    // Pages spread over different tables, and a 2 MiB page.
    let pages = [0x1_0000, 0x1_1000, 0x40_0000, 0x7fff_ffff_e000, 0x55_5555_4000];
    for (i, &v) in pages.iter().enumerate() {
        pt.map(v, 0x100_0000 + i as u64 * K4, Size4K, user, &mut frames).unwrap();
    }
    pt.map(0x4000_0000, 0x8000_0000, Size2M, user, &mut frames).unwrap();
    pt.map(HIGH, 0x9000_0000, Size4K, RW, &mut frames).unwrap();
    let mut seen = Vec::new();
    pt.mapped(0, u64::MAX, |v, t| seen.push((v, t.frame, t.size)));
    let want = [
        (0x1_0000, 0x100_0000, Size4K),
        (0x1_1000, 0x100_1000, Size4K),
        (0x40_0000, 0x100_2000, Size4K),
        (0x4000_0000, 0x8000_0000, Size2M),
        (0x55_5555_4000, 0x100_4000, Size4K),
        (0x7fff_ffff_e000, 0x100_3000, Size4K),
    ];
    assert_eq!(seen, want, "lower half only, in address order");
    seen.clear();
    pt.mapped(0x1_1000, 0x40_1000, |v, t| seen.push((v, t.frame, t.size)));
    assert_eq!(seen, want[1..3]);
    seen.clear();
    pt.mapped(0x4000_1000, 0x4040_0000, |v, t| seen.push((v, t.frame, t.size)));
    assert!(seen.is_empty(), "a huge page that starts before the range is not reported");
    let mut flags = Vec::new();
    pt.mapped(0x40_0000, 0x40_1000, |_, t| flags.push(t.flags));
    assert_eq!(flags, [tr(&pt, 0x40_0000).flags]);
    pt.mapped(0x50_0000, 0x40_0000, |_, _| panic!("an empty range visits nothing"));
}
