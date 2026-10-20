//! File systems in the kernel: one global [`Vfs`] (the host-tested
//! `poly-vfs` crate, ADR 0023) with a tmpfs as `/`, filled from the
//! initramfs at boot; the character devices in `/dev`; and each process's
//! open files ([`Files`]). Decisions are in ADR 0037.
//!
//! ```text
//!   personalities (later: openat, read, write, getdents64, ...)   kernel shell (ls, cat, mount)
//!          │  copy to / from user memory happens there, not here
//!          ▼
//!   fs::Files (per process: FsContext + FdTable + Cred, behind sleeping mutexes)
//!          │  open / read / write / lseek / dup / fstat / getdents on byte buffers
//!          ▼
//!   fs::vfs() ── poly_vfs::Vfs: mounts, path lookup, permissions, char devices
//!          │                          └── /dev/console, /dev/tty → terminals (crate::tty, ADR 0046)
//!          ▼                              /dev/null, /dev/zero, /dev/full
//!   Tmpfs on /  ◀── poly_cpio::unpack ◀── initramfs (boot module 0, QEMU -initrd)
//!   ext2 on /mnt (or wherever poly.mount= says) ── page cache ── block::Disk (disk.rs, cache.rs)
//! ```
//!
//! [`init`] runs once at boot, after interrupts and time are up and before
//! the self-test: it makes the tmpfs, unpacks the initramfs into it
//! ([`initrd`]), gives the initramfs's frames back to the frame allocator,
//! makes `/dev`, `/tmp` and `/proc` and the device nodes, and registers the
//! devices. Without an initramfs `/` starts empty but for those. Then it
//! makes the page cache, starts the writeback thread ([`cache`]) and
//! mounts the disks `poly.mount=` names ([`disk`], ADR 0040).
//!
//! # Locking
//!
//! The `Vfs` locks itself with short spinlocks that never mask interrupts
//! (ADR 0023); no interrupt handler enters it, and the kernel never calls it
//! with one of its own interrupt-masking spinlocks held. [`Files`] keeps the
//! descriptor table and the file system context behind sleeping
//! [`Mutex`](crate::sync::Mutex)es, held only for table changes and path
//! walks: reads and writes run with no lock of `Files` held, so a read that
//! blocks never stalls the process's other descriptors.

pub mod cache;
pub mod devices;
pub mod disk;
mod disk_tests;
mod files;
pub mod initrd;
pub mod poll;
pub mod proc;
mod proc_tests;
mod sysfs;
mod tests;

use crate::boot_info::BootInfo;
use crate::sync::SetOnce;
use crate::{kprintln, time};
use alloc::sync::Arc;
use poly_mem::Region;
use poly_vfs::{Clock, Timespec, Tmpfs, Vfs};

pub use files::{AT_REMOVEDIR, Files};
#[allow(unused_imports, reason = "the types of the `Files` API, for the system call layer")]
pub use poly_vfs::{AT_FDCWD, DeviceId, DirEntry, Errno, Fd, FileType, Ino, OpenFile, OpenFlags, Result, Stat, Whence};

/// Kernel flag saying the boot has the test initramfs (`cargo xtask`
/// builds it); the file system tests then check its contents.
pub const TEST_INITRD_FLAG: &str = "poly.test.initrd";

static VFS: SetOnce<Vfs> = SetOnce::new();

/// The kernel's VFS. Panics before [`init`], which runs early in boot,
/// before any process exists.
pub fn vfs() -> &'static Vfs {
    VFS.get().expect("fs::vfs used before fs::init")
}

/// Timestamps for tmpfs. There is no wall clock yet (no RTC driver), so
/// times count from boot: the first second after boot is 1970-01-01
/// 00:00:00 UTC. Files from the initramfs keep the times in the archive.
#[derive(Debug)]
struct UptimeClock;

impl Clock for UptimeClock {
    fn now(&self) -> Timespec {
        let ns = time::now_ns();
        Timespec::new((ns / 1_000_000_000) as i64, (ns % 1_000_000_000) as u32)
    }
}

/// Wall-clock timestamps (`time::realtime_ns`, the RTC at boot), for disk
/// file systems: their files outlive the boot, and ext4's journal and
/// e2fsck expect real times (a deletion time of a few seconds since 1970
/// reads as an orphan list link).
#[derive(Debug)]
struct WallClock;

impl Clock for WallClock {
    fn now(&self) -> Timespec {
        let ns = time::realtime_ns();
        Timespec::new((ns / 1_000_000_000) as i64, (ns % 1_000_000_000) as u32)
    }
}

/// The clock disk file systems take their timestamps from.
pub(crate) fn clock() -> Arc<dyn Clock> {
    Arc::new(WallClock)
}

/// Set up the file systems: a tmpfs on `/`, filled from the initramfs if
/// the boot loader passed one, and the devices. `held` is what
/// [`initrd::reserve`] returned: the initramfs's pages, which the frame
/// allocator gets back once the archive is unpacked. Call once, after the
/// heap, the clock and interrupts are up.
pub fn init(boot: &BootInfo, held: Option<Region>) {
    let root = Arc::new(Tmpfs::new(Arc::new(UptimeClock)));
    let vfs = Vfs::new(root);
    let ctx = vfs.root_context().expect("the root of a new tmpfs");
    match boot.initrd {
        Some(module) => initrd::unpack(&vfs, &ctx, module, held),
        None => kprintln!("fs: no initramfs (no boot module); / is an empty tmpfs"),
    }
    devices::install(&vfs, &ctx);
    proc::mount_all(&vfs, &ctx);
    drop(ctx);
    if VFS.set(vfs).is_err() {
        panic!("fs::init called twice");
    }
    cache::init();
    if let Some(arg) = boot.arg(disk::MOUNT_ARG) {
        disk::mount_from_cmdline(arg);
    }
}
