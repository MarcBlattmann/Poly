/* getdents on descriptors that cannot be read: EBADF for a closed fd, -1,
 * a large number, and an O_PATH descriptor of a directory. EBADF comes
 * before any check of the buffer or its size (size 0, NULL buffer). */
#include "a_sys.h"

#define SYS_getdents 78

#define D "/tmp/a_getdents_badfd"

static char buf[4096] __attribute__((aligned(8)));

int test_main(void) {
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    i64 fd = a_open(D, O_RDONLY | O_DIRECTORY, 0);
    check("setup_open", fd >= 0, fd);
    expect("setup_close", a_close(fd), 0);

    expect("closed_fd", syscall3(SYS_getdents, fd, buf, sizeof buf), -EBADF);
    expect("closed_fd_size_0", syscall3(SYS_getdents, fd, buf, 0), -EBADF);
    expect("closed_fd_null_buffer", syscall3(SYS_getdents, fd, 0, sizeof buf), -EBADF);
    expect("fd_minus_1", syscall3(SYS_getdents, -1, buf, sizeof buf), -EBADF);
    expect("fd_1000000", syscall3(SYS_getdents, 1000000, buf, sizeof buf), -EBADF);
    expect("fd_0x7fffffff", syscall3(SYS_getdents, 0x7fffffff, buf, sizeof buf), -EBADF);

    i64 op = a_open(D, O_PATH | O_DIRECTORY, 0);
    check("setup_o_path", op >= 0, op);
    expect("o_path_fd", syscall3(SYS_getdents, op, buf, sizeof buf), -EBADF);
    expect("o_path_fd_size_0", syscall3(SYS_getdents, op, buf, 0), -EBADF);
    expect("o_path_fd_null_buffer", syscall3(SYS_getdents, op, 0, sizeof buf), -EBADF);
    a_close(op);

    /* A good descriptor still works afterwards. */
    fd = a_open(D, O_RDONLY | O_DIRECTORY, 0);
    expect("good_fd", syscall3(SYS_getdents, fd, buf, sizeof buf), 48);
    a_close(fd);

    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
