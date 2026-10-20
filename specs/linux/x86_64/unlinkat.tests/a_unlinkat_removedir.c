/* unlinkat with AT_REMOVEDIR: only empty directories; "." is EINVAL, a
 * non-empty directory ENOTEMPTY, a file ENOTDIR. */
#include "a_sys.h"

#define D "/tmp/a_unlinkat_removedir"

int test_main(void) {
    struct a_stat st;
    expect("setup_mkdir", a_mkdir(D, 0700), 0);
    expect("setup_sub", a_mkdir(D "/sub", 0700), 0);
    expect("setup_inner", a_make_file(D "/sub/f", "", 0644), 0);
    expect("setup_empty", a_mkdir(D "/empty", 0700), 0);
    expect("nonempty_enotempty", a_rmdir(D "/sub"), -ENOTEMPTY);
    expect("file_enotdir", a_rmdir(D "/sub/f"), -ENOTDIR);
    expect("dot_einval", a_rmdir(D "/empty/."), -EINVAL);
    expect("dotdot_enotempty", a_rmdir(D "/empty/.."), -ENOTEMPTY);
    expect("missing_enoent", a_rmdir(D "/none"), -ENOENT);
    expect("rmdir_empty", a_rmdir(D "/empty"), 0);
    expect("empty_gone", a_stat_path(D "/empty", &st), -ENOENT);
    expect("rmdir_again_enoent", a_rmdir(D "/empty"), -ENOENT);
    expect("make_again", a_mkdir(D "/empty", 0700), 0);
    expect("trailing_slash_ok", a_rmdir(D "/empty/"), 0);
    expect("symlink_to_dir", syscall3(SYS_symlinkat, "sub", AT_FDCWD, D "/l"), 0);
    expect("removedir_symlink_enotdir", a_rmdir(D "/l"), -ENOTDIR);
    expect("unlink_symlink_to_dir", a_unlink(D "/l"), 0);
    expect("empty_sub_first", a_unlink(D "/sub/f"), 0);
    /* The open directory can be removed; its descriptor stays valid. */
    i64 fd = a_open(D "/sub", O_RDONLY | O_DIRECTORY, 0);
    expect("rmdir_open_dir", a_rmdir(D "/sub"), 0);
    expect("fstat_removed_dir", syscall2(SYS_fstat, fd, &st), 0);
    expect("removed_dir_nlink_0", st.st_nlink, 0);
    expect("create_in_removed_dir_enoent", syscall4(SYS_openat, fd, "x", O_RDWR | O_CREAT, 0600), -ENOENT);
    a_close(fd);
    expect("old_rmdir_syscall_file_enotdir", a_make_file(D "/g", "", 0644) == 0 ? syscall1(SYS_rmdir, D "/g") : -1,
           -ENOTDIR);
    expect("cleanup_g", a_unlink(D "/g"), 0);
    expect("old_rmdir_syscall", syscall1(SYS_rmdir, D), 0);
    return finish();
}
