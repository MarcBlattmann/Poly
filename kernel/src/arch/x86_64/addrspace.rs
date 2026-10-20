//! User address spaces: one PML4 per process, sharing the kernel half.
//!
//! An [`AddressSpace`] has its own PML4. The lower half (slots 0-255) is
//! the process's own and starts empty; the higher half (slots 256-511) is
//! copied from the kernel PML4 when the space is created and points at the
//! kernel's own PDPTs, so every kernel mapping is visible (to ring 0 only)
//! in every address space ([ADR 0010](../../../../docs/adr/0010-kernel-address-space.md)).
//! The kernel creates all of its PML4 entries at boot (`paging::init`), so
//! the copy never goes stale.
//!
//! # User half
//!
//! | Virtual | What |
//! |---|---|
//! | `0x0` – [`USER_START`] | never mapped: catches null-pointer accesses |
//! | [`USER_START`] – [`USER_END`] | user mappings: 4 KiB pages, user-accessible |
//! | [`USER_END`] – `0x7fff_ffff_ffff` | never mapped: the last page below the canonical hole |
//!
//! Keeping the last page unmapped means no `syscall` instruction can sit
//! at its very end, so the return address `syscall` leaves in RCX is always
//! canonical (see `syscall.rs` for why SYSRET cares).
//!
//! # Frames
//!
//! [`AddressSpace::map_zeroed`] takes fresh frames from `FRAMES` and the
//! space owns them: they are freed when they are unmapped and when the
//! space is dropped, together with every page table of the lower half and
//! the PML4. [`AddressSpace::map_frames`] maps frames the caller keeps
//! owning. Which is which is recorded in each page-table entry, in a bit
//! the CPU ignores ([`OWNED`]).
//!
//! Demand paging (`user::Memory`) fills one page at a time: it takes a
//! [`UserFrame`], fills it (zeros, or a file's bytes) and hands it to
//! [`AddressSpace::map_owned`]; [`AddressSpace::take`] unmaps a page and
//! hands its frame back out (a page parked while its region is
//! `PROT_NONE`). A `UserFrame` that is dropped frees its frame.
//! [`user_frames`] counts every frame held for user pages, so tests can
//! check that unmapping and teardown free them.
//!
//! # TLB and PCIDs
//!
//! Without PCIDs, every CR3 load flushes all non-global translations, so
//! only the space that is loaded can have stale entries; unmapping or
//! write-protecting a page of it invalidates the page with `INVLPG`.
//!
//! With PCIDs (CPUID.01H:ECX.PCID, CR4.PCIDE), translations are tagged with
//! the 12-bit PCID in CR3[11:0] and survive loads with bit 63 set
//! (SDM Vol. 3A §4.10.1, §4.10.4.1). Address spaces get PCIDs from a small
//! table, round robin; [`PCID_OWNER`] remembers which space each was last
//! given to. Loading a space whose PCID is still its own skips the flush;
//! otherwise it takes the next PCID and loads CR3 *without* bit 63, which
//! flushes whatever the previous owner left under it. A space that changes
//! while it is not loaded forgets its PCID, so its next load flushes too.
//! The kernel PML4 uses PCID 0. Kernel mappings are global, so they are
//! never flushed by any of this.
//!
//! TODO(smp): a space loaded on another CPU needs a TLB shootdown (IPI)
//! when it changes, and PCIDs are per CPU.
//!
//! # Locking
//!
//! Each space's tables are behind a spinlock, taken before `FRAMES`.

use super::cpu;
use super::paging::{self, BuddyFrames, KernelPhysMemory, fix};
use crate::frames::FRAMES;
use crate::sync::SpinLock;
use core::sync::atomic::{AtomicBool, AtomicU16, AtomicU64, Ordering};
use poly_mem::{BuddyAllocator, PAGE_SIZE};
use poly_paging::{ADDR_MASK, Cleared, ENTRIES, PageFlags, PageSize, PageTables, PhysMemory};

/// Lowest user address that can be mapped (64 KiB): the pages below stay
/// unmapped so that a null pointer, or a small offset from one, faults.
pub const USER_START: u64 = 0x1_0000;

/// End of the mappable user range: the last page of the lower half is
/// never mapped (see the module docs).
pub const USER_END: u64 = 0x7fff_ffff_f000;

/// First address above the lower half: user addresses are below it, and
/// so are the addresses SYSRET may return to.
pub const LOWER_HALF_END: u64 = 0x0000_8000_0000_0000;

/// Set in every leaf whose frame the space owns (allocated by
/// [`AddressSpace::map_zeroed`], freed on unmap). The CPU ignores the bit.
const OWNED: PageFlags = PageFlags::SOFTWARE_9;

/// CR3 bit 63 with CR4.PCIDE = 1: keep the translations of the PCID being
/// loaded (SDM Vol. 3A §4.10.4.1).
const CR3_NO_FLUSH: u64 = 1 << 63;

/// Access rights of a user page. x86 cannot express write-only or
/// execute-only pages: any non-empty set is readable.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Prot(u8);

impl Prot {
    pub const NONE: Prot = Prot(0);
    pub const READ: Prot = Prot(1);
    pub const WRITE: Prot = Prot(2);
    pub const EXEC: Prot = Prot(4);
    pub const RW: Prot = Prot(1 | 2);
    pub const RX: Prot = Prot(1 | 4);

    /// The rights with these `PROT_READ` (1), `PROT_WRITE` (2) and
    /// `PROT_EXEC` (4) bits (Linux UAPI `asm-generic/mman-common.h`);
    /// other bits are ignored.
    pub const fn from_bits(bits: u8) -> Prot {
        Prot(bits & 7)
    }

    /// The `PROT_*` bits.
    pub const fn bits(self) -> u8 {
        self.0
    }

    pub const fn contains(self, other: Prot) -> bool {
        self.0 & other.0 == other.0
    }

    pub const fn union(self, other: Prot) -> Prot {
        Prot(self.0 | other.0)
    }

    /// The leaf flags for these rights. `None` for [`Prot::NONE`], which
    /// has no present-page encoding (leave such pages unmapped).
    fn flags(self) -> Option<PageFlags> {
        if self == Prot::NONE {
            return None;
        }
        let mut f = PageFlags::PRESENT | PageFlags::USER;
        if self.contains(Prot::WRITE) {
            f |= PageFlags::WRITABLE;
        }
        if !self.contains(Prot::EXEC) {
            f |= PageFlags::NO_EXECUTE;
        }
        Some(fix(f))
    }

    /// What a present user leaf allows.
    fn of(flags: PageFlags) -> Prot {
        let mut p = Prot::READ;
        if flags.contains(PageFlags::WRITABLE) {
            p = p.union(Prot::WRITE);
        }
        if !flags.contains(PageFlags::NO_EXECUTE) {
            p = p.union(Prot::EXEC);
        }
        p
    }
}

impl core::ops::BitOr for Prot {
    type Output = Prot;
    fn bitor(self, rhs: Prot) -> Prot {
        self.union(rhs)
    }
}

impl core::fmt::Debug for Prot {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let bit = |p: Prot, c: char| if self.contains(p) { c } else { '-' };
        write!(f, "{}{}{}", bit(Prot::READ, 'r'), bit(Prot::WRITE, 'w'), bit(Prot::EXEC, 'x'))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MapError {
    /// Part of the range lies outside [`USER_START`]..[`USER_END`].
    OutsideUserSpace,
    /// An address or length is not a multiple of the page size.
    Misaligned,
    /// A page of the range is already mapped.
    AlreadyMapped,
    /// A page of the range is not mapped.
    NotMapped,
    /// [`Prot::NONE`] cannot be mapped.
    NoAccess,
    /// No frame for a page or a page table.
    OutOfMemory,
}

/// Where a user page leads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Mapping {
    /// Physical address of the byte asked about.
    pub phys: u64,
    pub prot: Prot,
    /// Whether the space owns (and will free) the frame.
    pub owned: bool,
}

/// Whether CR4.PCIDE is set.
static PCID: AtomicBool = AtomicBool::new(false);

/// PCIDs handed out, 1 to `PCIDS - 1` (0 is the kernel's).
const PCIDS: usize = 256;

/// The [`AddressSpace::id`] each PCID was last given to.
static PCID_OWNER: [AtomicU64; PCIDS] = [const { AtomicU64::new(0) }; PCIDS];
static NEXT_PCID: AtomicU16 = AtomicU16::new(1);
static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// Frames held for user pages: owned by an address space, or by a
/// [`UserFrame`] on its way in or out of one.
static USER_FRAMES: AtomicU64 = AtomicU64::new(0);

/// How many frames are held for user pages right now, in all address
/// spaces together (page tables not included).
pub fn user_frames() -> u64 {
    USER_FRAMES.load(Ordering::Relaxed)
}

/// One frame for a user page, not mapped anywhere: filled by the kernel,
/// then given to [`AddressSpace::map_owned`]. Dropping it frees the frame.
#[derive(Debug)]
pub struct UserFrame {
    phys: u64,
}

impl UserFrame {
    /// A zeroed frame, or `None` if memory ran out.
    pub fn zeroed() -> Option<UserFrame> {
        let phys = FRAMES.lock().as_mut().and_then(|f| f.allocate(0))?;
        USER_FRAMES.fetch_add(1, Ordering::Relaxed);
        let mut frame = UserFrame { phys };
        frame.bytes_mut().fill(0);
        Some(frame)
    }

    /// The frame's contents, through the direct map.
    pub fn bytes_mut(&mut self) -> &mut [u8; PAGE_SIZE as usize] {
        let virt = paging::phys_to_virt(self.phys).expect("frame outside the direct map");
        // SAFETY: the frame belongs to this value alone (allocated for it,
        // or taken out of the page tables by `take`), it is RAM reached
        // through the direct map, writable and page-aligned, and the borrow
        // of `self` keeps any other access out while the reference lives.
        unsafe { &mut *(virt as *mut [u8; PAGE_SIZE as usize]) }
    }

    /// Physical address of the frame.
    pub fn phys(&self) -> u64 {
        self.phys
    }

    /// Give up ownership without freeing: the caller now owns the frame.
    fn into_phys(self) -> u64 {
        let phys = self.phys;
        core::mem::forget(self);
        phys
    }
}

impl Drop for UserFrame {
    fn drop(&mut self) {
        USER_FRAMES.fetch_sub(1, Ordering::Relaxed);
        if let Some(f) = FRAMES.lock().as_mut() {
            f.free(self.phys, 0).expect("user frame freed twice");
        }
    }
}

/// Turn on PCIDs. Call once, with the kernel PML4 loaded (PCID 0): CR4.PCIDE
/// can only be set while CR3[11:0] is 0.
///
/// # Safety
/// The CPU has PCIDs (CPUID.01H:ECX[17]).
pub(super) unsafe fn enable_pcid() {
    assert_eq!(cpu::read_cr3() & 0xfff, 0, "PCIDE needs PCID 0 loaded");
    // SAFETY: the caller checked CPUID; CR3[11:0] is 0, so setting PCIDE
    // changes no translation (SDM Vol. 3A §4.10.1).
    unsafe { cpu::write_cr4(cpu::read_cr4() | cpu::CR4_PCIDE) };
    PCID.store(true, Ordering::Relaxed);
}

pub fn pcid_enabled() -> bool {
    PCID.load(Ordering::Relaxed)
}

/// Load the kernel PML4 unless it is loaded already: for kernel threads,
/// which have no address space of their own.
pub(super) fn activate_kernel() {
    let kernel = paging::kernel_root();
    if cpu::read_cr3() & ADDR_MASK != kernel {
        let flags = if pcid_enabled() { CR3_NO_FLUSH } else { 0 };
        // SAFETY: the kernel PML4 maps the kernel, which is all that runs
        // (this stack is a kernel stack). Its lower half is empty, so no
        // translation under PCID 0 needs flushing.
        unsafe { cpu::write_cr3(kernel | flags) };
    }
}

/// A user address space. Shared between the threads of a process (behind
/// an `Arc`); dropping the last reference frees everything it owns.
#[derive(Debug)]
pub struct AddressSpace {
    tables: SpinLock<PageTables<KernelPhysMemory>>,
    /// Unique for the kernel's lifetime: names the space in [`PCID_OWNER`].
    id: u64,
    /// The PCID last given to this space; 0 if it has none (or must flush).
    pcid: AtomicU16,
    root: u64,
    /// Pages mapped with a frame the space owns.
    resident: AtomicU64,
}

impl AddressSpace {
    /// A new space with an empty lower half and the kernel's higher half.
    pub fn new() -> Result<AddressSpace, MapError> {
        let kernel = paging::kernel_root();
        assert_ne!(kernel, 0, "user address space before the kernel page tables");
        let root = FRAMES.lock().as_mut().and_then(|f| f.allocate(0)).ok_or(MapError::OutOfMemory)?;
        let mut mem = KernelPhysMemory;
        for i in 0..ENTRIES {
            let entry = if i < ENTRIES / 2 { 0 } else { mem.read_entry(kernel, i) };
            mem.write_entry(root, i, entry);
        }
        Ok(AddressSpace {
            tables: SpinLock::new(PageTables::from_root(root, mem)),
            id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
            pcid: AtomicU16::new(0),
            root,
            resident: AtomicU64::new(0),
        })
    }

    /// Physical address of the PML4.
    pub fn root(&self) -> u64 {
        self.root
    }

    /// Whether CR3 points at this space.
    pub fn is_active(&self) -> bool {
        cpu::read_cr3() & ADDR_MASK == self.root
    }

    /// Map fresh zeroed frames at `[virt, virt + len)` with `prot`. The
    /// space owns them. All or nothing: if any page is mapped already, or
    /// memory runs out, nothing changes.
    pub fn map_zeroed(&self, virt: u64, len: u64, prot: Prot) -> Result<(), MapError> {
        let end = check_range(virt, len)?;
        let flags = prot.flags().ok_or(MapError::NoAccess)? | OWNED;
        let mut tables = self.tables.lock();
        free_slots(&tables, virt, end)?;
        let mut frames = FRAMES.lock();
        let alloc = frames.as_mut().ok_or(MapError::OutOfMemory)?;
        for page in (virt..end).step_by(PAGE_SIZE as usize) {
            let mapped = alloc.allocate(0).ok_or(MapError::OutOfMemory).and_then(|frame| {
                let ptr = paging::phys_to_virt(frame).expect("frame outside the direct map") as *mut u8;
                // SAFETY: a frame just allocated, so nothing else uses it,
                // reached through the direct map, which maps it writable.
                unsafe { ptr.write_bytes(0, PAGE_SIZE as usize) };
                let res = tables.map(page, frame, PageSize::Size4K, flags, &mut BuddyFrames(alloc));
                res.map(|()| self.gained_page()).map_err(|_| {
                    alloc.free(frame, 0).expect("freeing a frame just allocated");
                    MapError::OutOfMemory
                })
            });
            if let Err(e) = mapped {
                for done in (virt..page).step_by(PAGE_SIZE as usize) {
                    self.unmap_page(&mut tables, alloc, done);
                }
                return Err(e);
            }
        }
        Ok(())
    }

    /// Map the frames at `[phys, phys + len)` at `[virt, virt + len)` with
    /// `prot`, without taking ownership: unmapping or dropping the space
    /// leaves them alone. All or nothing, like
    /// [`map_zeroed`](Self::map_zeroed).
    ///
    /// # Safety
    /// The frames stay allocated, and hold nothing user mode must not see or
    /// change, for as long as they are mapped here.
    pub unsafe fn map_frames(&self, virt: u64, phys: u64, len: u64, prot: Prot) -> Result<(), MapError> {
        let end = check_range(virt, len)?;
        if !phys.is_multiple_of(PAGE_SIZE) {
            return Err(MapError::Misaligned);
        }
        let flags = prot.flags().ok_or(MapError::NoAccess)?;
        let mut tables = self.tables.lock();
        free_slots(&tables, virt, end)?;
        let mut frames = FRAMES.lock();
        let alloc = frames.as_mut().ok_or(MapError::OutOfMemory)?;
        for (page, frame) in (virt..end).step_by(PAGE_SIZE as usize).zip((phys..).step_by(PAGE_SIZE as usize)) {
            if tables.map(page, frame, PageSize::Size4K, flags, &mut BuddyFrames(alloc)).is_err() {
                for done in (virt..page).step_by(PAGE_SIZE as usize) {
                    self.unmap_page(&mut tables, alloc, done);
                }
                return Err(MapError::OutOfMemory);
            }
        }
        Ok(())
    }

    /// Map `frame` at the page `virt` with `prot`; the space owns it from
    /// now on. On error (the page is mapped already, no memory for a page
    /// table, ...) the frame is freed.
    pub fn map_owned(&self, virt: u64, frame: UserFrame, prot: Prot) -> Result<(), MapError> {
        check_range(virt, PAGE_SIZE)?;
        let flags = prot.flags().ok_or(MapError::NoAccess)? | OWNED;
        let mut tables = self.tables.lock();
        free_slots(&tables, virt, virt + PAGE_SIZE)?;
        let mut frames = FRAMES.lock();
        let alloc = frames.as_mut().ok_or(MapError::OutOfMemory)?;
        if tables.map(virt, frame.phys(), PageSize::Size4K, flags, &mut BuddyFrames(alloc)).is_err() {
            drop(frames);
            return Err(MapError::OutOfMemory);
        }
        // The frame was counted in USER_FRAMES when it was allocated.
        frame.into_phys();
        self.resident.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    /// Unmap the page `virt` and hand its frame out, instead of freeing it,
    /// if the space owns it. `None` if nothing is mapped there or the frame
    /// is not the space's (then it is just unmapped).
    pub fn take(&self, virt: u64) -> Option<UserFrame> {
        check_range(virt, PAGE_SIZE).ok()?;
        let mut tables = self.tables.lock();
        let (old, freed) = tables.unmap_and_prune(virt).ok()?;
        self.flush(virt);
        let mut frames = FRAMES.lock();
        let alloc = frames.as_mut().expect("address spaces need the frame allocator");
        for &table in freed.as_slice() {
            alloc.free(table, 0).expect("page table freed twice");
        }
        if !old.flags.contains(OWNED) {
            return None;
        }
        self.resident.fetch_sub(1, Ordering::Relaxed);
        // The count of user frames stays: the frame moves to the UserFrame.
        Some(UserFrame { phys: old.frame })
    }

    /// The pages mapped in `[start, end)`, in address order, with where
    /// each leads (empty for a range outside user space). Walks only the
    /// page tables that exist (`fork` visits whole regions this way).
    pub fn mapped_pages(&self, start: u64, end: u64) -> alloc::vec::Vec<(u64, Mapping)> {
        let mut out = alloc::vec::Vec::new();
        let (start, end) = (start.max(USER_START), end.min(USER_END));
        let tables = self.tables.lock();
        tables.mapped(start, end, |virt, t| {
            out.push((virt, Mapping { phys: t.phys, prot: Prot::of(t.flags), owned: t.flags.contains(OWNED) }));
        });
        out
    }

    /// Keep the page `virt` mapped, with `prot`, but give its frame up: it
    /// is no longer the space's (unmapping or dropping the space leaves it
    /// alone), and the caller owns it from now on (copy on write after
    /// `fork`: the frame becomes shared). `None` if nothing is mapped there
    /// or the frame is not the space's.
    ///
    /// # Safety
    /// The caller keeps the returned frame allocated, and holding nothing
    /// user mode must not see, for as long as it stays mapped here.
    pub unsafe fn disown(&self, virt: u64, prot: Prot) -> Option<UserFrame> {
        check_range(virt, PAGE_SIZE).ok()?;
        let flags = prot.flags()?;
        let mut tables = self.tables.lock();
        let t = tables.translate(virt).ok()?;
        if !t.flags.contains(OWNED) {
            return None;
        }
        tables.update_flags(virt, flags).expect("translated above");
        self.flush(virt);
        self.resident.fetch_sub(1, Ordering::Relaxed);
        // The count of user frames stays: the frame moves to the UserFrame.
        Some(UserFrame { phys: t.frame })
    }

    /// Take ownership of `frame`, which is mapped at `virt` already but not
    /// owned by the space (the reverse of [`disown`](Self::disown): the
    /// last user of a shared frame keeps it), and give the page `prot`.
    /// Hands the frame back if `virt` does not map it, or maps an owned
    /// frame.
    pub fn adopt(&self, virt: u64, frame: UserFrame, prot: Prot) -> Result<(), UserFrame> {
        let Some(flags) = prot.flags() else { return Err(frame) };
        if check_range(virt, PAGE_SIZE).is_err() {
            return Err(frame);
        }
        let mut tables = self.tables.lock();
        match tables.translate(virt) {
            Ok(t) if t.frame == frame.phys() && !t.flags.contains(OWNED) => {}
            _ => return Err(frame),
        }
        tables.update_flags(virt, flags | OWNED).expect("translated above");
        self.flush(virt);
        frame.into_phys();
        self.resident.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    /// Pages mapped with a frame the space owns.
    pub fn resident_pages(&self) -> u64 {
        self.resident.load(Ordering::Relaxed)
    }

    /// Unmap every page of `[virt, virt + len)` that is mapped (holes are
    /// skipped), free the frames the space owns and the page tables that
    /// became empty.
    pub fn unmap(&self, virt: u64, len: u64) -> Result<(), MapError> {
        let end = check_range(virt, len)?;
        let mut tables = self.tables.lock();
        let mut frames = FRAMES.lock();
        let alloc = frames.as_mut().ok_or(MapError::OutOfMemory)?;
        for page in (virt..end).step_by(PAGE_SIZE as usize) {
            self.unmap_page(&mut tables, alloc, page);
        }
        Ok(())
    }

    /// Change the rights of every page of `[virt, virt + len)`, all of
    /// which must be mapped (else nothing changes).
    pub fn protect(&self, virt: u64, len: u64, prot: Prot) -> Result<(), MapError> {
        let end = check_range(virt, len)?;
        let flags = prot.flags().ok_or(MapError::NoAccess)?;
        let mut tables = self.tables.lock();
        for page in (virt..end).step_by(PAGE_SIZE as usize) {
            tables.translate(page).map_err(|_| MapError::NotMapped)?;
        }
        for page in (virt..end).step_by(PAGE_SIZE as usize) {
            let owned = tables.translate(page).expect("checked above").flags & OWNED;
            tables.update_flags(page, flags | owned).expect("checked above");
            self.flush(page);
        }
        Ok(())
    }

    /// Where the user address `virt` leads, if it is mapped.
    pub fn translate(&self, virt: u64) -> Option<Mapping> {
        if !(USER_START..USER_END).contains(&virt) {
            return None;
        }
        let t = self.tables.lock().translate(virt).ok()?;
        Some(Mapping { phys: t.phys, prot: Prot::of(t.flags), owned: t.flags.contains(OWNED) })
    }

    /// Copy `data` to the user address `virt` of this space, through the
    /// direct map: works whether or not the space is loaded, and whatever
    /// the pages' rights (the loader fills read-only text this way). Every
    /// page must be mapped; on error nothing has been written.
    pub fn write(&self, virt: u64, data: &[u8]) -> Result<(), MapError> {
        let tables = self.tables.lock();
        let len = data.len() as u64;
        each_chunk(&tables, virt, len, |_, _, _| ())?;
        each_chunk(&tables, virt, len, |kernel, n, off| {
            let chunk = &data[off..off + n];
            // SAFETY: `kernel` is the direct-map address of `n` bytes of a
            // frame mapped in this space, which stays mapped while `tables`
            // is locked. A user thread may be writing it concurrently, but
            // plain bytes have no invalid values.
            unsafe { core::ptr::copy_nonoverlapping(chunk.as_ptr(), kernel as *mut u8, n) };
        })
    }

    /// Copy from the user address `virt` of this space into `buf`, through
    /// the direct map (see [`write`](Self::write)).
    pub fn read(&self, virt: u64, buf: &mut [u8]) -> Result<(), MapError> {
        let tables = self.tables.lock();
        let len = buf.len() as u64;
        each_chunk(&tables, virt, len, |_, _, _| ())?;
        each_chunk(&tables, virt, len, |kernel, n, off| {
            let chunk = &mut buf[off..off + n];
            // SAFETY: as in `write`.
            unsafe { core::ptr::copy_nonoverlapping(kernel as *const u8, chunk.as_mut_ptr(), n) };
        })
    }

    /// Load this space into CR3 (see the module docs for the PCIDs).
    /// Called by the scheduler when it switches to one of its threads.
    pub(super) fn activate(&self) {
        let cr3 = cpu::read_cr3();
        if !pcid_enabled() {
            if cr3 & ADDR_MASK != self.root {
                // SAFETY: the higher half is the kernel's, so the running
                // code, its stack and all kernel data stay mapped.
                unsafe { cpu::write_cr3(self.root) };
            }
            return;
        }
        let pcid = self.pcid.load(Ordering::Relaxed);
        if pcid != 0 && cr3 & ADDR_MASK == self.root && cr3 & 0xfff == u64::from(pcid) {
            return;
        }
        if pcid != 0 && PCID_OWNER[usize::from(pcid)].load(Ordering::Relaxed) == self.id {
            // SAFETY: as above; everything tagged with this PCID was put
            // there by this space and is still valid for it.
            unsafe { cpu::write_cr3(self.root | u64::from(pcid) | CR3_NO_FLUSH) };
            return;
        }
        let next = NEXT_PCID.load(Ordering::Relaxed);
        NEXT_PCID.store(if usize::from(next) + 1 == PCIDS { 1 } else { next + 1 }, Ordering::Relaxed);
        PCID_OWNER[usize::from(next)].store(self.id, Ordering::Relaxed);
        self.pcid.store(next, Ordering::Relaxed);
        // SAFETY: as above. Bit 63 clear flushes what the PCID's previous
        // owner (or this space, before it changed) left behind.
        unsafe { cpu::write_cr3(self.root | u64::from(next)) };
    }

    /// Unmap one page if it is mapped; flush it and free what is no longer
    /// used.
    fn unmap_page(&self, tables: &mut PageTables<KernelPhysMemory>, alloc: &mut BuddyAllocator<'_>, page: u64) {
        let Ok((old, freed)) = tables.unmap_and_prune(page) else { return };
        self.flush(page);
        if old.flags.contains(OWNED) {
            alloc.free(old.frame, 0).expect("user frame freed twice");
            self.resident.fetch_sub(1, Ordering::Relaxed);
            USER_FRAMES.fetch_sub(1, Ordering::Relaxed);
        }
        for &table in freed.as_slice() {
            alloc.free(table, 0).expect("page table freed twice");
        }
    }

    /// Count a page mapped with a frame the space owns.
    fn gained_page(&self) {
        self.resident.fetch_add(1, Ordering::Relaxed);
        USER_FRAMES.fetch_add(1, Ordering::Relaxed);
    }

    /// Drop any cached translation of `page` after its entry changed: with
    /// `INVLPG` if the space is loaded, else by making its next load flush
    /// (only PCIDs keep translations across loads).
    fn flush(&self, page: u64) {
        if self.is_active() {
            cpu::invlpg(page);
        } else {
            self.pcid.store(0, Ordering::Relaxed);
        }
    }
}

impl Drop for AddressSpace {
    fn drop(&mut self) {
        if self.is_active() {
            activate_kernel();
        }
        // Not loaded anywhere now (one CPU), so no TLB holds a translation
        // through these tables: they and the pages can go at once. Should
        // the PCID come back to a later space, its first load flushes.
        let mut tables = self.tables.lock();
        let mut frames = FRAMES.lock();
        let alloc = frames.as_mut().expect("address spaces need the frame allocator");
        tables.clear(0..ENTRIES / 2, |c| {
            let frame = match c {
                Cleared::Page { page, .. } if page.flags.contains(OWNED) => {
                    USER_FRAMES.fetch_sub(1, Ordering::Relaxed);
                    page.frame
                }
                Cleared::Page { .. } => return,
                Cleared::Table(frame) => frame,
            };
            alloc.free(frame, 0).expect("user frame or page table freed twice");
        });
        alloc.free(self.root, 0).expect("PML4 freed twice");
    }
}

/// Check that `[virt, virt + len)` is page-aligned user space; returns the end.
fn check_range(virt: u64, len: u64) -> Result<u64, MapError> {
    if !virt.is_multiple_of(PAGE_SIZE) || !len.is_multiple_of(PAGE_SIZE) {
        return Err(MapError::Misaligned);
    }
    match virt.checked_add(len) {
        Some(end) if virt >= USER_START && end <= USER_END => Ok(end),
        _ => Err(MapError::OutsideUserSpace),
    }
}

/// Fail with [`MapError::AlreadyMapped`] if any page of `[virt, end)` is
/// mapped.
fn free_slots(tables: &PageTables<KernelPhysMemory>, virt: u64, end: u64) -> Result<(), MapError> {
    match (virt..end).step_by(PAGE_SIZE as usize).any(|page| tables.translate(page).is_ok()) {
        true => Err(MapError::AlreadyMapped),
        false => Ok(()),
    }
}

/// Call `f(kernel, n, offset)` for each piece of the user range
/// `[virt, virt + len)` that lies in one page: its direct-map address, its
/// length and its offset from `virt`. Fails (before calling `f` for that
/// piece) if a page is not mapped or the range leaves user space.
fn each_chunk(
    tables: &PageTables<KernelPhysMemory>,
    virt: u64,
    len: u64,
    mut f: impl FnMut(u64, usize, usize),
) -> Result<(), MapError> {
    let end = virt.checked_add(len).filter(|&e| virt >= USER_START && e <= USER_END);
    let end = end.ok_or(MapError::OutsideUserSpace)?;
    let mut at = virt;
    while at < end {
        let piece = (PAGE_SIZE - at % PAGE_SIZE).min(end - at);
        let phys = tables.translate(at).map_err(|_| MapError::NotMapped)?.phys;
        let kernel = paging::phys_range_to_virt(phys, piece).expect("user frame outside the direct map");
        f(kernel, piece as usize, (at - virt) as usize);
        at += piece;
    }
    Ok(())
}

fn free_frames() -> u64 {
    FRAMES.lock().as_ref().map_or(0, |f| f.free_frames())
}

crate::kernel_test! {
    fn user_address_spaces_share_the_kernel_half() -> TestResult {
        let space = AddressSpace::new().map_err(|_| "no memory for an address space")?;
        let (mem, kernel) = (KernelPhysMemory, paging::kernel_root());
        if (0..ENTRIES / 2).any(|i| mem.read_entry(space.root(), i) != 0) {
            return Err("a new address space has something in its lower half");
        }
        if (ENTRIES / 2..ENTRIES).any(|i| mem.read_entry(space.root(), i) != mem.read_entry(kernel, i)) {
            return Err("a new address space's higher half differs from the kernel's");
        }
        // Every kernel PML4 entry exists from boot on, the MMIO window's
        // too, so a copy made now never misses a later kernel mapping.
        let mmio_slot = ((paging::MMIO_BASE >> 39) & 0x1ff) as usize;
        if mem.read_entry(kernel, mmio_slot) & PageFlags::PRESENT.bits() == 0 {
            return Err("the MMIO window has no PML4 entry");
        }
        if space.is_active() {
            return Err("a new address space is loaded");
        }
        Ok(())
    }
}

crate::kernel_test! {
    fn map_protect_unmap_and_free_user_pages() -> TestResult {
        const BASE: u64 = 0x40_0000;
        // No other thread or interrupt handler may take frames while the
        // count is compared (the shell runs this at runtime too).
        let _only = crate::smp::exclusive();
        let before = free_frames();
        let space = AddressSpace::new().map_err(|_| "no memory for an address space")?;
        space.map_zeroed(BASE, 3 * PAGE_SIZE, Prot::RW).map_err(|_| "map_zeroed failed")?;
        let m = space.translate(BASE + PAGE_SIZE + 8).ok_or("mapped page not found")?;
        if m.prot != Prot::RW || !m.owned || m.phys % PAGE_SIZE != 8 {
            return Err("a zeroed page has the wrong rights or owner");
        }
        let mut buf = [0xffu8; 16];
        space.read(BASE + PAGE_SIZE - 8, &mut buf).map_err(|_| "read across pages failed")?;
        if buf != [0; 16] {
            return Err("fresh user pages are not zeroed");
        }
        space.write(BASE + PAGE_SIZE - 4, b"poly-user").map_err(|_| "write across pages failed")?;
        space.read(BASE + PAGE_SIZE - 4, &mut buf[..9]).map_err(|_| "read back failed")?;
        if &buf[..9] != b"poly-user" {
            return Err("data written across a page boundary did not read back");
        }
        space.protect(BASE, PAGE_SIZE, Prot::RX).map_err(|_| "protect failed")?;
        let m = space.translate(BASE).ok_or("protected page not found")?;
        if m.prot != Prot::RX || !m.owned {
            return Err("protect did not change the rights, or lost the owner");
        }
        let errors = [
            (space.map_zeroed(BASE + PAGE_SIZE, PAGE_SIZE, Prot::RW), MapError::AlreadyMapped),
            (space.map_zeroed(0, PAGE_SIZE, Prot::RW), MapError::OutsideUserSpace),
            (space.map_zeroed(USER_START - PAGE_SIZE, PAGE_SIZE, Prot::RW), MapError::OutsideUserSpace),
            (space.map_zeroed(USER_END, PAGE_SIZE, Prot::RW), MapError::OutsideUserSpace),
            (space.map_zeroed(USER_END - PAGE_SIZE, 2 * PAGE_SIZE, Prot::RW), MapError::OutsideUserSpace),
            (space.map_zeroed(BASE + 1, PAGE_SIZE, Prot::RW), MapError::Misaligned),
            (space.map_zeroed(BASE + 8 * PAGE_SIZE, PAGE_SIZE, Prot::NONE), MapError::NoAccess),
            (space.protect(BASE, 4 * PAGE_SIZE, Prot::READ), MapError::NotMapped),
            (space.write(BASE + 3 * PAGE_SIZE - 1, b"xy"), MapError::NotMapped),
        ];
        if errors.iter().any(|(got, want)| *got != Err(*want)) {
            return Err("a bad request was not refused as expected");
        }
        if space.translate(BASE + 3 * PAGE_SIZE).is_some() || space.translate(USER_END).is_some() {
            return Err("translate found a page that is not mapped");
        }
        // Unmap the middle page (holes are skipped), then drop the rest.
        space.unmap(BASE + PAGE_SIZE, 4 * PAGE_SIZE).map_err(|_| "unmap failed")?;
        if space.translate(BASE + PAGE_SIZE).is_some() || space.translate(BASE).is_none() {
            return Err("unmap removed the wrong pages");
        }
        if free_frames() >= before {
            return Err("mapped pages took no frames");
        }
        drop(space);
        if free_frames() != before {
            return Err("dropping an address space did not free its pages, tables and PML4");
        }
        Ok(())
    }
}

crate::kernel_test! {
    fn mapped_frames_stay_with_their_owner() -> TestResult {
        // One frame mapped into two spaces (shared memory): both see it, and
        // neither frees it. Alone: see the test above.
        let _only = crate::smp::exclusive();
        let frame = FRAMES.lock().as_mut().and_then(|f| f.allocate(0)).ok_or("out of frames")?;
        let virt = paging::phys_to_virt(frame).ok_or("frame outside the direct map")? as *mut u64;
        // SAFETY: a frame this test just allocated, through the direct map.
        unsafe { virt.write_volatile(0x5ba7ed) };
        let before = free_frames();
        let result = (|| {
            let a = AddressSpace::new().map_err(|_| "no memory for an address space")?;
            let b = AddressSpace::new().map_err(|_| "no memory for an address space")?;
            for (space, at) in [(&a, 0x40_0000), (&b, 0x7000_0000)] {
                // SAFETY: the frame stays allocated until after both spaces
                // are gone, and holds nothing secret.
                unsafe { space.map_frames(at, frame, PAGE_SIZE, Prot::READ) }.map_err(|_| "map_frames failed")?;
                let m = space.translate(at).ok_or("shared frame not mapped")?;
                let mut word = [0u8; 8];
                space.read(at, &mut word).map_err(|_| "read failed")?;
                if m.phys != frame || m.owned || m.prot != Prot::READ || u64::from_le_bytes(word) != 0x5ba7ed {
                    return Err("a shared frame is mapped wrong");
                }
            }
            a.unmap(0x40_0000, PAGE_SIZE).map_err(|_| "unmap failed")?;
            Ok(())
        })();
        let after = free_frames();
        let freed = FRAMES.lock().as_mut().map(|f| f.free(frame, 0).is_ok());
        result?;
        if after != before || freed != Some(true) {
            return Err("an address space freed a frame it does not own");
        }
        Ok(())
    }
}

crate::kernel_test! {
    fn pcids_tag_translations_when_the_cpu_has_them() -> TestResult {
        // Load spaces by hand on this (kernel) thread, with interrupts off
        // so no switch reloads CR3 meanwhile.
        let _irq = cpu::InterruptsOff::new();
        let a = AddressSpace::new().map_err(|_| "no memory for an address space")?;
        let b = AddressSpace::new().map_err(|_| "no memory for an address space")?;
        let pcid = || cpu::read_cr3() & 0xfff;
        a.activate();
        let pa = pcid();
        b.activate();
        let pb = pcid();
        a.activate();
        let again = pcid();
        let active = a.is_active() && !b.is_active();
        // A change while b is not loaded makes its next load flush, with a
        // new PCID.
        b.map_zeroed(0x40_0000, PAGE_SIZE, Prot::RW).map_err(|_| "map failed")?;
        b.unmap(0x40_0000, PAGE_SIZE).map_err(|_| "unmap failed")?;
        b.activate();
        let pb2 = pcid();
        activate_kernel();
        let back = cpu::read_cr3() & ADDR_MASK == paging::kernel_root() && pcid() == 0;
        if !active || !back {
            return Err("activate did not load the right PML4");
        }
        let ok = if pcid_enabled() {
            pa != 0 && pb != 0 && pa != pb && again == pa && pb2 != 0 && pb2 != pb
        } else {
            pa == 0 && pb == 0 && again == 0 && pb2 == 0
        };
        if !ok {
            crate::kprintln!("user: PCIDs {pa:#x} {pb:#x} {again:#x} {pb2:#x}, enabled: {}", pcid_enabled());
            return Err("PCIDs were not handed out as expected");
        }
        Ok(())
    }
}
