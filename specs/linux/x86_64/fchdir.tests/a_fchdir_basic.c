/* fchdir: to the directory an open descriptor refers to (O_RDONLY or
 * O_PATH); files and bad descriptors fail. */
#include "a_sys.h"

#define D "/tmp/a_fchdir_basic"

int test_main(void) {
    char buf[128];
    struct a_stat st;
    expect("setup_mkdir", a_mkdir(D, 0700), 0);
    expect("setup_sub", a_mkdir(D "/sub", 0700), 0);
    expect("setup_file", a_make_file(D "/sub/f", "", 0644), 0);
    i64 d = a_open(D "/sub", O_RDONLY | O_DIRECTORY, 0);
    expect("fchdir", syscall1(SYS_fchdir, d), 0);
    syscall2(SYS_getcwd, buf, sizeof buf);
    check("cwd_is_sub", str_eq(buf, D "/sub"), buf[0]);
    expect("relative_after_fchdir", a_stat_path("f", &st), 0);
    a_close(d);
    i64 p = a_open(D, O_PATH | O_DIRECTORY, 0);
    expect("fchdir_path_fd", syscall1(SYS_fchdir, p), 0);
    syscall2(SYS_getcwd, buf, sizeof buf);
    check("cwd_is_base", str_eq(buf, D), buf[0]);
    a_close(p);
    i64 f = a_open(D "/sub/f", O_RDONLY, 0);
    expect("fchdir_file_enotdir", syscall1(SYS_fchdir, f), -ENOTDIR);
    a_close(f);
    expect("fchdir_closed_ebadf", syscall1(SYS_fchdir, f), -EBADF);
    expect("fchdir_negative_ebadf", syscall1(SYS_fchdir, -1), -EBADF);
    i32 pp[2];
    syscall2(SYS_pipe2, pp, 0);
    expect("fchdir_pipe_enotdir", syscall1(SYS_fchdir, pp[0]), -ENOTDIR);
    a_close(pp[0]);
    a_close(pp[1]);
    syscall2(SYS_getcwd, buf, sizeof buf);
    check("failures_kept_cwd", str_eq(buf, D), buf[0]);
    expect("chdir_root", syscall1(SYS_chdir, "/"), 0);
    expect("cleanup_f", a_unlink(D "/sub/f"), 0);
    expect("cleanup_sub", a_rmdir(D "/sub"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
