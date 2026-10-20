/* ftruncate: a negative length is EINVAL; read-only, directory, pipe and
 * O_PATH descriptors fail; truncate by path fails on directories. */
#include "a_sys.h"

#define D "/tmp/a_ftruncate_errors"

int test_main(void) {
    struct a_stat st;
    i32 p[2];
    expect("setup_mkdir", a_mkdir(D, 0700), 0);
    expect("setup_file", a_make_file(D "/f", "abc", 0644), 0);
    i64 fd = a_open(D "/f", O_RDWR, 0);
    expect("negative_einval", syscall2(SYS_ftruncate, fd, -1), -EINVAL);
    syscall2(SYS_fstat, fd, &st);
    expect("size_unchanged", st.st_size, 3);
    a_close(fd);
    fd = a_open(D "/f", O_RDONLY, 0);
    expect("rdonly_einval", syscall2(SYS_ftruncate, fd, 0), -EINVAL);
    a_close(fd);
    fd = a_open(D, O_RDONLY | O_DIRECTORY, 0);
    expect("directory_einval", syscall2(SYS_ftruncate, fd, 0), -EINVAL);
    a_close(fd);
    syscall2(SYS_pipe2, p, 0);
    expect("pipe_einval", syscall2(SYS_ftruncate, p[1], 0), -EINVAL);
    a_close(p[0]);
    a_close(p[1]);
    fd = a_open(D "/f", O_PATH, 0);
    expect("path_fd_ebadf", syscall2(SYS_ftruncate, fd, 0), -EBADF);
    a_close(fd);
    expect("closed_ebadf", syscall2(SYS_ftruncate, 99, 0), -EBADF);
    expect("truncate_dir_eisdir", syscall2(SYS_truncate, D, 0), -EISDIR);
    expect("truncate_missing_enoent", syscall2(SYS_truncate, D "/none", 0), -ENOENT);
    expect("truncate_negative_einval", syscall2(SYS_truncate, D "/f", -5), -EINVAL);
    expect("truncate_bad_path_efault", syscall2(SYS_truncate, (const char *)8, 0), -EFAULT);
    /* With O_APPEND it still works. */
    fd = a_open(D "/f", O_WRONLY | O_APPEND, 0);
    expect("append_fd_ok", syscall2(SYS_ftruncate, fd, 1), 0);
    a_close(fd);
    a_stat_path(D "/f", &st);
    expect("append_fd_result", st.st_size, 1);
    expect("cleanup_f", a_unlink(D "/f"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
