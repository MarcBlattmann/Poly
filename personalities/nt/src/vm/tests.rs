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
