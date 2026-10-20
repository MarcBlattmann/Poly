/* mkdirat: relative to a directory descriptor; descriptors that are not
 * directories; a long chain of nested directories. */
#include "a_sys.h"

#define D "/tmp/a_mkdirat_dirfd"

static char path[600];

int test_main(void) {
    struct a_stat st;
    expect("mkdir_base", a_mkdir(D, 0700), 0);
    i64 dfd = a_open(D, O_RDONLY | O_DIRECTORY, 0);
    expect("mkdirat_relative", syscall3(SYS_mkdirat, dfd, "sub", 0700), 0);
    expect("made_under_dirfd", a_stat_path(D "/sub", &st), 0);
    expect("mkdirat_nested_relative", syscall3(SYS_mkdirat, dfd, "sub/inner", 0700), 0);
    expect("mkdirat_relative_exists", syscall3(SYS_mkdirat, dfd, "sub", 0700), -EEXIST);
    i64 pfd = a_open(D, O_PATH | O_DIRECTORY, 0);
    expect("mkdirat_path_dirfd", syscall3(SYS_mkdirat, pfd, "viapath", 0700), 0);
    a_close(pfd);
    expect("file", a_make_file(D "/f", "", 0644), 0);
    i64 ffd = a_open(D "/f", O_RDONLY, 0);
    expect("file_dirfd_enotdir", syscall3(SYS_mkdirat, ffd, "x", 0700), -ENOTDIR);
    a_close(ffd);
    expect("bad_dirfd_ebadf", syscall3(SYS_mkdirat, 99, "x", 0700), -EBADF);
    expect("absolute_ignores_bad_dirfd", syscall3(SYS_mkdirat, 99, D "/abs", 0700), 0);

    /* Twenty levels deep, each made relative to the one before. */
    i64 cur = syscall1(SYS_dup, dfd);
    int ok = 1;
    for (int i = 0; i < 20; i++) {
        ok &= syscall3(SYS_mkdirat, cur, "n", 0700) == 0;
        i64 next = syscall4(SYS_openat, cur, "n", O_RDONLY | O_DIRECTORY, 0);
        a_close(cur);
        cur = next;
    }
    check("twenty_levels", ok && cur >= 0, cur);
    a_close(cur);
    char *p = str_cpy(path, D);
    for (int i = 0; i < 20; i++) p = str_cpy(p, "/n");
    expect("deepest_exists", a_stat_path(path, &st), 0);
    for (int i = 20; i > 0; i--) {
        ok &= a_rmdir(path) == 0;
        path[str_len(path) - 2] = 0;
    }
    check("remove_twenty_levels", ok, 0);

    a_close(dfd);
    expect("cleanup_abs", a_rmdir(D "/abs"), 0);
    expect("cleanup_f", a_unlink(D "/f"), 0);
    expect("cleanup_viapath", a_rmdir(D "/viapath"), 0);
    expect("cleanup_inner", a_rmdir(D "/sub/inner"), 0);
    expect("cleanup_sub", a_rmdir(D "/sub"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
