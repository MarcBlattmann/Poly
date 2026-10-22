/* mkdir(path, mode) (syscall 83) creates an empty directory and returns
 * 0: a directory with the requested mode less the umask (0777 under umask
 * 022 gives 0755), link count 2, owned by the caller's effective user and
 * group, on the same device as its parent, whose ".." is the parent. The
 * parent's link count goes up by one, and back down after rmdir. */
#include "a_sys.h"

#define D "/tmp/c_mkdir_basic"

int test_main(void) {
    struct a_stat st, parent, dotdot;
    expect("setup_parent", a_mkdir(D, 0755), 0);
    expect("parent_stat_before", a_stat_path(D, &parent), 0);
    u64 nlink_before = parent.st_nlink;

    expect("mkdir_returns_0", syscall2(SYS_mkdir, D "/new", 0777), 0);
    expect("stat_new", a_lstat_path(D "/new", &st), 0);
    expect("is_directory", st.st_mode & S_IFMT, S_IFDIR);
    expect("mode_0777_less_umask_022", st.st_mode, S_IFDIR | 0755);
    expect("nlink_2", st.st_nlink, 2);
    expect("uid_is_euid", st.st_uid, syscall0(SYS_geteuid));
    expect("gid_is_egid", st.st_gid, syscall0(SYS_getegid));
    expect("same_device_as_parent", st.st_dev == parent.st_dev, 1);
    expect("new_inode", st.st_ino != parent.st_ino, 1);
    expect("stat_dotdot", a_stat_path(D "/new/..", &dotdot), 0);
    expect("dotdot_is_parent", dotdot.st_ino == parent.st_ino && dotdot.st_dev == parent.st_dev, 1);
    expect("parent_stat_after", a_stat_path(D, &parent), 0);
    expect("parent_nlink_plus_1", parent.st_nlink, nlink_before + 1);

    /* The new directory is usable at once. */
    expect("file_inside", a_make_file(D "/new/f", "x", 0644), 0);
    expect("subdir_inside", syscall2(SYS_mkdir, D "/new/sub", 0700), 0);
    expect("new_nlink_3_with_subdir", a_stat_path(D "/new", &st) == 0 ? (i64)st.st_nlink : -1, 3);
    expect("cleanup_sub", a_rmdir(D "/new/sub"), 0);
    expect("cleanup_f", a_unlink(D "/new/f"), 0);
    expect("rmdir_new", a_rmdir(D "/new"), 0);
    expect("parent_nlink_back", a_stat_path(D, &parent) == 0 ? (i64)parent.st_nlink : -1, (i64)nlink_before);
    expect("gone", a_lstat_path(D "/new", &st), -ENOENT);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
