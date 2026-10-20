# 0052: ext4 in poly-ext2: extents, checksums, and a jbd2 journal with replay and ordered-mode transactions

**Status:** Accepted. Extends [ADR 0032](0032-ext2-driver.md) (the ext2
driver) and [ADR 0040](0040-page-cache-and-ext2-on-virtio-blk.md) (the page
cache, mounting on virtio-blk).

## Context

Phase 4's "ext4 on the persistent disk (extents, journal)": mount volumes as
`mke2fs -t ext4` makes them by default, which on the build host (e2fsprogs
1.47) means `has_journal ext_attr resize_inode dir_index filetype extent
64bit flex_bg sparse_super large_file huge_file dir_nlink extra_isize
metadata_csum`, and keep them consistent across crashes the way ext4 does:
with the journal.

ADR 0032's driver read and wrote ext2 with ordered metadata writes and
refused ext3 journals and every ext4 `INCOMPAT` feature. Its structure
(one locked `Ctx` per operation, the bitmaps, directories, inodes and block
maps as modules) holds for ext4 too: ext4 is ext2 with wider fields, other
ways to map blocks, checksums, more flexible placement of group metadata,
and a journal.

Sources: the kernel.org document "ext4 Data Structures and Algorithms"
(superblock, group descriptors, inode, extent tree, directory entries and
their tails, hash tree blocks, inline data, extended attributes, checksums,
the jbd2 journal: superblock, block header, descriptor tags, revoke blocks,
commit block, checksum v3, the replay passes), RFC 3720 for CRC-32C, and the
e2fsprogs manual pages (`mke2fs(8)`, `e2fsck(8)`, `debugfs(8)`,
`ext4(5)`). No Linux or e2fsprogs source was read. Where the document is
loose (exactly which bytes a checksum covers), the host's `mke2fs`,
`e2fsck` and `debugfs` decided, as test oracles: images they make must
verify, everything we write must pass `e2fsck -fn`, journals `debugfs`
writes (`jo -c`, `jw`, `jc`) must replay to the same bytes as `e2fsck`'s
replay.

## Decision

**One crate, `poly-ext2`, for ext2, ext3 and ext4**, not a `poly-ext4`
beside it: everything above the block mapping is shared, and splitting
would mean two copies of directories, allocation and the operations, or a
third crate both depend on. The crate name stays (the kernel and xtask use
it); `Ext2::fs_type()` (and the VFS `FsInfo` name) says `ext2`, `ext3` or
`ext4`. New modules: `crc` (CRC-32C, CRC-16), `csum` (what each checksum
covers), `extent` (extent trees), `journal` (jbd2), `inline` (inline data).

**Features.**

| Feature | Support |
|---|---|
| `64bit` | 48-bit block numbers, 64-byte descriptors, high halves of counts and checksums |
| `extent` | read, write, split, merge, grow, shrink, any depth (5 at most), uninitialised extents read as zeros and split when written |
| `flex_bg` | group metadata anywhere: "fixed metadata" is a sorted list of runs built at mount, not "the group's own blocks" |
| `sparse_super2` | the two backup groups from `s_backup_bgs` |
| `metadata_csum`, `metadata_csum_seed` | CRC-32C verified on reading (superblock and descriptors at mount; inodes in use, bitmaps, extent blocks, directory leaf and index blocks when read: `EBADMSG`) and set on writing; extended attribute blocks set when their reference count changes |
| `uninit_bg` (`gdt_csum`) | CRC-16 descriptors; with either checksum the `BLOCK_UNINIT`/`INODE_UNINIT` flags (bitmaps worked out, written and the flag cleared on first change) and `bg_itable_unused` (records past it read as unused, shrunk when an inode past it is handed out) |
| `huge_file` | 48-bit `i_blocks`, in file system blocks with `HUGE_FILE_FL` |
| `dir_nlink` | a directory past 64999 links counts 1 |
| `extra_isize` | nanosecond times, creation time in new inodes (as in ADR 0032), checksum high half |
| `dir_index` | indexed directories read through their leaves; the hash functions are not specified in the public document, so the index is dropped when we change such a directory (flag cleared, index blocks rewritten as empty leaves with checksum tails); `e2fsck -D` rebuilds |
| `inline_data` | files, directories and symbolic links in the inode read; the volume mounts read-only |
| `has_journal` | replay at mount (no checksums, v1 unverified, v3; not v2, not fast commits), and our own transactions (below) |
| orphan list | processed at mount (delete unlinked, finish truncations), used for files unlinked while open and truncations longer than one transaction |

Refused (`IncompatFeatures` or `Unsupported`): `meta_bg`, `mmp`,
`ea_inode`, encryption, casefolding, `largedir`, `bigalloc`, external
journals, block sizes above 4 KiB. Unknown `RO_COMPAT` features (quota,
project, verity, `orphan_present`, ...) mount read-only.

**Extents.** The root is decoded from `i_block`, other nodes from their
blocks, checked (magic, counts, depth, sorted non-overlapping entries,
every extent clear of fixed metadata). Mapping a hole extends the
neighbouring extent if the new block is adjacent, else inserts an entry; a
full node splits (a leaf filled by appending keeps its entries and the new
one starts the next leaf, otherwise halves), the new node's key goes into
the parent, and a full root moves its entries into a new block one level
down. Index keys always equal their child's first key. Blocks a split needs
are allocated before anything changes. Truncation cuts from the end, frees
emptied nodes and collapses the tree while the root's only child fits in
the root. New nodes are written as "fresh" before the nodes that point to
them (ADR 0032's rule 2) when there is no journal.

**Freeing.** Freed blocks and inodes are cleared in their bitmaps at once
(the bitmap change belongs to the same transaction as the pointer
removal) but *held*: the allocator skips them until the next barrier or
commit. ADR 0032's pending lists changed into this. (It also fixed a gap
in ADR 0032's ordering: renaming over a file in the same directory block
dropped the old inode without a barrier after the name moved.)

**The journal: transactions of whole operations, checkpointed at once.**

- Metadata writes (`write_meta`, `write_fresh`, inode records, bitmaps,
  the superblock and changed descriptor blocks at commit) go into the
  *running transaction*: block number -> contents, which metadata reads see.
  File data is written in place.
- A commit (at `sync`, so the kernel's writeback thread commits every 5
  seconds; before an operation when the transaction passed half its limit
  or freed space it may need is held; at unmount) writes revoke blocks,
  descriptor blocks with their tags and the logged blocks (escaped if they
  begin with the magic), flushes (file data written so far is durable now:
  "ordered" mode), writes the commit block, flushes, then writes every
  block home. The transaction limit is a quarter of the log.
- The tail moves lazily: the journal superblock points at the first
  transaction since the log was last empty and is rewritten only when that
  changes: before the first commit after a clean log (made durable by the
  commit's first flush), and when the next transaction does not fit before
  the end of the log, after a flush makes all checkpoints durable (then
  the log restarts at its beginning, and nothing older is replayed). So a
  commit costs two flushes, not four.
- Replay therefore may copy old transactions again: harmless for metadata
  that only changes through the journal, wrong for a block that became
  file data since. Freeing a block that the log holds (since the tail)
  writes a revoke record in the freeing transaction, and replay skips a
  block revoked at or after the transaction that logged it.
- Operations are atomic: nothing commits in the middle of one, except
  truncations and writes that outgrow the transaction (a huge fragmented
  file): truncation then cuts 8192 logical blocks per step from the end and
  commits between steps, with the inode on the orphan list (and its size
  already the new one) so a crash is finished at mount; a long write
  commits the part written so far with the size set to it.
- Mounting: a probe mount reads the superblock, descriptors and the
  journal inode, replays if `needs_recovery` is set or the log is not
  empty (refusing with `EROFS` on a read-only device), marks the log empty,
  clears `needs_recovery`; then the real mount reads the replayed volume,
  sets `needs_recovery`, sets the journal's features (revoke, 64-bit tags
  on `64bit` volumes, checksum v3 with `metadata_csum`), and processes the
  orphan list. Unmounting commits, flushes, marks the log empty, clears
  `needs_recovery` and marks the volume clean.

**Formatting.** `format` with `FormatOptions::ext4(block_size, journal)`
makes ext4 with `64bit`, `extent`, `metadata_csum`, `huge_file`,
`dir_nlink`, `extra_isize` and a checksum v3 journal (an extent-mapped
inode 8, its map backed up in the superblock), without `flex_bg`,
`resize_inode` or `dir_index`; `e2fsck -fn` passes on what it makes.

**In the kernel** nothing changes but the names: `fs::disk::mount` mounts
any of the three (the log line says which, and how many transactions were
replayed and orphans processed), `/proc/filesystems` lists ext3 and ext4.
`cargo xtask` builds a second test disk, 16 MiB of ext4 with a 4 MiB
journal made by `format` and filled through the driver on the host (the
same bytes every run), attached as `virtio3` and mounted on `/ext4`.

## Consequences

- Host tests (`crates/poly-ext2/tests/ext4.rs`, `journal.rs`, and the
  model and corruption tests): `mke2fs -t ext4 -d` images in eight
  configurations (defaults, 1 KiB, no journal, `uninit_bg`, 32-bit 2 KiB,
  128-byte inodes, `metadata_csum_seed` with `sparse_super2`, no
  `flex_bg`), with `e2fsck -fyD` indexes, read back exactly; a workload on
  each and on `format`'s volumes passes `e2fsck -fn` and reads back with
  `debugfs`; extent trees of depth 2 split, merge and shrink back into the
  inode, `e2fsck`-clean at every step; damaged checksums are refused;
  debugfs-written journals (checksum v3 and none, 32 and 64-bit tags,
  several descriptors, a revoke) replay to the same bytes as e2fsck's
  replay; a workload with renames over files, fragmented files, long
  truncations and files deleted while open, cut at random points, gives
  volumes that `e2fsck -fn` finds clean after either replay, with no
  renamed-over name lost; randomized model runs against tmpfs on journaled
  and unjournaled ext4. Boot tests: the ext4 disk's files through the page
  cache, a remount, a file persisting across a reboot (`e2fsck -fn` on the
  image in between), and a boot that syncs and ends without unmounting,
  after which the next mount replays the journal and the file is there.
- Indexed directories lose their index when we change them: lookups in
  very large directories stay linear until `e2fsck -D` runs. Writing hash
  trees needs the hash functions (`half_md4`, `tea`), which the public
  document names but does not define.
- Each metadata block of a transaction is written twice (log and home) and
  kept in memory until the commit; a commit costs two device flushes. The
  writeback interval bounds how much a crash loses (5 seconds of changes),
  as with Linux's default `commit=5`.
- Delayed allocation in the page cache (ADR 0040) means a file's data
  reaches ext4 at writeback; a crash between a `rename` over a file
  (committed) and the writeback of the new file's data leaves the name on a
  file without its data (Linux has the `auto_da_alloc` heuristic for this;
  we do not).
- Not yet: `fsync` of one file (it syncs the whole volume: one commit),
  fast commits, checksum v2 journals, `flex_bg`-aware allocation (new
  blocks go near the inode's group, as on ext2), preallocation
  (`fallocate`), extended attributes beyond releasing blocks, quotas,
  `orphan_file`, `largedir`, block sizes above 4 KiB.
