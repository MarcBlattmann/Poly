# 0177: The mount tree: copies of mounts, locks, superblock owners, and what a mapping of an overlay file maps

**Status:** Accepted (implementation on branch `impl-linux-pkg-2`). Second
round of the kernel part of
[ADR 0168](0168-kernel-interfaces-for-linux-package-formats.md), from the
spec sheets of `linux/api/pkg/` that were sharpened in PR #188
(`mount_bind_propagation`, `userns_mount`, `userns_idmap`, `mntapi_*`,
`pivot_root_flow`, `overlayfs_copyup`). The mount table itself is
[ADR 0122](0122-aio-io-uring-mqueue-mount-api-landlock-fanotify.md)'s and
[ADR 0054](0054-namespaces-and-seccomp.md)'s.

## Context

Flatpak, bubblewrap and container-style installers build a root out of
bind mounts inside a user namespace. Four things the mount table could not
say stood in their way:

- `MS_BIND | MS_REC` made the same single mount as a plain bind: the mounts
  below the source were not there in the copy.
- A mount namespace copied for a user namespace was as free as its
  original: its root could unmount what a mount hides and drop `nosuid` or
  `nodev` from a mount it had inherited.
- A file system did not know who made it, so the root of a user namespace
  could reconfigure the real root file system through `fspick`.
- A mount that holds an inode (a bind mount of a namespace file) lost that
  hold in every copy, and releasing it could happen under the mount table's
  lock.

## Decisions

1. **A copy of a mount is one function** (`MountTable::copy_of`): the same
   superblock and root, the per-mount flags, the extra attributes and the
   flag locks of the original. A bind mount, a recursive bind's copies, an
   `open_tree` clone, a copied namespace and a copy made by propagation all
   go through it, so none of them forgets a property. In particular **a
   bind mount has the flags of the mount it was made from**; the other
   flags of the mount(2) call are ignored (mount(2): "the bind mount has
   the same mount options as the underlying mount"). Before, a bind took
   the call's flags, which no caller relied on.

2. **A recursive bind copies the tree** (`Vfs::bind(.., recursive)`,
   `Vfs::copy_below`). Every mount on a directory at or below the source,
   at any depth, is copied onto the same directory of the copy of its
   parent; an unbindable mount and what is below it are left out. The
   originals are listed before the first copy is made, so a bind whose
   target lies inside its own source copies the tree once. The copies are
   mounts like any other (listed in `mountinfo`, unmounted from the bottom
   up, `EBUSY` for the top while they are there, all gone with
   `MNT_DETACH`). `open_tree(OPEN_TREE_CLONE | AT_RECURSIVE)` uses the same
   function with weak links to the detached top. A recursive bind onto a
   shared mount reaches the peers whole (`propagate_mount` copies what the
   new mount brought below it).

3. **Mounts can be locked** (`poly_vfs::mount::lock`, a bit set per mount).
   When a mount namespace is copied for a user namespace other than the
   one that owns the original, every mount of the copy gets:
   - `PLACE` (all but the first mount, which is on no mount point):
     `umount2`, with or without `MNT_DETACH`, and a move are `EINVAL`. The
     mount goes only with the mount it is on.
   - one lock per flag it has (`ro`, `nosuid`, `nodev`, `noexec`): a remount
     or `mount_setattr` that leaves the flag out is `EPERM`. A flag added
     later is not locked.
   - `ATIME`: naming another access time mode is `EPERM`.

   A copy keeps the flag locks. The top of a bind is not locked in place
   (who made it may unmount it); the copies below it in a recursive bind
   keep `PLACE`. A plain bind, or a plain clone, of a directory with a mount
   locked in place below it is `EINVAL`, because the copy would show what
   that mount covers. A mount made on top of a locked one is not locked.
   The first mount of the copy is not locked in place, which is what lets a
   sandbox `pivot_root` and then detach the old root with everything below
   it.

   "Another user namespace" is decided by the owners (`ns::make`), not by
   the flags of the call: a process that entered a user namespace earlier
   and unshares its mount namespace later gets a locked copy too.

4. **A superblock records the user namespace that made it**
   (`Mount::sb_owner`, the namespace's inode number; 0 is the initial one).
   The VFS only stores the number; the kernel sets it where mount(2) and
   `fsmount` make a file system and asks the personality
   (`ns::capable_in_user_ns`) before it changes a file system as a whole: a
   remount without `MS_BIND` and `FSCONFIG_CMD_RECONFIGURE` need
   `CAP_SYS_ADMIN` in that namespace (`EPERM`). `fspick` itself still
   succeeds on any mount. A namespace that is gone leaves the initial one
   to decide.

5. **What leaves the mount table is dropped after its lock is released.**
   `MountTable::remove_tree` returns the mounts it removed, and `umount`,
   `umount_expire`, `drop_namespace` and the propagation of an unmount let
   go of them outside the lock. A mount that pins its root releases it when
   it is dropped; for a bind-mounted mount namespace file that release ends
   a mount namespace, which takes the same lock. Copies of a pinning mount
   pin too (`Mount::pin_like`), also outside the lock (`Vfs::settle_pins`),
   because pinning asks the file system.

6. **`pivot_root(".", ".")` stacks the old root beneath the new one**
   (`Mount::is_beneath`). Poly's path walk follows the mounts stacked on a
   process's root (that is how the real root file system replaces the
   initial RAM disk), so an old root mounted on the new root's top
   directory would be *on top* and stay visible. A mount marked beneath is
   on that directory but not entered by lookups: the new tree is `/` at
   once. An unmount of the root directory of a namespace's first mount
   means the mount beneath it, so `umount2(".", MNT_DETACH)` removes the old
   root as pivot_root(2) describes. Moving the mount ends the mark.

7. **`move_mount` across namespaces and kinds.** A mount attached in a
   namespace other than the target's, and a mount that was attached once
   and is in no namespace any more, are `EINVAL`; a directory mount onto a
   file and the mount of a file onto a directory are `EINVAL` (both
   directions, also for `MS_MOVE`); the first mount of a namespace as
   source is `ELOOP` (everything is below it).

8. **The capabilities over files of a user namespace have a scope.** They
   now reach the VFS (`CAP_CHOWN`, `CAP_FOWNER`, `CAP_DAC_OVERRIDE`,
   `CAP_DAC_READ_SEARCH`, `CAP_FSETID`) together with the kernel ids the
   namespace maps (`poly_vfs::IdScope`, cached per namespace as
   `ns::user::IdRanges`), and `Cred::has_over` counts a capability only for
   a file whose owner and group are both in the scope. Before, a thread in
   a user namespace had no capability over any file. The personality still
   does not depend on the VFS: it hands the ranges over in `FsIds`.

9. **A shared mapping of an overlay file maps the layer's file.**
   `FileSystem::backing_file(ino, for_write)` names the file that holds a
   file's data; the overlay answers with the upper copy (made now for a
   writable shared mapping) or the lower file. The kernel's `FileSource`
   then shows that file's cached pages, so the mapping, the upper
   directory and the merged view agree at once, and no second cache sits in
   front of the overlay. A read-only mapping made before a later copy-up
   keeps showing the lower file, as on Linux.

## Consequences

- `Vfs::bind` takes `recursive` instead of flags; `Vfs::remount` takes the
  access time mode and applies the locks; `MountTable::remove_tree` returns
  what it removed. Host tests in `crates/poly-vfs/src/tests/mountapi.rs`
  cover the tree copy, the locks rule by rule, pinned copies, the stacked
  old root and `move_mount`'s refusals.
- `/proc/<pid>/mountinfo` shows the source of a copy from its superblock
  (`mounts::source_of`), so the copies of a recursive bind, of a namespace
  and of propagation show their original's source instead of the file
  system's name.
- Not done: id-mapped mounts (`MOUNT_ATTR_IDMAP` with a real user namespace
  is `EINVAL`), `MOVE_MOUNT_BENEATH`, `NOROOT`'s effect on a set-user-ID
  program's ambient set beyond capabilities(7)'s rule, and the access time
  lock does not cover `nodiratime`.
