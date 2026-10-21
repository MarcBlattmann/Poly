//! An NT process's virtual memory: allocations (reserved address ranges)
//! and the state of every page in them, as `NtAllocateVirtualMemory`,
//! `NtFreeVirtualMemory`, `NtProtectVirtualMemory`, `NtQueryVirtualMemory`
//! and the section views see it.
//!
//! # The model (Learn: "Page State", "Reserving and Committing Memory",
//! "VirtualAlloc", "ZwAllocateVirtualMemory")
//!
//! - An *allocation* is a range reserved by one call: private memory, or a
//!   view of a section (`MEM_MAPPED`, `MEM_IMAGE`). It starts at a multiple
//!   of the allocation granularity, [`ALLOCATION_GRANULARITY`] (64 KiB),
//!   and is a whole number of [`PAGE_SIZE`] (4 KiB) pages (`SYSTEM_INFO`'s
//!   `dwAllocationGranularity` and `dwPageSize` on x64).
//! - Every page of an allocation is *reserved* (no storage, any access
//!   faults) or *committed* with a protection; pages outside allocations
//!   are *free*. `NtQueryVirtualMemory` reports runs of pages with the
//!   same state and protection.
//! - Reserving rounds a given base down to 64 KiB and the end up to a page
//!   ("the specified address is rounded down to the nearest multiple of the
//!   allocation granularity", VirtualAlloc); committing in a reservation
//!   rounds the range out to pages. Without a base the system picks the
//!   lowest free 64 KiB-aligned range, or the highest with `MEM_TOP_DOWN`.
//! - Committed pages read as zero until written; decommitting frees them
//!   (their contents are gone); releasing frees the whole allocation and
//!   must name its base.
//! - `PAGE_GUARD`: the first access to a guard page faults with
//!   `STATUS_GUARD_PAGE_VIOLATION` and turns the page's guard status off
//!   ("Creating Guard Pages"). A thread's stack grows that way: when its
//!   guard page fires, the page below becomes the new guard page and the
//!   thread goes on, until the last page of the reservation, which is never
//!   committed ("Thread Stack Size"); then `STATUS_STACK_OVERFLOW`.
//!
//! Each change goes to the kernel's page tables through [`Backend`] after
//! the model has accepted it. Where Learn does not say which status a
//! failure returns, the code picks one and says "to be confirmed by the
//! spec agent against real Windows"; ADR 0063 lists them.

use crate::backend::{Backend, ViewSource};
use crate::consts::*;
use crate::protect::{HwProt, Protection};
use alloc::collections::BTreeMap;
use alloc::vec;
use alloc::vec::Vec;
use core::result::Result;
use poly_ntobj::status::*;
use poly_ntobj::{NtStatus, ObjectId};

/// `SYSTEM_INFO.dwPageSize` on x64.
pub const PAGE_SIZE: u64 = 0x1000;
/// `SYSTEM_INFO.dwAllocationGranularity`.
pub const ALLOCATION_GRANULARITY: u64 = 0x1_0000;
/// The lowest address an allocation may use: the first 64 KiB are never
/// mapped, so null pointers (and small offsets from them) always fault.
pub const LOWEST_ADDRESS: u64 = 0x1_0000;
/// One past the highest address an allocation may use. The kernel's user
/// space ends one page below 128 TiB; the last 64 KiB granule below that is
/// left unused so every allocation fits whole granules. To be confirmed by
/// the spec agent against real Windows (`lpMaximumApplicationAddress`).
pub const ADDRESS_END: u64 = 0x7FFF_FFFF_0000;

/// `STATUS_STACK_OVERFLOW` (`ntstatus.h`).
pub const STATUS_STACK_OVERFLOW: NtStatus = NtStatus(0xC000_00FD);

pub const fn page_down(a: u64) -> u64 {
    a & !(PAGE_SIZE - 1)
}

pub fn page_up(a: u64) -> Option<u64> {
    Some(a.checked_add(PAGE_SIZE - 1)? & !(PAGE_SIZE - 1))
}

const fn granule_down(a: u64) -> u64 {
    a & !(ALLOCATION_GRANULARITY - 1)
}

fn granule_up(a: u64) -> Option<u64> {
    Some(a.checked_add(ALLOCATION_GRANULARITY - 1)? & !(ALLOCATION_GRANULARITY - 1))
}

/// One page of an allocation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PageState {
    Reserved,
    Committed(Protection),
}

/// A view of a section, as the allocation that holds it remembers it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ViewInfo {
    /// The section's key in [`crate::system::NtSystem`].
    pub section: u64,
    /// The section object; the view holds a pointer reference on it.
    pub object: ObjectId,
    /// The kernel's id for the section's pages.
    pub backing: u64,
    /// An image view (`MEM_IMAGE`); otherwise `MEM_MAPPED`.
    pub image: bool,
    /// The section offset of the view's first byte.
    pub offset: u64,
    /// Mapped shared with the section's other views (else copy on write).
    pub shared: bool,
    /// The section's page protection, which limits the view's (`None` for
    /// images: their pages are private copies).
    pub max: Option<Protection>,
    /// The section is `SEC_RESERVE`: pages may be committed in the view.
    pub reserve: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AllocKind {
    Private,
    View(ViewInfo),
}

/// A reserved range.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Allocation {
    pub base: u64,
    /// Bytes, a multiple of the page size.
    pub size: u64,
    /// The protection given when it was reserved (`AllocationProtect`).
    pub protect: u32,
    pub kind: AllocKind,
    /// A thread's stack: its guard page grows it.
    pub stack: bool,
    pub pages: Vec<PageState>,
}

impl Allocation {
    pub fn end(&self) -> u64 {
        self.base + self.size
    }

    pub fn contains(&self, addr: u64) -> bool {
        (self.base..self.end()).contains(&addr)
    }

    fn index(&self, addr: u64) -> usize {
        ((addr - self.base) / PAGE_SIZE) as usize
    }

    /// `MEM_PRIVATE`, `MEM_MAPPED` or `MEM_IMAGE`.
    pub fn type_value(&self) -> u32 {
        match self.kind {
            AllocKind::Private => MEM_PRIVATE,
            AllocKind::View(v) if v.image => MEM_IMAGE,
            AllocKind::View(_) => MEM_MAPPED,
        }
    }

    pub fn view(&self) -> Option<ViewInfo> {
        match self.kind {
            AllocKind::View(v) => Some(v),
            AllocKind::Private => None,
        }
    }

    /// Whether the page tables map it shared (a data view) or private.
    fn shared(&self) -> bool {
        self.view().is_some_and(|v| v.shared)
    }

    /// The page at `addr`.
    pub fn page(&self, addr: u64) -> PageState {
        self.pages[self.index(addr)]
    }
}

/// `MEMORY_BASIC_INFORMATION` (`winnt.h`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct MemoryBasicInformation {
    pub base_address: u64,
    pub allocation_base: u64,
    pub allocation_protect: u32,
    pub region_size: u64,
    pub state: u32,
    pub protect: u32,
    pub type_: u32,
}

impl MemoryBasicInformation {
    /// The x64 layout (offsets in [`MEMORY_BASIC_INFORMATION_SIZE`]);
    /// `PartitionId` is 0.
    pub fn to_bytes(&self) -> [u8; MEMORY_BASIC_INFORMATION_SIZE] {
        let mut b = [0u8; MEMORY_BASIC_INFORMATION_SIZE];
        b[0..8].copy_from_slice(&self.base_address.to_le_bytes());
        b[8..16].copy_from_slice(&self.allocation_base.to_le_bytes());
        b[16..20].copy_from_slice(&self.allocation_protect.to_le_bytes());
        b[24..32].copy_from_slice(&self.region_size.to_le_bytes());
        b[32..36].copy_from_slice(&self.state.to_le_bytes());
        b[36..40].copy_from_slice(&self.protect.to_le_bytes());
        b[40..44].copy_from_slice(&self.type_.to_le_bytes());
        b
    }
}

/// What a fault on a page the kernel could not resolve means.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AccessFault {
    /// A stack's guard page fired and the stack grew: the thread goes on.
    /// The stack's committed part now starts at `limit` (the TEB's
    /// `StackLimit`).
    StackGrown { limit: u64 },
    /// An exception: `STATUS_GUARD_PAGE_VIOLATION` (the guard status is
    /// off now), `STATUS_STACK_OVERFLOW` or `STATUS_ACCESS_VIOLATION`.
    Exception(NtStatus),
}

/// A view to add to the address space ([`VirtualMemory::insert_view`]).
#[derive(Clone, Debug)]
pub struct NewView {
    /// The requested base (rounded down to 64 KiB), or 0 for anywhere.
    pub base: u64,
    pub size: u64,
    pub zero_bits: u64,
    pub top_down: bool,
    /// `AllocationProtect`.
    pub protect: u32,
    pub info: ViewInfo,
    pub pages: Vec<PageState>,
}

/// The address space of one NT process. See the module docs.
#[derive(Clone, Debug)]
pub struct VirtualMemory {
    lowest: u64,
    end: u64,
    allocations: BTreeMap<u64, Allocation>,
}

impl Default for VirtualMemory {
    fn default() -> Self {
        Self::new()
    }
}

/// The highest base (exclusive) `ZeroBits` allows. Learn
/// ("ZwAllocateVirtualMemory", "ZwMapViewOfSection"): "the number of
/// high-order address bits that must be zero in the base address", less
/// than 21. Counted from bit 63 of the address; 0 means no limit, larger
/// values are `STATUS_INVALID_PARAMETER_3`. Both readings are to be
/// confirmed by the spec agent against real Windows (bit masks, and
/// counting from bit 31, are not documented for x64).
pub fn zero_bits_limit(zero_bits: u64) -> Result<u64, NtStatus> {
    match zero_bits {
        0 => Ok(u64::MAX),
        1..=20 => Ok(1 << (64 - zero_bits)),
        _ => Err(STATUS_INVALID_PARAMETER_3),
    }
}

/// Call `f(first, count, key)` for each run of consecutive indices in
/// `range` whose `key` is the same `Some`.
fn runs<K: PartialEq + Copy>(
    range: core::ops::Range<usize>,
    mut key: impl FnMut(usize) -> Option<K>,
    mut f: impl FnMut(usize, usize, K),
) {
    let mut current: Option<(usize, K)> = None;
    for i in range.clone() {
        let k = key(i);
        match (current, k) {
            (Some((_, c)), Some(k)) if c == k => {}
            (Some((start, c)), _) => {
                f(start, i - start, c);
                current = k.map(|k| (i, k));
            }
            (None, k) => current = k.map(|k| (i, k)),
        }
    }
    if let Some((start, c)) = current {
        f(start, range.end - start, c);
    }
}

impl VirtualMemory {
    /// The whole user address space, from [`LOWEST_ADDRESS`] to
    /// [`ADDRESS_END`], free.
    pub fn new() -> VirtualMemory {
        Self::with_range(LOWEST_ADDRESS, ADDRESS_END)
    }

    /// An address space of `[lowest, end)` (granule-aligned).
    pub fn with_range(lowest: u64, end: u64) -> VirtualMemory {
        VirtualMemory { lowest, end, allocations: BTreeMap::new() }
    }

    pub fn end(&self) -> u64 {
        self.end
    }

    /// The allocation holding `addr`.
    pub fn allocation(&self, addr: u64) -> Option<&Allocation> {
        self.allocations.range(..=addr).next_back().map(|(_, a)| a).filter(|a| a.contains(addr))
    }

    fn allocation_mut(&mut self, addr: u64) -> Option<&mut Allocation> {
        self.allocations.range_mut(..=addr).next_back().map(|(_, a)| a).filter(|a| a.contains(addr))
    }

    /// Every allocation, by address.
    pub fn allocations(&self) -> impl Iterator<Item = &Allocation> {
        self.allocations.values()
    }

    fn is_free(&self, start: u64, end: u64) -> bool {
        self.allocations.range(..end).next_back().is_none_or(|(_, a)| a.end() <= start)
    }

    /// A free 64 KiB-aligned range of `size` bytes whose base is below
    /// `base_limit`: the lowest, or with `top_down` the highest.
    fn find_free(&self, size: u64, top_down: bool, base_limit: u64) -> Option<u64> {
        let mut gaps = Vec::new();
        let mut at = self.lowest;
        for a in self.allocations.values() {
            if a.base > at {
                gaps.push((at, a.base));
            }
            at = at.max(a.end());
        }
        if self.end > at {
            gaps.push((at, self.end));
        }
        let fit = |&(start, end): &(u64, u64)| -> Option<u64> {
            if top_down {
                let mut base = granule_down(end.checked_sub(size)?);
                if base_limit != u64::MAX {
                    base = base.min(granule_down(base_limit.checked_sub(1)?));
                }
                (base >= start).then_some(base)
            } else {
                let base = granule_up(start)?;
                (base.checked_add(size)? <= end && base < base_limit).then_some(base)
            }
        };
        if top_down { gaps.iter().rev().find_map(fit) } else { gaps.iter().find_map(fit) }
    }

    /// Where a new allocation of `size` bytes goes: at `base` (rounded down
    /// to 64 KiB, the end rounded up to a page) or anywhere. Returns its
    /// base and size.
    ///
    /// - A range outside user space: `STATUS_INVALID_PARAMETER_2`; a size
    ///   that overflows: `STATUS_INVALID_PARAMETER_4` (both to be confirmed
    ///   by the spec agent).
    /// - A range that is not free: `STATUS_CONFLICTING_ADDRESSES`.
    /// - No room: `STATUS_NO_MEMORY`.
    pub fn place(&self, base: u64, size: u64, top_down: bool, zero_bits: u64) -> Result<(u64, u64), NtStatus> {
        if base != 0 {
            let start = granule_down(base);
            let end = base.checked_add(size).and_then(page_up).ok_or(STATUS_INVALID_PARAMETER_4)?;
            if start < self.lowest || end > self.end {
                return Err(STATUS_INVALID_PARAMETER_2);
            }
            if !self.is_free(start, end) {
                return Err(STATUS_CONFLICTING_ADDRESSES);
            }
            return Ok((start, end - start));
        }
        let limit = zero_bits_limit(zero_bits)?;
        let size = page_up(size).ok_or(STATUS_INVALID_PARAMETER_4)?;
        let base = self.find_free(size, top_down, limit).ok_or(STATUS_NO_MEMORY)?;
        Ok((base, size))
    }

    /// `NtAllocateVirtualMemory` on this address space. Returns the base
    /// and size of what was reserved or committed (written back to the
    /// caller's `BaseAddress` and `RegionSize`).
    ///
    /// - `AllocationType` needs `MEM_COMMIT` and/or `MEM_RESERVE`, or
    ///   `MEM_RESET` / `MEM_RESET_UNDO` alone; unknown bits:
    ///   `STATUS_INVALID_PARAMETER_5`. `MEM_WRITE_WATCH`, `MEM_PHYSICAL` and
    ///   `MEM_LARGE_PAGES` are `STATUS_NOT_SUPPORTED` for now. Statuses to
    ///   be confirmed by the spec agent.
    /// - A size of 0: `STATUS_INVALID_PARAMETER_4` (to be confirmed).
    /// - `MEM_RESERVE`, or `MEM_COMMIT` without a base: a new allocation
    ///   ([`place`](Self::place)); with `MEM_COMMIT` all its pages are
    ///   committed.
    /// - `MEM_COMMIT` with a base: the pages from `BaseAddress` to
    ///   `BaseAddress + RegionSize` must lie in one allocation
    ///   (`STATUS_CONFLICTING_ADDRESSES` otherwise; to be confirmed).
    ///   Reserved pages are committed; committed ones keep their contents
    ///   ("an attempt to commit a page that is already committed does not
    ///   cause the function to fail", VirtualAlloc) and take the new
    ///   protection (to be confirmed). In a view only pages of a
    ///   `SEC_RESERVE` section can be committed (other views: success if
    ///   every page is committed, else `STATUS_CONFLICTING_ADDRESSES`; to
    ///   be confirmed).
    /// - `MEM_RESET` / `MEM_RESET_UNDO`: the range must be committed
    ///   (`STATUS_CONFLICTING_ADDRESSES` / `STATUS_NOT_COMMITTED`); the
    ///   contents are kept, which `MEM_RESET` allows.
    #[allow(clippy::too_many_arguments)]
    pub fn allocate(
        &mut self,
        b: &mut dyn Backend,
        base: u64,
        size: u64,
        zero_bits: u64,
        alloc_type: u32,
        protect: u32,
    ) -> Result<(u64, u64), NtStatus> {
        const KNOWN: u32 = MEM_COMMIT
            | MEM_RESERVE
            | MEM_RESET
            | MEM_RESET_UNDO
            | MEM_TOP_DOWN
            | MEM_WRITE_WATCH
            | MEM_PHYSICAL
            | MEM_LARGE_PAGES;
        if alloc_type & !KNOWN != 0 {
            return Err(STATUS_INVALID_PARAMETER_5);
        }
        if alloc_type & (MEM_RESET | MEM_RESET_UNDO) != 0 {
            let only_one = alloc_type == MEM_RESET || alloc_type == MEM_RESET_UNDO;
            if !only_one {
                return Err(STATUS_INVALID_PARAMETER_5);
            }
            return self.reset(base, size);
        }
        if alloc_type & (MEM_COMMIT | MEM_RESERVE) == 0 {
            return Err(STATUS_INVALID_PARAMETER_5);
        }
        if alloc_type & (MEM_WRITE_WATCH | MEM_PHYSICAL | MEM_LARGE_PAGES) != 0 {
            return Err(STATUS_NOT_SUPPORTED);
        }
        let prot = Protection::new(protect, true)?;
        if size == 0 {
            return Err(STATUS_INVALID_PARAMETER_4);
        }
        let commit = alloc_type & MEM_COMMIT != 0;
        if alloc_type & MEM_RESERVE != 0 || base == 0 {
            let (start, len) = self.place(base, size, alloc_type & MEM_TOP_DOWN != 0, zero_bits)?;
            let state = if commit { PageState::Committed(prot) } else { PageState::Reserved };
            let pages = vec![state; (len / PAGE_SIZE) as usize];
            let allocation =
                Allocation { base: start, size: len, protect, kind: AllocKind::Private, stack: false, pages };
            if commit {
                b.commit(start, len, prot.hw(false))?;
            }
            self.allocations.insert(start, allocation);
            return Ok((start, len));
        }
        let (start, end) = self.page_range(base, size)?;
        self.commit_range(b, start, end, prot)
    }

    /// The pages `[page_down(base), page_up(base + size))`, which must lie in
    /// one allocation.
    fn page_range(&self, base: u64, size: u64) -> Result<(u64, u64), NtStatus> {
        let start = page_down(base);
        let end = base.checked_add(size).and_then(page_up).ok_or(STATUS_INVALID_PARAMETER_4)?;
        let a = self.allocation(start).ok_or(STATUS_CONFLICTING_ADDRESSES)?;
        if end > a.end() {
            return Err(STATUS_CONFLICTING_ADDRESSES);
        }
        Ok((start, end))
    }

    fn reset(&self, base: u64, size: u64) -> Result<(u64, u64), NtStatus> {
        if size == 0 {
            return Err(STATUS_INVALID_PARAMETER_4);
        }
        let (start, end) = self.page_range(base, size)?;
        let a = self.allocation(start).expect("checked by page_range");
        if (a.index(start)..a.index(end - 1) + 1).any(|i| a.pages[i] == PageState::Reserved) {
            return Err(STATUS_NOT_COMMITTED);
        }
        Ok((start, end - start))
    }

    fn commit_range(
        &mut self,
        b: &mut dyn Backend,
        start: u64,
        end: u64,
        prot: Protection,
    ) -> Result<(u64, u64), NtStatus> {
        let a = self.allocation(start).expect("checked by page_range");
        let (first, last) = (a.index(start), a.index(end - 1) + 1);
        let base = a.base;
        match a.kind {
            AllocKind::Private => {
                // Fresh pages first: if the kernel runs out of memory nothing
                // has changed but the pages committed so far, which stay.
                let pages = a.pages.clone();
                let mut result = Ok(());
                runs(
                    first..last,
                    |i| (pages[i] == PageState::Reserved).then_some(()),
                    |i, n, ()| {
                        if result.is_ok() {
                            let at = base + i as u64 * PAGE_SIZE;
                            result = b.commit(at, n as u64 * PAGE_SIZE, prot.hw(false));
                            if result.is_ok() {
                                self.allocation_mut(base).expect("live").pages[i..i + n]
                                    .fill(PageState::Committed(prot));
                            }
                        }
                    },
                );
                result?;
                runs(
                    first..last,
                    |i| (pages[i] != PageState::Reserved && pages[i] != PageState::Committed(prot)).then_some(()),
                    |i, n, ()| {
                        b.protect(base + i as u64 * PAGE_SIZE, n as u64 * PAGE_SIZE, prot.hw(false));
                    },
                );
                self.allocation_mut(base).expect("live").pages[first..last].fill(PageState::Committed(prot));
            }
            AllocKind::View(v) if v.reserve && !v.image => {
                if v.max.is_some_and(|max| !crate::protect::view_allowed(max, prot)) {
                    return Err(STATUS_SECTION_PROTECTION);
                }
                let offset = v.offset + (start - base);
                b.commit_section(v.backing, offset, end - start);
                b.protect(start, end - start, prot.hw(v.shared));
                self.allocation_mut(base).expect("live").pages[first..last].fill(PageState::Committed(prot));
                self.section_committed(v.section, base, offset, end - start);
            }
            AllocKind::View(_) => {
                if a.pages[first..last].contains(&PageState::Reserved) {
                    return Err(STATUS_CONFLICTING_ADDRESSES);
                }
            }
        }
        Ok((start, end - start))
    }

    /// Pages `[offset, offset + len)` of section `section` were committed
    /// through the view at `except`: the process's other views of it see
    /// them committed too, with their own protection.
    fn section_committed(&mut self, section: u64, except: u64, offset: u64, len: u64) {
        for a in self.allocations.values_mut() {
            let Some(v) = a.view() else { continue };
            if v.section != section || a.base == except {
                continue;
            }
            let (lo, hi) = (offset.max(v.offset), (offset + len).min(v.offset + a.size));
            if lo >= hi {
                continue;
            }
            let Ok(prot) = Protection::new(a.protect, false) else { continue };
            let (first, last) = (((lo - v.offset) / PAGE_SIZE) as usize, ((hi - v.offset) / PAGE_SIZE) as usize);
            for p in &mut a.pages[first..last] {
                if *p == PageState::Reserved {
                    *p = PageState::Committed(prot);
                }
            }
        }
    }

    /// `NtFreeVirtualMemory` on this address space. Returns the base and
    /// size freed. Private memory only: a view is
    /// `STATUS_UNABLE_TO_DELETE_SECTION` (unmap it instead; to be
    /// confirmed by the spec agent).
    ///
    /// - `FreeType` is `MEM_RELEASE` or `MEM_DECOMMIT`, not both and
    ///   nothing else (`STATUS_INVALID_PARAMETER_4`; to be confirmed).
    /// - Nothing allocated at `BaseAddress`: `STATUS_MEMORY_NOT_ALLOCATED`.
    /// - `MEM_RELEASE` with a size of 0 frees the whole allocation, whose
    ///   base the address must be (`STATUS_FREE_VM_NOT_AT_BASE`); with a
    ///   size, the range must be the whole allocation
    ///   (`STATUS_UNABLE_TO_FREE_VM`; releasing part of one is to be
    ///   confirmed by the spec agent).
    /// - `MEM_DECOMMIT` with a size of 0 decommits from the address to the
    ///   end of the allocation; otherwise the pages of the range, which
    ///   must lie in it (`STATUS_UNABLE_TO_FREE_VM`). Reserved pages in the
    ///   range are fine.
    pub fn free(&mut self, b: &mut dyn Backend, base: u64, size: u64, free_type: u32) -> Result<(u64, u64), NtStatus> {
        if free_type != MEM_RELEASE && free_type != MEM_DECOMMIT {
            return Err(STATUS_INVALID_PARAMETER_4);
        }
        let start = page_down(base);
        let a = self.allocation(start).ok_or(STATUS_MEMORY_NOT_ALLOCATED)?;
        if a.kind != AllocKind::Private {
            return Err(STATUS_UNABLE_TO_DELETE_SECTION);
        }
        let (abase, aend) = (a.base, a.end());
        let end = if size == 0 {
            aend
        } else {
            base.checked_add(size).and_then(page_up).ok_or(STATUS_INVALID_PARAMETER_3)?
        };
        if free_type == MEM_RELEASE {
            if size == 0 && start != abase {
                return Err(STATUS_FREE_VM_NOT_AT_BASE);
            }
            if start != abase || end != aend {
                return Err(STATUS_UNABLE_TO_FREE_VM);
            }
            b.release(abase, aend - abase);
            self.allocations.remove(&abase);
            return Ok((abase, aend - abase));
        }
        if end > aend {
            return Err(STATUS_UNABLE_TO_FREE_VM);
        }
        b.release(start, end - start);
        let a = self.allocation_mut(start).expect("checked above");
        let (first, last) = (a.index(start), a.index(end - 1) + 1);
        a.pages[first..last].fill(PageState::Reserved);
        Ok((start, end - start))
    }

    /// `NtProtectVirtualMemory` on this address space: returns the base and
    /// size changed and the old protection of the first page.
    ///
    /// - The pages of `[BaseAddress, BaseAddress + RegionSize)` (one page
    ///   for a size of 0; to be confirmed) must lie in one allocation
    ///   (`STATUS_CONFLICTING_ADDRESSES`) and all be committed
    ///   (`STATUS_NOT_COMMITTED`): "all pages in the specified region must
    ///   be within the same reserved region ... the pages cannot span
    ///   adjacent reserved regions" and must be committed (VirtualProtect).
    ///   Statuses to be confirmed by the spec agent.
    /// - Private memory refuses the copy-on-write protections; a data view
    ///   allows what its section's protection allows
    ///   (`STATUS_SECTION_PROTECTION`); an image view allows anything (its
    ///   pages are private copies).
    pub fn protect(
        &mut self,
        b: &mut dyn Backend,
        base: u64,
        size: u64,
        new: u32,
    ) -> Result<(u64, u64, u32), NtStatus> {
        let start = page_down(base);
        let end = if size == 0 {
            start + PAGE_SIZE
        } else {
            base.checked_add(size).and_then(page_up).ok_or(STATUS_INVALID_PARAMETER_3)?
        };
        let a = self.allocation(start).ok_or(STATUS_CONFLICTING_ADDRESSES)?;
        if end > a.end() {
            return Err(STATUS_CONFLICTING_ADDRESSES);
        }
        let prot = Protection::new(new, a.kind == AllocKind::Private)?;
        if let Some(v) = a.view()
            && let Some(max) = v.max
            && !crate::protect::view_allowed(max, prot)
        {
            return Err(STATUS_SECTION_PROTECTION);
        }
        let (first, last) = (a.index(start), a.index(end - 1) + 1);
        let old = match a.pages[first] {
            PageState::Committed(p) => p.value(),
            PageState::Reserved => return Err(STATUS_NOT_COMMITTED),
        };
        if a.pages[first..last].contains(&PageState::Reserved) {
            return Err(STATUS_NOT_COMMITTED);
        }
        let shared = a.shared();
        b.protect(start, end - start, prot.hw(shared));
        self.allocation_mut(start).expect("checked").pages[first..last].fill(PageState::Committed(prot));
        Ok((start, end - start, old))
    }

    /// `NtQueryVirtualMemory(MemoryBasicInformation)` for `addr`: the run of
    /// pages from `addr`'s page on with the same state, protection and
    /// allocation. Free memory reports `MEM_FREE`, `PAGE_NOACCESS` and an
    /// allocation base, protection and type of 0 ("undefined" per
    /// VirtualQuery; to be confirmed by the spec agent), and a reserved
    /// page a protection of 0. An address at or above the end of user
    /// space is `STATUS_INVALID_PARAMETER_2` (to be confirmed).
    pub fn query(&self, addr: u64) -> Result<MemoryBasicInformation, NtStatus> {
        if addr >= self.end {
            return Err(STATUS_INVALID_PARAMETER_2);
        }
        let page = page_down(addr);
        let Some(a) = self.allocation(page) else {
            let next = self.allocations.range(page..).next().map_or(self.end, |(&b, _)| b);
            return Ok(MemoryBasicInformation {
                base_address: page,
                region_size: next - page,
                state: MEM_FREE,
                protect: PAGE_NOACCESS,
                ..Default::default()
            });
        };
        let i = a.index(page);
        let state = a.pages[i];
        let n = a.pages[i..].iter().take_while(|p| **p == state).count();
        let (state, protect) = match state {
            PageState::Reserved => (MEM_RESERVE, 0),
            PageState::Committed(p) => (MEM_COMMIT, p.value()),
        };
        Ok(MemoryBasicInformation {
            base_address: page,
            allocation_base: a.base,
            allocation_protect: a.protect,
            region_size: n as u64 * PAGE_SIZE,
            state,
            protect,
            type_: a.type_value(),
        })
    }

    /// A fault the kernel's memory map could not resolve at `addr` (the
    /// page is not mapped, or not with the rights the access needs). See
    /// [`AccessFault`]. A guard page loses its guard status first and is
    /// mapped with its protection.
    pub fn access_fault(&mut self, b: &mut dyn Backend, addr: u64) -> AccessFault {
        let page = page_down(addr);
        let Some(a) = self.allocation_mut(page) else {
            return AccessFault::Exception(STATUS_ACCESS_VIOLATION);
        };
        let i = a.index(page);
        let PageState::Committed(p) = a.pages[i] else {
            return AccessFault::Exception(STATUS_ACCESS_VIOLATION);
        };
        if !p.is_guard() {
            return AccessFault::Exception(STATUS_ACCESS_VIOLATION);
        }
        let unguarded = p.without_guard();
        a.pages[i] = PageState::Committed(unguarded);
        let shared = a.shared();
        let (stack, base) = (a.stack, a.base);
        b.protect(page, PAGE_SIZE, unguarded.hw(shared));
        if !stack {
            return AccessFault::Exception(STATUS_GUARD_PAGE_VIOLATION);
        }
        // The last page of a stack's reservation is never committed.
        if i < 2 {
            return AccessFault::Exception(STATUS_STACK_OVERFLOW);
        }
        let below = page - PAGE_SIZE;
        let a = self.allocation_mut(base).expect("live");
        if a.pages[i - 1] == PageState::Reserved {
            if b.commit(below, PAGE_SIZE, HwProt::NONE).is_err() {
                return AccessFault::Exception(STATUS_STACK_OVERFLOW);
            }
            a.pages[i - 1] = PageState::Committed(Protection::new(unguarded.value() | PAGE_GUARD, true).unwrap_or(p));
        }
        AccessFault::StackGrown { limit: page }
    }

    /// Mark the private allocation at `base` as a thread's stack.
    pub fn set_stack(&mut self, base: u64) {
        if let Some(a) = self.allocations.get_mut(&base) {
            a.stack = true;
        }
    }

    /// Put a view in the address space (the caller maps its pages next,
    /// and removes it again with [`remove`](Self::remove) if that fails).
    /// Returns its base. Placement as for [`place`](Self::place).
    pub fn insert_view(&mut self, v: NewView) -> Result<u64, NtStatus> {
        let (base, size) = self.place(v.base, v.size, v.top_down, v.zero_bits)?;
        if size != v.size || v.pages.len() as u64 * PAGE_SIZE != size {
            // A base that is not 64 KiB-aligned would shift the view.
            return Err(STATUS_MAPPED_ALIGNMENT);
        }
        let a =
            Allocation { base, size, protect: v.protect, kind: AllocKind::View(v.info), stack: false, pages: v.pages };
        self.allocations.insert(base, a);
        Ok(base)
    }

    /// Take the view holding `addr` out of the address space
    /// (`NtUnmapViewOfSection` takes "any virtual address within the
    /// view", Learn). Not a view: `STATUS_NOT_MAPPED_VIEW`. The caller
    /// releases its pages and its section reference.
    pub fn remove_view(&mut self, addr: u64) -> Result<Allocation, NtStatus> {
        let a = self.allocation(addr).ok_or(STATUS_NOT_MAPPED_VIEW)?;
        if a.view().is_none() {
            return Err(STATUS_NOT_MAPPED_VIEW);
        }
        let base = a.base;
        Ok(self.allocations.remove(&base).expect("live"))
    }

    /// Take the allocation at `base` out (a view whose mapping failed).
    pub fn remove(&mut self, base: u64) -> Option<Allocation> {
        self.allocations.remove(&base)
    }

    /// Take every allocation out (the process ends).
    pub fn clear(&mut self) -> Vec<Allocation> {
        core::mem::take(&mut self.allocations).into_values().collect()
    }
}

/// Map the pages of view `a` (just inserted) in runs of equal protection.
/// Reserved pages of a `SEC_RESERVE` view are mapped with the view's
/// protection; the kernel refuses to fault them in until they are
/// committed.
pub fn map_view_pages(b: &mut dyn Backend, a: &Allocation, source: &ViewSource) -> Result<(), NtStatus> {
    let v = a.view().expect("a view");
    let fallback = Protection::new(a.protect, false).unwrap_or(Protection::new(PAGE_NOACCESS, false).expect("valid"));
    let mut result = Ok(());
    runs(
        0..a.pages.len(),
        |i| {
            Some(match a.pages[i] {
                PageState::Committed(p) => p.hw(v.shared),
                PageState::Reserved => fallback.hw(v.shared),
            })
        },
        |i, n, prot| {
            if result.is_err() {
                return;
            }
            let at = i as u64 * PAGE_SIZE;
            let map = crate::backend::ViewMap { source: source.clone(), offset: v.offset + at, prot, shared: v.shared };
            result = b.map_view(a.base + at, n as u64 * PAGE_SIZE, &map);
        },
    );
    if result.is_err() {
        b.release(a.base, a.size);
    }
    result
}

#[cfg(test)]
mod tests;
