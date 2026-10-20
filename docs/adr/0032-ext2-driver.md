# 0032: ext2 in a host-tested crate over a block access trait, metadata ordered by flushes instead of a journal

**Status:** Accepted

## Context

Phase 4 asks for a persistent file system on virtio-blk, "ext2 first, then
ext4". The VFS ([ADR 0023](0023-vfs-tmpfs-and-cpio.md)) defines the
`FileSystem` trait a disk file system implements; the block layer
([ADR 0014](0014-block-layer.md), [ADR 0017](0017-block-layer-in-the-kernel.md))
gives the kernel a `block::Disk` per disk: a request queue and a write-back
buffer cache, whole-block reads and writes, and `sync`.

ext2 has no journal. Linux's ext2 makes no promise about the order in which
metadata reaches the disk and relies on `e2fsck` after a crash. e2fsck
repairs any state, but some states lose data a user considers safe: two
files sharing a block (a freed block reused while the old pointer is still
on disk), an indirect block read as garbage pointers (the pointer reached
the disk before the block's contents), and a name that points at nothing
(`rename` over a file whose new inode was not on disk yet).

The format comes from public documentation: Dave Poirier's "The Second
Extended File System" and, for the fields ext4 added to ext2's structures
(nanosecond timestamps in large inodes, the 32-bit device number encoding),
the ext4 wiki's disk layout pages. No kernel or e2fsprogs source was read;
the host's `mke2fs`, `debugfs` and `e2fsck` binaries make and check test
images.

## Decision

**`crates/poly-ext2`**, `no_std` with `alloc` (like `poly-vfs`), depends on
`poly-vfs` and `poly-block` only. `Ext2<D: BlockIo>` mounts a volume and
implements `FileSystem`; `format` makes a new one.

**`BlockIo` is the disk**: synchronous whole-device-block `read`, `write`
and `flush` (durable and ordered), `&self`. The kernel implements it for its
`block::Disk` in a few lines (`read`, `write_block` per block, `sync`); the
crate brings `CachedIo` (a `poly-block` `BufferCache` in front of any
`IoQueue`) and `DeviceIo` (a `BlockDevice` used synchronously). File system
blocks of 1, 2 and 4 KiB map onto any device block size, with
read-modify-write when a device block is the larger.

**What is supported.** Revisions 0 and 1; `filetype` is the only
`INCOMPAT` feature accepted (others refuse the mount); `sparse_super` and
`large_file` are the `RO_COMPAT` features written (others mount read-only);
`COMPAT` features are ignored as allowed: hash-indexed directories are read
linearly and lose their index flag when changed, extended attribute blocks
are released by reference count when their inode goes, reserved descriptor
blocks are never allocated. `large_file` is set when a file reaches 2 GiB.
Inodes keep every field the driver does not know (it patches fields into
the record read from disk).

**One lock per volume, no caches between operations.** Every operation
reads the inodes and blocks it needs through `BlockIo` (the kernel's buffer
cache makes that cheap), works on copies and writes them back, so each
`FileSystem` call is atomic. Kept in memory: the superblock and group
descriptors (written at sync), the bitmap block in use, pins, and the
pending frees below. An operation caches the indirect blocks it walks.

**Allocation is group-local**: directories go to the group with the fewest
directories among those with above-average free inodes and blocks, other
inodes to their directory's group (else by quadratic probing), blocks to
the first free one after the file's previous block. Root-reserved blocks
are not enforced (the file system never sees credentials).

**Ordering without a journal** (mount option `ordered`, on by default):

1. *No reuse before the old pointer is gone*: freed blocks and inodes wait
   on a pending list and become allocatable after the next barrier.
2. *No pointer to an uninitialised block*: new indirect, directory and
   symbolic link blocks are written as "fresh"; writing anything that may
   point to one (an inode, an existing indirect or directory block) first
   takes a barrier.
3. *No moment without a name*: `rename` takes a barrier before the new
   name is written (so the renamed file, its blocks and its data are
   durable: replacing a file by renaming a new one over it is safe), and
   another before the old name is removed.

A barrier is `BlockIo::flush` (for the kernel: `Disk::sync`) followed by
releasing pending frees. Link counts go up before a name is added and down
after one is removed, so a crash leaves them too high, never too low. New
inodes get no barrier before their entry (a crash leaves an entry pointing
at a free inode, which e2fsck removes), and file data is not ordered (a
crash may show stale data in a block being allocated), as with Linux's
ext2. The volume is marked not clean while mounted read-write and clean at
`unmount`; detected corruption marks it as having errors.

**Corruption is an error, not a panic**: every block number is checked
against the volume and the fixed metadata of its group before use, every
inode number against the inode count, every directory record against its
block; loops over on-disk structures are bounded. Corrupt structures answer
`EUCLEAN`, device failures `EIO`.

## Consequences

- Tests run on the host in seconds: images made by `mke2fs` (1/2/4 KiB
  blocks, revision 0, with and without `filetype`/`sparse_super`, small
  and large inodes, hash-indexed directories) read back byte for byte;
  everything the driver writes passes `e2fsck -fn` and reads back with
  `debugfs rdump`; a randomised model test against tmpfs; random byte
  changes in metadata never panic; and crash images (everything before a
  flush plus any subset of the writes after it) are repaired by e2fsck
  without shared blocks or garbage pointers, with renamed-over names
  intact. Without e2fsprogs the tests use `format` and skip the e2fsck
  checks.
- Ordered mode costs a flush per `mkdir`, per directory growth, per
  operation that allocates indirect blocks, and two per `rename`. With the
  kernel's cache a flush writes every dirty block of the disk; if that
  shows, the next step is a dirty-block epoch in the buffer cache, not a
  change here.
- The volume lock is a spinlock held across disk I/O: correct with a
  preemptive scheduler, wasteful under contention (a spinner burns its time
  slice while the holder sleeps in the block layer). When the kernel has a
  sleeping mutex usable from crates, the lock becomes a parameter.
- Not yet: a page cache above the file system (reads and writes go through
  the block cache), writeback timing (the kernel should call `sync`
  periodically and `unmount` at shutdown), an orphan list (an unlinked file
  still open at a crash is found by e2fsck), `statfs` in the VFS (the crate
  has `Ext2::stats`), extended attributes and ACLs, ext3 journals, ext4.
