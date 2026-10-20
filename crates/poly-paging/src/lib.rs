//! x86-64 4-level page tables.
//!
//! Builds and edits the PML4 → PDPT → PD → PT hierarchy described in the
//! Intel SDM Vol. 3A §4.5 ("4-Level Paging and 5-Level Paging") and the AMD
//! APM Vol. 2 §5.3 ("Long-Mode Page Translation"). Leaves can be 4 KiB pages
//! (in a PT), 2 MiB pages (PS = 1 in a PD entry) or 1 GiB pages (PS = 1 in a
//! PDPT entry). 5-level paging (LA57) is not supported: virtual addresses are
//! 48 bits, sign-extended.
//!
//! Nothing here touches hardware, so all of it is unit-tested on the host:
//!
//! - [`PhysMemory`] reads and writes page-table entries by *physical*
//!   address. In the kernel this goes through the direct map of RAM; in tests
//!   it is a simulated physical memory.
//! - [`FrameAllocator`] hands out 4 KiB frames for new intermediate tables.
//!
//! # TLB
//!
//! Editing a page table does not update the TLB or the paging-structure
//! caches (SDM Vol. 3A §4.10). Making a non-present entry present needs no
//! flush, but every operation that removes or weakens a mapping
//! ([`PageTables::unmap`], [`PageTables::unmap_and_prune`],
//! [`PageTables::update_flags`]) leaves stale translations behind. The caller
//! must invalidate them (`INVLPG` or a CR3 reload, plus a shootdown on other
//! CPUs that use these tables) before relying on the change, and before
//! reusing any frame that the old translation pointed to. This crate never
//! flushes anything itself; it does not know which CPUs have these tables
//! loaded.
//!
//! # Intermediate entry policy
//!
//! Effective permissions are the combination of every entry on the walk
//! (SDM Vol. 3A §4.6): a page is writable only if every level has R/W = 1,
//! user-accessible only if every level has U/S = 1, and executable only if no
//! level has XD = 1. So that the leaf alone decides, entries that point to a
//! lower table are kept *permissive*:
//!
//! - always `PRESENT | WRITABLE`, never `NO_EXECUTE`;
//! - `USER` is added along the whole path whenever a user mapping is created
//!   below it (by [`PageTables::map`], [`PageTables::map_range`] or
//!   [`PageTables::update_flags`]), and is never removed again. Supervisor
//!   leaves under such a path stay supervisor-only because their own U/S is 0;
//! - no caching bits (PWT/PCD), so the tables themselves are write-back;
//! - existing intermediate entries met on a mapping walk are brought into
//!   this shape (R/W set, XD cleared), even if something else built them.
//!
//! # Freeing tables
//!
//! [`PageTables::unmap_and_prune`] unlinks intermediate tables that became
//! completely empty and returns their frames instead of freeing them: the
//! paging-structure caches may still point at them until the TLB flush for
//! the unmapped address has completed on every CPU, so only the caller knows
//! when they are safe to reuse.

#![no_std]

use core::fmt;
use core::ops::{BitAnd, BitOr, BitOrAssign, Not};

/// Number of entries in every paging structure (SDM Vol. 3A §4.5: each table
/// is 4 KiB of 64-bit entries).
pub const ENTRIES: usize = 512;

/// Size of a page-table frame and of the smallest page.
pub const FRAME_SIZE: u64 = 4096;

/// Bits 51:12 of an entry: the physical address of the next table or of a
/// 4 KiB page. Bits above MAXPHYADDR are reserved and must be zero; 52 is the
/// architectural maximum (SDM Vol. 3A §4.1.4, §4.5, Figure 4-11).
pub const ADDR_MASK: u64 = 0x000f_ffff_ffff_f000;

/// First physical address that cannot be encoded in an entry.
pub const PHYS_LIMIT: u64 = 1 << 52;

/// Whether `virt` is canonical for 48-bit linear addresses: bits 63:47 all
/// equal (SDM Vol. 1 §3.3.7.1, "Canonical Addressing").
pub const fn is_canonical(virt: u64) -> bool {
    (((virt << 16) as i64) >> 16) as u64 == virt
}

/// Index of `virt` in the table at `level` (4 = PML4 ... 1 = PT). Each level
/// consumes 9 bits above the 12-bit page offset (SDM Vol. 3A §4.5.4).
const fn index(virt: u64, level: usize) -> usize {
    ((virt >> (12 + 9 * (level - 1))) & 0x1ff) as usize
}

/// Flags of a page-table entry: every bit outside [`ADDR_MASK`].
///
/// Bit positions are from the SDM Vol. 3A §4.5, Tables 4-15 to 4-20
/// ("Format of a PML4 Entry" through "Format of a Page-Table Entry that Maps
/// a 4-KByte Page"), and agree with the AMD APM Vol. 2 §5.4.1.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct PageFlags(u64);

impl PageFlags {
    /// P (bit 0): the entry is valid. Clear means any access faults.
    pub const PRESENT: PageFlags = PageFlags(1 << 0);
    /// R/W (bit 1): writes allowed (SDM Vol. 3A §4.6; supervisor writes to
    /// read-only pages fault only when CR0.WP = 1).
    pub const WRITABLE: PageFlags = PageFlags(1 << 1);
    /// U/S (bit 2): user-mode (CPL 3) accesses allowed (SDM Vol. 3A §4.6).
    pub const USER: PageFlags = PageFlags(1 << 2);
    /// PWT (bit 3): page-level write-through (SDM Vol. 3A §4.9.2).
    pub const WRITE_THROUGH: PageFlags = PageFlags(1 << 3);
    /// PCD (bit 4): page-level cache disable (SDM Vol. 3A §4.9.2).
    pub const CACHE_DISABLE: PageFlags = PageFlags(1 << 4);
    /// A (bit 5): set by the CPU when the entry is used for a translation
    /// (SDM Vol. 3A §4.8).
    pub const ACCESSED: PageFlags = PageFlags(1 << 5);
    /// D (bit 6): set by the CPU on a write through a leaf entry
    /// (SDM Vol. 3A §4.8). Ignored in non-leaf entries.
    pub const DIRTY: PageFlags = PageFlags(1 << 6);
    /// PS (bit 7) in a PDPT or PD entry: the entry maps a 1 GiB or 2 MiB page
    /// instead of pointing to a table. Must be 0 in a PML4 entry. In a PT
    /// entry the same bit is PAT. [`PageTables`] manages this bit itself.
    pub const HUGE: PageFlags = PageFlags(1 << 7);
    /// G (bit 8) in a leaf: the translation survives CR3 loads when
    /// CR4.PGE = 1 (SDM Vol. 3A §4.10.2.4). Ignored in non-leaf entries.
    pub const GLOBAL: PageFlags = PageFlags(1 << 8);
    /// XD (bit 63): instruction fetches are not allowed (SDM Vol. 3A §4.6).
    /// Only valid with IA32_EFER.NXE = 1; otherwise the bit is reserved and
    /// a set bit causes a reserved-bit page fault.
    pub const NO_EXECUTE: PageFlags = PageFlags(1 << 63);

    pub const fn empty() -> PageFlags {
        PageFlags(0)
    }

    /// Flags from raw entry bits; address bits are dropped.
    pub const fn from_bits(bits: u64) -> PageFlags {
        PageFlags(bits & !ADDR_MASK)
    }

    pub const fn bits(self) -> u64 {
        self.0
    }

    pub const fn contains(self, other: PageFlags) -> bool {
        self.0 & other.0 == other.0
    }

    pub const fn intersects(self, other: PageFlags) -> bool {
        self.0 & other.0 != 0
    }

    pub const fn union(self, other: PageFlags) -> PageFlags {
        PageFlags(self.0 | other.0)
    }

    pub const fn difference(self, other: PageFlags) -> PageFlags {
        PageFlags(self.0 & !other.0)
    }
}

impl BitOr for PageFlags {
    type Output = PageFlags;
    fn bitor(self, rhs: PageFlags) -> PageFlags {
        self.union(rhs)
    }
}

impl BitOrAssign for PageFlags {
    fn bitor_assign(&mut self, rhs: PageFlags) {
        *self = self.union(rhs);
    }
}

impl BitAnd for PageFlags {
    type Output = PageFlags;
    fn bitand(self, rhs: PageFlags) -> PageFlags {
        PageFlags(self.0 & rhs.0)
    }
}

impl Not for PageFlags {
    type Output = PageFlags;
    /// Complement within the flag bits (address bits stay clear).
    fn not(self) -> PageFlags {
        PageFlags::from_bits(!self.0)
    }
}

impl fmt::Debug for PageFlags {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        const NAMES: [(PageFlags, &str); 10] = [
            (PageFlags::PRESENT, "PRESENT"),
            (PageFlags::WRITABLE, "WRITABLE"),
            (PageFlags::USER, "USER"),
            (PageFlags::WRITE_THROUGH, "WRITE_THROUGH"),
            (PageFlags::CACHE_DISABLE, "CACHE_DISABLE"),
            (PageFlags::ACCESSED, "ACCESSED"),
            (PageFlags::DIRTY, "DIRTY"),
            (PageFlags::HUGE, "HUGE"),
            (PageFlags::GLOBAL, "GLOBAL"),
            (PageFlags::NO_EXECUTE, "NO_EXECUTE"),
        ];
        f.write_str("PageFlags(")?;
        let mut rest = self.0;
        let mut first = true;
        for (flag, name) in NAMES {
            if self.contains(flag) {
                if !first {
                    f.write_str(" | ")?;
                }
                f.write_str(name)?;
                rest &= !flag.0;
                first = false;
            }
        }
        if rest != 0 {
            if !first {
                f.write_str(" | ")?;
            }
            write!(f, "{rest:#x}")?;
        }
        f.write_str(")")
    }
}

/// Flags of entries that point to a lower table; see the crate docs.
const TABLE_FLAGS: u64 = PageFlags::PRESENT.0 | PageFlags::WRITABLE.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PageSize {
    /// 4 KiB, mapped by a PT entry.
    Size4K,
    /// 2 MiB, mapped by a PD entry with PS = 1.
    Size2M,
    /// 1 GiB, mapped by a PDPT entry with PS = 1. Only on CPUs with
    /// CPUID.80000001H:EDX.Page1GB [bit 26] (SDM Vol. 3A §4.1.4).
    Size1G,
}

impl PageSize {
    pub const fn bytes(self) -> u64 {
        match self {
            PageSize::Size4K => 1 << 12,
            PageSize::Size2M => 1 << 21,
            PageSize::Size1G => 1 << 30,
        }
    }

    /// Level of the table holding the leaf entry (1 = PT, 2 = PD, 3 = PDPT).
    const fn level(self) -> usize {
        match self {
            PageSize::Size4K => 1,
            PageSize::Size2M => 2,
            PageSize::Size1G => 3,
        }
    }

    const fn from_level(level: usize) -> PageSize {
        match level {
            1 => PageSize::Size4K,
            2 => PageSize::Size2M,
            _ => PageSize::Size1G,
        }
    }

    const fn is_aligned(self, addr: u64) -> bool {
        addr & (self.bytes() - 1) == 0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// A virtual address is not canonical, or a range crosses the
    /// non-canonical hole.
    NonCanonical,
    /// An address or length is not aligned to the page size.
    Misaligned,
    /// A physical address does not fit in an entry (at or above 2^52).
    InvalidPhysical,
    /// The slot is already in use, by a mapping of the same or a smaller
    /// page size or by a lower-level table (possibly an empty one left by
    /// [`PageTables::unmap`]; [`PageTables::unmap_and_prune`] removes those).
    AlreadyMapped,
    /// A larger page covers the address, and the operation would need
    /// splitting it: mapping a smaller page inside it, or unmapping or
    /// changing anything but the whole page.
    HugePageConflict,
    /// The frame allocator ran dry while creating an intermediate table.
    OutOfFrames,
    /// Nothing is mapped at the address.
    NotMapped,
}

/// Access to page-table memory by physical address.
///
/// The mapper only ever passes the root it was given, frames it got from a
/// [`FrameAllocator`] and addresses it read out of present non-leaf entries,
/// so an implementation only has to make those reachable. In the kernel this
/// is the direct map; entry accesses should be single 64-bit volatile (or
/// atomic) operations, since the CPU updates the A and D bits concurrently.
pub trait PhysMemory {
    /// Entry `index` (`0..512`) of the table in the frame at `table`.
    fn read_entry(&self, table: u64, index: usize) -> u64;

    /// Overwrite entry `index` of the table at `table`.
    fn write_entry(&mut self, table: u64, index: usize, value: u64);

    /// Clear all 512 entries of a fresh table.
    fn zero_table(&mut self, table: u64) {
        for i in 0..ENTRIES {
            self.write_entry(table, i, 0);
        }
    }
}

/// Source of 4 KiB frames for new page tables.
pub trait FrameAllocator {
    /// Physical address of a free 4 KiB-aligned frame below 2^52. It need not
    /// be zeroed: the mapper clears it through [`PhysMemory::zero_table`].
    fn allocate_frame(&mut self) -> Option<u64>;

    /// Give back a frame from [`FrameAllocator::allocate_frame`] that was
    /// never made reachable from the tables (used when an operation fails
    /// half-way). Tables unlinked by [`PageTables::unmap_and_prune`] are
    /// *not* returned this way; see there.
    fn deallocate_frame(&mut self, frame: u64);
}

/// Where a virtual address leads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Translation {
    /// The physical address `virt` translates to (frame + offset).
    pub phys: u64,
    /// Physical base of the page.
    pub frame: u64,
    pub size: PageSize,
    /// Flags of the leaf entry, without [`PageFlags::HUGE`] (that is
    /// `size`). With the intermediate policy above these are also the
    /// effective permissions.
    pub flags: PageFlags,
}

/// A mapping removed by [`PageTables::unmap`].
///
/// The translation may still be cached: flush the TLB for the page before
/// reusing `frame` (see the crate docs).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[must_use = "the TLB still holds the old translation; flush it before reusing the frame"]
pub struct Unmapped {
    /// Physical base of the page that was mapped.
    pub frame: u64,
    pub size: PageSize,
    /// Flags the leaf had, without [`PageFlags::HUGE`].
    pub flags: PageFlags,
}

/// Frames of page tables unlinked by [`PageTables::unmap_and_prune`], at
/// most one each of PT, PD and PDPT (the root is never freed).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
#[must_use = "these frames leak unless returned to the frame allocator after the TLB flush"]
pub struct FreedTables {
    frames: [u64; 3],
    len: usize,
}

impl FreedTables {
    /// Lowest level first.
    pub fn as_slice(&self) -> &[u64] {
        &self.frames[..self.len]
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    fn push(&mut self, frame: u64) {
        self.frames[self.len] = frame;
        self.len += 1;
    }
}

/// The tables visited on a walk: `path[4 - level]` is the table at `level`,
/// so `path[0]` is the PML4.
type Path = [u64; 4];

/// Result of a read-only walk for [`PageTables::map`]: the slot is free.
struct Probe {
    path: Path,
    /// Level of the deepest existing table on the way to the leaf. Tables
    /// for `level - 1` down to the leaf level must be created.
    level: usize,
}

/// A present leaf found by a walk.
struct Leaf {
    path: Path,
    level: usize,
    entry: u64,
}

impl Leaf {
    fn size(&self) -> PageSize {
        PageSize::from_level(self.level)
    }

    fn frame(&self) -> u64 {
        self.entry & ADDR_MASK & !(self.size().bytes() - 1)
    }

    fn flags(&self) -> PageFlags {
        PageFlags::from_bits(self.entry).difference(PageFlags::HUGE)
    }
}

/// A 4-level page-table hierarchy rooted at a PML4, together with the
/// accessor for its memory.
#[derive(Debug)]
pub struct PageTables<M> {
    root: u64,
    mem: M,
}

impl<M: PhysMemory> PageTables<M> {
    /// Allocate and clear a new, empty PML4.
    pub fn new(mut mem: M, frames: &mut impl FrameAllocator) -> Result<PageTables<M>, Error> {
        let root = frames.allocate_frame().ok_or(Error::OutOfFrames)?;
        mem.zero_table(root);
        Ok(PageTables { root, mem })
    }

    /// Take over an existing hierarchy, e.g. the one CR3 points to. `root`
    /// must be the 4 KiB-aligned physical address of a valid PML4.
    pub fn from_root(root: u64, mem: M) -> PageTables<M> {
        debug_assert!(root & !ADDR_MASK == 0, "PML4 address must be 4 KiB aligned and below 2^52");
        PageTables { root, mem }
    }

    /// Physical address of the PML4 (the address part of CR3).
    pub fn root(&self) -> u64 {
        self.root
    }

    pub fn memory(&self) -> &M {
        &self.mem
    }

    pub fn memory_mut(&mut self) -> &mut M {
        &mut self.mem
    }

    pub fn into_memory(self) -> M {
        self.mem
    }

    /// Map one page of `size` at `virt` to `phys`, creating intermediate
    /// tables as needed.
    ///
    /// `PRESENT` is always set and `HUGE` is set exactly for 2 MiB and 1 GiB
    /// pages, whatever `flags` says. With `USER` in `flags`, U/S is also set
    /// on every entry along the path. On error nothing has changed and no
    /// frame is kept.
    pub fn map(
        &mut self,
        virt: u64,
        phys: u64,
        size: PageSize,
        flags: PageFlags,
        frames: &mut impl FrameAllocator,
    ) -> Result<(), Error> {
        check_virt(virt, size)?;
        check_phys(phys, size)?;
        let probe = self.probe(virt, size)?;
        self.apply(virt, phys, size, flags, probe, frames)
    }

    /// Map `[virt, virt + len)` to `[phys, phys + len)` with the largest pages
    /// that fit, never larger than `largest`: at each step the biggest page
    /// size to which both addresses are aligned and that does not run past
    /// the end. This is how the kernel maps all of RAM. Pass
    /// [`PageSize::Size2M`] as `largest` on CPUs without 1 GiB pages.
    ///
    /// All three of `virt`, `phys` and `len` must be 4 KiB aligned. The whole
    /// range is checked before anything is written, so an overlap with an
    /// existing mapping fails cleanly. If frames run out half-way, the pages
    /// mapped by this call are unmapped again (empty tables it created may
    /// stay linked) and [`Error::OutOfFrames`] is returned; as with
    /// [`PageTables::unmap`], flush the range if these tables are live.
    pub fn map_range(
        &mut self,
        virt: u64,
        phys: u64,
        len: u64,
        flags: PageFlags,
        largest: PageSize,
        frames: &mut impl FrameAllocator,
    ) -> Result<(), Error> {
        let small = PageSize::Size4K;
        if !small.is_aligned(virt) || !small.is_aligned(phys) || !small.is_aligned(len) {
            return Err(Error::Misaligned);
        }
        if len == 0 {
            return Ok(());
        }
        let last = virt.checked_add(len - 1).ok_or(Error::NonCanonical)?;
        // Both ends canonical and on the same side of the hole.
        if !is_canonical(virt) || !is_canonical(last) || (virt >> 63) != (last >> 63) {
            return Err(Error::NonCanonical);
        }
        match phys.checked_add(len - 1) {
            Some(p) if p < PHYS_LIMIT => {}
            _ => return Err(Error::InvalidPhysical),
        }

        // Pages of the range are disjoint, so mapping one never makes another
        // one's slot busy: checking them all up front is enough.
        let mut off = 0;
        while off < len {
            let size = largest_fit(virt + off, phys + off, len - off, largest);
            self.probe(virt + off, size)?;
            off += size.bytes();
        }

        let mut off = 0;
        while off < len {
            let size = largest_fit(virt + off, phys + off, len - off, largest);
            let res = match self.probe(virt + off, size) {
                Ok(probe) => self.apply(virt + off, phys + off, size, flags, probe, frames),
                Err(e) => Err(e),
            };
            if let Err(e) = res {
                self.unmap_first(virt, phys, off, largest);
                return Err(e);
            }
            off += size.bytes();
        }
        Ok(())
    }

    /// Where `virt` leads, if anywhere.
    pub fn translate(&self, virt: u64) -> Result<Translation, Error> {
        if !is_canonical(virt) {
            return Err(Error::NonCanonical);
        }
        let leaf = self.find(virt)?;
        let frame = leaf.frame();
        Ok(Translation {
            phys: frame + (virt & (leaf.size().bytes() - 1)),
            frame,
            size: leaf.size(),
            flags: leaf.flags(),
        })
    }

    /// Remove the page that starts at `virt`, whatever its size. Intermediate
    /// tables stay, even if empty.
    ///
    /// `virt` must be the first address of the page; inside a larger page
    /// this is [`Error::HugePageConflict`]. The caller must flush the TLB for
    /// the page (on every CPU using these tables) before reusing the frame.
    pub fn unmap(&mut self, virt: u64) -> Result<Unmapped, Error> {
        let leaf = self.find_page(virt)?;
        self.mem.write_entry(leaf.path[4 - leaf.level], index(virt, leaf.level), 0);
        Ok(Unmapped { frame: leaf.frame(), size: leaf.size(), flags: leaf.flags() })
    }

    /// Like [`PageTables::unmap`], then unlink every table on the path that
    /// is now completely empty (all entries zero), bottom-up. The PML4 is
    /// never freed.
    ///
    /// The unlinked frames are returned rather than freed: until the TLB
    /// flush for `virt` has completed on every CPU using these tables, the
    /// paging-structure caches may still walk through them. Hand them to the
    /// frame allocator only after that.
    pub fn unmap_and_prune(&mut self, virt: u64) -> Result<(Unmapped, FreedTables), Error> {
        let leaf = self.find_page(virt)?;
        self.mem.write_entry(leaf.path[4 - leaf.level], index(virt, leaf.level), 0);
        let mut freed = FreedTables::default();
        let mut level = leaf.level;
        while level < 4 {
            let table = leaf.path[4 - level];
            if (0..ENTRIES).any(|i| self.mem.read_entry(table, i) != 0) {
                break;
            }
            self.mem.write_entry(leaf.path[4 - level - 1], index(virt, level + 1), 0);
            freed.push(table);
            level += 1;
        }
        Ok((Unmapped { frame: leaf.frame(), size: leaf.size(), flags: leaf.flags() }, freed))
    }

    /// Replace the flags of the page that starts at `virt` and return the
    /// old ones (both without [`PageFlags::HUGE`]). The frame and page size
    /// stay; `PRESENT` stays set (use [`PageTables::unmap`] to remove a page).
    /// Adding `USER` also sets U/S along the path.
    ///
    /// As with [`PageTables::unmap`], `virt` must be the first address of the
    /// page, and the caller must flush the TLB for the page whenever the
    /// change removes a permission.
    pub fn update_flags(&mut self, virt: u64, flags: PageFlags) -> Result<PageFlags, Error> {
        let leaf = self.find_page(virt)?;
        let size = leaf.size();
        if flags.contains(PageFlags::USER) {
            self.make_path_permissive(virt, &leaf.path, leaf.level + 1, true);
        }
        self.mem.write_entry(leaf.path[4 - leaf.level], index(virt, leaf.level), leaf_entry(leaf.frame(), size, flags));
        Ok(leaf.flags())
    }

    /// Read-only walk towards the slot for a `size` page at `virt`; fails if
    /// it is not free.
    fn probe(&self, virt: u64, size: PageSize) -> Result<Probe, Error> {
        let target = size.level();
        let mut path = [self.root, 0, 0, 0];
        let mut level = 4;
        loop {
            let entry = self.mem.read_entry(path[4 - level], index(virt, level));
            let present = entry & PageFlags::PRESENT.0 != 0;
            if level == target {
                return if present { Err(Error::AlreadyMapped) } else { Ok(Probe { path, level }) };
            }
            if !present {
                return Ok(Probe { path, level });
            }
            // PS is reserved in a PML4 entry, so only PDPT and PD entries can
            // be leaves on the way down.
            if level <= 3 && entry & PageFlags::HUGE.0 != 0 {
                return Err(Error::HugePageConflict);
            }
            level -= 1;
            path[4 - level] = entry & ADDR_MASK;
        }
    }

    /// Second half of [`PageTables::map`]: allocate all missing tables first,
    /// so running out of frames leaves nothing behind, then link them in
    /// bottom-up so the new mapping becomes reachable with the last write.
    fn apply(
        &mut self,
        virt: u64,
        phys: u64,
        size: PageSize,
        flags: PageFlags,
        probe: Probe,
        frames: &mut impl FrameAllocator,
    ) -> Result<(), Error> {
        let target = size.level();
        let missing = probe.level - target;
        // new[0] becomes the table at probe.level - 1, new[missing - 1] the one at target.
        let mut new = [0u64; 3];
        for i in 0..missing {
            match frames.allocate_frame() {
                Some(frame) => {
                    debug_assert!(frame & !ADDR_MASK == 0, "frame allocator returned a bad frame {frame:#x}");
                    new[i] = frame;
                }
                None => {
                    for &frame in &new[..i] {
                        frames.deallocate_frame(frame);
                    }
                    return Err(Error::OutOfFrames);
                }
            }
        }

        let user = flags.contains(PageFlags::USER);
        self.make_path_permissive(virt, &probe.path, probe.level + 1, user);

        let table_flags = TABLE_FLAGS | if user { PageFlags::USER.0 } else { 0 };
        let mut entry = leaf_entry(phys, size, flags);
        let mut level = target;
        for &table in new[..missing].iter().rev() {
            self.mem.zero_table(table);
            self.mem.write_entry(table, index(virt, level), entry);
            entry = table | table_flags;
            level += 1;
        }
        self.mem.write_entry(probe.path[4 - level], index(virt, level), entry);
        Ok(())
    }

    /// Bring the existing non-leaf entries at levels `from..=4` of `path`
    /// into the shape of the intermediate policy.
    fn make_path_permissive(&mut self, virt: u64, path: &Path, from: usize, user: bool) {
        let want = TABLE_FLAGS | if user { PageFlags::USER.0 } else { 0 };
        for level in from..=4 {
            let table = path[4 - level];
            let i = index(virt, level);
            let old = self.mem.read_entry(table, i);
            let new = (old | want) & !PageFlags::NO_EXECUTE.0;
            if new != old {
                self.mem.write_entry(table, i, new);
            }
        }
    }

    /// The present leaf covering `virt`.
    fn find(&self, virt: u64) -> Result<Leaf, Error> {
        let mut path = [self.root, 0, 0, 0];
        let mut level = 4;
        loop {
            let entry = self.mem.read_entry(path[4 - level], index(virt, level));
            if entry & PageFlags::PRESENT.0 == 0 {
                return Err(Error::NotMapped);
            }
            if level == 1 || (level <= 3 && entry & PageFlags::HUGE.0 != 0) {
                return Ok(Leaf { path, level, entry });
            }
            level -= 1;
            path[4 - level] = entry & ADDR_MASK;
        }
    }

    /// The present leaf whose page starts exactly at `virt`.
    fn find_page(&self, virt: u64) -> Result<Leaf, Error> {
        check_virt(virt, PageSize::Size4K)?;
        let leaf = self.find(virt)?;
        if !leaf.size().is_aligned(virt) {
            return Err(Error::HugePageConflict);
        }
        Ok(leaf)
    }

    /// Undo the first `len` bytes of a [`PageTables::map_range`] call.
    fn unmap_first(&mut self, virt: u64, phys: u64, len: u64, largest: PageSize) {
        let mut off = 0;
        while off < len {
            let size = largest_fit(virt + off, phys + off, len - off, largest);
            let res = self.unmap(virt + off);
            debug_assert!(matches!(res, Ok(u) if u.size == size), "rollback found {res:?}");
            off += size.bytes();
        }
    }
}

fn check_virt(virt: u64, size: PageSize) -> Result<(), Error> {
    if !is_canonical(virt) {
        Err(Error::NonCanonical)
    } else if !size.is_aligned(virt) {
        Err(Error::Misaligned)
    } else {
        Ok(())
    }
}

fn check_phys(phys: u64, size: PageSize) -> Result<(), Error> {
    if !size.is_aligned(phys) {
        Err(Error::Misaligned)
    } else if phys >= PHYS_LIMIT {
        Err(Error::InvalidPhysical)
    } else {
        Ok(())
    }
}

/// Leaf entry for a page at `phys` (already aligned and in range).
fn leaf_entry(phys: u64, size: PageSize, flags: PageFlags) -> u64 {
    let mut bits = flags.difference(PageFlags::HUGE).union(PageFlags::PRESENT);
    if size != PageSize::Size4K {
        bits |= PageFlags::HUGE;
    }
    phys | bits.0
}

/// Largest page size, up to `largest`, for the next step of a range mapping.
fn largest_fit(virt: u64, phys: u64, remaining: u64, largest: PageSize) -> PageSize {
    for size in [PageSize::Size1G, PageSize::Size2M] {
        if size <= largest && size.is_aligned(virt) && size.is_aligned(phys) && remaining >= size.bytes() {
            return size;
        }
    }
    PageSize::Size4K
}

#[cfg(test)]
mod tests;
