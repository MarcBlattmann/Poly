/* faccessat2: relative paths, O_PATH descriptors with AT_EMPTY_PATH, and
 * missing or wrong-type components. */
#include "a_sys.h"

#define D "/tmp/a_faccessat2_dirfd"

int test_main(void) {
    expect("setup_mkdir", a_mkdir(D, 0700), 0);
    expect("setup_sub", a_mkdir(D "/sub", 0700), 0);
    expect("setup_file", a_make_file(D "/sub/f", "", 0700), 0);
    i64 dfd = a_open(D, O_RDONLY | O_DIRECTORY, 0);
    expect("relative", syscall4(SYS_faccessat2, dfd, "sub/f", X_OK, 0), 0);
    expect("relative_dotdot", syscall4(SYS_faccessat2, dfd, "sub/../sub/f", F_OK, 0), 0);
    expect("relative_missing", syscall4(SYS_faccessat2, dfd, "sub/g", F_OK, 0), -ENOENT);
    expect("relative_through_file", syscall4(SYS_faccessat2, dfd, "sub/f/x", F_OK, 0), -ENOTDIR);
    expect("dirfd_empty_path", syscall4(SYS_faccessat2, dfd, "", W_OK, AT_EMPTY_PATH), 0);
    i64 p = a_open(D "/sub/f", O_PATH, 0);
    expect("path_fd_empty_path", syscall4(SYS_faccessat2, p, "", X_OK, AT_EMPTY_PATH), 0);
    expect("path_fd_as_dirfd_enotdir", syscall4(SYS_faccessat2, p, "x", F_OK, 0), -ENOTDIR);
    a_close(p);
    expect("bad_dirfd", syscall4(SYS_faccessat2, 99, "sub", F_OK, 0), -EBADF);
    expect("absolute_ignores_dirfd", syscall4(SYS_faccessat2, 99, D "/sub", F_OK, 0), 0);
    a_close(dfd);
    expect("cleanup_f", a_unlink(D "/sub/f"), 0);
    expect("cleanup_sub", a_rmdir(D "/sub"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
