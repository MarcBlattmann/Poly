/* open(2): the lowest free descriptor, the three access modes and mode 3,
 * and relative paths resolved against the working directory. */
#include "a_sys.h"

#define SYS_open 2
#define D "/tmp/a_open_basic"

static i64 op(const char *p, i64 flags, i64 mode) { return syscall3(SYS_open, p, flags, mode); }

int test_main(void) {
    char c;
    expect("setup_mkdir", a_mkdir(D, 0700), 0);
    expect("setup_file", a_make_file(D "/f", "abc", 0600), 0);

    i64 fd = op(D "/f", O_RDONLY, 0);
    expect("rdonly_fd_3", fd, 3);
    expect("rdonly_read", a_read(fd, &c, 1), 1);
    expect("rdonly_read_byte", c, 'a');
    expect("rdonly_write_ebadf", a_write(fd, "x", 1), -EBADF);

    i64 fd2 = op(D "/f", O_WRONLY, 0);
    expect("wronly_fd_4", fd2, 4);
    expect("wronly_read_ebadf", a_read(fd2, &c, 1), -EBADF);
    expect("wronly_write", a_write(fd2, "X", 1), 1);

    i64 fd3 = op(D "/f", O_RDWR, 0);
    expect("rdwr_fd_5", fd3, 5);
    expect("rdwr_read_sees_write", a_read(fd3, &c, 1) == 1 ? c : -1, 'X');
    expect("rdwr_write", a_write(fd3, "Y", 1), 1);

    i64 fd4 = op(D "/f", 3, 0);
    expect("accmode_3_opens_fd_6", fd4, 6);
    expect("accmode_3_read_ebadf", a_read(fd4, &c, 1), -EBADF);
    expect("accmode_3_write_ebadf", a_write(fd4, "Z", 1), -EBADF);

    /* Closing a low number makes it the next one handed out. */
    a_close(fd2);
    expect("gap_4_reused", op(D "/f", O_RDONLY, 0), 4);
    a_close(fd);
    expect("gap_3_reused", op(D "/f", O_RDONLY, 0), 3);
    expect("next_after_gaps_7", op(D "/f", O_RDONLY, 0), 7);
    for (int i = 3; i <= 7; i++) a_close(i);

    /* A relative path starts at the working directory. */
    expect("chdir", syscall1(SYS_chdir, D), 0);
    fd = op("f", O_RDONLY, 0);
    expect("relative_open_fd_3", fd, 3);
    expect("relative_read", a_read(fd, &c, 1) == 1 ? c : -1, 'X');
    a_close(fd);
    expect("dot_slash_relative", (fd = op("./f", O_RDONLY, 0)), 3);
    a_close(fd);
    expect("dotdot_relative", (fd = op("../a_open_basic/f", O_RDONLY, 0)), 3);
    a_close(fd);
    expect("relative_missing_enoent", op("nope", O_RDONLY, 0), -ENOENT);
    expect("chdir_back", syscall1(SYS_chdir, "/"), 0);
    expect("relative_after_chdir_back_enoent", op("f", O_RDONLY, 0), -ENOENT);

    expect("cleanup_unlink", a_unlink(D "/f"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
