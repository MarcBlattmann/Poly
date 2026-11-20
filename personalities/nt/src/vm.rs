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
//! - A *placeholder* (Learn, "VirtualAlloc2", "VirtualFree", "MapViewOfFile3",
//!   "UnmapViewOfFile2") is a private reservation made with `MEM_RESERVE_PLACEHOLDER`
//!   (`PAGE_NOACCESS`, nothing can be committed in it). It can be split
//!   (`MEM_RELEASE | MEM_PRESERVE_PLACEHOLDER` on part of it), joined again with its neighbours
//!   (`MEM_RELEASE | MEM_COALESCE_PLACEHOLDERS` over their whole range), and replaced by a
//!   private allocation (`MEM_RESERVE | MEM_REPLACE_PLACEHOLDER`) or a view
//!   (`MEM_REPLACE_PLACEHOLDER`) of exactly its range, which `MEM_PRESERVE_PLACEHOLDER` turns
//!   back into the placeholder. See [`Placeholder`].
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
use crate::pagemap::PageMap;
use crate::protect::{HwProt, Protection};
use alloc::collections::BTreeMap;
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

/// How an allocation stands to placeholders (see the module docs).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Placeholder {
    /// An ordinary allocation.
    #[default]
    No,
    /// A placeholder: private, every page reserved, `PAGE_NOACCESS`.
    Is,
    /// A private allocation or a view that replaced a placeholder of exactly its range:
    /// `MEM_PRESERVE_PLACEHOLDER` makes it that placeholder again.
    Replaced,
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
    pub pages: PageMap,
    pub placeholder: Placeholder,
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
        self.pages.get(self.index(addr))
    }

    /// A placeholder of `[base, base + size)`.
    fn new_placeholder(base: u64, size: u64) -> Allocation {
        Allocation {
            base,
            size,
            protect: PAGE_NOACCESS,
            kind: AllocKind::Private,
            stack: false,
            pages: PageMap::new((size / PAGE_SIZE) as usize, PageState::Reserved),
            placeholder: Placeholder::Is,
        }
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

impl MemoryBasicInformation {
    /// A copy-on-write page that was written through its view is a private, read-write page from
    /// then on (the sheet of `NtMapViewOfSection`, rule `copy_on_write`): `written(addr)` says
    /// whether the page at `addr` was. The run stops where that changes.
    pub fn with_copied_pages(mut self, mut written: impl FnMut(u64) -> bool) -> Self {
        let rw = match self.protect {
            PAGE_WRITECOPY => PAGE_READWRITE,
            PAGE_EXECUTE_WRITECOPY => PAGE_EXECUTE_READWRITE,
            _ => return self,
        };
        if self.state != MEM_COMMIT || self.type_ == MEM_PRIVATE {
            return self;
        }
        let pages = self.region_size / PAGE_SIZE;
        let first = written(self.base_address);
        let mut n = 1;
        while n < pages && written(self.base_address + n * PAGE_SIZE) == first {
            n += 1;
        }
        self.region_size = n * PAGE_SIZE;
        if first {
            self.protect = rw;
        }
        self
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
    pub pages: PageMap,
    /// `MEM_REPLACE_PLACEHOLDER`: the view replaces the placeholder at `base` of exactly its size.
    pub replace: bool,
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

    /// `[addr, addr + len)` was written through the kernel (`NtWriteVirtualMemory`): every
    /// copy-on-write page it touches became a private copy and reports the writable protection
    /// (`PAGE_WRITECOPY` -> `PAGE_READWRITE`, `PAGE_EXECUTE_WRITECOPY` -> `PAGE_EXECUTE_READWRITE`),
    /// as on Windows (the sheet of `VirtualProtectEx`).
    pub fn mark_written(&mut self, addr: u64, len: u64) {
        if len == 0 {
            return;
        }
        let end = addr.saturating_add(len);
        let mut page = page_down(addr);
        while page < end {
            if let Some(a) = self.allocation_mut(page) {
                let i = a.index(page);
                if let PageState::Committed(p) = a.pages.get(i)
                    && p.is_copy_on_write()
                {
                    let writable = if p.base() == PAGE_WRITECOPY { PAGE_READWRITE } else { PAGE_EXECUTE_READWRITE };
                    let value = (p.value() & !0xFF) | writable;
                    if let Ok(q) = Protection::new(value, false) {
                        a.pages.fill(i..i + 1, PageState::Committed(q));
                    }
                }
            }
            page += PAGE_SIZE;
        }
    }

    /// Whether `[addr, addr + size)` lies in one view of an image section (`MEM_IMAGE`): what
    /// the prohibit-dynamic-code mitigation lets be executable, as Windows maps modules as
    /// image sections (ADR 0196).
    pub fn is_image_view(&self, addr: u64, size: u64) -> bool {
        let Some(a) = self.allocation(page_down(addr)) else { return false };
        a.type_value() == MEM_IMAGE && addr.checked_add(size).is_some_and(|end| end <= a.end())
    }

    fn allocation_mut(&mut self, addr: u64) -> Option<&mut Allocation> {
        self.allocations.range_mut(..=addr).next_back().map(|(_, a)| a).filter(|a| a.contains(addr))
    }

    /// Whether every allocation `[base, base + size)` touches is private memory, not a view or a
    /// placeholder (an empty range: the allocation at `base`): what a process's own memory calls may
    /// change under its lock alone (ADR 0262; views need the system's sections).
    pub fn only_private(&self, base: u64, size: u64) -> bool {
        let start = page_down(base);
        let end = if size == 0 {
            self.allocation(start).map_or(start.saturating_add(1), Allocation::end)
        } else {
            base.saturating_add(size)
        };
        let first = self.allocation(start).map_or(start, |a| a.base);
        self.allocations
            .range(first..end.max(first.saturating_add(1)))
            .all(|(_, a)| a.kind == AllocKind::Private && a.placeholder != Placeholder::Is)
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
        let base = self.find_free(size, top_down, limit).ok_or(if size > self.end - self.lowest {
            // More than the whole address space has room for (Windows: `ERROR_INVALID_ADDRESS`).
            STATUS_CONFLICTING_ADDRESSES
        } else {
            STATUS_NO_MEMORY
        })?;
        Ok((base, size))
    }

    /// The bytes of private memory committed in this address space (what job memory limits count).
    pub fn private_committed_bytes(&self) -> u64 {
        self.allocations
            .values()
            .filter(|a| matches!(a.kind, AllocKind::Private) && a.placeholder != Placeholder::Is)
            .map(|a| {
                let committed: usize = a
                    .pages
                    .runs_in(0..a.pages.len())
                    .filter(|(_, s)| matches!(s, PageState::Committed(_)))
                    .map(|(r, _)| r.len())
                    .sum();
                committed as u64 * PAGE_SIZE
            })
            .sum()
    }

    /// The committed bytes of private allocation pages that a `NtFreeVirtualMemory` of `base`,
    /// `size` would give back (`size` 0: to the end of the allocation; 0 outside one).
    pub fn committed_in(&self, base: u64, size: u64) -> u64 {
        let start = page_down(base);
        let Some(a) = self.allocation(start) else { return 0 };
        if a.kind != AllocKind::Private || a.placeholder == Placeholder::Is {
            return 0;
        }
        let end = if size == 0 { a.end() } else { base.saturating_add(size).min(a.end()) };
        if end <= start {
            return 0;
        }
        let committed: usize = a
            .pages
            .runs_in(a.index(start)..a.index(end - 1) + 1)
            .filter(|(_, s)| matches!(s, PageState::Committed(_)))
            .map(|(r, _)| r.len())
            .sum();
        committed as u64 * PAGE_SIZE
    }

    /// The bytes a `NtAllocateVirtualMemory` of `MEM_COMMIT` would newly commit: the whole size for
    /// a new allocation, the reserved pages of the range for a commit inside one.
    pub fn commit_charge(&self, base: u64, size: u64, alloc_type: u32) -> u64 {
        let whole = page_up(size).unwrap_or(u64::MAX);
        if alloc_type & MEM_RESERVE != 0 || base == 0 {
            return whole;
        }
        let Some(a) = self.allocation(page_down(base)) else { return 0 };
        let start = page_down(base);
        let end = base.saturating_add(size).min(a.end());
        if end <= start {
            return 0;
        }
        let reserved: usize = a
            .pages
            .runs_in(a.index(start)..a.index(end - 1) + 1)
            .filter(|(_, s)| matches!(s, PageState::Reserved))
            .map(|(r, _)| r.len())
            .sum();
        reserved as u64 * PAGE_SIZE
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
    /// - `MEM_RESERVE | MEM_RESERVE_PLACEHOLDER` (and `MEM_TOP_DOWN`), `PAGE_NOACCESS`: a new
    ///   placeholder (another type: `STATUS_INVALID_PARAMETER_5`, another protection:
    ///   `STATUS_INVALID_PAGE_PROTECTION`). `MEM_RESERVE | MEM_REPLACE_PLACEHOLDER` (and
    ///   `MEM_COMMIT`): the placeholder whose base and size are exactly the request's becomes a
    ///   private allocation (any other range: `STATUS_CONFLICTING_ADDRESSES`). Committing in a
    ///   placeholder is `STATUS_CONFLICTING_ADDRESSES`. Statuses to be confirmed by the spec
    ///   agent (Learn names the rules, not the statuses).
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
            | MEM_LARGE_PAGES
            | MEM_RESERVE_PLACEHOLDER
            | MEM_REPLACE_PLACEHOLDER;
        if alloc_type & !KNOWN != 0 {
            return Err(STATUS_INVALID_PARAMETER_5);
        }
        if alloc_type & MEM_RESERVE_PLACEHOLDER != 0 {
            if alloc_type & !(MEM_RESERVE | MEM_RESERVE_PLACEHOLDER | MEM_TOP_DOWN) != 0
                || alloc_type & MEM_RESERVE == 0
            {
                return Err(STATUS_INVALID_PARAMETER_5);
            }
            if protect != PAGE_NOACCESS {
                return Err(STATUS_INVALID_PAGE_PROTECTION);
            }
            if size == 0 {
                return Err(STATUS_INVALID_PARAMETER_4);
            }
            let (start, len) = self.place(base, size, alloc_type & MEM_TOP_DOWN != 0, zero_bits)?;
            self.allocations.insert(start, Allocation::new_placeholder(start, len));
            return Ok((start, len));
        }
        if alloc_type & MEM_REPLACE_PLACEHOLDER != 0 {
            if alloc_type & !(MEM_RESERVE | MEM_REPLACE_PLACEHOLDER | MEM_COMMIT) != 0 || alloc_type & MEM_RESERVE == 0
            {
                return Err(STATUS_INVALID_PARAMETER_5);
            }
            let prot = Protection::new(protect, true)?;
            let len = self
                .allocations
                .get(&base)
                .filter(|a| a.placeholder == Placeholder::Is && page_up(size) == Some(a.size))
                .map(|a| a.size)
                .ok_or(STATUS_CONFLICTING_ADDRESSES)?;
            let commit = alloc_type & MEM_COMMIT != 0;
            if commit {
                b.commit(base, len, prot.hw(false))?;
            }
            let state = if commit { PageState::Committed(prot) } else { PageState::Reserved };
            let a = self.allocations.get_mut(&base).expect("found above");
            a.placeholder = Placeholder::Replaced;
            a.protect = protect;
            a.pages = PageMap::new((len / PAGE_SIZE) as usize, state);
            return Ok((base, len));
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
            let pages = PageMap::new((len / PAGE_SIZE) as usize, state);
            let allocation = Allocation {
                base: start,
                size: len,
                protect,
                kind: AllocKind::Private,
                stack: false,
                pages,
                placeholder: Placeholder::No,
            };
            if commit {
                b.commit(start, len, prot.hw(false))?;
            }
            self.allocations.insert(start, allocation);
            return Ok((start, len));
        }
        let (start, end) = self.page_range(base, size)?;
        if self.allocation(start).is_some_and(|a| a.placeholder == Placeholder::Is) {
            return Err(STATUS_CONFLICTING_ADDRESSES);
        }
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
        if a.pages.contains(a.index(start)..a.index(end - 1) + 1, &PageState::Reserved) {
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
                // has changed but the pages committed so far, which stay. Only the
                // ranges are taken out of the map, not a copy of it (ADR 0250).
                let ranges = |keep: &dyn Fn(PageState) -> bool| -> Vec<core::ops::Range<usize>> {
                    a.pages.runs_in(first..last).filter(|&(_, s)| keep(s)).map(|(r, _)| r).collect()
                };
                let fresh = ranges(&|s| s == PageState::Reserved);
                let others = ranges(&|s| s != PageState::Reserved && s != PageState::Committed(prot));
                let mut result = Ok(());
                for r in fresh {
                    if result.is_ok() {
                        let at = base + r.start as u64 * PAGE_SIZE;
                        result = b.commit(at, r.len() as u64 * PAGE_SIZE, prot.hw(false));
                        if result.is_ok() {
                            self.allocation_mut(base).expect("live").pages.fill(r, PageState::Committed(prot));
                        }
                    }
                }
                result?;
                for r in others {
                    b.protect(base + r.start as u64 * PAGE_SIZE, r.len() as u64 * PAGE_SIZE, prot.hw(false));
                }
                self.allocation_mut(base).expect("live").pages.fill(first..last, PageState::Committed(prot));
            }
            AllocKind::View(v) if v.reserve && !v.image => {
                if v.max.is_some_and(|max| !crate::protect::view_allowed(max, prot)) {
                    return Err(STATUS_SECTION_PROTECTION);
                }
                let offset = v.offset + (start - base);
                b.commit_section(v.backing, offset, end - start);
                b.protect(start, end - start, prot.hw(v.shared));
                self.allocation_mut(base).expect("live").pages.fill(first..last, PageState::Committed(prot));
                self.section_committed(v.section, base, offset, end - start);
            }
            AllocKind::View(_) => {
                if a.pages.contains(first..last, &PageState::Reserved) {
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
            a.pages.replace(first..last, PageState::Reserved, PageState::Committed(prot));
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
    /// - `MEM_RELEASE | MEM_PRESERVE_PLACEHOLDER` ([`preserve`](Self::preserve)) and
    ///   `MEM_RELEASE | MEM_COALESCE_PLACEHOLDERS` ([`coalesce`](Self::coalesce)): placeholders.
    pub fn free(&mut self, b: &mut dyn Backend, base: u64, size: u64, free_type: u32) -> Result<(u64, u64), NtStatus> {
        match free_type {
            MEM_RELEASE | MEM_DECOMMIT => {}
            t if t == MEM_RELEASE | MEM_PRESERVE_PLACEHOLDER => return self.preserve(b, base, size),
            t if t == MEM_RELEASE | MEM_COALESCE_PLACEHOLDERS => return self.coalesce(base, size),
            _ => return Err(STATUS_INVALID_PARAMETER_4),
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
        a.pages.fill(first..last, PageState::Reserved);
        Ok((start, end - start))
    }

    /// `MEM_RELEASE | MEM_PRESERVE_PLACEHOLDER`. Learn ("VirtualFree"): on a placeholder it
    /// splits off `[base, base + size)` as a placeholder of its own (the parts before and after
    /// stay placeholders); on a private allocation that replaced a placeholder it frees the
    /// allocation back to that placeholder. A view is unmapped with `UnmapViewOfFile2` instead
    /// (`STATUS_UNABLE_TO_DELETE_SECTION`).
    ///
    /// Poly's readings, to be confirmed by the spec agent: a split is at the allocation
    /// granularity (`STATUS_INVALID_PARAMETER_2` / `_3` for a base / size that is not a multiple
    /// of 64 KiB), a size of 0 or the placeholder's whole range splits nothing and succeeds, a
    /// range past the placeholder is `STATUS_CONFLICTING_ADDRESSES`; a replaced allocation is
    /// freed whole (`STATUS_UNABLE_TO_FREE_VM` for part of it); any other allocation is
    /// `STATUS_CONFLICTING_ADDRESSES`.
    pub fn preserve(&mut self, b: &mut dyn Backend, base: u64, size: u64) -> Result<(u64, u64), NtStatus> {
        let a = self.allocation(page_down(base)).ok_or(STATUS_MEMORY_NOT_ALLOCATED)?;
        if a.kind != AllocKind::Private {
            return Err(STATUS_UNABLE_TO_DELETE_SECTION);
        }
        let (abase, aend) = (a.base, a.end());
        match a.placeholder {
            Placeholder::Is => {
                if !base.is_multiple_of(ALLOCATION_GRANULARITY) {
                    return Err(STATUS_INVALID_PARAMETER_2);
                }
                if !size.is_multiple_of(ALLOCATION_GRANULARITY) {
                    return Err(STATUS_INVALID_PARAMETER_3);
                }
                let end = if size == 0 { aend } else { base.checked_add(size).ok_or(STATUS_INVALID_PARAMETER_3)? };
                if end > aend {
                    return Err(STATUS_CONFLICTING_ADDRESSES);
                }
                self.allocations.remove(&abase);
                for (lo, hi) in [(abase, base), (base, end), (end, aend)] {
                    if hi > lo {
                        self.allocations.insert(lo, Allocation::new_placeholder(lo, hi - lo));
                    }
                }
                Ok((base, end - base))
            }
            Placeholder::Replaced => {
                let end = if size == 0 {
                    aend
                } else {
                    base.checked_add(size).and_then(page_up).ok_or(STATUS_INVALID_PARAMETER_3)?
                };
                if base != abase || end != aend {
                    return Err(STATUS_UNABLE_TO_FREE_VM);
                }
                b.release(abase, aend - abase);
                self.allocations.insert(abase, Allocation::new_placeholder(abase, aend - abase));
                Ok((abase, aend - abase))
            }
            Placeholder::No => Err(STATUS_CONFLICTING_ADDRESSES),
        }
    }

    /// `MEM_RELEASE | MEM_COALESCE_PLACEHOLDERS`. Learn ("VirtualFree"): "lpAddress and dwSize
    /// must exactly match the overall range of the placeholders to be merged": `[base, base +
    /// size)` must start at a placeholder's base, end at one's end, and hold nothing but
    /// placeholders with no gap between them; they become one. Anything else is
    /// `STATUS_CONFLICTING_ADDRESSES`, a size of 0 `STATUS_INVALID_PARAMETER_3` (statuses to be
    /// confirmed by the spec agent).
    pub fn coalesce(&mut self, base: u64, size: u64) -> Result<(u64, u64), NtStatus> {
        if size == 0 {
            return Err(STATUS_INVALID_PARAMETER_3);
        }
        let end = base.checked_add(size).and_then(page_up).ok_or(STATUS_INVALID_PARAMETER_3)?;
        let mut at = base;
        for a in self.allocations.range(base..end).map(|(_, a)| a) {
            if a.base != at || a.placeholder != Placeholder::Is || a.end() > end {
                return Err(STATUS_CONFLICTING_ADDRESSES);
            }
            at = a.end();
        }
        if at != end || !self.allocations.contains_key(&base) {
            return Err(STATUS_CONFLICTING_ADDRESSES);
        }
        let parts: Vec<u64> = self.allocations.range(base..end).map(|(&k, _)| k).collect();
        for k in parts {
            self.allocations.remove(&k);
        }
        self.allocations.insert(base, Allocation::new_placeholder(base, end - base));
        Ok((base, end - base))
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
        let mut prot = Protection::new(new, a.kind == AllocKind::Private)?;
        // A page of an image view that is made writable stays copy-on-write until it is written:
        // it reports `PAGE_WRITECOPY` (the sheet of `VirtualProtectEx`: `PAGE_READWRITE` on an
        // image page).
        if a.type_value() == MEM_IMAGE {
            let cow = match prot.base() {
                PAGE_READWRITE => Some(PAGE_WRITECOPY),
                PAGE_EXECUTE_READWRITE => Some(PAGE_EXECUTE_WRITECOPY),
                _ => None,
            };
            if let Some(cow) = cow
                && let Ok(q) = Protection::new((prot.value() & !0xFF) | cow, false)
            {
                prot = q;
            }
        }
        if let Some(v) = a.view()
            && let Some(max) = v.max
            && !crate::protect::view_allowed(max, prot)
        {
            return Err(STATUS_SECTION_PROTECTION);
        }
        let (first, last) = (a.index(start), a.index(end - 1) + 1);
        let old = match a.pages.get(first) {
            PageState::Committed(p) => p.value(),
            PageState::Reserved => return Err(STATUS_NOT_COMMITTED),
        };
        if a.pages.contains(first..last, &PageState::Reserved) {
            return Err(STATUS_NOT_COMMITTED);
        }
        let shared = a.shared();
        b.protect(start, end - start, prot.hw(shared));
        self.allocation_mut(start).expect("checked").pages.fill(first..last, PageState::Committed(prot));
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
        let state = a.pages.get(i);
        let n = a.pages.run_length_from(i);
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
        self.access_fault_in_stack(b, addr, None)
    }

    /// [`access_fault`](Self::access_fault) for a thread whose TEB says its
    /// stack ends at `stack_base` (`NT_TIB.StackBase`): a guard page below it,
    /// in the allocation that holds the stack, grows that stack as a stack
    /// Poly made does.
    pub fn access_fault_in_stack(&mut self, b: &mut dyn Backend, addr: u64, stack_base: Option<u64>) -> AccessFault {
        let page = page_down(addr);
        let Some(a) = self.allocation_mut(page) else {
            return AccessFault::Exception(STATUS_ACCESS_VIOLATION);
        };
        let i = a.index(page);
        // A function of an x64 program needs no stack probe for a frame of up to 8 KiB (Learn,
        // "_chkstk Routine": "for x64 compilers it is 8K"), so it may touch the page below a
        // stack's guard page without touching the guard page first: the stack grows over both,
        // as if the guard page had been touched (Chrome's video decoder threads do this).
        if a.pages.get(i) == PageState::Reserved
            && i + 1 < a.pages.len()
            && matches!(a.pages.get(i + 1), PageState::Committed(g) if g.is_guard())
        {
            let teb_stack = stack_base.is_some_and(|top| addr < top && top > a.base && a.contains(top - 1));
            if a.stack || teb_stack {
                let base = a.base;
                let above = page + PAGE_SIZE;
                if let AccessFault::Exception(e) = self.access_fault_in_stack(b, above, stack_base) {
                    return AccessFault::Exception(e);
                }
                // The guard page that growth put below `above` is this page: grow through it too.
                let a = self.allocation_mut(base).expect("live");
                if matches!(a.pages.get(a.index(page)), PageState::Committed(g) if g.is_guard()) {
                    return self.access_fault_in_stack(b, addr, stack_base);
                }
            }
        }
        let a = self.allocation_mut(page).expect("live");
        let i = a.index(page);
        let PageState::Committed(p) = a.pages.get(i) else {
            return AccessFault::Exception(STATUS_ACCESS_VIOLATION);
        };
        if !p.is_guard() {
            return AccessFault::Exception(STATUS_ACCESS_VIOLATION);
        }
        let unguarded = p.without_guard();
        a.pages.fill(i..i + 1, PageState::Committed(unguarded));
        let shared = a.shared();
        let teb_stack = stack_base.is_some_and(|top| addr < top && top > a.base && a.contains(top - 1));
        let (stack, base) = (a.stack || teb_stack, a.base);
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
        if a.pages.get(i - 1) == PageState::Reserved {
            if b.commit(below, PAGE_SIZE, HwProt::NONE).is_err() {
                return AccessFault::Exception(STATUS_STACK_OVERFLOW);
            }
            a.pages.fill(
                i - 1..i,
                PageState::Committed(Protection::new(unguarded.value() | PAGE_GUARD, true).unwrap_or(p)),
            );
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
    ///
    /// With `replace` (`MEM_REPLACE_PLACEHOLDER`, Learn "MapViewOfFile3") the view takes the place
    /// of the placeholder whose base and size are exactly its own (`STATUS_CONFLICTING_ADDRESSES`
    /// otherwise; to be confirmed by the spec agent).
    pub fn insert_view(&mut self, v: NewView) -> Result<u64, NtStatus> {
        let (base, size, placeholder) = if v.replace {
            let found = self.allocations.get(&v.base).filter(|a| a.placeholder == Placeholder::Is && a.size == v.size);
            if found.is_none() {
                return Err(STATUS_CONFLICTING_ADDRESSES);
            }
            (v.base, v.size, Placeholder::Replaced)
        } else {
            let (base, size) = self.place(v.base, v.size, v.top_down, v.zero_bits)?;
            (base, size, Placeholder::No)
        };
        if size != v.size || v.pages.len() as u64 * PAGE_SIZE != size {
            // A base that is not 64 KiB-aligned would shift the view.
            return Err(STATUS_MAPPED_ALIGNMENT);
        }
        let a = Allocation {
            base,
            size,
            protect: v.protect,
            kind: AllocKind::View(v.info),
            stack: false,
            pages: v.pages,
            placeholder,
        };
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

    /// Take the allocation at `base` out (a view whose mapping failed). A view that had replaced
    /// a placeholder leaves the placeholder behind.
    pub fn remove(&mut self, base: u64) -> Option<Allocation> {
        let a = self.allocations.remove(&base)?;
        if a.placeholder == Placeholder::Replaced {
            self.allocations.insert(a.base, Allocation::new_placeholder(a.base, a.size));
        }
        Some(a)
    }

    /// The view holding `addr` became the placeholder it replaced (`UnmapViewOfFile2` with
    /// `MEM_PRESERVE_PLACEHOLDER`, after [`remove_view`](Self::remove_view) took it out), or any
    /// unmapped view's range becomes a placeholder (Poly's `kernelbase` asks it right after the
    /// unmap). `STATUS_CONFLICTING_ADDRESSES` if the range is taken again.
    pub fn put_placeholder(&mut self, base: u64, size: u64) -> Result<(), NtStatus> {
        if !self.is_free(base, base + size) {
            return Err(STATUS_CONFLICTING_ADDRESSES);
        }
        self.allocations.insert(base, Allocation::new_placeholder(base, size));
        Ok(())
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
    // Runs of equal hardware protection (two page states can map alike).
    let mut pending: Option<(usize, usize, HwProt)> = None;
    let map_run = |b: &mut dyn Backend, (first, n, prot): (usize, usize, HwProt), result: &mut Result<(), NtStatus>| {
        if result.is_ok() {
            let at = first as u64 * PAGE_SIZE;
            let map = crate::backend::ViewMap { source: source.clone(), offset: v.offset + at, prot, shared: v.shared };
            *result = b.map_view(a.base + at, n as u64 * PAGE_SIZE, &map);
        }
    };
    for (range, state) in a.pages.runs_in(0..a.pages.len()) {
        let prot = match state {
            PageState::Committed(p) => p.hw(v.shared),
            PageState::Reserved => fallback.hw(v.shared),
        };
        pending = match pending {
            Some((first, n, p)) if p == prot => Some((first, n + range.len(), p)),
            Some(run) => {
                map_run(b, run, &mut result);
                Some((range.start, range.len(), prot))
            }
            None => Some((range.start, range.len(), prot)),
        };
    }
    if let Some(run) = pending {
        map_run(b, run, &mut result);
    }
    if result.is_err() {
        b.release(a.base, a.size);
    }
    result
}

#[cfg(test)]
mod tests;
