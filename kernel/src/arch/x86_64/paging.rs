//! Kernel page tables: a direct map of physical memory, the kernel image with
//! per-section permissions (W^X), guard pages below the kernel stacks, and
//! uncached (or, for framebuffers, write-combining) mappings of device
//! memory.
//!
//! The page-table editing itself is `poly-paging` (host-tested); this module
//! decides what to map and drives the CPU.
//!
//! # Virtual memory layout
//!
//! | Virtual | Size | What |
//! |---|---|---|
//! | `0x0000_0000_0000_0000` – `0x0000_7fff_ffff_ffff` | 128 TiB | lower half: empty here; each user address space has its own (`addrspace.rs`) |
//! | [`DIRECT_MAP_BASE`] + phys | up to 64 TiB | direct map: every region of the memory map, read-write, no-execute |
//! | [`MMIO_BASE`] | 512 GiB | device memory from [`map_mmio`] (uncached) and [`map_mmio_wc`] (write-combining), no-execute |
//! | [`THREAD_STACKS_BASE`] | 2 MiB | kernel thread stacks from [`map_thread_stack`], read-write, no-execute, each above unmapped guard pages |
//! | [`KERNEL_VMA`] + phys | image | kernel image, per-section permissions (see `kernel/linker.ld`) |
//!
//! Every kernel mapping is global (CR4.PGE, when the CPU has it), so it
//! survives the CR3 switches between per-process address spaces. Those copy
//! the kernel half of this PML4 (slots 256-511) when they are created, so
//! every PML4 entry the kernel will ever use is made here, at boot, and
//! never changes afterwards: the direct map's, the kernel image's, an empty
//! PDPT for the MMIO window, which [`map_mmio`] fills later, and the tables
//! of the thread stack area down to its one page table, whose entries
//! [`map_thread_stack`] and [`unmap_thread_stack`] set and clear.
//!
//! The direct map covers the regions of the memory map, not the holes between
//! them (see [`poly_mem::DirectMap`]), so [`phys_to_virt`] answers `None` for a
//! hole. RAM (usable, kernel, ACPI) is mapped write-back. Reserved regions may
//! be device memory (the legacy VGA window below 1 MiB, PCI windows the
//! firmware reserves), so they are mapped uncached, which is also what
//! [`map_mmio`] uses: no cached alias of device memory exists. The direct-map
//! alias of the kernel's text and read-only data is read-only, like the image
//! mapping itself, so no mapping lets the kernel write its own code.
//!
//! # Bootstrapping
//!
//! The new tables are built while the boot stub's tables are still loaded, so
//! every table frame must be reachable through the boot mapping (physical
//! [0, 1 GiB) at [`KERNEL_VMA`]). [`init`] runs before the buddy allocator
//! exists and takes its frames from a bump allocator over usable RAM below
//! 1 GiB; it returns the memory map with those frames marked
//! [`RegionKind::Kernel`], so the buddy allocator never hands them out.
//! [`phys_to_virt`] goes through the boot mapping until CR3 points at the new
//! tables and through the direct map from then on, and page-table memory is
//! always reached through it. Later tables (for [`map_mmio`]) come from
//! [`FRAMES`]. The boot stub's own four tables stay where they are, in the
//! kernel image.
//!
//! # Memory types
//!
//! A page's PAT, PCD and PWT bits select one of eight memory types in the
//! IA32_PAT MSR (SDM Vol. 3A §11.12.3). [`init`] keeps the power-on table
//! except for entry 1 (PWT alone), which it changes from write-through to
//! write-combining before any mapping uses it (ADR 0019). So: no bits is
//! write-back (RAM), PCD | PWT is uncached (registers, [`map_mmio`]), PWT
//! alone is write-combining (framebuffers, [`map_mmio_wc`]). Every CPU must
//! load the same table; application processors will copy it when they
//! start.
//!
//! # Locking
//!
//! The kernel page tables are behind a spinlock; take it before [`FRAMES`].
//! The entries of the thread stack area's page table are not: each thread
//! slot's stack pages belong to that slot, and the scheduler maps and
//! unmaps them under its own lock, with no allocation (the tables exist).

use super::{BOOT_MAPPED_PHYS, KERNEL_VMA, cpu, gdt};
use crate::frames::FRAMES;
use crate::kprintln;
use crate::sync::SpinLock;
use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use poly_mem::{BuddyAllocator, BumpFrameAllocator, DirectMap, MemoryMap, PAGE_SIZE, Region, RegionKind, align_up};
use poly_paging::{ENTRIES, FRAME_SIZE, FrameAllocator, PageFlags, PageSize, PageTables, PhysMemory};

/// Where physical address 0 appears in the direct map (PML4 slot 256, the
/// start of the higher half).
pub const DIRECT_MAP_BASE: u64 = 0xffff_8000_0000_0000;

/// Physical memory at or above this is not direct-mapped: half of the higher
/// half, PML4 slots 256 to 383.
pub const DIRECT_MAP_SIZE: u64 = 64 << 40;

/// Start of the window [`map_mmio`] hands out (PML4 slot 384).
pub const MMIO_BASE: u64 = 0xffff_c000_0000_0000;

/// Size of the MMIO window: one PML4 slot.
pub const MMIO_SIZE: u64 = 512 << 30;

/// Start of the kernel thread stack area (PML4 slot 385), which
/// [`map_thread_stack`] fills: `thread::STACKS` lays the stacks out in it,
/// each above unmapped guard pages (ADR 0050).
pub const THREAD_STACKS_BASE: u64 = 0xffff_c080_0000_0000;

/// Size of the thread stack area: what one page table maps, so its tables
/// are made once at boot and the area never needs another one.
pub const THREAD_STACKS_SIZE: u64 = (ENTRIES as u64) * PAGE_SIZE;

/// PCD | PWT: uncached (UC): PAT entry 3, which is UC at power-on (SDM
/// Vol. 3A §11.12.3, Table 11-11) and which [`init`] leaves alone.
const UNCACHED: PageFlags = PageFlags::CACHE_DISABLE.union(PageFlags::WRITE_THROUGH);

/// PWT alone: PAT entry 1, write-through at power-on, write-combining (WC)
/// once [`init`] has programmed [`PAT`].
const WRITE_COMBINING: PageFlags = PageFlags::WRITE_THROUGH;

/// The IA32_PAT value [`init`] loads: the power-on table (entries 0-7 WB,
/// WT, UC-, UC, WB, WT, UC-, UC; SDM Vol. 3A Table 11-12) with entry 1 set
/// to WC (memory type 0x01, Table 11-10).
pub const PAT: u64 = 0x0007_0406_0007_0106;

/// Whether IA32_PAT holds [`PAT`], so [`WRITE_COMBINING`] means WC.
static WC: AtomicBool = AtomicBool::new(false);

unsafe extern "C" {
    static __text_start: u8;
    static __text_end: u8;
    static __rodata_start: u8;
    static __rodata_end: u8;
    static __data_start: u8;
    static __data_end: u8;
    static __boot_stack_guard: u8;
    static __boot_stack_bottom: u8;
    static __boot_stack_top: u8;
    static __double_fault_stack_guard: u8;
    static __nmi_stack_guard: u8;
}

/// A page-aligned part of the kernel image and how it is mapped.
#[derive(Clone, Copy, Debug)]
struct Section {
    name: &'static str,
    start: u64,
    end: u64,
    writable: bool,
    executable: bool,
}

impl Section {
    fn flags(&self) -> PageFlags {
        let mut f = PageFlags::PRESENT | PageFlags::GLOBAL;
        if self.writable {
            f |= PageFlags::WRITABLE;
        }
        if !self.executable {
            f |= PageFlags::NO_EXECUTE;
        }
        f
    }
}

/// Everything of the kernel image that the kernel page tables map. The rest
/// (the 32-bit boot stub and its tables) stays reachable through the direct
/// map only.
fn sections() -> [Section; 6] {
    let text = (&raw const __text_start) as u64..(&raw const __text_end) as u64;
    let rodata = (&raw const __rodata_start) as u64..(&raw const __rodata_end) as u64;
    let data = (&raw const __data_start) as u64..(&raw const __data_end) as u64;
    let boot_stack = (&raw const __boot_stack_bottom) as u64..(&raw const __boot_stack_top) as u64;
    let df_stack = gdt::double_fault_stack();
    let nmi_stack = gdt::nmi_stack();
    let section = |name, r: core::ops::Range<u64>, writable, executable| Section {
        name,
        start: r.start,
        end: r.end,
        writable,
        executable,
    };
    [
        section("text", text, false, true),
        section("rodata", rodata, false, false),
        section("data", data, true, false),
        section("boot stack", boot_stack, true, false),
        section("double-fault stack", df_stack, true, false),
        section("NMI stack", nmi_stack, true, false),
    ]
}

/// The unmapped page below each kernel stack, with the stack's name.
fn guard_pages() -> [(&'static str, u64); 3] {
    [
        ("boot", (&raw const __boot_stack_guard) as u64),
        ("double-fault", (&raw const __double_fault_stack_guard) as u64),
        ("NMI", (&raw const __nmi_stack_guard) as u64),
    ]
}

/// Name of the stack whose guard page holds `addr`, if any: a fault there is
/// a kernel stack overflow.
pub fn stack_guard_hit(addr: u64) -> Option<&'static str> {
    guard_pages().into_iter().find(|&(_, guard)| (guard..guard + PAGE_SIZE).contains(&addr)).map(|(name, _)| name)
}

/// Whether the XD bit is in use (IA32_EFER.NXE set). Without it the bit is
/// reserved, so [`fix`] strips it from every entry.
static NX: AtomicBool = AtomicBool::new(false);

/// `flags` as the CPU accepts them: without the XD bit if NXE is off.
pub(super) fn fix(flags: PageFlags) -> PageFlags {
    if NX.load(Ordering::Relaxed) { flags } else { flags.difference(PageFlags::NO_EXECUTE) }
}

fn is_ram(kind: RegionKind) -> bool {
    matches!(kind, RegionKind::Usable | RegionKind::Kernel | RegionKind::AcpiReclaimable | RegionKind::AcpiNvs)
}

fn direct_map_flags(kind: RegionKind) -> PageFlags {
    let flags = PageFlags::PRESENT | PageFlags::WRITABLE | PageFlags::GLOBAL | PageFlags::NO_EXECUTE;
    if is_ram(kind) { flags } else { flags | UNCACHED }
}

/// The direct map's spans: written once by [`init`], then read without a
/// lock by [`phys_to_virt`].
struct DirectMapCell {
    active: AtomicBool,
    map: UnsafeCell<DirectMap>,
}

// SAFETY: `map` is written only by `activate`, once, before `active` is set
// with Release ordering; `get` reads it only after seeing `active` with
// Acquire ordering, and nothing writes it after that.
unsafe impl Sync for DirectMapCell {}

impl DirectMapCell {
    fn get(&self) -> Option<&DirectMap> {
        // SAFETY: see the Sync impl: once `active` is set, `map` never changes.
        self.active.load(Ordering::Acquire).then(|| unsafe { &*self.map.get() })
    }

    /// # Safety
    /// Call at most once, while no other CPU runs.
    unsafe fn activate(&self, map: DirectMap) {
        assert!(!self.active.load(Ordering::Relaxed), "direct map activated twice");
        // SAFETY: `active` is still clear, so no reader looks at `map`, and
        // the caller guarantees no other writer.
        unsafe { *self.map.get() = map };
        self.active.store(true, Ordering::Release);
    }
}

static DIRECT_MAP: DirectMapCell =
    DirectMapCell { active: AtomicBool::new(false), map: UnsafeCell::new(DirectMap::new()) };

/// Virtual address of physical address `phys`: in the direct map once the
/// kernel page tables are loaded, in the boot mapping before. `None` if it is
/// not mapped (a hole in the memory map, or beyond the boot mapping).
pub fn phys_to_virt(phys: u64) -> Option<u64> {
    phys_range_to_virt(phys, 1)
}

/// Like [`phys_to_virt`], but only if all of `[phys, phys + len)` is mapped
/// (contiguously, as the mapping is linear).
pub fn phys_range_to_virt(phys: u64, len: u64) -> Option<u64> {
    match DIRECT_MAP.get() {
        // Covered addresses are below DIRECT_MAP_SIZE, so this cannot overflow.
        Some(map) => map.covers(phys, len).then(|| DIRECT_MAP_BASE + phys),
        None => (phys.checked_add(len.max(1))? <= BOOT_MAPPED_PHYS).then_some(KERNEL_VMA + phys),
    }
}

/// The physical address behind `[virt, virt + len)` if all of it lies in
/// the direct map, which maps physical memory linearly, so the range is
/// physically contiguous too, or on one thread stack, whose pages are
/// mapped in order. `None` for anything else (the kernel image, MMIO,
/// holes in the memory map) and before the direct map is in use.
pub fn virt_range_to_phys(virt: u64, len: u64) -> Option<u64> {
    if (THREAD_STACKS_BASE..THREAD_STACKS_BASE + THREAD_STACKS_SIZE).contains(&virt) {
        return stack_range_to_phys(virt, len);
    }
    let map = DIRECT_MAP.get()?;
    let phys = virt.checked_sub(DIRECT_MAP_BASE).filter(|&p| p < DIRECT_MAP_SIZE)?;
    map.covers(phys, len).then_some(phys)
}

/// Page-table memory, reached through [`phys_to_virt`].
#[derive(Debug)]
pub struct KernelPhysMemory;

impl KernelPhysMemory {
    fn entry(table: u64, index: usize) -> *mut u64 {
        assert!(index < ENTRIES, "page-table index out of range");
        let virt = phys_range_to_virt(table, FRAME_SIZE).expect("page table outside the mapped physical memory");
        (virt as *mut u64).wrapping_add(index)
    }
}

impl PhysMemory for KernelPhysMemory {
    fn read_entry(&self, table: u64, index: usize) -> u64 {
        // SAFETY: poly-paging only passes the root and table frames it got
        // from our frame allocators or read out of present non-leaf entries:
        // RAM that belongs to these tables and that `phys_to_virt` maps
        // writable (checked in `entry`). The CPU may set A/D bits
        // concurrently, so the access is one volatile 64-bit read.
        unsafe { Self::entry(table, index).read_volatile() }
    }

    fn write_entry(&mut self, table: u64, index: usize, value: u64) {
        // SAFETY: as in `read_entry`; a single 64-bit store, so the CPU
        // never sees a torn entry.
        unsafe { Self::entry(table, index).write_volatile(value) }
    }
}

/// Frames for the first page tables, before the buddy allocator exists:
/// usable RAM below the boot mapping limit, front to back. Remembers which
/// frames it handed out so [`init`] can take them out of the memory map.
struct EarlyFrames<'a> {
    bump: BumpFrameAllocator<'a>,
    runs: [Region; 8],
    len: usize,
}

impl<'a> EarlyFrames<'a> {
    fn new(memory: &'a MemoryMap) -> Self {
        EarlyFrames { bump: BumpFrameAllocator::new(memory), runs: [Region::new(0, 0, RegionKind::Kernel); 8], len: 0 }
    }

    fn runs(&self) -> &[Region] {
        &self.runs[..self.len]
    }
}

impl FrameAllocator for EarlyFrames<'_> {
    fn allocate_frame(&mut self) -> Option<u64> {
        let frame = self.bump.allocate()?;
        if frame + PAGE_SIZE > BOOT_MAPPED_PHYS {
            return None;
        }
        let full = self.len == self.runs.len();
        match self.runs[..self.len].last_mut() {
            Some(run) if run.end == frame => run.end += PAGE_SIZE,
            _ if full => return None,
            _ => {
                self.runs[self.len] = Region::new(frame, frame + PAGE_SIZE, RegionKind::Kernel);
                self.len += 1;
            }
        }
        Some(frame)
    }

    fn deallocate_frame(&mut self, _frame: u64) {
        // Only called when a mapping fails half-way, and `init` panics then.
    }
}

/// Frames from the buddy allocator, for page tables created after [`init`].
pub(super) struct BuddyFrames<'a, 'm>(pub(super) &'a mut BuddyAllocator<'m>);

impl FrameAllocator for BuddyFrames<'_, '_> {
    fn allocate_frame(&mut self) -> Option<u64> {
        self.0.allocate(0)
    }

    fn deallocate_frame(&mut self, frame: u64) {
        let freed = self.0.free(frame, 0);
        debug_assert!(freed.is_ok(), "page-table frame {frame:#x} was not allocated: {freed:?}");
    }
}

/// The kernel page tables and what is mapped in the MMIO window so far.
struct Kernel {
    tables: PageTables<KernelPhysMemory>,
    /// Largest page size the CPU supports: 1 GiB or 2 MiB.
    largest: PageSize,
    /// First free address in the MMIO window.
    mmio_next: u64,
}

static KERNEL: SpinLock<Option<Kernel>> = SpinLock::new(None);

/// Physical address of the kernel PML4, once [`init`] has loaded it.
static KERNEL_ROOT: AtomicU64 = AtomicU64::new(0);

/// The kernel PML4: what CR3 holds while no user address space is loaded,
/// and where user address spaces copy the kernel half from. 0 before
/// [`init`].
pub fn kernel_root() -> u64 {
    KERNEL_ROOT.load(Ordering::Acquire)
}

/// PML4 slot of the MMIO window.
const MMIO_SLOT: usize = ((MMIO_BASE >> 39) & 0x1ff) as usize;

/// Physical address of the page table that maps the thread stack area
/// ([`THREAD_STACKS_BASE`]); 0 before [`init`].
static STACKS_PT: AtomicU64 = AtomicU64::new(0);

const _: () =
    assert!(THREAD_STACKS_BASE.is_multiple_of(THREAD_STACKS_SIZE), "the stack area needs a page table of its own");
const _: () = assert!(THREAD_STACKS_BASE >= MMIO_BASE + MMIO_SIZE && THREAD_STACKS_BASE < KERNEL_VMA);

/// The stack area's page-table entry indices for `[virt, virt + len)`, if
/// that is whole pages inside the area and the area is set up.
fn stack_pages(virt: u64, len: u64) -> Option<(u64, core::ops::Range<usize>)> {
    let pt = STACKS_PT.load(Ordering::Acquire);
    let offset = virt.checked_sub(THREAD_STACKS_BASE)?;
    let end = offset.checked_add(len)?;
    let whole = offset.is_multiple_of(PAGE_SIZE) && len.is_multiple_of(PAGE_SIZE);
    (pt != 0 && whole && end <= THREAD_STACKS_SIZE)
        .then_some((pt, (offset / PAGE_SIZE) as usize..(end / PAGE_SIZE) as usize))
}

/// Map the `len` bytes of physical memory at `phys` (whole frames, which
/// must be RAM) at `virt` in the thread stack area: read-write, no-execute.
///
/// # Safety
/// The pages at `[virt, virt + len)` must be unmapped and owned by the
/// caller: no other code maps or unmaps them meanwhile (`thread` gives each
/// thread slot its own part of the area, under its scheduler lock). The
/// frames must stay allocated until [`unmap_thread_stack`].
pub unsafe fn map_thread_stack(virt: u64, phys: u64, len: u64) {
    let (pt, pages) = stack_pages(virt, len).expect("thread stack outside the stack area");
    assert!(phys.is_multiple_of(PAGE_SIZE), "thread stack frames not page-aligned");
    let flags = fix(PageFlags::PRESENT | PageFlags::WRITABLE | PageFlags::GLOBAL | PageFlags::NO_EXECUTE);
    let first = pages.start;
    for i in pages {
        let page = virt + (i - first) as u64 * PAGE_SIZE;
        let frame = phys + (i - first) as u64 * PAGE_SIZE;
        assert_eq!(KernelPhysMemory.read_entry(pt, i), 0, "thread stack page {page:#x} is mapped already");
        KernelPhysMemory.write_entry(pt, i, frame | flags.bits());
        // Not needed for a page that was not present, which the TLB does
        // not cache (SDM Vol. 3A §4.10.2.3); cheap insurance.
        cpu::invlpg(page);
    }
}

/// Unmap `[virt, virt + len)` in the thread stack area, which
/// [`map_thread_stack`] mapped; its frames can be freed afterwards.
///
/// # Safety
/// The caller owns the pages, as for [`map_thread_stack`], and nothing uses
/// them any more (the thread that ran on the stack has exited and been
/// switched away from). Other CPUs, once they run kernel code, will need a
/// TLB shootdown here.
pub unsafe fn unmap_thread_stack(virt: u64, len: u64) {
    let (pt, pages) = stack_pages(virt, len).expect("thread stack outside the stack area");
    let first = pages.start;
    for i in pages {
        KernelPhysMemory.write_entry(pt, i, 0);
        // The mapping was global: a CR3 switch would not drop it.
        cpu::invlpg(virt + (i - first) as u64 * PAGE_SIZE);
    }
}

/// The physical address behind `[virt, virt + len)` if it lies in the
/// thread stack area, every page of it is mapped, and the pages are
/// physically contiguous (each stack is one buddy block, mapped in order).
fn stack_range_to_phys(virt: u64, len: u64) -> Option<u64> {
    let start = virt & !(PAGE_SIZE - 1);
    let end = align_up(virt.checked_add(len.max(1))?, PAGE_SIZE);
    let (pt, pages) = stack_pages(start, end - start)?;
    let frame = |i: usize| {
        let entry = KernelPhysMemory.read_entry(pt, i);
        (entry & PageFlags::PRESENT.bits() != 0).then_some(entry & poly_paging::ADDR_MASK)
    };
    let base = frame(pages.start)?;
    let first = pages.start;
    pages.into_iter().all(|i| frame(i) == Some(base + (i - first) as u64 * PAGE_SIZE)).then_some(base + (virt - start))
}

/// How `poly-paging` links a table it creates (its intermediate entry
/// policy: present and writable, the rest left to the leaves).
const TABLE_ENTRY_FLAGS: PageFlags = PageFlags::PRESENT.union(PageFlags::WRITABLE);

/// Build the kernel page tables for the normalized map `memory`, load them
/// and drop the boot mapping. Returns `memory` with the page-table frames
/// marked [`RegionKind::Kernel`]; hand that map to the frame allocator.
///
/// # Safety
/// Call once, on the boot CPU, with interrupts off, before anything keeps a
/// pointer from [`phys_to_virt`]: those point into the boot mapping, which is
/// gone afterwards.
pub unsafe fn init(memory: &MemoryMap) -> MemoryMap {
    let features = cpu::features();
    if features.nx {
        // SAFETY: CPUID says the CPU has NXE. Setting it only makes the XD
        // bit valid; the boot tables do not use it.
        unsafe { cpu::write_msr(cpu::MSR_EFER, cpu::read_msr(cpu::MSR_EFER) | cpu::EFER_NXE) };
    }
    NX.store(features.nx, Ordering::Relaxed);
    if features.pat {
        // SAFETY: CPUID says the CPU has IA32_PAT. Only entry 1 changes, and
        // nothing maps memory with it yet: the boot tables set none of PAT,
        // PCD and PWT, and the kernel tables are built below. So no cache
        // line or TLB entry has the old type, and the CR3 load below flushes
        // the TLB anyway (SDM Vol. 3A §11.12.4).
        unsafe { cpu::write_msr(cpu::MSR_PAT, PAT) };
        WC.store(true, Ordering::Relaxed);
    }
    let largest = if features.page_1g { PageSize::Size1G } else { PageSize::Size2M };

    let direct = DirectMap::from_map(memory, DIRECT_MAP_SIZE);
    let mut early = EarlyFrames::new(memory);
    let mut tables = PageTables::new(KernelPhysMemory, &mut early).expect("no low memory for the kernel page tables");
    map_direct(&mut tables, &direct, largest, &mut early);
    for s in sections() {
        let res = tables.map_range(s.start, s.start - KERNEL_VMA, s.end - s.start, fix(s.flags()), largest, &mut early);
        res.unwrap_or_else(|e| panic!("mapping the kernel {}: {e:?}", s.name));
    }
    // The MMIO window's PDPT exists from the start, empty, so `map_mmio`
    // never adds a PML4 entry that user address spaces would lack.
    let pdpt = early.allocate_frame().expect("no low memory for the kernel page tables");
    tables.memory_mut().zero_table(pdpt);
    let root = tables.root();
    tables.memory_mut().write_entry(root, MMIO_SLOT, pdpt | TABLE_ENTRY_FLAGS.bits());
    // The thread stack area's PDPT, page directory and page table, empty.
    let mut link = |table: u64, index: usize| {
        let next = early.allocate_frame().expect("no low memory for the kernel page tables");
        tables.memory_mut().zero_table(next);
        tables.memory_mut().write_entry(table, index, next | TABLE_ENTRY_FLAGS.bits());
        next
    };
    let index = |level: u32| ((THREAD_STACKS_BASE >> (12 + 9 * level)) & 0x1ff) as usize;
    let stacks_pdpt = link(root, index(3));
    let stacks_pd = link(stacks_pdpt, index(2));
    let stacks_pt = link(stacks_pd, index(1));

    let mapped: u64 = direct.spans().iter().map(Region::len).sum();
    let (spans, end) = (direct.spans().len(), direct.end());
    // SAFETY: the new tables map the kernel image at the same addresses as
    // the boot tables do, so the running code, this stack and every static
    // stay where they are. The caller guarantees that nothing still uses the
    // rest of the boot mapping (identity map and low memory at KERNEL_VMA).
    unsafe { cpu::write_cr3(tables.root()) };
    // SAFETY: called once, on the only running CPU; the tables just loaded
    // map exactly `direct` at DIRECT_MAP_BASE.
    unsafe { DIRECT_MAP.activate(direct) };
    KERNEL_ROOT.store(tables.root(), Ordering::Release);
    STACKS_PT.store(stacks_pt, Ordering::Release);
    if features.pge {
        // SAFETY: CPUID says the CPU has global pages; enabling them changes
        // no translation.
        unsafe { cpu::write_cr4(cpu::read_cr4() | cpu::CR4_PGE) };
    }

    let mut map = memory.clone();
    for run in early.runs() {
        map.push(*run).expect("memory map full");
    }
    let table_bytes: u64 = early.runs().iter().map(Region::len).sum();
    let yes_no = |b: bool| if b { "yes" } else { "no" };
    kprintln!(
        "mm: direct map at {DIRECT_MAP_BASE:#x}: {spans} ranges, {} MiB, up to {end:#x}; \
         NX: {}, 1 GiB pages: {}, global pages: {}, write-combining (PAT): {}",
        mapped >> 20,
        yes_no(features.nx),
        yes_no(features.page_1g),
        yes_no(features.pge),
        yes_no(features.pat)
    );
    kprintln!(
        "mm: kernel page tables at {:#x} ({} KiB); text r-x, rodata r--, data rw-, stacks rw- above guard pages; \
         boot identity map removed",
        tables.root(),
        table_bytes >> 10
    );
    *KERNEL.lock() = Some(Kernel { tables, largest, mmio_next: MMIO_BASE });
    map.normalized().expect("memory map full")
}

/// Map every span of `direct` at [`DIRECT_MAP_BASE`]. The image's text and
/// read-only data stay read-only through this alias too.
fn map_direct(
    tables: &mut PageTables<KernelPhysMemory>,
    direct: &DirectMap,
    largest: PageSize,
    frames: &mut impl FrameAllocator,
) {
    let ro_start = (&raw const __text_start) as u64 - KERNEL_VMA;
    let ro_end = (&raw const __rodata_end) as u64 - KERNEL_VMA;
    for span in direct.spans() {
        let flags = direct_map_flags(span.kind);
        let pieces = [
            (span.start, span.end.min(ro_start), flags),
            (span.start.max(ro_start), span.end.min(ro_end), flags.difference(PageFlags::WRITABLE)),
            (span.start.max(ro_end), span.end, flags),
        ];
        for (start, end, flags) in pieces.into_iter().filter(|(s, e, _)| s < e) {
            let res = tables.map_range(DIRECT_MAP_BASE + start, start, end - start, fix(flags), largest, frames);
            res.unwrap_or_else(|e| panic!("direct-mapping {start:#x}-{end:#x}: {e:?}"));
        }
    }
}

/// How a device mapping is cached.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Caching {
    /// UC: every access reaches the device, in program order. For
    /// registers.
    Uncached,
    /// WC: writes collect in the CPU's write-combining buffers and reach the
    /// device in bursts, possibly out of order; reads are uncached. For
    /// framebuffers, where only the final pixels matter.
    WriteCombining,
}

impl Caching {
    const fn flags(self) -> PageFlags {
        match self {
            Caching::Uncached => UNCACHED,
            Caching::WriteCombining => WRITE_COMBINING,
        }
    }
}

/// Map the device memory at `[phys, phys + len)` for the kernel: uncached,
/// writable, never executable. Returns the virtual address of `phys`.
///
/// `None` before the kernel page tables or the frame allocator are set up,
/// when the window or the frames run out, and for ranges that overlap RAM
/// (an uncached alias of memory that the direct map caches would break cache
/// coherence). Each call creates a new mapping, with an unmapped page after
/// it; mappings are never removed yet.
pub fn map_mmio(phys: u64, len: u64) -> Option<u64> {
    map_device(phys, len, Caching::Uncached)
}

/// Like [`map_mmio`], but write-combining: for a framebuffer. Also `None`
/// when the CPU has no PAT, and for ranges that overlap the direct map at
/// all: it maps reserved ranges uncached, and two mappings of one page with
/// different memory types are not allowed (SDM Vol. 3A §11.12.4).
pub fn map_mmio_wc(phys: u64, len: u64) -> Option<u64> {
    map_device(phys, len, Caching::WriteCombining)
}

fn map_device(phys: u64, len: u64, caching: Caching) -> Option<u64> {
    let start = phys & !(PAGE_SIZE - 1);
    let end = align_up(phys.checked_add(len.max(1))?, PAGE_SIZE);
    let overlaps = |s: &&Region| s.start < end && start < s.end;
    let spans = DIRECT_MAP.get()?.spans();
    let refused = match caching {
        Caching::Uncached => spans.iter().filter(overlaps).any(|s| is_ram(s.kind)),
        Caching::WriteCombining => !WC.load(Ordering::Relaxed) || spans.iter().any(|s| overlaps(&s)),
    };
    if refused || end > poly_paging::PHYS_LIMIT {
        return None;
    }
    let size = end - start;

    let mut kernel = KERNEL.lock();
    let k = kernel.as_mut()?;
    // Large pages need virt and phys to agree modulo the page size.
    let align = [PageSize::Size1G, PageSize::Size2M]
        .into_iter()
        .find(|&p| p <= k.largest && size >= p.bytes())
        .map_or(PAGE_SIZE, PageSize::bytes);
    let virt = align_up(k.mmio_next, align) + (start & (align - 1));
    if virt.checked_add(size)? > MMIO_BASE + MMIO_SIZE {
        return None;
    }
    let mut frames = FRAMES.lock();
    let flags = PageFlags::PRESENT | PageFlags::WRITABLE | PageFlags::GLOBAL | PageFlags::NO_EXECUTE | caching.flags();
    k.tables.map_range(virt, start, size, fix(flags), k.largest, &mut BuddyFrames(frames.as_mut()?)).ok()?;
    k.mmio_next = virt + size + PAGE_SIZE;
    Some(virt + (phys - start))
}

/// How the kernel mapping at `virt` is cached, if it is a device mapping
/// ([`map_mmio`], [`map_mmio_wc`], or the direct map of a reserved range);
/// `None` for write-back memory and for addresses not mapped.
pub fn caching(virt: u64) -> Option<Caching> {
    let flags = translate(virt).ok()?.flags;
    if flags.contains(UNCACHED) {
        Some(Caching::Uncached)
    } else if flags.contains(WRITE_COMBINING) && WC.load(Ordering::Relaxed) {
        Some(Caching::WriteCombining)
    } else {
        None
    }
}

/// Where `virt` leads in the kernel page tables.
fn translate(virt: u64) -> Result<poly_paging::Translation, &'static str> {
    let kernel = KERNEL.lock();
    let k = kernel.as_ref().ok_or("kernel page tables not set up")?;
    k.tables.translate(virt).map_err(|_| "not mapped")
}

crate::kernel_test! {
    fn kernel_sections_have_their_permissions() -> TestResult {
        let nx = NX.load(Ordering::Relaxed);
        for s in sections() {
            for virt in (s.start..s.end).step_by(PAGE_SIZE as usize) {
                let t = translate(virt)?;
                let wrong = t.phys != virt - KERNEL_VMA
                    || t.flags.contains(PageFlags::WRITABLE) != s.writable
                    || (nx && t.flags.contains(PageFlags::NO_EXECUTE) == s.executable)
                    || t.flags.contains(PageFlags::USER);
                if wrong {
                    kprintln!("paging: {} page {virt:#x}: {t:x?}", s.name);
                    return Err("kernel image page mapped with the wrong frame or permissions");
                }
            }
        }
        for (name, guard) in guard_pages() {
            if translate(guard).is_ok() {
                kprintln!("paging: guard page of the {name} stack at {guard:#x} is mapped");
                return Err("stack guard page is mapped");
            }
        }
        // No writable alias of the code through the direct map either.
        for s in &sections()[..2] {
            let alias = translate(DIRECT_MAP_BASE + (s.start - KERNEL_VMA))?;
            if alias.flags.contains(PageFlags::WRITABLE) {
                return Err("direct-map alias of the kernel text or rodata is writable");
            }
        }
        Ok(())
    }
}

/// Exit codes of `check_own_stack`.
const STACK_OK: usize = 0;
const STACK_NOT_IN_SLOT: usize = 1;
const STACK_BAD_MAPPING: usize = 2;
const STACK_GUARD_MAPPED: usize = 3;
const STACK_NO_BUS_ADDRESS: usize = 4;

/// A thread checks its own stack: in its slot of `thread::STACKS`, mapped
/// read-write and no-execute above an unmapped guard, and usable for DMA.
fn check_own_stack(_: usize) -> usize {
    let local = core::hint::black_box([0u8; 64]);
    let addr = local.as_ptr() as u64;
    let slot = crate::thread::current().slot();
    if crate::thread::STACKS.stack_holding(addr, 64) != Some(slot) {
        return STACK_NOT_IN_SLOT;
    }
    let (stack, guard) = (crate::thread::STACKS.stack(slot).unwrap(), crate::thread::STACKS.guard(slot).unwrap());
    let no_nx = !NX.load(Ordering::Relaxed);
    for page in stack.clone().step_by(PAGE_SIZE as usize) {
        match translate(page) {
            Ok(t) if t.flags.contains(PageFlags::WRITABLE) && (no_nx || t.flags.contains(PageFlags::NO_EXECUTE)) => {}
            _ => return STACK_BAD_MAPPING,
        }
    }
    if guard.step_by(PAGE_SIZE as usize).any(|page| translate(page).is_ok()) {
        return STACK_GUARD_MAPPED;
    }
    let phys = translate(addr).ok().map(|t| t.phys);
    // A buffer on the stack reaches a device; one that runs into the guard
    // does not.
    if virt_range_to_phys(addr, 64) != phys || virt_range_to_phys(stack.start - 8, 16).is_some() {
        return STACK_NO_BUS_ADDRESS;
    }
    STACK_OK
}

crate::kernel_test! {
    fn thread_stacks_sit_above_unmapped_guards() -> TestResult {
        let t = crate::thread::spawn(check_own_stack, 0).map_err(|_| "spawn failed")?;
        let slot = t.slot();
        match crate::thread::join(t) {
            Ok(STACK_OK) => {}
            Ok(STACK_NOT_IN_SLOT) => return Err("a thread's stack is not in its slot of the stack area"),
            Ok(STACK_BAD_MAPPING) => return Err("a thread stack page is not mapped read-write, no-execute"),
            Ok(STACK_GUARD_MAPPED) => return Err("the guard below a thread stack is mapped"),
            Ok(_) => return Err("a buffer on a thread stack got the wrong bus address"),
            Err(_) => return Err("join failed"),
        }
        // Joined, so reaped: the stack is unmapped and its frames are free.
        let stack = crate::thread::STACKS.stack(slot).ok_or("no stack slot")?;
        if stack.step_by(PAGE_SIZE as usize).any(|page| translate(page).is_ok()) {
            return Err("a reaped thread's stack is still mapped");
        }
        Ok(())
    }
}

crate::kernel_test! {
    fn boot_identity_map_is_gone() -> TestResult {
        let kernel = KERNEL.lock();
        let k = kernel.as_ref().ok_or("kernel page tables not set up")?;
        if cpu::read_cr3() & poly_paging::ADDR_MASK != k.tables.root() {
            return Err("CR3 does not point at the kernel page tables");
        }
        if (0..ENTRIES / 2).any(|i| k.tables.memory().read_entry(k.tables.root(), i) != 0) {
            return Err("something is mapped in the lower half");
        }
        // The identity map, and the boot mapping's view of low memory and of
        // the boot stub at KERNEL_VMA.
        for virt in [0, 0x10_0000, BOOT_MAPPED_PHYS - PAGE_SIZE, KERNEL_VMA, KERNEL_VMA + 0x10_0000] {
            if k.tables.translate(virt) != Err(poly_paging::Error::NotMapped) {
                return Err("a boot mapping is still present");
            }
        }
        Ok(())
    }
}

crate::kernel_test! {
    fn map_mmio_maps_devices_uncached(env: &TestEnv) -> TestResult {
        let check = |virt: u64| -> crate::ktest::TestResult {
            let t = translate(virt)?;
            let nx = !NX.load(Ordering::Relaxed) || t.flags.contains(PageFlags::NO_EXECUTE);
            if !t.flags.contains(UNCACHED | PageFlags::WRITABLE) || !nx || t.flags.contains(PageFlags::USER) {
                return Err("MMIO mapped with the wrong flags");
            }
            Ok(())
        };
        // SAFETY: IA32_APIC_BASE exists on every x86-64 CPU (they all have an
        // integrated local APIC); reading it has no side effects.
        let base = unsafe { cpu::read_msr(cpu::MSR_APIC_BASE) } & poly_paging::ADDR_MASK;
        let lapic = crate::mm::map_mmio(base, PAGE_SIZE).ok_or("could not map the local APIC")?;
        check(lapic)?;
        // SAFETY: `lapic` maps the local APIC's register page, uncached.
        // Offset 0x30 is the read-only Local APIC Version Register (SDM
        // Vol. 3A, "Local APIC Version Register"); reading it has no effect.
        let version = unsafe { ((lapic + 0x30) as *const u32).read_volatile() };
        kprintln!("paging: local APIC at {base:#x} mapped at {lapic:#x}, version register {version:#x}");
        // Bits 7:0 are 1XH for an integrated APIC; bits 23:16 hold the
        // number of LVT entries minus one, which is at least 3.
        if version & 0xf0 != 0x10 || (version >> 16) & 0xff < 3 {
            return Err("local APIC version register does not look like an APIC");
        }

        // The other device ranges drivers will need (QEMU q35 addresses, as
        // the ACPI tables report them): I/O APIC, HPET and the PCI ECAM
        // window for bus 0, which the memory map lists as reserved.
        for (phys, len) in [(0xfec0_0000, 0x1000), (0xfed0_0000, 0x400), (0xb000_0000, 1 << 20)] {
            let virt = crate::mm::map_mmio(phys, len).ok_or("could not map a device range")?;
            check(virt)?;
            check(virt + align_up(len, PAGE_SIZE) - PAGE_SIZE)?;
            if translate(virt)?.phys != phys {
                return Err("MMIO mapping points at the wrong address");
            }
        }

        let ram = env.memory.regions().iter().find(|r| r.kind == RegionKind::Usable).ok_or("no usable RAM")?;
        if crate::mm::map_mmio(ram.start, PAGE_SIZE).is_some() {
            return Err("map_mmio made an uncached alias of RAM");
        }
        if caching(lapic) != Some(Caching::Uncached) {
            return Err("caching() does not see an MMIO mapping as uncached");
        }
        Ok(())
    }
}

crate::kernel_test! {
    fn pat_entry_1_is_write_combining(env: &TestEnv) -> TestResult {
        if !cpu::features().pat {
            return Ok(());
        }
        // SAFETY: CPUID says IA32_PAT exists; reading it has no effect.
        let pat = unsafe { cpu::read_msr(cpu::MSR_PAT) };
        if pat != PAT {
            kprintln!("paging: IA32_PAT is {pat:#018x}, expected {PAT:#018x}");
            return Err("IA32_PAT does not hold the kernel's table");
        }
        // No write-combining alias of anything the direct map covers, RAM or
        // reserved (the ECAM window is reserved, so it is mapped uncached).
        let ram = env.memory.regions().iter().find(|r| r.kind == RegionKind::Usable).ok_or("no usable RAM")?;
        let reserved = env.memory.regions().iter().find(|r| r.kind == RegionKind::Reserved && r.start >= 1 << 20);
        for r in [Some(ram), reserved].into_iter().flatten() {
            if crate::mm::map_mmio_wc(r.start, PAGE_SIZE).is_some() {
                kprintln!("paging: {:#x} ({:?}) mapped write-combining", r.start, r.kind);
                return Err("map_mmio_wc mapped memory the direct map covers");
            }
        }
        // The direct map is write-back for RAM.
        let ram_virt = crate::arch::phys_to_virt(ram.start).ok_or("RAM outside the direct map")?;
        if caching(ram_virt).is_some() {
            return Err("the direct map of RAM is not write-back");
        }
        Ok(())
    }
}
