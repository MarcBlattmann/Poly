//! User address spaces: the virtual memory areas (VMAs) of one process, the
//! `mmap` / `munmap` / `mprotect` / `brk` bookkeeping on them, and the
//! decision what a user page fault means.
//!
//! Semantics follow the Linux man pages (`mmap(2)`, `munmap(2)`,
//! `mprotect(2)`, `brk(2)`); the NT memory services can be built on the same
//! regions later. Nothing here touches hardware. The split with the kernel
//! (decided in `docs/adr/0026-user-address-space-regions.md`):
//!
//! | This crate ([`VmMap`]) | The kernel |
//! |---|---|
//! | which ranges are mapped, with which protection and backing | page tables: which pages are present, with which permissions |
//! | where a new mapping goes (address search, `MAP_FIXED`, hints) | frames, frame reference counts (including the page cache's own reference) |
//! | splitting and merging regions, the map count limit | unmapping the pages of ranges this crate reports as removed, TLB flushes |
//! | the program break and the heap region | reading file pages, end-of-file (`SIGBUS`) |
//! | stack growth, the guard gap | delivering `SIGSEGV`, or `EFAULT` for a fault inside `copy_from_user` |
//! | what a fault means ([`FaultAction`]) | doing it, then returning to user space |
//!
//! Per-page state stays in the kernel's page tables: the crate never stores
//! anything per page. On a fault the kernel describes the faulting page in
//! one word ([`PageState`]: absent, or present with the reference count of
//! its frame) and [`VmMap::fault`] answers with an action. That keeps the
//! region map small (one entry per region, not per page) and leaves a single
//! source of truth for what is mapped.
//!
//! # Address space
//!
//! [`Layout`] bounds the address space. The defaults are [`USER_MIN_ADDR`]
//! (64 KiB) to [`USER_END`] (`0x7fff_ffff_f000`):
//!
//! - nothing is ever mapped below 64 KiB, so a null pointer plus a small
//!   offset always faults, in user space and in the kernel (Linux's usual
//!   `vm.mmap_min_addr`, and also the lowest address NT hands out);
//! - user space is the lower half of the 48-bit canonical address space,
//!   which ends at `0x8000_0000_0000`, minus its **last page**. A `syscall`
//!   instruction in the last page would return to a non-canonical address,
//!   and `sysret` to a non-canonical `rcx` faults in ring 0 on Intel CPUs
//!   with user-controlled registers. Keeping that page unmappable rules it
//!   out once and for all.
//!
//! The stack sits at the top ([`Layout::stack_top`]) and may grow down to
//! [`Layout::stack_max`] bytes. Mappings without a fixed address go
//! **top-down** below [`Layout::mmap_base`], which leaves at least 128 MiB
//! (or the stack limit, if larger) plus the guard gap for the stack, and the
//! area above the program image free for the heap to grow into. When nothing
//! fits below `mmap_base` the search takes the lowest hole anywhere.
//! [`Search::BottomUp`] gives the opposite order (the classic layout, with
//! `mmap_base` set low). Randomising the layout (ASLR) is the kernel's job:
//! it adjusts `stack_top`, `mmap_base` and the program break before use.
//!
//! ```text
//! USER_END  ┌──────────────┐
//!           │ stack        │ grows down to stack_top - stack_max
//!           │ guard gap    │ nothing is placed here; the stack never grows into a region
//! mmap_base ├──────────────┤
//!           │ mmap area    │ top-down search
//!           │      ↓       │
//!           │      ↑       │
//!           │ heap (brk)   │
//!           │ program      │
//! min_addr  └──────────────┘
//! ```
//!
//! # Regions
//!
//! A [`Vma`] is a page-aligned range with a protection ([`Prot`]), a ceiling
//! for that protection ([`Vma::max_prot`], which `mprotect` may not exceed:
//! a shared mapping of a read-only file must never become writable), a
//! [`Backing`] (private anonymous memory, or a file or memory object at an
//! offset, private or shared) and a [`Kind`] (ordinary, heap, stack).
//! Regions never overlap, and two neighbours that touch and agree on
//! everything (for files: the same file at continuing offsets) are always
//! merged, so the map is the smallest description of the address space.
//! Splits happen on partial unmaps and partial protection changes; every
//! operation re-merges what it touched. The map count limit
//! ([`Layout::max_map_count`]) is checked against the exact count after
//! merging, before anything changes: an operation that fails changes
//! nothing (Linux may leave a partial `mprotect` behind; this crate does not).
//!
//! Shared anonymous memory (`MAP_SHARED | MAP_ANONYMOUS`) is a shared
//! mapping of a memory object the kernel creates for it, passed as a
//! [`Backing::File`]: its pages must be found again by every process that
//! shares the mapping after `fork`, so they cannot live only in one
//! process's page tables.
//!
//! # Memory
//!
//! Regions live in a `BTreeMap` keyed by start address, so this crate needs
//! `alloc`. A process can have tens of thousands of regions (the default
//! limit is 65 530), so a fixed table would be either too small or far too
//! large for the common case of a few dozen; the kernel has a heap by the
//! time processes exist. Lookups are `O(log n)`. The free-space search walks
//! regions from `mmap_base` downwards and stops at the first hole that fits,
//! which is usually near the top; a fragmented space makes it linear in the
//! number of regions (an augmented tree with the largest gap per subtree
//! would fix that if it ever matters).
//!
//! # The kernel's side
//!
//! - **`mmap`**: build a [`MapRequest`], call [`VmMap::map`]; for every
//!   region in [`Mapped::replaced`] unmap its present pages, drop their frame
//!   references and the file handle, then flush the TLB for the range.
//! - **`munmap`**: [`VmMap::unmap`], then the same for the returned regions.
//! - **`mprotect`**: [`VmMap::protect`], then for every present page in the
//!   range set the permissions [`Vma::page_prot`] gives (with `refs`, the
//!   frame's reference count) and flush. Pages whose protection becomes
//!   [`Prot::NONE`] keep their frame behind a non-present entry, so that
//!   making them accessible again preserves their contents.
//! - **`brk`**: [`VmMap::brk`] returns the new break (the old one on failure,
//!   as the system call does) and the regions to unmap when shrinking.
//! - **Page fault**: see [`VmMap::fault`].

#![no_std]

extern crate alloc;

use alloc::collections::BTreeMap;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::fmt;
use core::ops::BitOr;

/// Size of a page. Every region starts and ends on a page boundary.
pub const PAGE_SIZE: u64 = 4096;

const PAGE_MASK: u64 = PAGE_SIZE - 1;

/// Lowest user address that can ever be mapped by default (64 KiB).
pub const USER_MIN_ADDR: u64 = 0x1_0000;

/// End (exclusive) of user space by default: the lower canonical half of a
/// 48-bit address space minus its last page (see the crate docs).
pub const USER_END: u64 = 0x7fff_ffff_f000;

/// Default stack size limit: 8 MiB, the usual `RLIMIT_STACK` soft limit.
pub const DEFAULT_STACK_MAX: u64 = 8 << 20;

/// Smallest distance between [`Layout::stack_top`] and the top of the mmap
/// area (before the guard gap), so a small stack limit set later (with
/// `setrlimit`) can still be raised without colliding with mappings.
pub const MIN_STACK_RESERVE: u64 = 128 << 20;

/// Default gap kept free below a stack region: 256 pages, the default of
/// Linux's documented `stack_guard_gap=` boot parameter.
pub const DEFAULT_STACK_GUARD_GAP: u64 = 256 * PAGE_SIZE;

/// Default limit on the number of regions: Linux's documented default for
/// the `vm.max_map_count` sysctl.
pub const DEFAULT_MAX_MAP_COUNT: usize = 65_530;

/// `addr` rounded down to a page boundary.
pub const fn page_down(addr: u64) -> u64 {
    addr & !PAGE_MASK
}

/// `addr` rounded up to a page boundary, or `None` if that overflows.
pub const fn page_up(addr: u64) -> Option<u64> {
    match addr.checked_add(PAGE_MASK) {
        Some(a) => Some(a & !PAGE_MASK),
        None => None,
    }
}

/// Whether `addr` is a multiple of [`PAGE_SIZE`].
pub const fn is_page_aligned(addr: u64) -> bool {
    addr & PAGE_MASK == 0
}

/// Page protection: any combination of read, write and execute.
///
/// The bit values are those of `PROT_READ`, `PROT_WRITE` and `PROT_EXEC` in
/// the Linux UAPI header `asm-generic/mman-common.h`, so a Linux `prot`
/// argument converts with [`Prot::from_bits`].
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Prot(u8);

impl Prot {
    /// No access (`PROT_NONE`).
    pub const NONE: Prot = Prot(0);
    /// `PROT_READ`.
    pub const READ: Prot = Prot(0x1);
    /// `PROT_WRITE`.
    pub const WRITE: Prot = Prot(0x2);
    /// `PROT_EXEC`.
    pub const EXEC: Prot = Prot(0x4);
    /// Read and write.
    pub const RW: Prot = Prot(0x3);
    /// Read and execute.
    pub const RX: Prot = Prot(0x5);
    /// Read, write and execute.
    pub const ALL: Prot = Prot(0x7);

    /// The protection with these `PROT_*` bits, or `None` if `bits` has any
    /// other bit set.
    pub const fn from_bits(bits: u64) -> Option<Prot> {
        if bits & !0x7 == 0 { Some(Prot(bits as u8)) } else { None }
    }

    /// The `PROT_*` bits.
    pub const fn bits(self) -> u8 {
        self.0
    }

    /// Whether every permission in `other` is also in `self`.
    pub const fn contains(self, other: Prot) -> bool {
        self.0 & other.0 == other.0
    }

    /// `self` without the permissions in `other`.
    pub const fn without(self, other: Prot) -> Prot {
        Prot(self.0 & !other.0)
    }

    /// Whether a page with this protection may be accessed this way.
    ///
    /// x86-64 page tables cannot express write-only or execute-only pages:
    /// every present user page is readable. So, as on Linux for x86-64, any
    /// protection other than [`Prot::NONE`] allows reads. Writes need
    /// [`Prot::WRITE`] and instruction fetches need [`Prot::EXEC`].
    pub const fn allows(self, access: Access) -> bool {
        match access {
            Access::Read => self.0 != 0,
            Access::Write => self.contains(Prot::WRITE),
            Access::Execute => self.contains(Prot::EXEC),
        }
    }
}

/// `prot` without write permission, for a page that must fault on its next
/// write. A writable page is readable on x86-64, so it stays readable.
fn write_protected(prot: Prot) -> Prot {
    if prot.contains(Prot::WRITE) { prot.without(Prot::WRITE) | Prot::READ } else { prot }
}

impl BitOr for Prot {
    type Output = Prot;

    fn bitor(self, rhs: Prot) -> Prot {
        Prot(self.0 | rhs.0)
    }
}

impl fmt::Debug for Prot {
    /// `rwx`, with `-` for a missing permission, as in `/proc/<pid>/maps`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let c = |p: Prot, ch: char| if self.contains(p) { ch } else { '-' };
        write!(f, "{}{}{}", c(Prot::READ, 'r'), c(Prot::WRITE, 'w'), c(Prot::EXEC, 'x'))
    }
}

/// How a page was accessed when it faulted. On x86-64 the page-fault error
/// code says it: bit 1 (W/R) is a write, bit 4 (I/D) an instruction fetch,
/// anything else a read.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Access {
    Read,
    Write,
    Execute,
}

/// What a region is for. Regions of different kinds never merge.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    /// Anything `mmap` creates, and the program image.
    Normal,
    /// The program break area that [`VmMap::brk`] grows and shrinks.
    Heap,
    /// The main thread's stack ([`VmMap::setup_stack`]): it grows down on a
    /// fault just below it, and keeps [`Layout::stack_guard_gap`] free
    /// below itself.
    Stack,
}

/// What is behind the pages of a region.
///
/// `F` is the kernel's file handle (for example a reference-counted pointer
/// to an open file or memory object). The crate clones it when a region is
/// split and hands it back in removed regions; it compares handles with
/// `==` to decide whether two neighbouring regions map the same file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Backing<F> {
    /// Private anonymous memory: zero-filled on first touch.
    Anonymous,
    /// A file or memory object; `offset` is the file offset of the region's
    /// first page.
    File {
        file: F,
        offset: u64,
        /// `MAP_SHARED`: writes go to the file and are seen by everyone who
        /// maps it. Otherwise `MAP_PRIVATE`: writes go to a private copy of
        /// the page (copy on write).
        shared: bool,
    },
}

/// One virtual memory area: `[start, end)`, page-aligned, non-empty.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Vma<F> {
    pub start: u64,
    pub end: u64,
    pub prot: Prot,
    /// The most `mprotect` may grant (`prot` is always contained in it).
    pub max_prot: Prot,
    pub kind: Kind,
    pub backing: Backing<F>,
    /// `MADV_WIPEONFORK` (madvise(2)): a child made by fork gets the range
    /// zero-filled instead of a copy. Private anonymous memory only.
    pub wipe_on_fork: bool,
    /// Locked in memory (mlock(2), mlock2(2), mlockall(2), `MAP_LOCKED`):
    /// counted in `VmLck` and against `RLIMIT_MEMLOCK`. Poly never pages
    /// memory out, so the flag is bookkeeping; populating a locked range is
    /// the kernel's job (unless the lock was made "on fault").
    pub locked: bool,
    /// Sealed by mseal(2): the region can no longer be unmapped, moved,
    /// resized, re-protected or replaced (`EPERM`) for the life of the
    /// address space. Inherited by a fork child (the map is copied).
    pub sealed: bool,
    /// Registered with a userfaultfd(2) object for missing-page faults:
    /// that object's id ([`VmMap::set_userfault`]), 0 for none. A fault on
    /// an absent page is then [`FaultAction::UserFault`]. Not inherited by
    /// a fork child (the kernel clears it, [`VmMap::clear_userfault`]).
    pub uffd: u32,
    /// The name prctl(2) `PR_SET_VMA_ANON_NAME` gave the region (anonymous
    /// memory only; `/proc/<pid>/maps` shows `[anon:<name>]`). Inherited
    /// by a fork child; regions with different names never merge.
    pub name: Option<Arc<str>>,
}

impl<F: Clone + PartialEq> Vma<F> {
    /// Size in bytes.
    pub fn size(&self) -> u64 {
        self.end - self.start
    }

    /// Whether `addr` lies inside the region.
    pub fn contains(&self, addr: u64) -> bool {
        self.start <= addr && addr < self.end
    }

    /// Whether writes reach a shared file or memory object.
    pub fn is_shared(&self) -> bool {
        matches!(self.backing, Backing::File { shared: true, .. })
    }

    /// The file offset that backs `addr`, for a file-backed region that
    /// contains it.
    pub fn file_offset(&self, addr: u64) -> Option<u64> {
        match self.backing {
            Backing::File { offset, .. } if self.contains(addr) => Some(offset + (addr - self.start)),
            _ => None,
        }
    }

    /// The permissions the page-table entry of a present page of this
    /// region gets, given the reference count of its frame.
    ///
    /// A private region's page whose frame is also referenced elsewhere
    /// (by another process after `fork`, or by the page cache) is mapped
    /// without write permission, so that a write faults and gets a copy;
    /// [`Prot::NONE`] means the page keeps its frame behind a non-present
    /// entry.
    pub fn page_prot(&self, refs: u32) -> Prot {
        if !self.is_shared() && refs > 1 { write_protected(self.prot) } else { self.prot }
    }

    /// The part `[start, end)` of this region (which must contain it).
    fn clip(&self, start: u64, end: u64) -> Vma<F> {
        debug_assert!(self.start <= start && start < end && end <= self.end);
        let backing = match &self.backing {
            Backing::Anonymous => Backing::Anonymous,
            Backing::File { file, offset, shared } => {
                Backing::File { file: file.clone(), offset: offset + (start - self.start), shared: *shared }
            }
        };
        Vma {
            start,
            end,
            prot: self.prot,
            max_prot: self.max_prot,
            kind: self.kind,
            backing,
            wipe_on_fork: self.wipe_on_fork,
            locked: self.locked,
            sealed: self.sealed,
            uffd: self.uffd,
            name: self.name.clone(),
        }
    }

    /// Whether `next` starts where `self` ends and the two describe one
    /// continuous region.
    fn can_merge(&self, next: &Vma<F>) -> bool {
        self.end == next.start
            && self.prot == next.prot
            && self.max_prot == next.max_prot
            && self.kind == next.kind
            && self.wipe_on_fork == next.wipe_on_fork
            && self.locked == next.locked
            && self.sealed == next.sealed
            && self.uffd == next.uffd
            && self.name == next.name
            && match (&self.backing, &next.backing) {
                (Backing::Anonymous, Backing::Anonymous) => true,
                (
                    Backing::File { file: a, offset: ao, shared: ash },
                    Backing::File { file: b, offset: bo, shared: bsh },
                ) => a == b && ash == bsh && ao.checked_add(self.size()) == Some(*bo),
                _ => false,
            }
    }

    /// The range nothing else may be placed in: the region, plus the guard
    /// gap below a stack.
    fn blocked(&self, guard_gap: u64) -> (u64, u64) {
        let gap = if self.kind == Kind::Stack { guard_gap } else { 0 };
        (self.start.saturating_sub(gap), self.end)
    }
}

/// `backing` for the pages `by` bytes further on (a file's offset moves
/// with them).
fn shifted<F: Clone>(backing: &Backing<F>, by: u64) -> Backing<F> {
    match backing {
        Backing::Anonymous => Backing::Anonymous,
        Backing::File { file, offset, shared } => {
            Backing::File { file: file.clone(), offset: offset + by, shared: *shared }
        }
    }
}

/// Which way the free-space search goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Search {
    /// The highest hole that ends at or below [`Layout::mmap_base`]; if
    /// there is none, the lowest hole anywhere (the default).
    TopDown,
    /// The lowest hole that starts at or above [`Layout::mmap_base`]; if
    /// there is none, the highest hole anywhere.
    BottomUp,
}

/// The bounds and limits of one address space. All addresses and sizes are
/// page-aligned ([`VmMap::new`] checks).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Layout {
    /// Lowest mappable address; at least one page, so address 0 never is.
    pub min_addr: u64,
    /// End (exclusive) of the mappable range; at most [`USER_END`].
    pub max_addr: u64,
    /// End (exclusive) of the stack region.
    pub stack_top: u64,
    /// How far below `stack_top` the stack may grow (`RLIMIT_STACK`).
    pub stack_max: u64,
    /// Free space kept below a stack region (searches avoid it, the stack
    /// does not grow closer than this to the region below it).
    pub stack_guard_gap: u64,
    /// Where the free-space search starts (see [`Search`]).
    pub mmap_base: u64,
    pub search: Search,
    /// Most regions the map may hold (`vm.max_map_count`).
    pub max_map_count: usize,
    /// Most bytes the program break may move above its start
    /// (`RLIMIT_DATA`; unlimited by default).
    pub heap_max: u64,
}

impl Layout {
    /// The default layout with the stack ending at `stack_top` and limited to
    /// `stack_max` bytes: user space from [`USER_MIN_ADDR`] to [`USER_END`],
    /// top-down search from `stack_top - max(stack_max, 128 MiB) - guard gap`.
    pub const fn new(stack_top: u64, stack_max: u64) -> Layout {
        let reserve = if stack_max > MIN_STACK_RESERVE { stack_max } else { MIN_STACK_RESERVE };
        let mmap_base = page_down(stack_top.saturating_sub(reserve).saturating_sub(DEFAULT_STACK_GUARD_GAP));
        Layout {
            min_addr: USER_MIN_ADDR,
            max_addr: USER_END,
            stack_top,
            stack_max,
            stack_guard_gap: DEFAULT_STACK_GUARD_GAP,
            mmap_base: if mmap_base < USER_MIN_ADDR { USER_MIN_ADDR } else { mmap_base },
            search: Search::TopDown,
            max_map_count: DEFAULT_MAX_MAP_COUNT,
            heap_max: u64::MAX,
        }
    }

    /// Lowest address the stack may grow down to.
    pub fn stack_floor(&self) -> u64 {
        self.stack_top.saturating_sub(self.stack_max).max(self.min_addr)
    }

    fn check(&self) -> Result<(), Error> {
        let aligned =
            [self.min_addr, self.max_addr, self.stack_top, self.stack_max, self.stack_guard_gap, self.mmap_base]
                .iter()
                .all(|&a| is_page_aligned(a));
        let ok = aligned
            && self.min_addr >= PAGE_SIZE
            && self.min_addr < self.max_addr
            && self.max_addr <= USER_END
            && self.min_addr < self.stack_top
            && self.stack_top <= self.max_addr
            && self.min_addr <= self.mmap_base
            && self.mmap_base <= self.max_addr
            && self.max_map_count >= 1;
        if ok { Ok(()) } else { Err(Error::Invalid) }
    }
}

impl Default for Layout {
    fn default() -> Layout {
        Layout::new(USER_END, DEFAULT_STACK_MAX)
    }
}

/// Why an operation failed. Nothing has changed when an error is returned.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Error {
    /// A misaligned address or offset, a zero length where one is not
    /// allowed, a range outside the address space for `munmap`, or an
    /// invalid [`Layout`] (`EINVAL`).
    Invalid,
    /// No room, a range outside the address space, a hole in an `mprotect`
    /// range, or too many regions (`ENOMEM`).
    NoMemory,
    /// `MAP_FIXED_NOREPLACE` (or stack setup) over an existing region
    /// (`EEXIST`).
    Exists,
    /// The protection exceeds what the region or file allows (`EACCES`).
    Access,
    /// A fixed mapping below [`Layout::min_addr`] (`EPERM`, what Linux
    /// returns to unprivileged programs below `vm.mmap_min_addr`; the diff
    /// runner should confirm it).
    Permission,
    /// The file offset plus the length does not fit in 64 bits
    /// (`EOVERFLOW`). A negative `off_t` is the personality's to reject.
    Overflow,
    /// A range [`VmMap::remap`] was given is not inside one region
    /// (`EFAULT`, as mremap(2) says).
    Fault,
    /// A region [`VmMap::set_anon_name`] was given is not anonymous memory
    /// (`EBADF`, prctl(2) `PR_SET_VMA_ANON_NAME` on a file mapping).
    NotAnonymous,
}

impl Error {
    /// The Linux errno for this error: values from the UAPI headers
    /// `asm-generic/errno-base.h` (`EPERM` 1, `EBADF` 9, `ENOMEM` 12, `EACCES` 13,
    /// `EFAULT` 14, `EEXIST` 17, `EINVAL` 22) and `asm-generic/errno.h` (`EOVERFLOW` 75).
    pub const fn linux_errno(self) -> u64 {
        match self {
            Error::Permission => 1,
            Error::NotAnonymous => 9,
            Error::NoMemory => 12,
            Error::Access => 13,
            Error::Exists => 17,
            Error::Invalid => 22,
            Error::Overflow => 75,
            Error::Fault => 14,
        }
    }
}

/// Where [`VmMap::map`] puts a new mapping.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Placement {
    /// Anywhere; 0 means no preference. Otherwise the address, rounded up
    /// to a page and raised to [`Layout::min_addr`] ("a nearby page
    /// boundary, but always above or equal to `mmap_min_addr`", `mmap(2)`),
    /// is used if the range is free and the stack's guard gap is not in it.
    Hint(u64),
    /// Exactly here, replacing whatever was mapped (`MAP_FIXED`).
    Fixed(u64),
    /// Exactly here, failing with [`Error::Exists`] if anything is mapped in
    /// the range (`MAP_FIXED_NOREPLACE`).
    FixedNoReplace(u64),
}

/// The arguments of [`VmMap::map`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MapRequest<F> {
    pub placement: Placement,
    /// Length in bytes, rounded up to whole pages.
    pub len: u64,
    pub prot: Prot,
    /// The most `prot` may ever be. For a shared file mapping the
    /// personality leaves out [`Prot::WRITE`] when the file is not open for
    /// writing; for a file on a `noexec` mount it leaves out
    /// [`Prot::EXEC`] (and refuses `PROT_EXEC` in the `mmap` call itself
    /// with `EPERM`, as `mmap(2)` documents, before calling
    /// [`VmMap::map`]). [`Prot::ALL`] otherwise.
    pub max_prot: Prot,
    pub backing: Backing<F>,
}

impl<F> MapRequest<F> {
    /// Private anonymous memory anywhere.
    pub fn anonymous(len: u64, prot: Prot) -> MapRequest<F> {
        MapRequest { placement: Placement::Hint(0), len, prot, max_prot: Prot::ALL, backing: Backing::Anonymous }
    }

    /// `file` from `offset`, shared or private, anywhere.
    pub fn file(len: u64, prot: Prot, file: F, offset: u64, shared: bool) -> MapRequest<F> {
        MapRequest {
            placement: Placement::Hint(0),
            len,
            prot,
            max_prot: Prot::ALL,
            backing: Backing::File { file, offset, shared },
        }
    }

    /// The same request at `placement`.
    pub fn at(self, placement: Placement) -> MapRequest<F> {
        MapRequest { placement, ..self }
    }

    /// The same request with a protection ceiling.
    pub fn with_max_prot(self, max_prot: Prot) -> MapRequest<F> {
        MapRequest { max_prot, ..self }
    }
}

/// The result of [`VmMap::map`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mapped<F> {
    /// Start of the new mapping.
    pub addr: u64,
    /// The parts of earlier regions that a fixed mapping replaced, in
    /// address order. The kernel must unmap their pages (and drop their file
    /// handles) before the new mapping's pages can fault in.
    pub replaced: Vec<Vma<F>>,
}

/// Where [`VmMap::remap`] may put the region (mremap(2)'s flags).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RemapPlacement {
    /// Where it is: grow only into free space right after it.
    InPlace,
    /// Where it is if it can grow there, else anywhere (`MREMAP_MAYMOVE`).
    MayMove,
    /// At this address, replacing what is there (`MREMAP_MAYMOVE |
    /// MREMAP_FIXED`).
    Fixed(u64),
}

/// Pages [`VmMap::remap`] moved: the kernel moves the pages of `[from,
/// from + len)` to `[to, to + len)`, after unmapping the pages of
/// [`Remap::removed`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Move {
    pub from: u64,
    pub to: u64,
    pub len: u64,
}

/// The result of [`VmMap::remap`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Remap<F> {
    /// Where the region now starts.
    pub addr: u64,
    /// Pages to move, if the region moved.
    pub moved: Option<Move>,
    /// What the call removed: the end of a shrunk range, and for
    /// [`RemapPlacement::Fixed`] what was at the new address. Their pages
    /// go first, as for [`VmMap::unmap`].
    pub removed: Vec<Vma<F>>,
}

/// The result of [`VmMap::brk`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Brk<F> {
    /// The program break after the call: the requested one on success, the
    /// unchanged one on failure (what the Linux system call returns).
    pub brk: u64,
    /// Heap pages a shrinking break gave up; the kernel unmaps them.
    pub removed: Vec<Vma<F>>,
}

/// What the kernel knows about the faulting page from its page tables.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PageState {
    /// No frame behind the page.
    Absent,
    /// A frame is mapped (or kept behind a non-present entry because the
    /// region is [`Prot::NONE`]). `refs` counts every user of the frame:
    /// each address space that maps it, plus the page cache if the frame is
    /// a cached file page.
    Present { refs: u32 },
}

/// `si_code` for a `SIGSEGV`, from the Linux UAPI header
/// `asm-generic/siginfo.h`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SegvCode {
    /// `SEGV_MAPERR`: nothing is mapped at the address.
    MapErr,
    /// `SEGV_ACCERR`: the region does not allow this access.
    AccErr,
}

impl SegvCode {
    /// The `si_code` value.
    pub const fn si_code(self) -> i32 {
        match self {
            SegvCode::MapErr => 1,
            SegvCode::AccErr => 2,
        }
    }
}

/// What the kernel must do about a user page fault. `page` is the faulting
/// page; `prot` is the permissions to give its page-table entry, which is
/// always user-accessible: present unless [`Prot::NONE`], writable exactly
/// with [`Prot::WRITE`], no-execute without [`Prot::EXEC`] (reads need no
/// bit of their own).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FaultAction<F> {
    /// Map a new zeroed frame.
    ZeroFill { page: u64, prot: Prot },
    /// Map the page at `offset` of `file`. If `private_copy` is set the frame
    /// must belong to this mapping alone (a copy of the file's page);
    /// otherwise the kernel may map a shared page-cache frame, which is why
    /// `prot` then lacks [`Prot::WRITE`] for private regions. A page wholly
    /// beyond the end of the file is the kernel's to report (`SIGBUS`).
    ReadFile { page: u64, file: F, offset: u64, prot: Prot, private_copy: bool },
    /// Copy the present frame into a new one, map that with `prot` and drop
    /// this mapping's reference to the old frame.
    CopyOnWrite { page: u64, prot: Prot },
    /// Keep the present frame and set the entry's permissions to `prot`.
    /// If the entry already had them, the fault came from a stale TLB entry
    /// (another CPU resolved it first): flush the page and return.
    SetPermissions { page: u64, prot: Prot },
    /// Deliver `SIGSEGV` with this code (or fail with `EFAULT` when the
    /// fault happened in the kernel's user-copy routines).
    Segv(SegvCode),
    /// The page is missing in a region registered with userfaultfd object
    /// `uffd` ([`Vma::uffd`]): the kernel reports the fault to it and
    /// waits until the page is filled (or raises `SIGBUS` if the object
    /// asked for that), then retries.
    UserFault { page: u64, uffd: u32 },
}

/// The regions of one address space. See the crate docs.
#[derive(Clone, Debug)]
pub struct VmMap<F> {
    layout: Layout,
    /// Regions keyed by their start address.
    vmas: BTreeMap<u64, Vma<F>>,
    brk: Option<BrkState>,
    /// mlockall(2) `MCL_FUTURE`: new regions start locked; the value says
    /// whether only on fault (`MCL_ONFAULT`), i.e. whether the kernel
    /// leaves them unpopulated.
    lock_future: Option<FutureLock>,
}

/// How mlockall(2) `MCL_FUTURE` locks new mappings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FutureLock {
    /// Locked and populated when made.
    Populate,
    /// Locked, pages brought in as they are touched (`MCL_ONFAULT`).
    OnFault,
}

#[derive(Clone, Copy, Debug)]
struct BrkState {
    start: u64,
    current: u64,
}

/// What [`VmMap::rewrite`] does to the range.
enum Op<F> {
    Remove,
    Insert(Vma<F>),
    Protect(Prot),
    WipeOnFork(bool),
    Lock(bool),
    Seal,
    UserFault(u32),
    Name(Option<Arc<str>>),
}

impl<F: Clone + PartialEq> VmMap<F> {
    /// An empty address space, or [`Error::Invalid`] if the layout is not
    /// page-aligned and ordered.
    pub fn new(layout: Layout) -> Result<VmMap<F>, Error> {
        layout.check()?;
        Ok(VmMap { layout, vmas: BTreeMap::new(), brk: None, lock_future: None })
    }

    pub fn layout(&self) -> &Layout {
        &self.layout
    }

    /// Number of regions.
    pub fn region_count(&self) -> usize {
        self.vmas.len()
    }

    /// All regions in address order.
    pub fn regions(&self) -> impl Iterator<Item = &Vma<F>> {
        self.vmas.values()
    }

    /// The region containing `addr`.
    pub fn find(&self, addr: u64) -> Option<&Vma<F>> {
        self.vmas.range(..=addr).next_back().map(|(_, v)| v).filter(|v| v.contains(addr))
    }

    /// Give file-backed regions another handle: each region whose handle
    /// `f` answers with `Some` takes the new one; the rest stay. Addresses,
    /// offsets and rights do not change, and nothing is merged or split. A
    /// forked copy of a map uses it for handles that must not be shared
    /// with the parent (a System V attachment is each process's own).
    pub fn replace_files(&mut self, mut f: impl FnMut(&F) -> Option<F>) {
        for v in self.vmas.values_mut() {
            if let Backing::File { file, .. } = &mut v.backing
                && let Some(new) = f(file)
            {
                *file = new;
            }
        }
    }

    /// The regions overlapping `[start, end)`, in address order.
    pub fn regions_in(&self, start: u64, end: u64) -> impl Iterator<Item = &Vma<F>> {
        let first = self.vmas.range(..start).next_back().map(|(_, v)| v).filter(|v| v.end > start && start < end);
        first.into_iter().chain(self.vmas.range(start..end.max(start)).map(|(_, v)| v))
    }
}

mod attrs;
mod mapping;
mod placement;
mod stack;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod model;
