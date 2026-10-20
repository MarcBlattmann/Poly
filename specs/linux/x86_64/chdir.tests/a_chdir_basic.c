/* chdir: relative paths resolve from the new directory; errors for
 * files, missing paths and directories without search permission (root
 * may search any directory). */
#include "a_sys.h"

#define D "/tmp/a_chdir_basic"

int test_main(void) {
    struct a_stat st;
    char buf[128];
    expect("setup_mkdir", a_mkdir(D, 0700), 0);
    expect("setup_sub", a_mkdir(D "/sub", 0700), 0);
    expect("setup_file", a_make_file(D "/sub/f", "hi", 0644), 0);
    expect("chdir", syscall1(SYS_chdir, D), 0);
    expect("relative_stat", a_stat_path("sub/f", &st), 0);
    expect("relative_size", st.st_size, 2);
    expect("relative_chdir", syscall1(SYS_chdir, "sub"), 0);
    i64 fd = a_open("f", O_RDONLY, 0);
    expect("relative_open", a_read(fd, buf, sizeof buf), 2);
    a_close(fd);
    expect("relative_create", a_make_file("new", "", 0644), 0);
    expect("created_in_cwd", a_stat_path(D "/sub/new", &st), 0);
    expect("chdir_file_enotdir", syscall1(SYS_chdir, "f"), -ENOTDIR);
    expect("chdir_missing_enoent", syscall1(SYS_chdir, "nope"), -ENOENT);
    expect("chdir_empty_enoent", syscall1(SYS_chdir, ""), -ENOENT);
    expect("chdir_bad_path_efault", syscall1(SYS_chdir, (const char *)8), -EFAULT);
    expect("chdir_through_file_enotdir", syscall1(SYS_chdir, "f/x"), -ENOTDIR);
    syscall2(SYS_getcwd, buf, sizeof buf);
    check("failed_chdirs_kept_cwd", str_eq(buf, D "/sub"), buf[0]);
    syscall3(SYS_fchmodat, AT_FDCWD, D "/sub", 0);
    expect("chdir_mode_0_as_root", syscall1(SYS_chdir, D "/sub"), 0);
    syscall3(SYS_fchmodat, AT_FDCWD, D "/sub", 0700);
    expect("chdir_dot", syscall1(SYS_chdir, "."), 0);
    expect("setup_link", syscall3(SYS_symlinkat, "sub", AT_FDCWD, D "/l"), 0);
    expect("chdir_link", syscall1(SYS_chdir, D "/l"), 0);
    expect("link_lands_in_sub", a_stat_path("f", &st), 0);
    expect("chdir_root", syscall1(SYS_chdir, "/"), 0);
    expect("cleanup_l", a_unlink(D "/l"), 0);
    expect("cleanup_new", a_unlink(D "/sub/new"), 0);
    expect("cleanup_f", a_unlink(D "/sub/f"), 0);
    expect("cleanup_sub", a_rmdir(D "/sub"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
