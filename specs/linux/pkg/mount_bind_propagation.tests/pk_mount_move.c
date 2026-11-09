/* MS_MOVE relocates a mount (and the mounts below it) to another directory
 * without unmounting: the files stay reachable, an open file keeps working,
 * the old path is the plain directory it covered. The source must be a mount
 * point (EINVAL otherwise); the target must exist (ENOENT); a mount cannot
 * be moved below itself (ELOOP) nor, when its parent mount is shared, at all
 * (EINVAL) - the same move works once the parent is private. The target may
 * already be a mount point: the moved mount is stacked on top of it. A move
 * keeps the mount's flags (read-only stays read-only). A moved mount can be
 * unmounted at its new place.
 * poly-diff: needs=root (mount needs CAP_SYS_ADMIN)
 * poly-diff: syscalls=off (the mount runs in a child in a mount namespace of its own)
 */
#include "p_mnt.h"

#define B "/tmp/pk_mmv"

static int body(void) {
    expect("tmpfs", pm_tmpfs(B), 0);
    pm_mkdirs(B "/a");
    pm_mkdirs(B "/c");
    pm_mkdirs(B "/d");
    pm_mkdirs(B "/e");
    pm_mkdirs(B "/plain");
    pm_put(B, "/a/under_a", "hidden-under-a", 0644);
    expect("mount_a", pm_mount("pka", B "/a", "tmpfs", 0, 0), 0);
    pm_put(B, "/a/moved_file", "mf", 0644);
    pm_mkdirs(B "/a/inner");
    expect("mount_inner", pm_mount("pkin", B "/a/inner", "tmpfs", 0, 0), 0);
    pm_put(B, "/a/inner/in", "in", 0644);
    i64 fd = a_open(B "/a/moved_file", O_RDONLY, 0);
    check("open_before", fd >= 0, fd);

    expect("move", pm_mount(B "/a", B "/c", 0, MS_MOVE, 0), 0);
    pm_expect_file("new_place", B "/c/moved_file", "mf");
    pm_expect_file("submount_came_along", B "/c/inner/in", "in");
    expect("old_path_plain", pm_exists(B "/a/moved_file"), 0);
    pm_expect_list("old_path_listing", B "/a", "under_a");
    char buf[4];
    memset(buf, 0, sizeof buf);
    expect("open_file_still_works", syscall4(SYS_pread64, fd, buf, 2, 0), 2);
    check("open_file_content", buf[0] == 'm' && buf[1] == 'f', buf[0]);
    a_close(fd);
    struct a_statfs fs;
    expect("statfs_new_place", syscall2(SYS_statfs, B "/c", &fs), 0);
    expect("tmpfs_new_place", fs.f_type, TMPFS_MAGIC);

    pm_expect_file("old_path_own_content", B "/a/under_a", "hidden-under-a");

    /* errors */
    expect("move_plain_dir", pm_mount(B "/plain", B "/d", 0, MS_MOVE, 0), -EINVAL);
    expect("move_missing_source", pm_mount(B "/nothing", B "/d", 0, MS_MOVE, 0), -ENOENT);
    expect("move_missing_target", pm_mount(B "/c", B "/nothing", 0, MS_MOVE, 0), -ENOENT);
    expect("move_into_itself", pm_mount(B "/c", B "/c/inner", 0, MS_MOVE, 0), -ELOOP);
    pm_expect_file("still_at_c", B "/c/moved_file", "mf");

    /* onto a mount point: stacked on top */
    expect("mount_e", pm_mount("pke", B "/e", "tmpfs", 0, 0), 0);
    pm_put(B, "/e/e_file", "e", 0644);
    expect("move_onto_mount", pm_mount(B "/c", B "/e", 0, MS_MOVE, 0), 0);
    pm_expect_file("stacked_top", B "/e/moved_file", "mf");
    expect("stacked_hides_e", pm_exists(B "/e/e_file"), 0);
    expect("unmount_inner_first", pm_umount(B "/e/inner", 0), 0);
    expect("unmount_moved", pm_umount(B "/e", 0), 0);
    pm_expect_file("e_below_back", B "/e/e_file", "e");
    expect("unmount_e", pm_umount(B "/e", 0), 0);

    /* flags are kept */
    expect("mount_ro", pm_mount("pkro", B "/a", "tmpfs", MS_RDONLY, 0), 0);
    expect("move_ro", pm_mount(B "/a", B "/d", 0, MS_MOVE, 0), 0);
    expect("moved_still_ro", a_make_file(B "/d/x", "x", 0644), -EROFS);
    expect("unmount_ro", pm_umount(B "/d", 0), 0);

    /* parent shared: no move */
    expect("mount_p", pm_mount("pkp", B "/a", "tmpfs", 0, 0), 0);
    expect("make_parent_shared", pm_mount(0, B, 0, MS_SHARED, 0), 0);
    expect("move_under_shared_parent", pm_mount(B "/a", B "/d", 0, MS_MOVE, 0), -EINVAL);
    expect("make_parent_private", pm_mount(0, B, 0, MS_PRIVATE, 0), 0);
    expect("move_under_private_parent", pm_mount(B "/a", B "/d", 0, MS_MOVE, 0), 0);
    expect("unmount_d", pm_umount(B "/d", 0), 0);
    return 0;
}

int test_main(void) {
    a_block_sigchld();
    a_rmdir(B);
    pm_in_ns(body);
    expect("cleanup", a_rmdir(B), 0);
    return finish();
}
