//! Model-level tests of [`VirtualMemory`] with a backend that only counts.

use super::*;
use crate::backend::{SectionSource, ViewMap};

#[derive(Default)]
struct Count {
    committed: u64,
    released: u64,
    protects: Vec<(u64, u64, HwProt)>,
}

impl Backend for Count {
    fn commit(&mut self, _: u64, len: u64, _: HwProt) -> Result<(), NtStatus> {
        self.committed += len;
        Ok(())
    }
    fn release(&mut self, _: u64, len: u64) {
        self.released += len;
    }
    fn protect(&mut self, addr: u64, len: u64, prot: HwProt) {
        self.protects.push((addr, len, prot));
    }
    fn map_view(&mut self, _: u64, _: u64, _: &ViewMap) -> Result<(), NtStatus> {
        Ok(())
    }
    fn write(&mut self, _: u64, _: &[u8]) -> Result<(), NtStatus> {
        Ok(())
    }
    fn create_section(&mut self, _: &SectionSource) -> Result<u64, NtStatus> {
        Ok(1)
    }
    fn commit_section(&mut self, _: u64, _: u64, _: u64) {}
    fn delete_section(&mut self, _: u64) {}
    fn file_size(&mut self, _: u64) -> Option<u64> {
        None
    }
    fn read_file(&mut self, _: u64) -> Result<Vec<u8>, NtStatus> {
        Err(STATUS_UNSUCCESSFUL)
    }
}

const RW: u32 = PAGE_READWRITE;

#[test]
fn zero_bits() {
    assert_eq!(zero_bits_limit(0), Ok(u64::MAX));
    assert_eq!(zero_bits_limit(17), Ok(1 << 47));
    assert_eq!(zero_bits_limit(20), Ok(1 << 44));
    assert_eq!(zero_bits_limit(21), Err(STATUS_INVALID_PARAMETER_3));
}

#[test]
fn only_new_pages_are_committed_and_changed_ones_reprotected() {
    let mut vm = VirtualMemory::new();
    let mut b = Count::default();
    let (base, _) = vm.allocate(&mut b, 0, 0x4000, 0, MEM_RESERVE, RW).unwrap();
    vm.allocate(&mut b, base + 0x1000, 0x1000, 0, MEM_COMMIT, RW).unwrap();
    assert_eq!(b.committed, 0x1000);
    // Commit all four: three are new, one changes protection.
    vm.allocate(&mut b, base, 0x4000, 0, MEM_COMMIT, PAGE_READONLY).unwrap();
    assert_eq!(b.committed, 0x4000);
    assert_eq!(b.protects, [(base + 0x1000, 0x1000, HwProt::R)]);
    let q = vm.query(base).unwrap();
    assert_eq!((q.region_size, q.protect), (0x4000, PAGE_READONLY));
    // Decommit with size 0 from inside: to the end of the allocation.
    assert_eq!(vm.free(&mut b, base + 0x2000, 0, MEM_DECOMMIT), Ok((base + 0x2000, 0x2000)));
    assert_eq!(b.released, 0x2000);
    assert_eq!(vm.query(base + 0x2000).unwrap().state, MEM_RESERVE);
}

#[test]
fn allocations_never_share_a_granule() {
    let mut vm = VirtualMemory::new();
    let mut b = Count::default();
    let (a, n) = vm.allocate(&mut b, 0, 0x1000, 0, MEM_RESERVE, RW).unwrap();
    let (c, _) = vm.allocate(&mut b, 0, 0x1000, 0, MEM_RESERVE, RW).unwrap();
    assert_eq!((a, n), (LOWEST_ADDRESS, 0x1000));
    assert_eq!(c, a + ALLOCATION_GRANULARITY);
    // A fixed request in the free tail of a's granule rounds down onto a.
    assert_eq!(vm.allocate(&mut b, a + 0x2000, 0x1000, 0, MEM_RESERVE, RW), Err(STATUS_CONFLICTING_ADDRESSES));
    assert_eq!(vm.query(a + 0x2000).unwrap().state, MEM_FREE);
}

#[test]
fn space_runs_out() {
    let mut vm = VirtualMemory::with_range(0x1_0000, 0x3_0000);
    let mut b = Count::default();
    vm.allocate(&mut b, 0, 0x1_0000, 0, MEM_RESERVE, RW).unwrap();
    vm.allocate(&mut b, 0, 0x8000, 0, MEM_RESERVE | MEM_TOP_DOWN, RW).unwrap();
    assert_eq!(vm.allocate(&mut b, 0, 0x1000, 0, MEM_RESERVE, RW), Err(STATUS_NO_MEMORY));
}

#[test]
fn guard_pages_outside_stacks_fire_once() {
    let mut vm = VirtualMemory::new();
    let mut b = Count::default();
    let (base, _) = vm.allocate(&mut b, 0, 0x2000, 0, MEM_COMMIT, RW | PAGE_GUARD).unwrap();
    assert_eq!(vm.access_fault(&mut b, base + 0x1000), AccessFault::Exception(STATUS_GUARD_PAGE_VIOLATION));
    assert_eq!(vm.access_fault(&mut b, base + 0x1000), AccessFault::Exception(STATUS_ACCESS_VIOLATION));
    let q = vm.query(base).unwrap();
    assert_eq!((q.protect, q.region_size), (RW | PAGE_GUARD, 0x1000));
    assert_eq!(vm.query(base + 0x1000).unwrap().protect, RW);
}

const GB: u64 = 1 << 30;

/// V8's sandbox on Windows (Chrome's renderer, ADR 0155 step 11): one large placeholder, split
/// into pieces, a piece replaced by private memory, freed back, and the pieces joined again.
#[test]
fn placeholders_split_replace_preserve_and_coalesce() {
    let mut vm = VirtualMemory::new();
    let mut b = Count::default();
    let kind = MEM_RESERVE | MEM_RESERVE_PLACEHOLDER;
    // Only PAGE_NOACCESS, only with MEM_RESERVE.
    assert_eq!(vm.allocate(&mut b, 0, 64 * GB, 0, kind, RW), Err(STATUS_INVALID_PAGE_PROTECTION));
    assert_eq!(
        vm.allocate(&mut b, 0, 64 * GB, 0, MEM_RESERVE_PLACEHOLDER, PAGE_NOACCESS),
        Err(STATUS_INVALID_PARAMETER_5)
    );
    assert_eq!(vm.allocate(&mut b, 0, 64 * GB, 0, kind | MEM_COMMIT, PAGE_NOACCESS), Err(STATUS_INVALID_PARAMETER_5));
    let (base, size) = vm.allocate(&mut b, 0, 64 * GB, 0, kind, PAGE_NOACCESS).unwrap();
    assert_eq!(size, 64 * GB);
    let q = vm.query(base).unwrap();
    assert_eq!((q.state, q.type_, q.allocation_base, q.region_size), (MEM_RESERVE, MEM_PRIVATE, base, 64 * GB));
    // Nothing is committed in a placeholder.
    assert_eq!(vm.allocate(&mut b, base, 0x1000, 0, MEM_COMMIT, RW), Err(STATUS_CONFLICTING_ADDRESSES));
    // Split off [base + 16 GB, base + 32 GB): three placeholders.
    let release = MEM_RELEASE | MEM_PRESERVE_PLACEHOLDER;
    assert_eq!(vm.free(&mut b, base + 16 * GB, 16 * GB, release), Ok((base + 16 * GB, 16 * GB)));
    assert_eq!(vm.query(base).unwrap().region_size, 16 * GB);
    assert_eq!(vm.query(base + 20 * GB).unwrap().allocation_base, base + 16 * GB);
    assert_eq!(vm.query(base + 40 * GB).unwrap().allocation_base, base + 32 * GB);
    // A split needs 64 KiB boundaries and stays inside the placeholder.
    assert_eq!(vm.free(&mut b, base + 0x1000, 0x1_0000, release), Err(STATUS_INVALID_PARAMETER_2));
    assert_eq!(vm.free(&mut b, base, 0x1000, release), Err(STATUS_INVALID_PARAMETER_3));
    assert_eq!(vm.free(&mut b, base, 17 * GB, release), Err(STATUS_CONFLICTING_ADDRESSES));
    // Replace the middle piece with private memory: exactly its range.
    let replace = MEM_RESERVE | MEM_REPLACE_PLACEHOLDER;
    assert_eq!(vm.allocate(&mut b, base + 16 * GB, GB, 0, replace, RW), Err(STATUS_CONFLICTING_ADDRESSES));
    assert_eq!(
        vm.allocate(&mut b, base + 16 * GB, 16 * GB, 0, replace | MEM_COMMIT, RW),
        Ok((base + 16 * GB, 16 * GB))
    );
    assert_eq!(b.committed, 16 * GB);
    assert_eq!(vm.query(base + 16 * GB).unwrap().state, MEM_COMMIT);
    // Pieces that are not all placeholders do not coalesce.
    let join = MEM_RELEASE | MEM_COALESCE_PLACEHOLDERS;
    assert_eq!(vm.free(&mut b, base, 64 * GB, join), Err(STATUS_CONFLICTING_ADDRESSES));
    // Freed back to the placeholder (part of it is not enough).
    assert_eq!(vm.free(&mut b, base + 16 * GB, GB, release), Err(STATUS_UNABLE_TO_FREE_VM));
    assert_eq!(vm.free(&mut b, base + 16 * GB, 16 * GB, release), Ok((base + 16 * GB, 16 * GB)));
    assert_eq!(b.released, 16 * GB);
    let q = vm.query(base + 16 * GB).unwrap();
    assert_eq!((q.state, q.allocation_protect, q.region_size), (MEM_RESERVE, PAGE_NOACCESS, 16 * GB));
    // The whole range must be named exactly.
    assert_eq!(vm.free(&mut b, base, 48 * GB, join), Err(STATUS_CONFLICTING_ADDRESSES));
    assert_eq!(vm.free(&mut b, base + GB, 63 * GB, join), Err(STATUS_CONFLICTING_ADDRESSES));
    assert_eq!(vm.free(&mut b, base, 64 * GB, join), Ok((base, 64 * GB)));
    assert_eq!(vm.query(base + 40 * GB).unwrap().allocation_base, base);
    // An ordinary allocation is neither split nor joined; a placeholder is released as usual.
    let (other, _) = vm.allocate(&mut b, 0, 0x2_0000, 0, MEM_RESERVE, RW).unwrap();
    assert_eq!(vm.free(&mut b, other, 0x1_0000, release), Err(STATUS_CONFLICTING_ADDRESSES));
    assert_eq!(vm.free(&mut b, base, 0, MEM_RELEASE), Ok((base, 64 * GB)));
    assert_eq!(vm.query(base).unwrap().state, MEM_FREE);
}

#[test]
fn committed_bytes_and_commit_charge() {
    let mut vm = VirtualMemory::new();
    let mut b = Count::default();
    assert_eq!(vm.private_committed_bytes(), 0);
    // A new allocation is charged whole; a reserve alone commits nothing.
    assert_eq!(vm.commit_charge(0, 0x3000, MEM_RESERVE | MEM_COMMIT), 0x3000);
    let (base, _) = vm.allocate(&mut b, 0, 0x10000, 0, MEM_RESERVE, RW).unwrap();
    assert_eq!(vm.private_committed_bytes(), 0);
    // Inside a reservation only the reserved pages of the range count.
    assert_eq!(vm.commit_charge(base, 0x4000, MEM_COMMIT), 0x4000);
    vm.allocate(&mut b, base, 0x2000, 0, MEM_COMMIT, RW).unwrap();
    assert_eq!(vm.private_committed_bytes(), 0x2000);
    assert_eq!(vm.commit_charge(base, 0x4000, MEM_COMMIT), 0x2000);
    assert_eq!(vm.commit_charge(base, 0x2000, MEM_COMMIT), 0);
    let (other, _) = vm.allocate(&mut b, 0, 0x5000, 0, MEM_RESERVE | MEM_COMMIT, RW).unwrap();
    assert_eq!(vm.private_committed_bytes(), 0x7000);
    vm.free(&mut b, other, 0, MEM_RELEASE).unwrap();
    assert_eq!(vm.private_committed_bytes(), 0x2000);
}

#[test]
fn an_address_space_wide_request_conflicts_where_a_small_one_has_no_memory() {
    let mut vm = VirtualMemory::new();
    let mut b = Count::default();
    // More than the user address space holds: the address range is the problem.
    assert_eq!(vm.allocate(&mut b, 0, (1 << 47) - 0x1_0000, 0, MEM_RESERVE, RW), Err(STATUS_CONFLICTING_ADDRESSES));
    // A big reservation that fits is fine.
    assert!(vm.allocate(&mut b, 0, 100 << 40, 0, MEM_RESERVE, RW).is_ok());
}

#[test]
fn a_copied_write_copy_page_reports_read_write() {
    let info = MemoryBasicInformation {
        base_address: 0x10000,
        allocation_base: 0x10000,
        allocation_protect: PAGE_WRITECOPY,
        region_size: 4 * PAGE_SIZE,
        state: MEM_COMMIT,
        protect: PAGE_WRITECOPY,
        type_: MEM_MAPPED,
    };
    // Nothing written: the run is as it was.
    let same = info.with_copied_pages(|_| false);
    assert_eq!((same.protect, same.region_size), (PAGE_WRITECOPY, 4 * PAGE_SIZE));
    // The first page was written: it is its own run, read-write; the rest stays write-copy.
    let first = info.with_copied_pages(|p| p == 0x10000);
    assert_eq!((first.protect, first.region_size), (PAGE_READWRITE, PAGE_SIZE));
    let second = MemoryBasicInformation { base_address: 0x11000, region_size: 3 * PAGE_SIZE, ..info }
        .with_copied_pages(|p| p == 0x10000);
    assert_eq!((second.protect, second.region_size), (PAGE_WRITECOPY, 3 * PAGE_SIZE));
    // An execute write-copy page becomes execute read-write.
    let x = MemoryBasicInformation { protect: PAGE_EXECUTE_WRITECOPY, type_: MEM_IMAGE, ..info }
        .with_copied_pages(|_| true);
    assert_eq!((x.protect, x.region_size), (PAGE_EXECUTE_READWRITE, 4 * PAGE_SIZE));
    // Private memory, free memory and other protections are not looked at.
    let private = MemoryBasicInformation { type_: MEM_PRIVATE, ..info }.with_copied_pages(|_| panic!("asked"));
    assert_eq!(private.protect, PAGE_WRITECOPY);
    let rw = MemoryBasicInformation { protect: PAGE_READWRITE, ..info }.with_copied_pages(|_| panic!("asked"));
    assert_eq!(rw.protect, PAGE_READWRITE);
}
