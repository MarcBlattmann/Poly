# 0168: The kernel interfaces Linux package formats need: squashfs, loop devices, overlayfs, FUSE, namespaces

**Status:** Accepted (implementation in progress on branch `impl-linux-pkg`).
It carries out the kernel part of [ADR 0132](0132-installing-linux-apps-of-every-package-format.md)
(decision 4), from the 35 spec sheets of the `linux/api/pkg/*` entries
(`specs/linux/pkg/`).

## Context

`dpkg`, `rpm`, AppImage runtimes, `flatpak` and `bubblewrap` run on Poly
unchanged. What they need from the kernel is a set of interfaces Poly's
Linux personality did not have: a read-only compressed file system, loop
devices, union mounts, user-space file systems, and the finishing touches of
the namespace, mount and seccomp interfaces a sandbox uses.

## Decisions

1. **Libraries first, kernel thin.** The logic lives in host-tested crates:
   - `poly-compress`: decoders for zlib (through `poly-png`'s inflate), LZ4
     blocks, LZO1X, xz (LZMA2) and zstd, written from the formats' public
     descriptions, bounded output, safe Rust. xz and zstd were also checked
     against 88 streams made by other encoders at several levels (up to
     1 MB, with incompressible data, chunking and four-stream Huffman
     literals).
   - `poly-squashfs`: the squashfs 4 reader as a `poly_vfs::FileSystem` over
     a byte `Source`: basic and extended inodes, fragments, sparse blocks,
     hard links, device nodes, xattrs, all five compressors, block sizes 4 KiB
     to 1 MiB. Legacy lzma (id 2) is refused as the reference refuses it;
     zstd is accepted (the reference build lacked it; the sheet says Poly
     should mount them).
   - `poly-loop`: loop-control and the loop devices (binding, status,
     capacity, read-only, autoclear, the device's own page cache), over
     `Env` (the caller's memory and descriptors) and `Registry` (VFS
     registration) traits so it is tested on the host.
2. **Block devices open like seekable character devices.** The VFS gets a
   second device registry (`register_block_device`): a block node whose
   number is registered opens through it (`CharDevice::size` for
   `SEEK_END`, `CharDevice::sync` for `fsync`). Disks stay mounted through
   the block layer as before.
3. **A loop device is "in use"** while a descriptor is open on it or a file
   system is mounted from it; `LOOP_CLR_FD` then only sets
   `LO_FLAGS_AUTOCLEAR` and the last close or unmount unbinds, as the
   `loop_attach` sheet records. A squashfs mount holds a `MountClaim`; the
   same device mounted again is the same `Arc<dyn FileSystem>` (same
   `st_dev`, same inode numbers) and never twice on one directory (`EBUSY`).
4. **Nodes.** Adding a loop device makes `/dev/loop<n>` (what devtmpfs does
   on Linux) and the kernel makes `/dev/loop-control`.

See the hand-off file of the track for what passes and what is open.
