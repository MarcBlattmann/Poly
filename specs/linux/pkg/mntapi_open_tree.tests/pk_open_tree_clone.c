/* open_tree(dfd, path, OPEN_TREE_CLONE) makes a detached bind mount of
 * what path names and returns a descriptor for its top. Path need not be a
 * mount point: a plain directory (or a file) is cloned the same way, and
 * the clone's top is that directory. The clone is in no namespace until
 * move_mount attaches it; its files are the original's files (changes show
 * on both sides), and mount options of the original carry over (a
 * read-only mount clones read-only). Attaching it again where it is
 * attached already is ELOOP; dropping the descriptor of an unattached clone discards it without
 * touching the original; unmounting the attached clone leaves the
 * original alone.
 * poly-diff: needs=root
 */
#include "p_mntapi.h"

#define BASE "/tmp/pk_open_tree_clone"

static void body(const char *base) {
    char p[128], q[160], c1[128], c2[128], c3[128];
    pm_path(p, base, "t");
    expect("mkdir_t", pm_mkdir(p), 0);
    expect("mount_t", pm_mount("t", p, "tmpfs", 0, "size=1m"), 0);
    expect("file_f", a_make_file(pm_path(q, p, "f"), "orig", 0644), 0);
    expect("dir_d", pm_mkdir(pm_path(q, p, "d")), 0);
    expect("file_g", a_make_file(pm_path(q, p, "d/g"), "gg", 0644), 0);

    /* clone of a mount point */
    i64 c = pm_open_tree(AT_FDCWD, p, OPEN_TREE_CLONE | OPEN_TREE_CLOEXEC);
    check("clone", c >= 3, c);
    expect("cloexec_set", syscall2(SYS_fcntl, c, F_GETFD), FD_CLOEXEC);
    expect("file_visible_through_fd", syscall4(SYS_newfstatat, c, "f", &(struct a_stat){0}, 0), 0);
    pm_path(c1, base, "c1");
    expect("mkdir_c1", pm_mkdir(c1), 0);
    expect("not_in_namespace_yet", pm_exists(pm_path(q, c1, "f")), -ENOENT);
    expect("attach", pm_attach(c, c1), 0);
    expect("file_visible", pm_exists(pm_path(q, c1, "f")), 0);
    expect("sub_dir_visible", pm_exists(pm_path(q, c1, "d/g")), 0);
    expect("clone_is_tmpfs", pm_statfs_blocks(c1), 256);
    expect("attach_again", pm_attach(c, c1), -ELOOP);

    /* same files */
    expect("create_in_clone", a_make_file(pm_path(q, c1, "new"), "n", 0644), 0);
    expect("visible_in_original", pm_exists(pm_path(q, p, "new")), 0);
    expect("create_in_original", a_make_file(pm_path(q, p, "new2"), "n", 0644), 0);
    expect("visible_in_clone", pm_exists(pm_path(q, c1, "new2")), 0);
    expect("unlink_in_clone", a_unlink(pm_path(q, c1, "new")), 0);
    expect("gone_in_original", pm_exists(pm_path(q, p, "new")), -ENOENT);
    a_close(c);

    /* the attached clone can be unmounted alone */
    expect("umount_clone", pm_umount2(c1, 0), 0);
    expect("clone_gone", pm_exists(pm_path(q, c1, "f")), -ENOENT);
    expect("original_stays", pm_exists(pm_path(q, p, "f")), 0);

    /* clone of a plain directory in a mount: its top is that directory */
    c = pm_open_tree(AT_FDCWD, pm_path(q, p, "d"), OPEN_TREE_CLONE | OPEN_TREE_CLOEXEC);
    check("clone_of_directory", c >= 3, c);
    struct a_stat st;
    expect("fstat_top", syscall2(SYS_fstat, c, &st), 0);
    expect("top_is_directory", st.st_mode & S_IFMT, S_IFDIR);
    expect("sees_g", syscall4(SYS_newfstatat, c, "g", &st, 0), 0);
    expect("does_not_see_f", syscall4(SYS_newfstatat, c, "f", &st, 0), -ENOENT);
    pm_path(c2, base, "c2");
    expect("mkdir_c2", pm_mkdir(c2), 0);
    expect("attach_dir_clone", pm_attach(c, c2), 0);
    expect("c2_has_g", pm_exists(pm_path(q, c2, "g")), 0);
    expect("c2_no_f", pm_exists(pm_path(q, c2, "f")), -ENOENT);
    expect("c2_same_fs", pm_statfs_blocks(c2), 256);
    a_close(c);
    expect("umount_c2", pm_umount2(c2, 0), 0);

    /* clone of a file: attach it over another file */
    pm_path(c3, base, "c3file");
    expect("target_file", a_make_file(c3, "target", 0644), 0);
    c = pm_open_tree(AT_FDCWD, pm_path(q, p, "f"), OPEN_TREE_CLONE);
    check("clone_of_file", c >= 3, c);
    expect("attach_file_clone", pm_attach(c, c3), 0);
    char buf[16];
    i64 fd = a_open(c3, O_RDONLY, 0);
    i64 n = fd < 0 ? fd : a_read(fd, buf, sizeof buf);
    check("file_shows_original_content", n == 4 && mem_eq(buf, "orig", 4), n);
    if (fd >= 0) a_close(fd);
    a_close(c);
    expect("umount_file_clone", pm_umount2(c3, 0), 0);

    /* unattached clones vanish, options carry over */
    c = pm_open_tree(AT_FDCWD, p, OPEN_TREE_CLONE);
    expect("drop_unattached", a_close(c), 0);
    expect("original_still_good", pm_exists(pm_path(q, p, "f")), 0);
    expect("make_ro", pm_mount(0, p, 0, MS_REMOUNT | MS_BIND | MS_RDONLY, 0), 0);
    c = pm_open_tree(AT_FDCWD, p, OPEN_TREE_CLONE);
    expect("attach_ro_clone", pm_attach(c, c1), 0);
    expect("clone_of_ro_is_ro", pm_can_create(c1), -EROFS);
    expect("ro_flag", pm_mntflags(c1) & ST_RDONLY, ST_RDONLY);
}

int test_main(void) { return pm_main(BASE, body); }
