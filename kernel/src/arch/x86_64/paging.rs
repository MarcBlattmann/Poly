//! Kernel page tables: a direct map of physical memory, the kernel image with
//! per-section permissions (W^X), guard pages below the kernel stacks, and
//! uncached mappings of device memory.
//!
//! The page-table editing itself is `poly-paging` (host-tested); this module
//! decides what to map and drives the CPU.
//!
//! # Virtual memory layout
//!
//! | Virtual | Size | What |
//! |---|---|---|
//! | `0x0000_0000_0000_0000` – `0x0000_7fff_ffff_ffff` | 128 TiB | lower half: nothing mapped (for user space later) |
//! | [`DIRECT_MAP_BASE`] + phys | up to 64 TiB | direct map: every region of the memory map, read-write, no-execute |
//! | [`MMIO_BASE`] | 512 GiB | device memory from [`map_mmio`]: uncached, no-execute |
//! | [`KERNEL_VMA`] + phys | image | kernel image, per-section permissions (see `kernel/linker.ld`) |
//!
//! Every kernel mapping is global (CR4.PGE, when the CPU has it), so it will
//! survive the CR3 switches between per-process address spaces.
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
//! # Locking
//!
//! The kernel page tables are behind a spinlock; take it before [`FRAMES`].

use super::{BOOT_MAPPED_PHYS, KERNEL_VMA, cpu, gdt};
use crate::frames::FRAMES;
use crate::kprintln;
use crate::sync::SpinLock;
use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicBool, Ordering};
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

/// PCD | PWT: uncached (UC) with the power-on PAT (SDM Vol. 3A §11.12.3,
/// Table 11-11: PAT entry 3 is UC). The kernel never reprograms the PAT.
const UNCACHED: PageFlags = PageFlags::CACHE_DISABLE.union(PageFlags::WRITE_THROUGH);

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
fn sections() -> [Section; 5] {
    let text = (&raw const __text_start) as u64..(&raw const __text_end) as u64;
    let rodata = (&raw const __rodata_start) as u64..(&raw const __rodata_end) as u64;
    let data = (&raw const __data_start) as u64..(&raw const __data_end) as u64;
    let boot_stack = (&raw const __boot_stack_bottom) as u64..(&raw const __boot_stack_top) as u64;
    let df_stack = gdt::double_fault_stack();
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
    ]
}

/// The unmapped page below each kernel stack, with the stack's name.
fn guard_pages() -> [(&'static str, u64); 2] {
    [("boot", (&raw const __boot_stack_guard) as u64), ("double-fault", (&raw const __double_fault_stack_guard) as u64)]
}

/// Name of the stack whose guard page holds `addr`, if any: a fault there is
/// a kernel stack overflow.
pub fn stack_guard_hit(addr: u64) -> Option<&'static str> {
    guard_pages().into_iter().find(|&(_, guard)| (guard..guard + PAGE_SIZE).contains(&addr)).map(|(name, _)| name)
}

/// Whether the XD bit is in use (IA32_EFER.NXE set). Without it the bit is
/// reserved, so [`fix`] strips it from every entry.
static NX: AtomicBool = AtomicBool::new(false);

fn fix(flags: PageFlags) -> PageFlags {
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
struct BuddyFrames<'a, 'm>(&'a mut BuddyAllocator<'m>);

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
    let largest = if features.page_1g { PageSize::Size1G } else { PageSize::Size2M };

    let direct = DirectMap::from_map(memory, DIRECT_MAP_SIZE);
    let mut early = EarlyFrames::new(memory);
    let mut tables = PageTables::new(KernelPhysMemory, &mut early).expect("no low memory for the kernel page tables");
    map_direct(&mut tables, &direct, largest, &mut early);
    for s in sections() {
        let res = tables.map_range(s.start, s.start - KERNEL_VMA, s.end - s.start, fix(s.flags()), largest, &mut early);
        res.unwrap_or_else(|e| panic!("mapping the kernel {}: {e:?}", s.name));
    }

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
         NX: {}, 1 GiB pages: {}, global pages: {}",
        mapped >> 20,
        yes_no(features.nx),
        yes_no(features.page_1g),
        yes_no(features.pge)
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

/// Map the device memory at `[phys, phys + len)` for the kernel: uncached,
/// writable, never executable. Returns the virtual address of `phys`.
///
/// `None` before the kernel page tables or the frame allocator are set up,
/// when the window or the frames run out, and for ranges that overlap RAM
/// (an uncached alias of memory that the direct map caches would break cache
/// coherence). Each call creates a new mapping, with an unmapped page after
/// it; mappings are never removed yet.
pub fn map_mmio(phys: u64, len: u64) -> Option<u64> {
    let start = phys & !(PAGE_SIZE - 1);
    let end = align_up(phys.checked_add(len.max(1))?, PAGE_SIZE);
    let ram = DIRECT_MAP.get()?.spans().iter().any(|s| is_ram(s.kind) && s.start < end && start < s.end);
    if ram || end > poly_paging::PHYS_LIMIT {
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
    let flags = fix(PageFlags::PRESENT | PageFlags::WRITABLE | PageFlags::GLOBAL | PageFlags::NO_EXECUTE | UNCACHED);
    k.tables.map_range(virt, start, size, flags, k.largest, &mut BuddyFrames(frames.as_mut()?)).ok()?;
    k.mmio_next = virt + size + PAGE_SIZE;
    Some(virt + (phys - start))
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
        Ok(())
    }
}
