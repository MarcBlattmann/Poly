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
5. **State per open file is the file system's.** `poly_vfs::FileSystem` has
   optional methods for a file system that keeps something for every
   `open`: `open_handle`, `create_open` (create and open in one step),
   `close_handle`, `flush_handle`, and `read_handle`, `write_handle`,
   `append_handle`, `truncate_handle`, `readdir_handle`, `sync_handle`. An
   `OpenFile` carries the handle and passes it back; the defaults keep
   nothing and work by inode, so no other file system changes.
   `open_truncates` says how `O_TRUNC` is done (by the VFS, always by the
   VFS, or by the open itself). FUSE is the user: `OPEN`, `OPENDIR` and
   `CREATE` give the server's file handle, which goes back in `READ`,
   `WRITE`, `READDIR`, `FSYNC`, `SETATTR` (ftruncate), `FLUSH` (every
   close of a descriptor, from the descriptor table) and `RELEASE` (when
   the open file is gone). `RELEASE` and the `FLUSH` at a process's end
   wait for nobody (`Conn::post`); a queued one is still read after the
   file system went, before `ENODEV`. A server that answers `OPEN`,
   `OPENDIR`, `FLUSH`, `FSYNC` or `CREATE` with `ENOSYS` is not asked again.
6. **A path walk needs no fresh attributes.** `FileSystem::walk_attr`
   (default: `getattr`) is what the walk asks for a directory it goes
   through. FUSE answers from what it knows (a node keeps its type; the
   root is the directory the mount options describe) and leaves search
   permission to the server unless the mount has `default_permissions`.
   So a mount whose server has not answered `INIT` can be walked to and
   unmounted, and cached names cost no `GETATTR`.
7. **Messages on `/dev/fuse` are not cut.** read(2) and write(2) move data
   through a 64 KiB bounce buffer; a device whose reads and writes are whole
   messages says so (`CharDevice::message_size`, `LinuxKernel::io_piece`)
   and gets each in one piece.
8. **seccomp user notifications** (`SECCOMP_RET_USER_NOTIF`) are a queue in
   the personality (`seccomp::notify::Listener`: pending, sent, answered),
   tested on the host. The personality does not depend on the VFS, so the
   kernel gives the descriptor's object and the sleeping (`Notifier`,
   `LinuxKernel::seccomp_wait`). A signal withdraws the notification and
   the call starts again; with the descriptor gone the call is `ENOSYS`.
9. **Namespace files.** The ioctls of ioctl_nsfs(2) are in the personality
   (`sys::nsfs`); a namespace is given out only inside the caller's own
   user or pid namespace (`EPERM` otherwise). A bind mount of something
   that is not its file system's root holds that inode (`Mount::pins_root`),
   which is how a bind-mounted namespace file keeps its namespace alive.
   Copies of such a mount (propagation, a copied mount namespace) do not
   hold it yet.
10. **A mapped loop device shows its backing file** (`Loop::mapped`): the
    mapping shares the file's pages, and from then on the device keeps no
    pages of its own, so reads, writes and mappings agree.

See the hand-off file of the track for what passes and what is open.
