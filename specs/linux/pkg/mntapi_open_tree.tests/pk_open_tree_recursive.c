/* OPEN_TREE_CLONE copies only the mount at path; mounts below it are not in
 * the clone (the directory where they were attached shows the parent
 * filesystem's own, here empty, directory). OPEN_TREE_CLONE | AT_RECURSIVE
 * copies the whole tree of mounts below it. Mounts marked unbindable are
 * left out of a recursive clone, while cloning an unbindable mount itself
 * is EINVAL. A recursive clone attached with move_mount brings all its
 * mounts along; unmounting its top with MNT_DETACH removes them all, and
 * plain umount2 of the top is EBUSY while mounts are below it.
 * AT_RECURSIVE without OPEN_TREE_CLONE is EINVAL.
 * poly-diff: needs=root
 */
#include "p_mntapi.h"

#define BASE "/tmp/pk_open_tree_recursive"

static void body(const char *base) {
    char t[128], sub[160], deep[160], un[160], q[200], r1[128], r2[128];
    pm_path(t, base, "t");
    expect("mkdir_t", pm_mkdir(t), 0);
    expect("mount_t", pm_mount("t", t, "tmpfs", 0, "size=1m"), 0);
    expect("mkdir_sub", pm_mkdir(pm_path(sub, t, "sub")), 0);
    expect("mount_sub", pm_mount("s", sub, "tmpfs", 0, "size=2m"), 0);
    expect("file_in_sub", pm_touch(pm_path(q, sub, "h")), 0);
    expect("mkdir_deep", pm_mkdir(pm_path(deep, sub, "deep")), 0);
    expect("mount_deep", pm_mount("d", deep, "tmpfs", 0, "size=3m"), 0);
    expect("file_in_deep", pm_touch(pm_path(q, deep, "k")), 0);
    expect("file_in_t", pm_touch(pm_path(q, t, "top")), 0);

    pm_path(r1, base, "r1");
    pm_path(r2, base, "r2");
    expect("mkdir_r1", pm_mkdir(r1), 0);
    expect("mkdir_r2", pm_mkdir(r2), 0);

    /* not recursive */
    i64 c = pm_open_tree(AT_FDCWD, t, OPEN_TREE_CLONE | OPEN_TREE_CLOEXEC);
    check("clone", c >= 3, c);
    expect("attach_plain", pm_attach(c, r1), 0);
    a_close(c);
    expect("top_file", pm_exists(pm_path(q, r1, "top")), 0);
    expect("sub_dir_exists_empty", pm_exists(pm_path(q, r1, "sub")), 0);
    expect("sub_content_missing", pm_exists(pm_path(q, r1, "sub/h")), -ENOENT);
    expect("sub_is_not_a_mount", pm_statfs_blocks(pm_path(q, r1, "sub")), 256);

    /* recursive */
    c = pm_open_tree(AT_FDCWD, t, OPEN_TREE_CLONE | AT_RECURSIVE | OPEN_TREE_CLOEXEC);
    check("recursive_clone", c >= 3, c);
    expect("attach_recursive", pm_attach(c, r2), 0);
    a_close(c);
    expect("r2_top_file", pm_exists(pm_path(q, r2, "top")), 0);
    expect("r2_sub_content", pm_exists(pm_path(q, r2, "sub/h")), 0);
    expect("r2_sub_is_mount", pm_statfs_blocks(pm_path(q, r2, "sub")), 512);
    expect("r2_deep_content", pm_exists(pm_path(q, r2, "sub/deep/k")), 0);
    expect("r2_deep_is_mount", pm_statfs_blocks(pm_path(q, r2, "sub/deep")), 768);
    expect("shares_files", a_make_file(pm_path(q, r2, "sub/newfile"), "n", 0644), 0);
    expect("shared_with_original", pm_exists(pm_path(q, sub, "newfile")), 0);
    expect("umount_top_busy", pm_umount2(r2, 0), -EBUSY);
    expect("umount_sub_alone", pm_umount2(pm_path(q, r2, "sub/deep"), 0), 0);
    expect("deep_unmounted_here", pm_exists(pm_path(q, r2, "sub/deep/k")), -ENOENT);
    expect("deep_still_in_original", pm_exists(pm_path(q, deep, "k")), 0);
    expect("detach_tree", pm_umount2(r2, MNT_DETACH), 0);
    expect("r2_gone", pm_exists(pm_path(q, r2, "top")), -ENOENT);
    expect("original_untouched", pm_exists(pm_path(q, deep, "k")), 0);

    /* unbindable */
    pm_path(un, t, "un");
    expect("mkdir_un", pm_mkdir(un), 0);
    expect("mount_un", pm_mount("u", un, "tmpfs", 0, "size=1m"), 0);
    expect("make_unbindable", pm_mount(0, un, 0, MS_UNBINDABLE, 0), 0);
    expect("clone_unbindable", pm_open_tree(AT_FDCWD, un, OPEN_TREE_CLONE), -EINVAL);
    expect("plain_open_of_unbindable", pm_open_tree(AT_FDCWD, un, 0) >= 3 ? 0 : -1, 0);
    c = pm_open_tree(AT_FDCWD, t, OPEN_TREE_CLONE | AT_RECURSIVE | OPEN_TREE_CLOEXEC);
    check("recursive_clone_with_unbindable_child", c >= 3, c);
    expect("attach_again_elsewhere", pm_attach(c, r2), 0);
    a_close(c);
    expect("sub_present", pm_exists(pm_path(q, r2, "sub/h")), 0);
    expect("unbindable_left_out", pm_statfs_blocks(pm_path(q, r2, "un")), 256);
    expect("detach_all", pm_umount2(r2, MNT_DETACH), 0);

    /* AT_RECURSIVE alone */
    expect("recursive_without_clone", pm_open_tree(AT_FDCWD, t, AT_RECURSIVE), -EINVAL);
}

int test_main(void) { return pm_main(BASE, body); }
