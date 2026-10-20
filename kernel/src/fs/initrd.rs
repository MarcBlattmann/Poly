//! The initramfs: a `newc` cpio archive the boot loader put in RAM (QEMU's
//! `-initrd`, passed as the first PVH boot module; see `boot_info.rs`),
//! unpacked into the root tmpfs at boot.
//!
//! Its pages must survive until then: the page-table bootstrap and the frame
//! allocator would otherwise hand them out. So [`reserve`] marks them
//! `Kernel` in the memory map before anything allocates, `frames::init`
//! manages them but starts with them allocated, and [`unpack`] gives them
//! back with `frames::free_range` once the archive is in the tmpfs.

use crate::boot_info::{BootInfo, Module};
use crate::sync::SetOnce;
use crate::{arch, frames, kprintln};
use alloc::string::String;
use poly_cpio::{UnpackError, UnpackStats};
use poly_mem::{FreeError, MemoryMap, PAGE_SIZE, Region, RegionKind, align_down, align_up};
use poly_vfs::{At, FsContext, Vfs};

/// What happened to the initramfs at boot.
#[derive(Clone, Debug)]
pub struct Report {
    /// Where the boot loader put it.
    pub module: Module,
    /// What was unpacked (up to the error, if there was one).
    pub stats: UnpackStats,
    /// Why unpacking stopped early, if it did.
    pub error: Option<String>,
    /// Frames given back to the frame allocator afterwards: every page the
    /// archive touched, or 0 if it was not in usable RAM.
    pub freed_frames: u64,
    /// Why the frame allocator refused them back, if it did (a bug).
    pub free_error: Option<FreeError>,
}

static REPORT: SetOnce<Report> = SetOnce::new();

/// The boot's initramfs, once [`unpack`] has run; `None` without one.
pub fn report() -> Option<&'static Report> {
    REPORT.get()
}

/// Keep the initramfs's pages away from everything that takes memory
/// before it is unpacked: returns `memory` (normalized) with them marked
/// `Kernel`, and the range of them that lies in usable RAM, so
/// `frames::init` can hold it and [`unpack`] give it back. `None` without
/// an initramfs, or for one in memory that is not RAM to reuse (the kernel
/// image, firmware areas), which is then only read.
///
/// An archive that starts in usable RAM but runs past its end into memory
/// the firmware reserved gives back only the pages below: QEMU puts the
/// archive at the top of RAM, and SeaBIOS's reservation there grows with
/// the devices it sets up (each virtio-blk disk), so with enough disks the
/// two overlap.
pub fn reserve(boot: &BootInfo, memory: &MemoryMap) -> (MemoryMap, Option<Region>) {
    let Some(m) = boot.initrd else { return (memory.clone(), None) };
    let (start, end) = (align_down(m.phys, PAGE_SIZE), align_up(m.end(), PAGE_SIZE));
    let usable = memory.regions().iter().find(|r| r.kind == RegionKind::Usable && r.start <= start && start < r.end);
    let held = usable.map(|r| Region::new(start, end.min(r.end), RegionKind::Kernel));
    let mut map = memory.clone();
    map.push(Region::new(start, end, RegionKind::Kernel)).expect("memory map full");
    kprintln!("boot: initramfs at {:#x}-{:#x} ({} KiB)", m.phys, m.end(), m.len.div_ceil(1024));
    match held {
        None => kprintln!("boot: the initramfs is not in usable RAM; its pages stay reserved"),
        Some(h) if h.end < end => {
            kprintln!("boot: the initramfs runs into reserved memory at {:#x}; only the pages below are reused", h.end)
        }
        Some(_) => {}
    }
    (map.normalized().expect("memory map full"), held)
}

/// Unpack the boot's initramfs at `module` into `vfs` (as root, at `ctx`'s
/// root), give `held`, its pages, back to the frame allocator, and log
/// what happened. A broken archive is logged; what came before the damage
/// stays unpacked.
pub fn unpack(vfs: &Vfs, ctx: &FsContext, module: Module, held: Option<Region>) {
    let report = unpack_and_release(vfs, ctx, module, held);
    let s = report.stats;
    kprintln!(
        "fs: initramfs unpacked into /: {} directories, {} files ({} bytes), {} symbolic links, {} device nodes, {} hard links",
        s.dirs,
        s.files,
        s.bytes,
        s.symlinks,
        s.nodes,
        s.hard_links
    );
    if let Some(e) = &report.error {
        kprintln!("fs: initramfs: unpacking stopped: {e}");
    }
    if let Some(r) = held {
        match report.free_error {
            None => kprintln!(
                "fs: initramfs: {} pages ({} KiB) at {:#x}-{:#x} returned to the frame allocator",
                report.freed_frames,
                report.freed_frames * PAGE_SIZE / 1024,
                r.start,
                r.end
            ),
            Some(e) => kprintln!("fs: initramfs: its pages could not be freed: {e:?}"),
        }
    }
    if REPORT.set(report).is_err() {
        panic!("the initramfs was unpacked twice");
    }
}

/// Unpack the archive at `module` into `vfs`, then free `held` (its pages,
/// taken with `frames::init` or `frames::allocate_range`). The free is the
/// last thing it does, so nothing allocates from those frames before the
/// caller looks at them.
pub fn unpack_and_release(vfs: &Vfs, ctx: &FsContext, module: Module, held: Option<Region>) -> Report {
    let mut report = Report { module, stats: UnpackStats::default(), error: None, freed_frames: 0, free_error: None };
    // 64-bit kernel: every u64 fits a usize.
    let len = module.len as usize;
    match arch::phys_range_to_virt(module.phys, module.len) {
        Some(virt) => {
            // SAFETY: the direct map maps [phys, phys + len) at `virt` for the
            // kernel's lifetime (checked above). Nothing writes it: `reserve`
            // kept it out of the page-table bootstrap, and the frame allocator
            // holds it as allocated (`held`) until it is freed below, after
            // the last use of `data`. Bytes need no alignment.
            let data = unsafe { core::slice::from_raw_parts(virt as *const u8, len) };
            match poly_cpio::unpack(data, vfs, ctx, At::Cwd) {
                Ok(stats) => report.stats = stats,
                Err(e) => report.error = Some(describe(&e)),
            }
        }
        None => report.error = Some(String::from("it is outside the direct map")),
    }
    if let Some(r) = held {
        match frames::free_range(r.start, r.end) {
            Ok((before, after)) => report.freed_frames = after - before,
            Err(e) => report.free_error = Some(e),
        }
    }
    report
}

fn describe(e: &UnpackError) -> String {
    match e {
        UnpackError::Archive(a) => alloc::format!("bad archive: {a:?}"),
        UnpackError::Entry { name, errno } => {
            alloc::format!("{}: {}", String::from_utf8_lossy(name), errno.name())
        }
    }
}
