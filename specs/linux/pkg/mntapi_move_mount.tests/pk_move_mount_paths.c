/* move_mount between paths: moving a mounted tree from one directory to
 * another. The source path must be the top of a mount (otherwise EINVAL);
 * after the move the old place shows what was underneath, the files seen at
 * the new place are the same files, and mounts below the moved one move along.
 * Moving a mount into a directory below itself or onto itself is ELOOP (so
 * is moving the root of the namespace). A missing source or target is
 * ENOENT; an empty path without MOVE_MOUNT_F_EMPTY_PATH / T_EMPTY_PATH is
 * ENOENT; a target that is a file for a directory mount is EINVAL.
 * Unknown flag bits (0x08, 0x80, 0x1000, all ones) are EINVAL; invalid
 * descriptors are EBADF; NULL pointers EFAULT.
 * poly-diff: needs=root
 */
#include "p_mntapi.h"

#define BASE "/tmp/pk_move_mount_paths"

static void body(const char *base) {
    char t[128], u[128], x[128], sub[160], file[128], q[200], q2[200];
    pm_path(t, base, "t");
    pm_path(u, base, "u");
    pm_path(x, base, "x");
    pm_path(file, base, "file");
    expect("mkdir_t", pm_mkdir(t), 0);
    expect("mkdir_u", pm_mkdir(u), 0);
    expect("mkdir_x", pm_mkdir(x), 0);
    expect("make_file", pm_touch(file), 0);
    expect("mount_t", pm_mount("t", t, "tmpfs", 0, "size=1m"), 0);
    expect("marker", pm_touch(pm_path(q, t, "marker")), 0);
    expect("mkdir_sub", pm_mkdir(pm_path(sub, t, "sub")), 0);
    expect("mount_sub", pm_mount("s", sub, "tmpfs", 0, "size=2m"), 0);
    expect("sub_file", pm_touch(pm_path(q, sub, "deep")), 0);

    expect("move", pm_move_mount(AT_FDCWD, t, AT_FDCWD, u, 0), 0);
    expect("marker_at_new_place", pm_exists(pm_path(q, u, "marker")), 0);
    expect("old_place_empty", pm_exists(pm_path(q, t, "marker")), -ENOENT);
    expect("old_place_is_underlying_fs", pm_statfs_blocks(t), 1024);
    expect("submount_moved_along", pm_exists(pm_path(q, u, "sub/deep")), 0);
    expect("submount_is_mount", pm_statfs_blocks(pm_path(q, u, "sub")), 512);
    expect("same_files", pm_touch(pm_path(q, u, "later")), 0);
    expect("size_kept", pm_statfs_blocks(u), 256);

    /* the source must be a mount top */
    expect("source_not_mount", pm_move_mount(AT_FDCWD, t, AT_FDCWD, x, 0), -EINVAL);
    expect("source_plain_subdir", pm_move_mount(AT_FDCWD, pm_path(q, base, "x"), AT_FDCWD, t, 0), -EINVAL);
    /* into itself */
    expect("onto_itself", pm_move_mount(AT_FDCWD, u, AT_FDCWD, u, 0), -ELOOP);
    expect("into_own_directory", pm_mkdir(pm_path(q, u, "inner")), 0);
    expect("below_itself", pm_move_mount(AT_FDCWD, u, AT_FDCWD, pm_path(q2, u, "inner"), 0), -ELOOP);
    expect("below_itself_missing_dir", pm_move_mount(AT_FDCWD, u, AT_FDCWD, pm_path(q2, u, "nowhere"), 0), -ENOENT);
    expect("move_base_into_itself", pm_move_mount(AT_FDCWD, base, AT_FDCWD, x, 0), -ELOOP);
    /* names */
    expect("source_missing", pm_move_mount(AT_FDCWD, pm_path(q, base, "none"), AT_FDCWD, x, 0), -ENOENT);
    expect("target_missing", pm_move_mount(AT_FDCWD, u, AT_FDCWD, pm_path(q, base, "none"), 0), -ENOENT);
    expect("source_empty", pm_move_mount(AT_FDCWD, "", AT_FDCWD, x, 0), -ENOENT);
    expect("target_empty", pm_move_mount(AT_FDCWD, u, AT_FDCWD, "", 0), -ENOENT);
    expect("target_is_file", pm_move_mount(AT_FDCWD, u, AT_FDCWD, file, 0), -EINVAL);
    /* flags and pointers */
    expect("flag_0x08", pm_move_mount(AT_FDCWD, u, AT_FDCWD, x, 0x08), -EINVAL);
    expect("flag_0x80", pm_move_mount(AT_FDCWD, u, AT_FDCWD, x, 0x80), -EINVAL);
    expect("flag_0x1000", pm_move_mount(AT_FDCWD, u, AT_FDCWD, x, 0x1000), -EINVAL);
    expect("flags_all", pm_move_mount(AT_FDCWD, u, AT_FDCWD, x, ~0UL), -EINVAL);
    expect("null_target", pm_move_mount(AT_FDCWD, u, AT_FDCWD, 0, 0), -EFAULT);
    expect("bad_source_pointer", pm_move_mount(AT_FDCWD, (const char *)1, AT_FDCWD, x, 0), -EFAULT);
    expect("bad_from_dfd", pm_move_mount(996, "u", AT_FDCWD, x, 0), -EBADF);
    expect("bad_to_dfd", pm_move_mount(AT_FDCWD, u, 996, "x", 0), -EBADF);
    expect("empty_source_bad_fd", pm_move_mount(996, "", AT_FDCWD, x, MOVE_MOUNT_F_EMPTY_PATH), -EBADF);
    expect("minus_one_source", pm_move_mount(-1, "", AT_FDCWD, x, MOVE_MOUNT_F_EMPTY_PATH), -EBADF);

    /* still where it was after all the errors */
    expect("still_at_u", pm_exists(pm_path(q, u, "marker")), 0);
    /* an absolute source path ignores from_dfd, the automount flags are accepted */
    expect("absolute_ignores_dfd_and_automount_flags",
           pm_move_mount(996, u, 996, x, MOVE_MOUNT_F_AUTOMOUNTS | MOVE_MOUNT_T_AUTOMOUNTS), 0);
    expect("moved_to_x", pm_exists(pm_path(q, x, "marker")), 0);
}

int test_main(void) { return pm_main(BASE, body); }
