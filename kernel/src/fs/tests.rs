//! Kernel tests of the file systems: the initramfs as `cargo xtask` builds
//! it, the devices, descriptor tables, tmpfs I/O, and giving the
//! initramfs's frames back.

use super::devices::Utf8Carry;
use super::initrd::{self, Report};
use super::*;
use crate::arch::cpu::InterruptsOff;
use crate::boot_info::Module;
use crate::frames::{self, FRAMES};
use crate::kernel_test;
use crate::process::Process;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;
use poly_mem::{PAGE_SIZE, RangeError, Region, RegionKind, align_down, align_up};
use poly_vfs::{Cred, Tmpfs, device};

/// Modification time of every entry in the test initramfs. Must match
/// `INITRAMFS_MTIME` in `tools/xtask`.
const INITRAMFS_MTIME: i64 = 1_700_000_000;

/// Byte `i` of `/test/big.bin` in the test initramfs. Must match
/// `big_bin` in `tools/xtask`.
fn big_bin(i: usize) -> u8 {
    (i * 7 + i / 251) as u8
}

/// Length of `/test/big.bin`: more than nine tmpfs pages, not a multiple.
const BIG_BIN_LEN: u64 = 40_000;

fn e(err: Errno) -> &'static str {
    err.name()
}

fn free_frames() -> u64 {
    FRAMES.lock().as_ref().map_or(0, |f| f.free_frames())
}

/// Any size: directories, and files whose text may change.
const ANY: u64 = u64::MAX;

/// Every file in the test initramfs: path, type, permissions, size.
const EXPECTED: [(&str, FileType, u16, u64); 16] = [
    ("/etc", FileType::Directory, 0o755, ANY),
    ("/etc/motd", FileType::Regular, 0o644, ANY),
    ("/etc/hostname", FileType::Regular, 0o644, 5),
    ("/test", FileType::Directory, 0o750, ANY),
    ("/test/hello.txt", FileType::Regular, 0o644, 26),
    ("/test/empty", FileType::Regular, 0o600, 0),
    ("/test/big.bin", FileType::Regular, 0o644, BIG_BIN_LEN),
    ("/test/script.sh", FileType::Regular, 0o755, 18),
    ("/test/tree/a/b/c", FileType::Directory, 0o700, ANY),
    ("/test/tree/a/b/c/leaf.txt", FileType::Regular, 0o444, 5),
    ("/test/link", FileType::Symlink, 0o777, 9),
    ("/test/abs-link", FileType::Symlink, 0o777, 9),
    ("/test/dangling", FileType::Symlink, 0o777, 7),
    ("/test/zero-node", FileType::CharDevice, 0o666, 0),
    ("/test/hard1", FileType::Regular, 0o640, 7),
    ("/test/hard2", FileType::Regular, 0o640, 7),
];

kernel_test! {
    fn initramfs_contents_appear_under_root(env: &TestEnv) -> TestResult {
        let f = Files::new();
        for dir in [b"/dev".as_slice(), b"/tmp", b"/proc"] {
            if f.stat(dir).map_err(e)?.kind() != Some(FileType::Directory) {
                return Err("/dev, /tmp or /proc is not a directory");
            }
        }
        if f.stat(b"/tmp").map_err(e)?.perm() != 0o1777 {
            return Err("/tmp is not world-writable with the sticky bit");
        }
        if !env.boot.has_flag(TEST_INITRD_FLAG) {
            // A boot without the test archive: a report only with a module.
            return if env.boot.initrd.is_some() == initrd::report().is_some() {
                Ok(())
            } else {
                Err("an initramfs report without a boot module, or none with one")
            };
        }
        let report = initrd::report().ok_or("the test initramfs was not unpacked")?;
        if report.error.is_some() {
            return Err("unpacking the test initramfs failed");
        }
        let s = report.stats;
        if s.symlinks != 3 || s.nodes != 2 || s.hard_links != 1 {
            crate::kprintln!("fs: unpacked {s:?}");
            return Err("the initramfs did not unpack the links and nodes it holds");
        }
        for (path, kind, perm, size) in EXPECTED {
            let st = f.lstat(path.as_bytes()).map_err(|_| "a file of the initramfs is missing")?;
            let size_ok = size == ANY || st.size == size;
            if st.kind() != Some(kind) || st.perm() != perm || !size_ok || st.uid != 0 || st.gid != 0 {
                crate::kprintln!("fs: {path}: mode {:o} size {} (expected {perm:o}, {size})", st.mode, st.size);
                return Err("a file of the initramfs has the wrong type, mode, size or owner");
            }
            if kind != FileType::Directory && st.mtime.sec != INITRAMFS_MTIME {
                return Err("a file of the initramfs lost its modification time");
            }
        }
        // Contents, through links too.
        let read = |p: &[u8]| f.read_to_end(p, 1 << 20).map(|(d, _)| d).map_err(e);
        if read(b"/test/hello.txt")? != b"Hello from the initramfs!\n" || read(b"/test/link")? != read(b"/test/hello.txt")? {
            return Err("hello.txt, or the link to it, reads wrong");
        }
        if !read(b"/test/abs-link")?.starts_with(b"Welcome to Poly") || read(b"/etc/hostname")? != b"poly\n" {
            return Err("/etc/motd (through an absolute link) or /etc/hostname reads wrong");
        }
        let big = read(b"/test/big.bin")?;
        if big.len() as u64 != BIG_BIN_LEN || big.iter().enumerate().any(|(i, &b)| b != big_bin(i)) {
            return Err("big.bin does not hold its pattern");
        }
        if read(b"/test/tree/a/b/c/leaf.txt")? != b"leaf\n" || !read(b"/test/empty")?.is_empty() {
            return Err("leaf.txt or empty reads wrong");
        }
        let targets: [(&[u8], &[u8]); 3] =
            [(b"/test/link", b"hello.txt"), (b"/test/abs-link", b"/etc/motd"), (b"/test/dangling", b"nowhere")];
        for (link, target) in targets {
            if f.readlinkat(AT_FDCWD, link).map_err(e)? != target {
                return Err("a symbolic link has the wrong target");
            }
        }
        if f.stat(b"/test/dangling") != Err(Errno::ENOENT) {
            return Err("a dangling link resolved");
        }
        // Hard links share the inode.
        let (h1, h2) = (f.lstat(b"/test/hard1").map_err(e)?, f.lstat(b"/test/hard2").map_err(e)?);
        if h1.ino != h2.ino || h1.nlink != 2 || read(b"/test/hard1")? != b"linked\n" {
            return Err("hard links do not share one inode with two links");
        }
        // The device node reaches /dev/zero's device.
        let node = f.lstat(b"/test/zero-node").map_err(e)?;
        let fd = f.open(b"/test/zero-node", OpenFlags::RDONLY, 0).map_err(e)?;
        let mut buf = [0xffu8; 8];
        let got = f.read(fd, &mut buf);
        f.close(fd).map_err(e)?;
        if node.rdev != device::ZERO || got != Ok(8) || buf != [0; 8] {
            return Err("the device node from the archive does not reach /dev/zero");
        }
        // The archive's /dev/console stays; the kernel adds the rest.
        let console = f.lstat(b"/dev/console").map_err(e)?;
        if console.rdev != device::CONSOLE || console.perm() != 0o600 || f.lstat(b"/dev/null").is_err() {
            return Err("/dev/console from the archive or /dev/null from the kernel is wrong");
        }
        let mut names: Vec<Vec<u8>> = f.read_dir(b"/test").map_err(e)?.into_iter().map(|(n, _, _)| n).collect();
        names.sort();
        let want: [&[u8]; 11] = [
            b"abs-link", b"big.bin", b"dangling", b"empty", b"hard1", b"hard2", b"hello.txt", b"link",
            b"script.sh", b"tree", b"zero-node",
        ];
        if names != want {
            return Err("/test does not list exactly the archive's entries");
        }
        Ok(())
    }
}

kernel_test! {
    fn initramfs_frames_return_to_the_allocator(env: &TestEnv) -> TestResult {
        // The boot's archive: every page it touched in usable RAM came back.
        if let Some(r) = initrd::report() {
            let (start, end) = (align_down(r.module.phys, PAGE_SIZE), align_up(r.module.end(), PAGE_SIZE));
            let usable = env.memory.regions().iter().find(|m| {
                m.kind == RegionKind::Usable && m.start <= start && start < m.end
            });
            let pages = usable.map_or(0, |m| (end.min(m.end) - start) / PAGE_SIZE);
            if r.free_error.is_some() || r.freed_frames != pages {
                return Err("the initramfs's pages were not all returned to the frame allocator");
            }
        }
        // The same path on an archive written into frames taken the way
        // `frames::init` holds the boot's: unpack, free, and the frames are
        // free again. Interrupts off: no other thread or handler may take
        // frames while the count is compared (the shell runs this too).
        let mut w = poly_cpio::Writer::new(poly_cpio::Format::Newc);
        w.dir(b"d", 0o755);
        w.file(b"d/f", 0o644, &[0x5a; 5000]);
        let archive = w.finish();
        let fs = Arc::new(Tmpfs::new(Arc::new(UptimeClock)));
        let vfs = poly_vfs::Vfs::new(fs);
        let ctx = vfs.root_context().map_err(e)?;
        let pages = align_up(archive.len() as u64, PAGE_SIZE) / PAGE_SIZE;
        let _irq = InterruptsOff::new();
        let phys = frames::allocate_contiguous(pages).ok_or("no frames for the archive")?;
        frames::free_contiguous(phys, pages).map_err(|_| "free_contiguous refused its run")?;
        let (start, end) = (phys, phys + pages * PAGE_SIZE);
        let before = free_frames();
        if frames::allocate_range(start, end) != Ok(pages) || free_frames() != before - pages {
            return Err("allocate_range did not take exactly the range");
        }
        if frames::allocate_range(start, start + PAGE_SIZE) != Err(RangeError::NotFree) {
            return Err("a frame was taken twice");
        }
        let virt = crate::arch::phys_range_to_virt(start, end - start).ok_or("frames outside the direct map")?;
        // SAFETY: frames this test took, through the direct map; the archive
        // fits (`pages` was sized from it).
        unsafe { core::ptr::copy_nonoverlapping(archive.as_ptr(), virt as *mut u8, archive.len()) };
        let module = Module { phys, len: archive.len() as u64 };
        let report: Report =
            initrd::unpack_and_release(&vfs, &ctx, module, Some(Region::new(start, end, RegionKind::Kernel)));
        // Free right now: nothing ran in between that could have taken them.
        let taken_again = frames::allocate_range(start, end);
        let freed_again = frames::free_range(start, end);
        drop(_irq);
        if report.error.is_some() || report.freed_frames != pages || report.free_error.is_some() {
            return Err("unpack_and_release did not unpack and free the range");
        }
        if taken_again != Ok(pages) || freed_again.is_err() {
            return Err("the frames were not free after unpacking");
        }
        if frames::free_range(start, end) != Err(poly_mem::FreeError::NotAllocated) {
            return Err("a range was freed twice");
        }
        let file = vfs
            .open(&ctx, &Cred::root(), poly_vfs::At::Cwd, b"/d/f", OpenFlags::RDONLY, 0)
            .map_err(e)?;
        let mut buf = vec![0u8; 6000];
        if file.read(&mut buf) != Ok(5000) || buf[..5000].iter().any(|&b| b != 0x5a) {
            return Err("the unpacked file does not hold the archive's data");
        }
        Ok(())
    }
}

kernel_test! {
    fn null_zero_full_and_console_behave() -> TestResult {
        let f = Files::new();
        let open = |p: &[u8]| f.open(p, OpenFlags::RDWR, 0).map_err(e);
        let null = open(b"/dev/null")?;
        let mut buf = [0xffu8; 64];
        if f.write(null, b"discarded") != Ok(9) || f.read(null, &mut buf) != Ok(0) || buf[0] != 0xff {
            return Err("/dev/null does not swallow writes and read end of file");
        }
        if f.lseek(null, 100, Whence::Set) != Ok(100) {
            return Err("/dev/null does not seek");
        }
        let zero = open(b"/dev/zero")?;
        if f.read(zero, &mut buf) != Ok(64) || buf != [0; 64] || f.write(zero, b"x") != Ok(1) {
            return Err("/dev/zero does not read zeros and take writes");
        }
        let full = open(b"/dev/full")?;
        if f.write(full, b"x") != Err(Errno::ENOSPC) || f.read(full, &mut buf[..3]) != Ok(3) {
            return Err("/dev/full does not refuse writes with ENOSPC");
        }
        let st = f.fstat(null).map_err(e)?;
        if st.kind() != Some(FileType::CharDevice) || st.rdev != device::NULL || st.perm() != 0o666 {
            return Err("/dev/null is not character device 1,3 with mode 0666");
        }
        for path in [b"/dev/console".as_slice(), b"/dev/tty"] {
            let fd = open(path)?;
            let msg = b"fs: console test: a line written to /dev/console\n";
            let wrote = f.write(fd, if path == b"/dev/tty" { b"" } else { msg });
            if f.read(fd, &mut buf) != Ok(0) || !wrote.is_ok_and(|n| n == 0 || n == msg.len()) {
                return Err("the console does not take writes and read end of file");
            }
            if f.lseek(fd, 0, Whence::Set) != Err(Errno::ESPIPE) || f.ioctl(fd, 0x5401, 0) != Err(Errno::ENOTTY) {
                return Err("the console seeks, or answers an ioctl");
            }
        }
        // Opening a node of an unregistered device fails with ENXIO.
        let tmp = b"/tmp/fs-test-nodev";
        let mut ctx = vfs().root_context().map_err(e)?;
        ctx.set_umask(0);
        vfs().mknod(&ctx, &Cred::root(), poly_vfs::At::Cwd, tmp, FileType::CharDevice, 0o600, DeviceId::new(240, 7))
            .map_err(e)?;
        let nodev = f.open(tmp, OpenFlags::RDONLY, 0);
        f.unlinkat(AT_FDCWD, tmp, 0).map_err(e)?;
        if nodev != Err(Errno::ENXIO) {
            return Err("a node without a device opened");
        }
        Ok(())
    }
}

kernel_test! {
    fn new_processes_get_the_console_on_0_1_2() -> TestResult {
        let p = Process::new(77, poly_abi::Personality::Linux, poly_binfmt::Format::Elf {
            bits: poly_binfmt::Bits::B64,
            arch: poly_binfmt::Arch::X86_64,
        });
        let f = &p.files;
        if f.count() != 3 {
            return Err("a new process does not have exactly fds 0, 1 and 2");
        }
        let first = f.file(0).map_err(e)?;
        for fd in 0..3 {
            let st = f.fstat(fd).map_err(e)?;
            if st.rdev != device::CONSOLE || f.cloexec(fd) != Ok(false) {
                return Err("fds 0-2 are not the console, or are close-on-exec");
            }
            if !Arc::ptr_eq(&first, &f.file(fd).map_err(e)?) {
                return Err("fds 0-2 are not one open file description");
            }
        }
        if f.status_flags(1).map_err(e)?.intersection(OpenFlags::ACCMODE) != OpenFlags::RDWR {
            return Err("the console is not open for reading and writing");
        }
        if f.write(1, b"") != Ok(0) || f.read(0, &mut [0u8; 4]) != Ok(0) {
            return Err("the console fds do not work");
        }
        if f.getcwd().map_err(e)? != b"/" {
            return Err("a new process does not start in /");
        }
        Ok(())
    }
}

kernel_test! {
    fn descriptors_follow_dup_close_and_cloexec_rules() -> TestResult {
        let f = Files::with_console();
        let path = b"/tmp/fs-test-fds";
        let a = f.open(path, OpenFlags::RDWR | OpenFlags::CREAT | OpenFlags::CLOEXEC, 0o600).map_err(e)?;
        if a != 3 || f.cloexec(a) != Ok(true) {
            return Err("open did not take fd 3, or lost O_CLOEXEC");
        }
        // The lowest free number, whatever was opened later.
        f.close(1).map_err(e)?;
        let b = f.dup(a).map_err(e)?;
        if b != 1 || f.cloexec(b) != Ok(false) {
            return Err("dup did not take the lowest fd, or kept close-on-exec");
        }
        // Duplicates share the offset.
        f.write(a, b"0123456789").map_err(e)?;
        if f.lseek(b, 0, Whence::Cur) != Ok(10) || f.lseek(b, 2, Whence::Set) != Ok(2) {
            return Err("a duplicate has its own offset");
        }
        let mut two = [0u8; 2];
        if f.read(a, &mut two) != Ok(2) || &two != b"23" {
            return Err("a read did not start at the shared offset");
        }
        // dup2 onto an open fd replaces it; dup3 sets close-on-exec as asked.
        if f.dup2(a, 2) != Ok(2) || f.fstat(2).map_err(e)?.rdev == device::CONSOLE || f.cloexec(2) != Ok(false) {
            return Err("dup2 did not replace fd 2");
        }
        if f.dup2(a, a) != Ok(a) || f.dup3(a, a, OpenFlags::CLOEXEC) != Err(Errno::EINVAL) {
            return Err("dup2 / dup3 onto the same fd misbehave");
        }
        if f.dup3(a, 10, OpenFlags::CLOEXEC) != Ok(10) || f.cloexec(10) != Ok(true) {
            return Err("dup3 did not set close-on-exec");
        }
        if f.dup_from(0, 5, false) != Ok(5) || f.set_cloexec(5, true).is_err() || f.cloexec(5) != Ok(true) {
            return Err("F_DUPFD or F_SETFD misbehave");
        }
        // Closed and never-opened descriptors are EBADF.
        f.close(b).map_err(e)?;
        if f.close(b) != Err(Errno::EBADF) || f.read(b, &mut two) != Err(Errno::EBADF) || f.fstat(-1).is_ok() {
            return Err("a closed fd is not EBADF");
        }
        // Exec closes exactly the close-on-exec ones.
        f.close_on_exec();
        let open: Vec<Fd> = (0..12).filter(|&fd| f.file(fd).is_ok()).collect();
        if open != [0, 2] {
            crate::kprintln!("fs: open after exec: {open:?}");
            return Err("close_on_exec closed the wrong descriptors");
        }
        // A fork copy shares descriptions but has its own table.
        let child = f.fork();
        child.close(0).map_err(e)?;
        if f.file(0).is_err() || !Arc::ptr_eq(&f.file(2).map_err(e)?, &child.file(2).map_err(e)?) {
            return Err("a forked table is not a copy sharing open files");
        }
        f.close_all();
        if f.count() != 0 {
            return Err("close_all left descriptors open");
        }
        f.unlinkat(AT_FDCWD, path, 0).map_err(e)?;
        Ok(())
    }
}

kernel_test! {
    fn tmpfs_files_round_trip() -> TestResult {
        let f = Files::new();
        f.mkdirat(AT_FDCWD, b"/tmp/fs-test", 0o755).map_err(e)?;
        let dir = f.open(b"/tmp/fs-test", OpenFlags::RDONLY | OpenFlags::DIRECTORY, 0).map_err(e)?;
        // Relative to a directory descriptor.
        let fd = f.openat(dir, b"data", OpenFlags::RDWR | OpenFlags::CREAT | OpenFlags::EXCL, 0o666).map_err(e)?;
        let data: Vec<u8> = (0..10_000u32).map(|i| (i % 251) as u8).collect();
        if f.write(fd, &data) != Ok(data.len()) {
            return Err("a write across pages was short");
        }
        let st = f.fstat(fd).map_err(e)?;
        if st.size != data.len() as u64 || st.perm() != 0o644 {
            return Err("size or mode (0666 masked by umask 022) is wrong");
        }
        if f.lseek(fd, 0, Whence::Set) != Ok(0) {
            return Err("lseek to 0 failed");
        }
        let mut back = vec![0u8; 12_000];
        if f.read(fd, &mut back) != Ok(data.len()) || back[..data.len()] != data[..] {
            return Err("the data did not read back");
        }
        if f.read(fd, &mut back) != Ok(0) {
            return Err("no end of file after the data");
        }
        // pwrite past the end leaves a hole of zeros; pread leaves the offset.
        if f.pwrite(fd, b"end", 12_000) != Ok(3) || f.pread(fd, &mut back[..4], 11_998) != Ok(4) {
            return Err("pwrite / pread failed");
        }
        if &back[..4] != b"\0\0en" || f.lseek(fd, 0, Whence::Cur) != Ok(data.len() as u64) {
            return Err("pread / pwrite moved the offset or the hole is not zeros");
        }
        if f.lseek(fd, -3, Whence::End) != Ok(12_000) || f.ftruncate(fd, 4).is_err() || f.fstat(fd).map_err(e)?.size != 4 {
            return Err("SEEK_END or ftruncate misbehave");
        }
        f.close(fd).map_err(e)?;
        // O_APPEND writes at the end; O_EXCL refuses an existing name.
        let app = f.openat(dir, b"data", OpenFlags::WRONLY | OpenFlags::APPEND, 0).map_err(e)?;
        f.write(app, b"++").map_err(e)?;
        f.close(app).map_err(e)?;
        if f.read_to_end(b"/tmp/fs-test/data", 64).map_err(e)?.0 != [0, 1, 2, 3, b'+', b'+'] {
            return Err("O_APPEND did not write at the end");
        }
        if f.openat(dir, b"data", OpenFlags::WRONLY | OpenFlags::CREAT | OpenFlags::EXCL, 0) != Err(Errno::EEXIST) {
            return Err("O_EXCL opened an existing file");
        }
        f.symlinkat(b"data", dir, b"link").map_err(e)?;
        let mut names: Vec<(Vec<u8>, u8)> =
            f.read_dir(b"/tmp/fs-test").map_err(e)?.into_iter().map(|(n, k, _)| (n, k.dirent_type())).collect();
        names.sort();
        if names != [(b"data".to_vec(), FileType::Regular.dirent_type()), (b"link".to_vec(), FileType::Symlink.dirent_type())] {
            return Err("getdents does not list the directory");
        }
        // Writing a directory or reading past ENOENT fails as documented.
        if f.write(dir, b"x") != Err(Errno::EBADF) || f.read(dir, &mut back) != Err(Errno::EISDIR) {
            return Err("a directory took a write or read");
        }
        f.close(dir).map_err(e)?;
        f.unlinkat(AT_FDCWD, b"/tmp/fs-test/link", 0).map_err(e)?;
        f.unlinkat(AT_FDCWD, b"/tmp/fs-test/data", 0).map_err(e)?;
        f.unlinkat(AT_FDCWD, b"/tmp/fs-test", AT_REMOVEDIR).map_err(e)?;
        if f.stat(b"/tmp/fs-test") != Err(Errno::ENOENT) || f.open(b"/nope/x", OpenFlags::RDONLY, 0) != Err(Errno::ENOENT) {
            return Err("removed files are still there");
        }
        Ok(())
    }
}

kernel_test! {
    fn working_directory_umask_and_status_flags() -> TestResult {
        let f = Files::new();
        if f.cred().uid != 0 || f.umask(0o077) != 0o022 {
            return Err("a new Files is not root with umask 022");
        }
        f.mkdirat(AT_FDCWD, b"/tmp/fs-test-cwd", 0o777).map_err(e)?;
        f.chdir(b"/tmp/fs-test-cwd").map_err(e)?;
        let fd = f.open(b"file", OpenFlags::WRONLY | OpenFlags::CREAT, 0o666).map_err(e)?;
        let (cwd, st) = (f.getcwd().map_err(e)?, f.stat(b"/tmp/fs-test-cwd/file").map_err(e)?);
        let dir_perm = f.stat(b".").map_err(e)?.perm();
        if cwd != b"/tmp/fs-test-cwd" || st.perm() != 0o600 || dir_perm != 0o700 {
            return Err("chdir, getcwd or the umask misbehave");
        }
        // F_SETFL turns on O_APPEND but cannot change the access mode.
        f.set_status_flags(fd, OpenFlags::APPEND | OpenFlags::RDWR).map_err(e)?;
        let flags = f.status_flags(fd).map_err(e)?;
        if !flags.contains(OpenFlags::APPEND) || flags.intersection(OpenFlags::ACCMODE) != OpenFlags::WRONLY {
            return Err("F_SETFL changed the wrong flags");
        }
        if f.fsync(fd).is_err() || f.fchdir(fd) != Err(Errno::ENOTDIR) {
            return Err("fsync failed, or fchdir took a file");
        }
        let root = f.open(b"/", OpenFlags::RDONLY | OpenFlags::DIRECTORY, 0).map_err(e)?;
        f.fchdir(root).map_err(e)?;
        if f.getcwd().map_err(e)? != b"/" || f.chdir(b"/etc/../nonexistent") != Err(Errno::ENOENT) {
            return Err("fchdir did not move to /, or chdir to nothing worked");
        }
        f.close_all();
        f.unlinkat(AT_FDCWD, b"/tmp/fs-test-cwd/file", 0).map_err(e)?;
        f.unlinkat(AT_FDCWD, b"/tmp/fs-test-cwd", AT_REMOVEDIR).map_err(e)?;
        Ok(())
    }
}

kernel_test! {
    fn console_writes_keep_utf8_characters_whole() -> TestResult {
        // A character split over writes prints once, whole; invalid bytes
        // print U+FFFD, also when a carried start is not continued.
        let cases: [(&[&[u8]], &str); 5] = [
            (&[b"plain ", b"text"], "plain text"),
            (&[b"gr\xc3", b"\xbcezi \xe2\x82", b"\xac!"], "gr\u{fc}ezi \u{20ac}!"),
            (&[b"\xf0\x9f", b"\x98", b"\x80"], "\u{1f600}"),
            (&[b"a\xffb\xc3(c"], "a\u{fffd}b\u{fffd}(c"),
            (&[b"\xe2\x82", b"x"], "\u{fffd}x"),
        ];
        for (writes, want) in cases {
            let mut carry = Utf8Carry::default();
            let mut out = String::new();
            for w in writes {
                carry.feed(w, &mut |s| out.push_str(s));
            }
            if out != want {
                crate::kprintln!("fs: console printed {out:?}, expected {want:?}");
                return Err("the console mangled UTF-8 split across writes");
            }
        }
        Ok(())
    }
}
