/* rmdir of a directory that is still open: it succeeds, the name is gone,
 * and the descriptor stays valid. fstat on it shows a directory with
 * link count 0; getdents64 on it fails with ENOENT (no entries, not even
 * "." and ".."); nothing can be created in it through the descriptor
 * (ENOENT for a file, a directory or a symbolic link), and a lookup
 * relative to it of "." still works while ".." is the old parent. A
 * second descriptor opened earlier sees the same. Closing works. */
#include "a_sys.h"

#define D "/tmp/c_rmdir_open_fd"
#define SYS_getdents64 217

static i64 rm(const char *p) { return syscall1(SYS_rmdir, p); }
static char dbuf[4096];

int test_main(void) {
    struct a_stat st, parent, dd;
    expect("setup", a_mkdir(D, 0755), 0);
    expect("setup_x", a_mkdir(D "/x", 0755), 0);
    expect("stat_parent", a_stat_path(D, &parent), 0);
    i64 fd = a_open(D "/x", O_RDONLY | O_DIRECTORY, 0);
    expect("open_x", fd >= 0, 1);
    i64 fd2 = a_open(D "/x", O_PATH, 0);
    expect("open_x_path", fd2 >= 0, 1);
    struct a_stat before;
    expect("fstat_before", syscall2(SYS_fstat, fd, &before), 0);
    expect("nlink_2_before", before.st_nlink, 2);

    expect("rmdir_open", rm(D "/x"), 0);
    expect("name_gone", a_lstat_path(D "/x", &st), -ENOENT);
    expect("fstat_after", syscall2(SYS_fstat, fd, &st), 0);
    expect("still_dir", st.st_mode & S_IFMT, S_IFDIR);
    expect("same_inode", st.st_ino == before.st_ino, 1);
    expect("nlink_0", st.st_nlink, 0);
    expect("fstat_path_fd", syscall2(SYS_fstat, fd2, &st), 0);
    expect("path_fd_nlink_0", st.st_nlink, 0);
    expect("getdents_enoent", syscall3(SYS_getdents64, fd, dbuf, sizeof dbuf), -ENOENT);

    expect("create_file_enoent", syscall4(SYS_openat, fd, "f", O_WRONLY | O_CREAT, 0644), -ENOENT);
    expect("mkdir_enoent", syscall3(SYS_mkdirat, fd, "d", 0755), -ENOENT);
    expect("symlink_enoent", syscall3(SYS_symlinkat, "t", fd, "l"), -ENOENT);
    expect("lookup_missing_enoent", syscall4(SYS_newfstatat, fd, "f", &st, 0), -ENOENT);
    expect("lookup_dot", syscall4(SYS_newfstatat, fd, ".", &st, 0), 0);
    expect("lookup_dot_nlink_0", st.st_nlink, 0);
    expect("lookup_dotdot", syscall4(SYS_newfstatat, fd, "..", &dd, 0), 0);
    expect("dotdot_is_old_parent", dd.st_ino == parent.st_ino, 1);
    expect("path_fd_mkdir_enoent", syscall3(SYS_mkdirat, fd2, "d", 0755), -ENOENT);
    expect("rmdir_dot_einval", syscall3(SYS_unlinkat, fd, ".", AT_REMOVEDIR), -EINVAL);

    /* The parent lost one link; the name can be made again as a new directory. */
    expect("stat_parent_after", a_stat_path(D, &st), 0);
    expect("parent_nlink_minus_1", st.st_nlink, parent.st_nlink - 1);
    expect("remake_x", a_mkdir(D "/x", 0755), 0);
    expect("stat_new_x", a_stat_path(D "/x", &st), 0);
    expect("new_x_nlink_2", st.st_nlink, 2);
    expect("old_fd_still_nlink_0", syscall2(SYS_fstat, fd, &st) == 0 ? (i64)st.st_nlink : -1, 0);
    expect("old_fd_create_still_enoent", syscall3(SYS_mkdirat, fd, "d", 0755), -ENOENT);
    expect("new_x_has_no_d", a_lstat_path(D "/x/d", &st), -ENOENT);

    expect("close_fd", a_close(fd), 0);
    expect("close_fd2", a_close(fd2), 0);
    expect("rm_new_x", rm(D "/x"), 0);
    expect("cleanup_rmdir", rm(D), 0);
    return finish();
}
