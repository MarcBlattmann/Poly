# 0037: The VFS in the kernel: a tmpfs root from the initramfs, devices, per-process files

**Status:** Accepted. Builds on [ADR 0023](0023-vfs-tmpfs-and-cpio.md)
(the VFS, tmpfs and cpio crates) and [ADR 0002](0002-pvh-boot-first.md)
(PVH boot).

## Context

ADR 0023 put the VFS, tmpfs and the cpio unpacker in host-tested crates
and sketched the kernel side: "build a `Tmpfs` with a clock, `unpack_fs`
the initramfs into it, `Vfs::new` it, register the console and the memory
devices, give the first process `root_context()` and an `FdTable` with the
console on 0, 1 and 2." Phase 3 needs exactly that before the first Linux
program can `write(1, ...)` and before `execve` can find a binary. Open
points were how the initramfs reaches the kernel and survives until it is
unpacked, what locks the kernel puts around the VFS, what `/dev/console`
does with reads, and what the API between the kernel and the system call
layer looks like.

Sources: the public Xen PVH boot ABI (`xen/include/public/arch-x86/hvm/
start_info.h`: `hvm_start_info.nr_modules`, `modlist_paddr`, and
`hvm_modlist_entry { paddr, size, cmdline_paddr, reserved }`), the
`cpio(5)` / initramfs buffer format descriptions, and the Linux manual
pages (`open(2)`, `dup(2)`, `fcntl(2)`, `console(4)`, `tty(4)`, `null(4)`,
`hier(7)`). No kernel source was read.

## Decision

**The initramfs is PVH boot module 0.** QEMU passes `-initrd` that way
with `-kernel` and a PVH note. `boot_info` reads the module list (present in
every start-info version) and keeps the first module as `BootInfo::initrd`;
other modules are ignored until something needs them.

**Its pages are held by the frame allocator, not hidden from it.** Before
the page-table bootstrap runs, `fs::initrd::reserve` marks the module's
pages `Kernel` in the normalized memory map, so the bootstrap's bump
allocator and the allocator's metadata placement never touch them. The
frame allocator then manages them from the start but begins with them
allocated: `frames::init(memory, held)` builds the buddy allocator from the
map with that range turned back to usable (`MemoryMap::with_kind`, which
overrides instead of "most restrictive wins") and takes it out with
`BuddyAllocator::allocate_range`, which claims exactly a given range of
free frames wherever the free blocks lie. After unpacking,
`frames::free_range` gives it back and the buddies merge as usual. The
alternative, leaving the pages out of the allocator and adding them later,
needs an allocator whose span can grow: QEMU puts the initramfs at the top
of RAM, often above the last usable byte the allocator would otherwise
span. An initramfs that is not inside usable RAM (it never is with QEMU) is
read but never freed.

**One global `Vfs`, on a tmpfs root.** `fs::init` runs once at boot, after
interrupts and the clock (tmpfs timestamps) and before the self-test: a
`Tmpfs` mounted on `/`, the initramfs unpacked into it with
`poly_cpio::unpack` through that same `Vfs` (so absolute symbolic links in
the archive resolve inside it), then `/dev`, `/tmp` (`01777`) and `/proc`
(an empty mount point for procfs) and the device nodes, each only if the
archive did not bring it. Without an initramfs `/` is just those. The `Vfs`
lives in a `SetOnce`; it locks itself (ADR 0023), so the kernel adds no
lock around it. Times count from boot, since there is no wall clock yet.

**Devices.** `/dev/console` (5, 1) and `/dev/tty` (5, 0) are one kernel
console device whose writes go through the kernel log's output path
(`log::_print`): the serial port now, the screen's terminal too when the
log reaches it, with the shell's prompt redrawn after them like after any
log line. It keeps a character split across writes whole (UTF-8) and prints
U+FFFD for invalid bytes. **Reads return end of file**: the serial input
belongs to the kernel shell, and sharing it properly is the TTY layer's job
(line discipline, foreground process group, Phase 4), which will make reads
block for a line. Blocking forever now would hang any program that reads
its standard input; end of file lets it finish. `ioctl` is `ENOTTY` for
now, so `isatty` says no. `/dev/null`, `/dev/zero` and `/dev/full` are the
crate's devices. The numbers are the manual pages', so nodes from a Linux
initramfs work unchanged.

**Per-process files: `fs::Files` on `Process`.** It holds the process's
`FsContext` (root, working directory, umask) and `FdTable`, each behind a
sleeping `Mutex` (lock order: context, then table), and its `Cred` (root
until Phase 4 brings users). A new process gets `/dev/console` opened
read-write once and duplicated onto 0, 1 and 2, as `init` gets it. The
mutexes are held for table changes and path walks only: `read`/`write`
first take the `Arc<OpenFile>` out of the table and run unlocked, so a read
that blocks never stalls the process's other descriptors, and a closed
file's last reference is dropped after the table's lock is released.

**The API works on kernel byte buffers.** `Files` has `open`/`openat`,
`read`, `write`, `pread`, `pwrite`, `close`, `lseek`, `dup`, `dup2`,
`dup3`, `dup_from` (`F_DUPFD`), `cloexec`/`set_cloexec` (`F_GETFD`/
`F_SETFD`), `status_flags`/`set_status_flags` (`F_GETFL`/`F_SETFL`),
`fstat`, `statat`/`stat`/`lstat`, `getdents` (a callback per entry; the
caller encodes `linux_dirent64`), `ioctl`, `ftruncate`, `fsync`,
`readlinkat`, `mkdirat`, `unlinkat`, `symlinkat`, `chdir`, `fchdir`,
`getcwd`, `umask`, `close_on_exec`, `close_all`, `fork` and `file(fd)`.
Errors are `poly_vfs::Errno` with Linux's numbers, flags Linux's bits.
Copying to and from user memory and encoding results is the system call
layer's job, so no page fault on user memory happens with a VFS or `Files`
lock held. This ADR does not implement any system call or change the
default system call hook.

**Shell.** `ls [path]` (mode, size or device number, name, link target),
`cat <file>...` (at most 64 KiB per file, so `/dev/zero` ends) and `mount`
(with `Vfs::mount_path`, added to the crate for this and `/proc/mounts`).

**Tests.** `cargo xtask` builds the initramfs images with `poly-cpio`'s
writer (no host `cpio` needed), byte for byte reproducible: a small default
one for `cargo xtask run` (`/etc/motd`, `/etc/hostname`, `/dev/console`)
and a test one that adds a tree of every entry kind. The self-test boots
pass the test image with `poly.test.initrd`, and one boot (on `-cpu max`)
has none, so both roots are tested.

## Consequences

- The Linux personality's file calls can be wired to `Files` directly; the
  hello-world milestone needs only `write` on fd 1.
- The initramfs costs no memory after boot, and nothing can allocate its
  pages before it is unpacked, wherever the boot loader puts it.
- `BuddyAllocator::allocate_range` is also how fixed physical ranges can be
  taken later (firmware buffers handed over at runtime).
- Reading `/dev/console` returns end of file until the TTY layer exists;
  programs that need input from the console (a shell) wait for Phase 4.
- `Cred` is root for every process until users and groups arrive.
- Not yet: `/proc`, `/sys`, a devtmpfs separate from the root, `pivot_root`
  to a disk root, an RTC for real timestamps, `O_TMPFILE`, pipes.
