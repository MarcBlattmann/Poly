/* move_mount(from_dfd, from_path, to_dfd, to_path, flags) attaches a
 * detached mount (from fsmount or open_tree(OPEN_TREE_CLONE)) at a
 * directory, or moves an attached one. With MOVE_MOUNT_F_EMPTY_PATH (4) the
 * source is the mount that from_dfd refers to; with MOVE_MOUNT_T_EMPTY_PATH
 * (0x40) the target is the directory to_dfd (a descriptor of a directory
 * opened with O_DIRECTORY or O_PATH). Otherwise the paths are relative to
 * their descriptors. The target keeps its old contents underneath: a second
 * mount at the same place hides the first. A directory mount needs a
 * directory target and a mount of a file (open_tree clone of a file) a
 * file target: the other combinations are EINVAL. A plain open_tree
 * descriptor (no clone) of an attached mount, or the fsmount descriptor of a
 * mount that is attached already, moves that mount to the new place; its
 * old place shows what was underneath. Moving a mount onto the place where it
 * is, or into itself, is ELOOP.
 * poly-diff: needs=root
 */
#include "p_mntapi.h"

#define BASE "/tmp/pk_move_mount_attach"

static void body(const char *base) {
    char a[128], b[128], c[128], d[128], file[128], q[160];
    pm_path(a, base, "a");
    pm_path(b, base, "b");
    pm_path(c, base, "c");
    pm_path(d, base, "d");
    pm_path(file, base, "file");
    expect("mkdir_a", pm_mkdir(a), 0);
    expect("mkdir_b", pm_mkdir(b), 0);
    expect("mkdir_c", pm_mkdir(c), 0);
    expect("mkdir_d", pm_mkdir(d), 0);
    expect("make_file", a_make_file(file, "plain file", 0644), 0);

    /* target by descriptor */
    i64 m = pm_tmpfs_fd("1m", 0, FSMOUNT_CLOEXEC);
    i64 afd = a_open(a, O_RDONLY | O_DIRECTORY, 0);
    check("open_a", afd >= 0, afd);
    expect("attach_to_dirfd", pm_move_mount(m, "", afd, "", MOVE_MOUNT_F_EMPTY_PATH | MOVE_MOUNT_T_EMPTY_PATH), 0);
    expect("a_is_new_fs", pm_statfs_blocks(a), 256);
    expect("empty_target_flag_missing_is_enoent", pm_move_mount(m, "", afd, "", MOVE_MOUNT_F_EMPTY_PATH), -ENOENT);

    /* the same mount, moved with its descriptor */
    expect("create_in_a", pm_touch(pm_path(q, a, "marker")), 0);
    expect("move_attached_via_fd", pm_attach(m, b), 0);
    expect("b_is_new_fs", pm_statfs_blocks(b), 256);
    expect("marker_moved_along", pm_exists(pm_path(q, b, "marker")), 0);
    expect("a_shows_underlying_again", pm_exists(pm_path(q, a, "marker")), -ENOENT);
    expect("a_is_the_base_fs", pm_statfs_blocks(a), 1024);
    expect("move_onto_itself", pm_attach(m, b), -ELOOP);

    /* a plain open_tree descriptor moves the attached mount too */
    i64 ot = pm_open_tree(AT_FDCWD, b, OPEN_TREE_CLOEXEC);
    check("open_tree_plain", ot >= 0, ot);
    expect("move_via_plain_descriptor", pm_attach(ot, c), 0);
    expect("c_has_marker", pm_exists(pm_path(q, c, "marker")), 0);
    expect("b_empty_again", pm_exists(pm_path(q, b, "marker")), -ENOENT);

    /* relative source and target */
    i64 basefd = a_open(base, O_RDONLY | O_DIRECTORY, 0);
    expect("move_by_relative_names", pm_move_mount(basefd, "c", basefd, "d", 0), 0);
    expect("d_has_marker", pm_exists(pm_path(q, d, "marker")), 0);
    expect("c_empty_again", pm_exists(pm_path(q, c, "marker")), -ENOENT);

    /* stacking at one place: the later mount hides the earlier */
    i64 m2 = pm_tmpfs_fd("2m", 0, FSMOUNT_CLOEXEC);
    expect("attach_over_mount", pm_attach(m2, d), 0);
    expect("hides_earlier", pm_exists(pm_path(q, d, "marker")), -ENOENT);
    expect("top_is_second", pm_statfs_blocks(d), 512);
    a_close(m2);
    expect("unmount_top", pm_umount2(d, 0), 0);
    expect("earlier_visible_again", pm_exists(pm_path(q, d, "marker")), 0);

    /* directory mounts and file mounts do not mix */
    i64 m3 = pm_tmpfs_fd("1m", 0, FSMOUNT_CLOEXEC);
    expect("directory_mount_on_file", pm_attach(m3, file), -EINVAL);
    i64 fc = pm_open_tree(AT_FDCWD, file, OPEN_TREE_CLONE | OPEN_TREE_CLOEXEC);
    check("clone_of_file", fc >= 0, fc);
    expect("file_mount_on_directory", pm_attach(fc, c), -EINVAL);
    char target[128];
    pm_path(target, base, "file2");
    expect("make_file2", a_make_file(target, "other", 0644), 0);
    expect("file_mount_on_file", pm_attach(fc, target), 0);
    char buf[16];
    i64 f = a_open(target, O_RDONLY, 0);
    i64 n = f < 0 ? f : a_read(f, buf, sizeof buf);
    check("file_content_replaced", n == 10 && mem_eq(buf, "plain file", 10), n);
    if (f >= 0) a_close(f);

    /* a source descriptor that is a plain file */
    i64 rf = a_open(file, O_RDONLY, 0);
    expect("source_regular_file", pm_move_mount(rf, "", AT_FDCWD, c, MOVE_MOUNT_F_EMPTY_PATH), -EINVAL);
}

int test_main(void) { return pm_main(BASE, body); }
