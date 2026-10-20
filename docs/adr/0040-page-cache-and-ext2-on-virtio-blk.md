# 0040: A page cache in a host-tested crate in front of disk file systems; ext2 mounted on virtio-blk with a sleeping volume lock

**Status:** Accepted. Builds on [ADR 0032](0032-ext2-driver.md) (the ext2
driver), [ADR 0023](0023-vfs-tmpfs-and-cpio.md) /
[ADR 0037](0037-vfs-and-initramfs-in-the-kernel.md) (the VFS),
[ADR 0036](0036-demand-paging-and-exec.md) (demand paging) and
[ADR 0017](0017-block-layer-in-the-kernel.md) /
[ADR 0018](0018-msi-x-and-interrupt-driven-disk-io.md) (the block layer).
Resolves the "not yet" points of ADR 0032 about a page cache, writeback
timing and unmounting at shutdown, and ADR 0036's refusal of shared file
mappings.

## Context

Phase 4 asks for "a persistent file system on virtio-blk (ext2 first, then
ext4), page cache and writeback". `poly-ext2` existed, host-tested but not
mounted; it reads and writes every block through `BlockIo`, holding a
spinlock across disk I/O. The kernel drove one virtio-blk device and left
the others alone. File data had one cache, the block layer's 64 KiB buffer
cache per disk, which knows blocks, not files: nothing a memory mapping
could share, and nothing that keeps a file's pages while a process maps
them. `user::Memory` gave every mapped file page a private copy and refused
`MAP_SHARED` of files.

Sources: the ext2 format documentation named in ADR 0032, the Linux manual
pages (`mmap(2)`, `msync(2)`, `sync(2)`, `fsync(2)`, `mount(8)`,
`e2fsck(8)`, `mke2fs(8)`) for the behaviour, and the virtio and QEMU
documentation for the devices. No kernel source was read.

## Decision

**The cache logic is the host-tested crate `poly-pagecache`** (`no_std` +
`alloc`, depends on `poly-vfs` only). The kernel provides frames and locks.

- **Pages are keyed by file system, inode and page index** (`Key`); a page
  is a `PageFrame` (a trait: allocate a zeroed 4 KiB frame, a pointer to
  it; dropping frees it) plus a dirty flag and a count of writable
  mappings, shared (`Arc`) between the index and its current users. The
  index (a `BTreeMap`, LRU stamps, a dirty set) sits behind a spinlock held
  only for in-memory work.
- **The cache wraps a file system** instead of being called by the VFS:
  `PageCache::register(inner)` returns a `CachedFs`, itself a
  `poly_vfs::FileSystem`, which is what gets mounted. Regular files' `read`,
  `write`, `append` and size changes go through the cache; every other
  operation passes through. The VFS and `OpenFile` did not change, and
  tmpfs (which keeps its pages in memory anyway) is not cached.
- **Reads** fill a missed page and up to 15 uncached pages after it with one
  call into the inner file system (read-ahead, stopped by a cached page,
  the end of the file or a frame allocation failure).
- **Writes** copy into pages (read first unless the write covers the page
  or the page lies wholly past the old end), mark them dirty, and set the
  size and modification time in the inner file system at once, so `stat`
  is right. The data reaches the file system at **writeback**: blocks are
  allocated then ("delayed allocation"). A failed writeback (a full disk)
  drops the data, keeps the page clean, counts the error and reports it
  from the file system's next `sync`, as Linux reports writeback errors at
  `fsync`. Pages past the end of a file are zero: truncation drops the pages
  past the new end and zeroes the rest of the last one, and growing a file
  zeroes what a mapping may have written past the old end.
- **Deleted files lose their pages before their inode number can come
  back.** `unlink`, `rename` over a file and the last `release` of an
  unlinked file run under a per-file-system namespace lock; afterwards, if
  the inner file system no longer has the victim inode, its pages are
  discarded (under the inode's lock, so not during its writeback). Creation
  also runs under that lock, so no file can be created with a number whose
  stale pages are still cached; it discards any pages of the new number as
  a safety net.
- **Locks are a type parameter.** Each inode has a lock held across
  everything that touches its pages, including the inner file system's I/O
  (fill, write, truncation, writeback, discard), so a page is filled once
  and writeback never races a write or truncation of the same file. These
  and the namespace locks are `poly_vfs::lock::RawLock`s, a new
  lock-and-unlock trait in `poly-vfs` (with `RawSpinLock` and a `Lock<L, T>`
  wrapper): the host uses the spinlock, the kernel its sleeping mutex
  (`sync::RawMutex`, a `Mutex<()>` locked and unlocked by hand). Lock order:
  namespace, inode, the inner file system's, index.
- **ext2's volume lock is the same parameter**: `Ext2<D, L = RawSpinLock>`,
  `Ext2::mount` (unchanged, spinlock) and `Ext2::mount_with_lock(dev,
  clock, opts, lock)`. The kernel passes a `RawMutex`, so a thread waiting
  for the volume sleeps while the holder waits for the disk.
- **Eviction**: clean pages that only the index holds (not mapped, not
  being copied) go least recently used first when the cache passes its
  limit, when a frame cannot be allocated (32 pages, then one more try),
  and on `shrink`. Dirty and mapped pages stay; the limit is soft.

**Mappings share the cache's frames.** `user::PageSource` gained two
default methods: `cached_page(offset)` (the cache's page, as a `PageRef`
that keeps it alive) and `shareable()`. `fs::cache::FileSource` (a VFS file
as a page source, pinning its inode) implements them for files on a disk
mount; `MemoryImage` and tmpfs files keep the old behaviour. On a read
fault the memory maps the cache's frame itself (`map_frames`, not owned by
the address space) and keeps the `PageRef` in a table next to its parked
frames, read-only in a private region; a write to it is a copy on write
(the region map already asks for one when a private page's frame has
another user). A page that crosses the region's limit (an ELF segment's
last page) still gets a private copy with its zero tail. `MAP_SHARED` of
a shareable source is allowed: its pages are mapped read-only first and
writable on the first write, from when on the page counts a writable
mapping and is dirty; writeback writes it each time and keeps it dirty
while it stays mapped writable, and it is written once more after the
mapping goes. Every path that drops a mapped page (unmap, `PROT_NONE`,
copy on write, the memory going away) unmaps it before letting go of the
`PageRef`, so a frame is never freed while mapped.

**Dropping file references during reaping is deferred.** The scheduler
drops an exited thread's user state in whichever thread switches next,
possibly inside the tick's interrupt handler, possibly one holding a
volume lock. Releasing a file on a disk mount takes sleeping locks (and
the cache's index spinlock, which does not mask interrupts), so
`thread::reaping()` tells drop code it runs there, and a `user::Memory`
dropped then unmaps its page-cache pages at once (spinlocks only) and
hands its regions (with their file references) and those pages to the
writeback thread (`fs::cache::release_later`). Its own frames are still
freed at once, as the frame-counting tests expect.

**In the kernel** (`kernel/src/fs/cache.rs`, `disk.rs`):

- One cache for all disk file systems, made at boot with a limit of a
  quarter of RAM; `KernelFrame` is a frame from `FRAMES` through the direct
  map, counted (`fs::cache::frames()`).
- A writeback thread wakes every 5 seconds (or when kicked) and syncs each
  disk mount that changed or has dirty pages (pages to ext2, then ext2 and
  the disk synced): data is durable within seconds. It also evicts half of
  the clean pages when less than 1/16 of RAM is free.
- `block::open(name)` brings up `virtio<n>`, the `n`-th virtio-blk function
  in PCI order, on first use; `virtio0` still comes up at boot, and a
  device nobody names (the watchdog test's spare) is left alone. Names
  follow PCI order, so they are stable for a given QEMU command line.
- `fs::disk::mount(disk, path, ro)`: ext2 on the disk (through `DiskIo`,
  the disk's buffer cache as `BlockIo`), registered with the cache, mounted
  on `path` (made if missing). `poly.mount=<disk>:<path>[:ro],...` mounts
  at boot; the shell's `mount <disk> <path> [ro]`, `umount <path>`,
  `sync` and `write <file> <words>` do it at runtime. `unmount` refuses a
  busy mount (open files, mappings), then writes back, syncs and marks the
  volume clean. `power::reboot` calls `fs::disk::shutdown` first: every
  volume written back and marked clean, files open or not.
- The test image (`cargo xtask`): 8 MiB of ext2 made with `poly-ext2`'s
  `format` and files written through the driver on the host (no e2fsprogs
  needed, the same bytes every run), attached as the third virtio-blk
  device (`virtio2`) to every boot with test devices and mounted on `/mnt`.
  A boot test writes a file there through the shell, syncs and reboots,
  runs `e2fsck -fn` on the image when e2fsprogs are installed, boots again
  from the same image and reads the file back.

## Consequences

- File data is cached twice on its way to the disk: in the page cache and,
  for a moment, in the disk's 64 KiB buffer cache, which now mostly holds
  metadata. Writing back through ext2 costs a copy per page; I/O straight
  from the page cache's frames (ext2 handing out block maps) would remove
  it.
- `write` never fails for lack of space; the error comes from `sync` or
  `fsync` (which syncs the whole file system: there is no per-file
  `fsync`), as with delayed allocation without reservations. Reserving
  blocks at `write` needs a free-space query in `FileSystem`.
- Every `write` updates the inode (size, `mtime`) in the inner file system;
  writeback updates `mtime` again. `atime` changes only when a page is read
  from the disk.
- A page mapped writable through `MAP_SHARED` is rewritten at every
  writeback pass while mapped, since without reverse mappings the cache
  cannot see the page-table dirty bits. Reverse mappings (page → address
  spaces) would let writeback write-protect and clean such pages, and let
  truncation unmap pages past the new end (today a mapping keeps a page
  that truncation dropped from the cache; nothing writes it back, and it
  does not raise `SIGBUS` as Linux would).
- Unmounting through `fs::disk::unmount` is the only path that syncs and
  marks the volume clean; a future `umount(2)` must go through it, not
  straight to `Vfs::umount`.
- A process's open files are released when its last reference goes; if
  that happens while its last thread is reaped, files on a disk mount
  would take sleeping locks in the wrong context. Process exit
  (`exit_group`) should close the files in the exiting thread, or the
  process record defer its drop like `user::Memory` does.
- `MAP_SHARED` of files on tmpfs is still `ENODEV`: tmpfs would need to hand
  out its own pages.
- Per-inode locks live in a map that is pruned when the cache shrinks;
  inodes touched once and never again keep an entry until then.
- ext4 (extents, journal) is not part of this: the roadmap keeps it as a
  milestone of its own.
