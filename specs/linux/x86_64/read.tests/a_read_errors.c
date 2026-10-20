/* read: errors for write-only, directory, O_PATH and closed descriptors
 * and for a buffer that is not mapped. */
#include "a_sys.h"

#define D "/tmp/a_read_errors"

int test_main(void) {
    char buf[8];
    expect("setup_mkdir", a_mkdir(D, 0700), 0);
    expect("setup_file", a_make_file(D "/f", "abc", 0644), 0);

    i64 fd = a_open(D "/f", O_WRONLY, 0);
    expect("wronly_ebadf", a_read(fd, buf, 1), -EBADF);
    a_close(fd);
    fd = a_open(D, O_RDONLY | O_DIRECTORY, 0);
    expect("directory_eisdir", a_read(fd, buf, sizeof buf), -EISDIR);
    a_close(fd);
    fd = a_open(D "/f", O_PATH, 0);
    expect("path_fd_ebadf", a_read(fd, buf, 1), -EBADF);
    a_close(fd);
    expect("closed_fd_ebadf", a_read(50, buf, 1), -EBADF);
    expect("negative_fd_ebadf", a_read(-1, buf, 1), -EBADF);

    fd = a_open(D "/f", O_RDONLY, 0);
    expect("null_buffer_efault", a_read(fd, (void *)0, 1), -EFAULT);
    expect("bad_buffer_efault", a_read(fd, (void *)16, 3), -EFAULT);
    expect("offset_unchanged_after_efault", syscall3(SYS_lseek, fd, 0, SEEK_CUR), 0);
    /* With nothing to read, a bad buffer is never touched. */
    expect("zero_count_bad_buffer_ok", a_read(fd, (void *)16, 0), 0);
    syscall3(SYS_lseek, fd, 0, SEEK_END);
    expect("at_eof_bad_buffer_ok", a_read(fd, (void *)16, 3), 0);
    a_close(fd);

    expect("cleanup_unlink", a_unlink(D "/f"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
