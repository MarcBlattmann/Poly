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
        Vma { start, end, prot: self.prot, max_prot: self.max_prot, kind: self.kind, backing, wipe_on_fork: self.wipe_on_fork }
    }

    /// Whether `next` starts where `self` ends and the two describe one
    /// continuous region.
    fn can_merge(&self, next: &Vma<F>) -> bool {
        self.end == next.start
            && self.prot == next.prot
            && self.max_prot == next.max_prot
            && self.kind == next.kind
            && self.wipe_on_fork == next.wipe_on_fork
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
}

impl Error {
    /// The Linux errno for this error: values from the UAPI headers
    /// `asm-generic/errno-base.h` (`EPERM` 1, `ENOMEM` 12, `EACCES` 13, `EFAULT` 14,
    /// `EEXIST` 17, `EINVAL` 22) and `asm-generic/errno.h` (`EOVERFLOW` 75).
    pub const fn linux_errno(self) -> u64 {
        match self {
            Error::Permission => 1,
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
}

/// The regions of one address space. See the crate docs.
#[derive(Clone, Debug)]
pub struct VmMap<F> {
    layout: Layout,
    /// Regions keyed by their start address.
    vmas: BTreeMap<u64, Vma<F>>,
    brk: Option<BrkState>,
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
}

impl<F: Clone + PartialEq> VmMap<F> {
    /// An empty address space, or [`Error::Invalid`] if the layout is not
    /// page-aligned and ordered.
    pub fn new(layout: Layout) -> Result<VmMap<F>, Error> {
        layout.check()?;
        Ok(VmMap { layout, vmas: BTreeMap::new(), brk: None })
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

    /// The regions overlapping `[start, end)`, in address order.
    pub fn regions_in(&self, start: u64, end: u64) -> impl Iterator<Item = &Vma<F>> {
        let first = self.vmas.range(..start).next_back().map(|(_, v)| v).filter(|v| v.end > start && start < end);
        first.into_iter().chain(self.vmas.range(start..end.max(start)).map(|(_, v)| v))
    }

    /// Create a mapping (`mmap`). Returns its address and, for
    /// [`Placement::Fixed`], the regions it replaced.
    ///
    /// Errors, checked in this order: zero length ([`Error::Invalid`]);
    /// `prot` beyond `max_prot` ([`Error::Access`]); a file offset that is
    /// not page-aligned ([`Error::Invalid`]); a length that does not fit in
    /// the address space ([`Error::NoMemory`]); a file offset plus length
    /// that overflows ([`Error::Overflow`]); for fixed placements a
    /// misaligned address ([`Error::Invalid`]), an address below
    /// [`Layout::min_addr`] ([`Error::Permission`]), a range past
    /// [`Layout::max_addr`] ([`Error::NoMemory`]), an occupied range for
    /// [`Placement::FixedNoReplace`] ([`Error::Exists`]); no hole large
    /// enough, or too many regions ([`Error::NoMemory`]).
    pub fn map(&mut self, req: MapRequest<F>) -> Result<Mapped<F>, Error> {
        let MapRequest { placement, len, prot, max_prot, backing } = req;
        if len == 0 {
            return Err(Error::Invalid);
        }
        if !max_prot.contains(prot) {
            return Err(Error::Access);
        }
        if let Backing::File { offset, .. } = backing
            && !is_page_aligned(offset)
        {
            return Err(Error::Invalid);
        }
        let len = page_up(len).ok_or(Error::NoMemory)?;
        if len > self.layout.max_addr - self.layout.min_addr {
            return Err(Error::NoMemory);
        }
        if let Backing::File { offset, .. } = backing
            && offset.checked_add(len).is_none()
        {
            return Err(Error::Overflow);
        }
        let addr = match placement {
            Placement::Fixed(addr) | Placement::FixedNoReplace(addr) => {
                if !is_page_aligned(addr) {
                    return Err(Error::Invalid);
                }
                if addr < self.layout.min_addr {
                    return Err(Error::Permission);
                }
                if addr.checked_add(len).is_none_or(|end| end > self.layout.max_addr) {
                    return Err(Error::NoMemory);
                }
                if matches!(placement, Placement::FixedNoReplace(_))
                    && self.regions_in(addr, addr + len).next().is_some()
                {
                    return Err(Error::Exists);
                }
                addr
            }
            Placement::Hint(hint) => self.place(hint, len).ok_or(Error::NoMemory)?,
        };
        let vma = Vma { start: addr, end: addr + len, prot, max_prot, kind: Kind::Normal, backing, wipe_on_fork: false };
        let replaced = self.rewrite(addr, addr + len, Op::Insert(vma))?;
        Ok(Mapped { addr, replaced })
    }

    /// Remove every mapping in `[addr, addr + len)` (`munmap`), splitting
    /// regions that straddle the ends. Holes are fine. Returns the removed
    /// parts in address order; the kernel unmaps their pages.
    ///
    /// Errors: a misaligned `addr`, a zero length or a range that leaves the
    /// address space ([`Error::Invalid`]); a split that would exceed the map
    /// count ([`Error::NoMemory`]).
    pub fn unmap(&mut self, addr: u64, len: u64) -> Result<Vec<Vma<F>>, Error> {
        if !is_page_aligned(addr) || len == 0 {
            return Err(Error::Invalid);
        }
        let end = page_up(len).and_then(|len| addr.checked_add(len)).ok_or(Error::Invalid)?;
        if end > self.layout.max_addr {
            return Err(Error::Invalid);
        }
        self.rewrite(addr, end, Op::Remove)
    }

    /// Change the protection of `[addr, addr + len)` (`mprotect`). A zero
    /// length succeeds without doing anything. The kernel then updates the
    /// present pages in the range (see the crate docs).
    ///
    /// Errors: a misaligned `addr` ([`Error::Invalid`]); a range that leaves
    /// the address space or contains unmapped pages ([`Error::NoMemory`],
    /// checked before the next one); `prot` beyond some region's
    /// [`Vma::max_prot`] ([`Error::Access`]); too many regions
    /// ([`Error::NoMemory`]).
    pub fn protect(&mut self, addr: u64, len: u64, prot: Prot) -> Result<(), Error> {
        if !is_page_aligned(addr) {
            return Err(Error::Invalid);
        }
        if len == 0 {
            return Ok(());
        }
        let end = page_up(len).and_then(|len| addr.checked_add(len)).ok_or(Error::NoMemory)?;
        if end > self.layout.max_addr {
            return Err(Error::NoMemory);
        }
        let mut covered = addr;
        let mut allowed = true;
        for v in self.regions_in(addr, end) {
            if v.start > covered {
                return Err(Error::NoMemory);
            }
            allowed &= v.max_prot.contains(prot);
            covered = v.end;
        }
        if covered < end {
            return Err(Error::NoMemory);
        }
        if !allowed {
            return Err(Error::Access);
        }
        self.rewrite(addr, end, Op::Protect(prot)).map(|_| ())
    }

    /// `MADV_WIPEONFORK` (`on`) or `MADV_KEEPONFORK` for the mapped parts
    /// of `[addr, addr + len)` (page aligned, `len` non-zero). Wiping is
    /// for private anonymous memory only: any other region in the range is
    /// [`Error::Invalid`] and nothing changes. A range with unmapped pages
    /// is [`Error::NoMemory`], after the mapped parts got the setting.
    pub fn set_wipe_on_fork(&mut self, addr: u64, len: u64, on: bool) -> Result<(), Error> {
        let end = addr.checked_add(len).ok_or(Error::NoMemory)?;
        if !is_page_aligned(addr) || !is_page_aligned(end) || len == 0 {
            return Err(Error::Invalid);
        }
        let mut covered = addr;
        let mut hole = false;
        let mut parts = Vec::new();
        for v in self.regions_in(addr, end) {
            if on && !(matches!(v.backing, Backing::Anonymous)) {
                return Err(Error::Invalid);
            }
            hole |= v.start > covered;
            parts.push((v.start.max(addr), v.end.min(end)));
            covered = v.end;
        }
        hole |= covered < end;
        for (s, e) in parts {
            self.rewrite(s, e, Op::WipeOnFork(on))?;
        }
        if hole { Err(Error::NoMemory) } else { Ok(()) }
    }

    /// Resize the mapping `[old, old + old_len)` to `new_len` bytes, moving
    /// it if `how` allows (`mremap`). Lengths are rounded up to pages.
    ///
    /// The range must lie inside one region of [`Kind::Normal`] (the break
    /// area and the stack are not remapped; they are the kernel's to grow).
    /// Shrinking removes the end of the range. Growing extends the region
    /// in place when the range ends where the region does and the space
    /// after it is free; otherwise, with [`RemapPlacement::MayMove`], the
    /// range moves to a free hole of the new size (search order as for
    /// [`VmMap::map`]) and grows there, the rest of the old region staying
    /// where it was. The moved part keeps its protection and backing (a
    /// file mapping keeps its offset; the grown pages continue the file).
    /// [`RemapPlacement::Fixed`] moves it to that address in every case,
    /// replacing whatever was there.
    ///
    /// Errors, in this order (mremap(2)): a misaligned `old` or a zero
    /// `new_len` ([`Error::Invalid`]), a zero `old_len` (Poly does not
    /// duplicate shared mappings: [`Error::Invalid`]); a range outside
    /// the address space or not inside one region, or in the break area or
    /// the stack ([`Error::Fault`]); for `Fixed`, a misaligned address or a
    /// new range overlapping the old one ([`Error::Invalid`]) or leaving the
    /// address space ([`Error::NoMemory`]); no room to grow in place without
    /// `MayMove`, no hole, or too many regions ([`Error::NoMemory`]). Nothing
    /// changes on error.
    pub fn remap(&mut self, old: u64, old_len: u64, new_len: u64, how: RemapPlacement) -> Result<Remap<F>, Error> {
        if !is_page_aligned(old) || new_len == 0 || old_len == 0 {
            return Err(Error::Invalid);
        }
        let old_len = page_up(old_len).ok_or(Error::Fault)?;
        let new_len = page_up(new_len).ok_or(Error::NoMemory)?;
        let old_end = old.checked_add(old_len).ok_or(Error::Fault)?;
        let vma = self.find(old).filter(|v| v.end >= old_end && v.kind == Kind::Normal).ok_or(Error::Fault)?.clone();
        if let RemapPlacement::Fixed(to) = how {
            if !is_page_aligned(to) {
                return Err(Error::Invalid);
            }
            let to_end = to.checked_add(new_len).ok_or(Error::NoMemory)?;
            if to < old_end && old < to_end {
                return Err(Error::Invalid);
            }
            if to < self.layout.min_addr || to_end > self.layout.max_addr {
                return Err(Error::NoMemory);
            }
        }
        // Work on a copy, so that an error half-way changes nothing.
        let mut next = self.clone();
        let mut removed = Vec::new();
        let kept = old_len.min(new_len);
        if new_len < old_len {
            removed.extend(next.rewrite(old + new_len, old_end, Op::Remove)?);
        }
        let grow_in_place = |m: &VmMap<F>| {
            vma.end == old_end
                && old.checked_add(new_len).is_some_and(|end| end <= m.layout.max_addr && m.is_free(old_end, end))
        };
        let to = match how {
            RemapPlacement::Fixed(to) => to,
            _ if new_len <= old_len => {
                *self = next;
                return Ok(Remap { addr: old, moved: None, removed });
            }
            _ if grow_in_place(&next) => {
                let grown = vma.clip(old, old_end);
                let tail =
                    Vma { start: old_end, end: old + new_len, backing: shifted(&grown.backing, old_len), ..grown };
                next.rewrite(old_end, old + new_len, Op::Insert(tail))?;
                *self = next;
                return Ok(Remap { addr: old, moved: None, removed });
            }
            RemapPlacement::InPlace => return Err(Error::NoMemory),
            RemapPlacement::MayMove => next.place(0, new_len).ok_or(Error::NoMemory)?,
        };
        // Move: the kept part leaves its place (its pages move, nothing is
        // freed), the destination is cleared and gets the region.
        let part = vma.clip(old, old + kept);
        next.rewrite(old, old + kept, Op::Remove)?;
        let moved = Vma { start: to, end: to + new_len, ..part };
        removed.extend(next.rewrite(to, to + new_len, Op::Insert(moved))?);
        *self = next;
        Ok(Remap { addr: to, moved: Some(Move { from: old, to, len: kept }), removed })
    }

    /// Create the stack region: the top `size` bytes below
    /// [`Layout::stack_top`], private anonymous memory with protection
    /// `prot` (read-write, plus execute for programs that ask for an
    /// executable stack). Returns its lowest address.
    ///
    /// Errors: `size` zero, misaligned, above [`Layout::stack_max`] or
    /// reaching below [`Layout::min_addr`] ([`Error::Invalid`]); the range is
    /// already mapped ([`Error::Exists`]); too many regions
    /// ([`Error::NoMemory`]).
    pub fn setup_stack(&mut self, size: u64, prot: Prot) -> Result<u64, Error> {
        let top = self.layout.stack_top;
        if size == 0 || !is_page_aligned(size) || size > self.layout.stack_max || size > top - self.layout.min_addr {
            return Err(Error::Invalid);
        }
        let start = top - size;
        if self.regions_in(start, top).next().is_some() {
            return Err(Error::Exists);
        }
        let vma = Vma {
            start,
            end: top,
            prot,
            max_prot: Prot::ALL,
            kind: Kind::Stack,
            backing: Backing::Anonymous,
            wipe_on_fork: false,
        };
        self.rewrite(start, top, Op::Insert(vma))?;
        Ok(start)
    }

    /// Set the start of the program break (the end of the program's data,
    /// possibly randomised by the kernel). The heap region begins at the
    /// next page boundary. Fails with [`Error::Invalid`] outside the
    /// address space.
    pub fn init_brk(&mut self, start: u64) -> Result<(), Error> {
        if start < self.layout.min_addr || start >= self.layout.max_addr {
            return Err(Error::Invalid);
        }
        self.brk = Some(BrkState { start, current: start });
        Ok(())
    }

    /// The initial program break, if [`VmMap::init_brk`] was called.
    pub fn brk_start(&self) -> Option<u64> {
        self.brk.map(|b| b.start)
    }

    /// Move the program break to `new` (`brk`).
    ///
    /// Growing maps the new pages as a private read-write heap region, but
    /// only if they are free (also of the stack's guard gap), inside the
    /// address space and within [`Layout::heap_max`]. Shrinking unmaps the
    /// whole pages above the new break. A break below the start (including
    /// `brk(0)`, the usual way to ask for the current break) changes nothing.
    /// Without [`VmMap::init_brk`] the break is 0 and cannot move.
    pub fn brk(&mut self, new: u64) -> Brk<F> {
        let Some(state) = self.brk else {
            return Brk { brk: 0, removed: Vec::new() };
        };
        let unchanged = Brk { brk: state.current, removed: Vec::new() };
        if new < state.start || new - state.start > self.layout.heap_max {
            return unchanged;
        }
        let (Some(old_end), Some(new_end)) = (page_up(state.current), page_up(new)) else {
            return unchanged;
        };
        if new_end > self.layout.max_addr {
            return unchanged;
        }
        let removed = if new_end < old_end {
            match self.rewrite(new_end, old_end, Op::Remove) {
                Ok(removed) => removed,
                Err(_) => return unchanged,
            }
        } else {
            if new_end > old_end {
                if !self.is_free(old_end, new_end) {
                    return unchanged;
                }
                let vma = Vma {
                    start: old_end,
                    end: new_end,
                    prot: Prot::RW,
                    max_prot: Prot::ALL,
                    kind: Kind::Heap,
                    backing: Backing::Anonymous,
                    wipe_on_fork: false,
                };
                if self.rewrite(old_end, new_end, Op::Insert(vma)).is_err() {
                    return unchanged;
                }
            }
            Vec::new()
        };
        self.brk = Some(BrkState { current: new, ..state });
        Brk { brk: new, removed }
    }

    /// Decide what a user page fault at `addr` means.
    ///
    /// The kernel's page-fault handler, for a fault on a user address (from
    /// user mode, or from the kernel's user-copy routines), with the address
    /// space locked:
    ///
    /// 1. read the faulting address (CR2) and turn the error code into an
    ///    [`Access`];
    /// 2. look at the page's entry: [`PageState::Absent`] if no frame is
    ///    behind it, otherwise [`PageState::Present`] with the frame's
    ///    reference count;
    /// 3. call this and carry out the [`FaultAction`], then return to the
    ///    faulting instruction (or deliver the signal / return `EFAULT`).
    ///
    /// The rules: an address in no region is a `SEGV_MAPERR`, unless it lies
    /// just below a stack region, above [`Layout::stack_floor`] and at least
    /// the guard gap above the region below; then the stack grows down to
    /// the page (this call changes the map) and the fault is handled as one
    /// in the stack. An access the region's protection does not allow (see
    /// [`Prot::allows`]) is a `SEGV_ACCERR`. Otherwise an absent page is
    /// zero-filled or read from the file, a write to a present private page
    /// whose frame is shared is copied, and anything else only needs the
    /// entry's permissions brought up to date.
    pub fn fault(&mut self, addr: u64, access: Access, page: PageState) -> FaultAction<F> {
        let page_addr = page_down(addr);
        if addr < self.layout.min_addr || addr >= self.layout.max_addr {
            return FaultAction::Segv(SegvCode::MapErr);
        }
        let key = match self.find(addr) {
            Some(v) => v.start,
            None => match self.grow_stack(page_addr, access) {
                Ok(key) => key,
                Err(code) => return FaultAction::Segv(code),
            },
        };
        let vma = &self.vmas[&key];
        if !vma.prot.allows(access) {
            return FaultAction::Segv(SegvCode::AccErr);
        }
        match page {
            PageState::Absent => match &vma.backing {
                Backing::Anonymous => FaultAction::ZeroFill { page: page_addr, prot: vma.prot },
                Backing::File { file, offset, shared } => {
                    let private_copy = !shared && access == Access::Write;
                    let prot = if *shared || private_copy { vma.prot } else { write_protected(vma.prot) };
                    let offset = offset + (page_addr - vma.start);
                    FaultAction::ReadFile { page: page_addr, file: file.clone(), offset, prot, private_copy }
                }
            },
            PageState::Present { refs } => {
                let prot = vma.page_prot(refs);
                if access == Access::Write && !prot.contains(Prot::WRITE) {
                    FaultAction::CopyOnWrite { page: page_addr, prot: vma.prot }
                } else {
                    FaultAction::SetPermissions { page: page_addr, prot }
                }
            }
        }
    }

    /// Grow the stack region above `page` down to it, if the rules allow.
    /// Returns the region's new start (its key).
    fn grow_stack(&mut self, page: u64, access: Access) -> Result<u64, SegvCode> {
        let above = self.vmas.range(page..).next().map(|(_, v)| v);
        let Some(stack) = above.filter(|v| v.kind == Kind::Stack) else {
            return Err(SegvCode::MapErr);
        };
        if page < self.layout.stack_floor() {
            return Err(SegvCode::MapErr);
        }
        let below = self.vmas.range(..page).next_back().map(|(_, v)| v);
        if below.is_some_and(|b| b.end.saturating_add(self.layout.stack_guard_gap) > page) {
            return Err(SegvCode::MapErr);
        }
        if !stack.prot.allows(access) {
            return Err(SegvCode::AccErr);
        }
        let old = stack.start;
        let mut grown = self.vmas.remove(&old).expect("the stack region is in the map");
        grown.start = page;
        // With a zero guard gap the stack can reach a compatible region below.
        if let Some(below) = self.vmas.range_mut(..page).next_back().map(|(_, v)| v)
            && below.can_merge(&grown)
        {
            below.end = grown.end;
            return Ok(below.start);
        }
        self.vmas.insert(page, grown);
        Ok(page)
    }

    /// Where a mapping of `len` bytes goes: the hint (rounded up to a page,
    /// and to `min_addr`) if that range is free, else the first hole in
    /// search order.
    fn place(&self, hint: u64, len: u64) -> Option<u64> {
        let l = &self.layout;
        if let Some(h) = page_up(hint).filter(|&h| h != 0).map(|h| h.max(l.min_addr))
            && h.checked_add(len).is_some_and(|end| end <= l.max_addr && self.is_free(h, end))
        {
            return Some(h);
        }
        // The fallback covers the whole space, so it also finds a hole that
        // straddles mmap_base.
        match l.search {
            Search::TopDown => self.top_down(len, l.mmap_base).or_else(|| self.bottom_up(len, l.min_addr)),
            Search::BottomUp => self.bottom_up(len, l.mmap_base).or_else(|| self.top_down(len, l.max_addr)),
        }
    }

    /// Whether `[start, end)` is free of regions and of stack guard gaps.
    fn is_free(&self, start: u64, end: u64) -> bool {
        let gap = self.layout.stack_guard_gap;
        for v in self.vmas.range(..end.saturating_add(gap)).rev().map(|(_, v)| v) {
            if v.end <= start {
                // Ends are sorted: nothing further down reaches the range.
                return true;
            }
            let (bs, be) = v.blocked(gap);
            if bs < end && be > start {
                return false;
            }
        }
        true
    }

    /// The highest `len`-byte hole that ends at or below `limit`.
    fn top_down(&self, len: u64, limit: u64) -> Option<u64> {
        let gap = self.layout.stack_guard_gap;
        let min = self.layout.min_addr;
        let mut top = limit;
        for v in self.vmas.range(..limit.saturating_add(gap)).rev().map(|(_, v)| v) {
            let (bs, be) = v.blocked(gap);
            let floor = be.max(min);
            if top >= floor && top - floor >= len {
                return Some(top - len);
            }
            top = top.min(bs);
            if top < min || top - min < len {
                return None;
            }
        }
        (top >= min && top - min >= len).then(|| top - len)
    }

    /// The lowest `len`-byte hole that starts at or above `base`.
    fn bottom_up(&self, len: u64, base: u64) -> Option<u64> {
        let gap = self.layout.stack_guard_gap;
        let max = self.layout.max_addr;
        let mut lo = base;
        let straddling = self.vmas.range(..base).next_back().map(|(_, v)| v).filter(|v| v.end > base);
        for v in straddling.into_iter().chain(self.vmas.range(base..).map(|(_, v)| v)) {
            let (bs, be) = v.blocked(gap);
            if bs >= lo && bs - lo >= len {
                return Some(lo);
            }
            lo = lo.max(be);
            if lo > max || max - lo < len {
                return None;
            }
        }
        (lo <= max && max - lo >= len).then_some(lo)
    }

    /// Apply `op` to `[start, end)`: split the regions that straddle the
    /// ends, remove, replace or re-protect the inside, and merge what now
    /// fits together, including the neighbours just outside the range.
    /// Returns the parts that were removed or replaced. Fails, changing
    /// nothing, if the result would exceed the map count.
    fn rewrite(&mut self, start: u64, end: u64, op: Op<F>) -> Result<Vec<Vma<F>>, Error> {
        debug_assert!(start < end && is_page_aligned(start) && is_page_aligned(end));
        // Everything that overlaps the range, plus the neighbours touching it.
        let left = self.vmas.range(..start).next_back().filter(|(_, v)| v.end >= start).map(|(&k, _)| k);
        let keys: Vec<u64> = left.into_iter().chain(self.vmas.range(start..=end).map(|(&k, _)| k)).collect();

        let mut pieces = Vec::with_capacity(keys.len() + 2);
        let mut removed = Vec::new();
        for k in &keys {
            let v = &self.vmas[k];
            if v.end <= start || v.start >= end {
                pieces.push(v.clone());
                continue;
            }
            if v.start < start {
                pieces.push(v.clip(v.start, start));
            }
            let inside = v.clip(v.start.max(start), v.end.min(end));
            match op {
                Op::Protect(prot) => pieces.push(Vma { prot, ..inside }),
                Op::WipeOnFork(wipe_on_fork) => pieces.push(Vma { wipe_on_fork, ..inside }),
                Op::Remove | Op::Insert(_) => removed.push(inside),
            }
            if v.end > end {
                pieces.push(v.clip(end, v.end));
            }
        }
        if let Op::Insert(vma) = op {
            let at = pieces.partition_point(|p| p.start < vma.start);
            pieces.insert(at, vma);
        }

        let mut merged: Vec<Vma<F>> = Vec::with_capacity(pieces.len());
        for p in pieces {
            match merged.last_mut() {
                Some(last) if last.can_merge(&p) => last.end = p.end,
                _ => merged.push(p),
            }
        }

        if self.vmas.len() - keys.len() + merged.len() > self.layout.max_map_count {
            return Err(Error::NoMemory);
        }
        for k in &keys {
            self.vmas.remove(k);
        }
        for v in merged {
            self.vmas.insert(v.start, v);
        }
        Ok(removed)
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod model;
