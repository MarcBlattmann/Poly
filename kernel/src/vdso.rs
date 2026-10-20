//! The vDSO in every Linux process (ADR 0044): the image (built from
//! `arch/x86_64/vdso/vdso.rs`, embedded as `arch::vdso::IMAGE`), its data
//! page, and mapping both at exec.
//!
//! ```text
//!   [vvar]  one page, read-only, shared: poly_vdso::Vvar, written by the kernel
//!   [vdso]  the image, read + execute, private  ◄── AT_SYSINFO_EHDR
//! ```
//!
//! - **The data page** is one frame every Linux process maps. It comes
//!   from the page cache (a one-page file of a private tmpfs registered
//!   with it), because that is how a memory map shares a kernel frame
//!   between address spaces: a shared, read-only file mapping whose
//!   source hands out the cache's page. This module holds a reference to
//!   the page forever, so the cache never evicts it, and writes the
//!   snapshot straight into the frame through the direct map. The region's
//!   protection ceiling is read-only: `mprotect` cannot make it writable.
//! - **The image** is a private file mapping of the embedded bytes,
//!   read and execute at most: each process gets its pages on demand, as
//!   for any executable (a debugger's breakpoint then copies a page).
//! - **Publishing.** [`publish`] writes a [`Snapshot`] (TSC conversion or
//!   "use the system call", the wall clock's offset, the tick, the time
//!   zone) under the seqlock, with interrupts off so no reader on this CPU
//!   waits for a preempted writer. The kernel publishes when the vDSO is
//!   set up, when the wall clock or the time zone is set, and when a test
//!   forces the TSC path ([`force_tsc_for_tests`]).
//! - Both are set up at the first exec ([`map_into`]); a failure leaves
//!   processes without a vDSO (no `AT_SYSINFO_EHDR`), which the C
//!   libraries handle by making system calls.

use crate::arch;
use crate::fs::cache::{self, CachedPage};
use crate::sync::{Mutex, SetOnce, SpinLock};
use crate::user::memory::{FileRef, IoError, MapRequest, Memory, MemoryImage, PAGE_SIZE, PageSource, Placement, Prot};
use crate::{kprintln, time};
use alloc::sync::Arc;
use core::sync::atomic::{AtomicBool, Ordering};
use poly_vdso::{Conversion, Mode, Snapshot, Timezone, Vvar};
use poly_vfs::{DeviceId, FileSystem, FileType, NewNode, Tmpfs};

/// What every process maps.
struct Vdso {
    image: Arc<dyn PageSource>,
    /// The image's length rounded up to whole pages.
    image_len: u64,
    vvar: Arc<VvarSource>,
}

static VDSO: SetOnce<Vdso> = SetOnce::new();
/// Serialises the setup (it may sleep: the page cache's locks).
static SETUP: Mutex<bool> = Mutex::new(false);
/// Serialises [`publish`] and keeps interrupts off while it writes.
static WRITER: SpinLock<()> = SpinLock::new(());
/// `settimeofday`'s time zone.
static TIMEZONE: SpinLock<Timezone> = SpinLock::new(Timezone { minuteswest: 0, dsttime: 0 });
/// A test made the vDSO read the TSC ([`force_tsc_for_tests`]).
static FORCED: SpinLock<Option<Conversion>> = SpinLock::new(None);
static REPORTED: AtomicBool = AtomicBool::new(false);

/// The data page as a mappable source: the page cache's page, shared.
struct VvarSource {
    page: CachedPage,
}

impl VvarSource {
    fn vvar(&self) -> &Vvar {
        let virt = arch::phys_to_virt(self.page.frame().phys()).expect("page cache frames are in the direct map");
        // SAFETY: the frame is a whole page of RAM that the page cache keeps
        // while `self.page` exists (forever: `VDSO` is never dropped); a
        // `Vvar` is atomics only (any bytes are valid, all access is
        // atomic) and fits in the page; nothing else in the kernel writes
        // the frame, and user mappings of it are read-only.
        unsafe { &*(virt as *const Vvar) }
    }
}

impl PageSource for VvarSource {
    fn len(&self) -> u64 {
        PAGE_SIZE
    }

    fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<usize, IoError> {
        let start = offset.min(PAGE_SIZE) as usize;
        let n = buf.len().min(PAGE_SIZE as usize - start);
        self.page.read(start, &mut buf[..n]);
        Ok(n)
    }

    fn cached_page(&self, offset: u64) -> Option<Result<CachedPage, IoError>> {
        (offset == 0).then(|| Ok(self.page.clone()))
    }

    fn shareable(&self) -> bool {
        true
    }

    fn label(&self) -> Option<&str> {
        Some("[vvar]")
    }
}

/// The image as a mappable source, named `[vdso]` in `/proc/<pid>/maps`
/// (proc_pid_maps(5)).
struct ImageSource(MemoryImage<&'static [u8]>);

impl PageSource for ImageSource {
    fn len(&self) -> u64 {
        self.0.len()
    }

    fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<usize, IoError> {
        self.0.read_at(offset, buf)
    }

    fn contents(&self) -> Option<&[u8]> {
        self.0.contents()
    }

    fn label(&self) -> Option<&str> {
        Some("[vdso]")
    }
}

/// The page the data lives in: a zeroed one-page file of a private tmpfs,
/// through the page cache.
fn data_page() -> Result<CachedPage, poly_vfs::Errno> {
    let fs = Arc::new(Tmpfs::new(crate::fs::clock()));
    let node = NewNode { kind: FileType::Regular, perm: 0o444, uid: 0, gid: 0, rdev: DeviceId::default() };
    let ino = fs.create(fs.root(), b"vvar", &node)?;
    fs.write(ino, 0, &[0; PAGE_SIZE as usize])?;
    cache::cache().register(fs).page(ino, 0)
}

/// Set the vDSO up once; `None` if that failed (then processes go
/// without).
fn setup() -> Option<&'static Vdso> {
    if let Some(v) = VDSO.get() {
        return Some(v);
    }
    let mut tried = SETUP.lock();
    if let Some(v) = VDSO.get() {
        return Some(v);
    }
    if *tried {
        return None;
    }
    *tried = true;
    let page = match data_page() {
        Ok(page) => page,
        Err(e) => {
            kprintln!("vdso: no data page ({}): processes run without a vDSO", e.name());
            return None;
        }
    };
    let image = arch::vdso::IMAGE;
    let vdso = Vdso {
        image: Arc::new(ImageSource(MemoryImage(image))),
        image_len: poly_vm::page_up(image.len() as u64).expect("a small image"),
        vvar: Arc::new(VvarSource { page }),
    };
    let v = VDSO.set(vdso).ok()?;
    publish();
    Some(v)
}

/// What the vDSO should know now.
fn snapshot() -> Snapshot {
    let forced = *FORCED.lock();
    let conversion = forced.or_else(arch::vdso::conversion);
    Snapshot {
        mode: if conversion.is_some() { Mode::Tsc } else { Mode::Syscall },
        conversion: conversion.unwrap_or_default(),
        realtime_offset_ns: time::realtime_offset_ns(),
        coarse_resolution_ns: time::NANOS_PER_TICK,
        timezone: *TIMEZONE.lock(),
    }
}

/// Write the current snapshot to the data page (if the vDSO is set up).
pub fn publish() {
    let Some(v) = VDSO.get() else { return };
    let s = snapshot();
    {
        let _writer = WRITER.lock();
        v.vvar.vvar().publish(&s);
    }
    if !REPORTED.swap(true, Ordering::Relaxed) {
        let how = match s.mode {
            Mode::Tsc => "reads the TSC",
            Mode::Syscall => "makes system calls (the clock does not read an invariant TSC)",
        };
        kprintln!("vdso: {} bytes, mapped into every Linux process; clock_gettime {how}", arch::vdso::IMAGE.len());
    }
}

/// What the data page holds now (tests).
pub fn current() -> Option<Snapshot> {
    VDSO.get().map(|v| v.vvar.vvar().read())
}

/// The time zone `settimeofday` stored (`gettimeofday` reports it).
pub fn timezone() -> Timezone {
    *TIMEZONE.lock()
}

/// Store `settimeofday`'s time zone.
pub fn set_timezone(tz: Timezone) {
    *TIMEZONE.lock() = tz;
    publish();
}

/// Make the vDSO read the TSC with its calibrated rate, aligned with the
/// clock now, even where the clock itself does not read the TSC (QEMU's
/// TCG): `true` while a test runs the vDSO's own clock path; `false` puts
/// back what the clock decides. Returns whether the TSC path is on.
pub fn force_tsc_for_tests(on: bool) -> bool {
    *FORCED.lock() = if on { arch::vdso::calibrated_conversion() } else { None };
    publish();
    current().is_some_and(|s| s.mode == Mode::Tsc)
}

/// Map the data page and the image into `memory`, the page right below
/// the image, anywhere the memory's layout puts mappings: the address of
/// the image's ELF header (`AT_SYSINFO_EHDR`), or `None` if there is no
/// vDSO.
pub fn map_into(memory: &Memory) -> Option<u64> {
    let v = setup()?;
    let image = MapRequest::file(v.image_len, Prot::RX, FileRef::new(Arc::clone(&v.image)), 0, false)
        .with_max_prot(Prot::RX)
        .at(Placement::Hint(0));
    let ehdr = memory.map(image).ok()?;
    let source: Arc<dyn PageSource> = v.vvar.clone();
    let data = MapRequest::file(PAGE_SIZE, Prot::READ, FileRef::new(source), 0, true)
        .with_max_prot(Prot::READ)
        .at(Placement::FixedNoReplace(ehdr - PAGE_SIZE));
    if memory.map(data).is_err() {
        let _ = memory.unmap(ehdr, v.image_len);
        return None;
    }
    Some(ehdr)
}

// --- Tests ------------------------------------------------------------------

use crate::kernel_test;
use crate::user::memory::Layout;

kernel_test! {
    fn the_image_is_a_shared_object_poly_elf_accepts() -> TestResult {
        let elf = poly_elf::Elf::parse(arch::vdso::IMAGE).map_err(|_| "poly-elf refuses the vDSO image")?;
        if elf.interp().is_some() {
            return Err("the vDSO asks for an interpreter");
        }
        if arch::vdso::IMAGE.get(16..18) != Some(&[3, 0]) {
            return Err("the vDSO is not ET_DYN");
        }
        Ok(())
    }
}

kernel_test! {
    fn map_into_puts_the_data_page_below_the_image() -> TestResult {
        let memory = Memory::new(Layout::default()).map_err(|_| "no memory map")?;
        let ehdr = map_into(&memory).ok_or("no vDSO")?;
        if ehdr % PAGE_SIZE != 0 {
            return Err("the image is not page aligned");
        }
        let mut head = [0u8; 64];
        memory.read(ehdr, &mut head).map_err(|_| "the image cannot be read")?;
        if head[..] != arch::vdso::IMAGE[..64] {
            return Err("the image's first bytes differ from the embedded vDSO");
        }
        // The data page is the kernel's: a published snapshot, sequence even.
        let mut seq = [0u8; 4];
        memory.read(ehdr - PAGE_SIZE, &mut seq).map_err(|_| "the data page cannot be read")?;
        let seq = u32::from_le_bytes(seq);
        if seq == 0 || seq % 2 != 0 {
            kprintln!("vdso: data page sequence {seq}");
            return Err("the data page holds no published snapshot");
        }
        let regions = memory.regions();
        let vvar = regions.iter().find(|r| r.start == ehdr - PAGE_SIZE).ok_or("no data page region")?;
        let image = regions.iter().find(|r| r.start == ehdr).ok_or("no image region")?;
        if vvar.prot != Prot::READ || vvar.max_prot != Prot::READ || image.prot != Prot::RX || image.max_prot != Prot::RX
        {
            return Err("the vDSO regions have the wrong protections");
        }
        // Neither can be made writable.
        if memory.protect(ehdr - PAGE_SIZE, PAGE_SIZE, Prot::RW).is_ok() || memory.protect(ehdr, PAGE_SIZE, Prot::RW).is_ok()
        {
            return Err("mprotect made the vDSO writable");
        }
        // Another memory maps the same data page.
        let other = Memory::new(Layout::default()).map_err(|_| "no memory map")?;
        let ehdr2 = map_into(&other).ok_or("no vDSO the second time")?;
        let mut seq2 = [0u8; 4];
        other.read(ehdr2 - PAGE_SIZE, &mut seq2).map_err(|_| "the second data page cannot be read")?;
        publish();
        let mut seq3 = [0u8; 4];
        other.read(ehdr2 - PAGE_SIZE, &mut seq3).map_err(|_| "the second data page cannot be read")?;
        if u32::from_le_bytes(seq2) != seq || u32::from_le_bytes(seq3) != seq + 2 {
            return Err("two processes do not see the one data page the kernel writes");
        }
        // A forked memory keeps both: the image (shared copy on write) and
        // the data page (a shared mapping: the same frame, updated live).
        let child = other.fork().map_err(|_| "fork failed")?;
        let mut child_head = [0u8; 64];
        child.read(ehdr2, &mut child_head).map_err(|_| "the child's image cannot be read")?;
        publish();
        let mut seq4 = [0u8; 4];
        child.read(ehdr2 - PAGE_SIZE, &mut seq4).map_err(|_| "the child's data page cannot be read")?;
        if child_head[..] != arch::vdso::IMAGE[..64] || u32::from_le_bytes(seq4) != seq + 4 {
            return Err("a forked memory lost the vDSO or its live data page");
        }
        Ok(())
    }
}

kernel_test! {
    fn the_snapshot_follows_the_wall_clock_and_forcing() -> TestResult {
        if setup().is_none() {
            return Err("no vDSO");
        }
        let before = current().ok_or("no snapshot")?;
        if before.realtime_offset_ns != time::realtime_offset_ns() || before.coarse_resolution_ns != time::NANOS_PER_TICK {
            return Err("the snapshot does not match the clocks");
        }
        let offset = time::realtime_offset_ns();
        let ahead = time::realtime_ns() + 3600 * time::NANOS_PER_SEC;
        time::set_realtime_ns(ahead).map_err(|()| "setting the wall clock failed")?;
        let moved = current().ok_or("no snapshot")?.realtime_offset_ns;
        time::restore_realtime_offset(offset);
        if moved.abs_diff(offset + 3600 * time::NANOS_PER_SEC) > time::NANOS_PER_SEC {
            return Err("the snapshot did not follow clock_settime");
        }
        if current().map(|s| s.realtime_offset_ns) != Some(offset) {
            return Err("the snapshot did not follow the restored wall clock");
        }
        if time::set_realtime_ns(1).is_ok() {
            return Err("the wall clock was set below the monotonic clock");
        }
        let natural = before.mode;
        let forced = force_tsc_for_tests(true);
        let s = current().ok_or("no snapshot")?;
        let check = (|| {
            if arch::time::info().and_then(|i| i.tsc_hz).is_none() {
                return Ok(());
            }
            if !forced || s.mode != Mode::Tsc {
                return Err("forcing the TSC path did not publish it");
            }
            // The forced conversion agrees with the clock now.
            let now = time::now_ns();
            let tsc = s.monotonic_ns(arch::cpu::rdtsc()).ok_or("no TSC time")?;
            if tsc.abs_diff(now) > 50 * time::NANOS_PER_MILLI {
                kprintln!("vdso: TSC time {tsc}, clock {now}");
                return Err("the forced TSC conversion disagrees with the clock");
            }
            Ok(())
        })();
        let back = force_tsc_for_tests(false);
        check?;
        if back != (natural == Mode::Tsc) || current().map(|s| s.mode) != Some(natural) {
            return Err("the snapshot did not go back to the clock's own mode");
        }
        Ok(())
    }
}
