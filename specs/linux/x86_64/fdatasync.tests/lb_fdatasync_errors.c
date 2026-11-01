/* fdatasync(2) errors: EBADF for a descriptor that is not open or is
 * O_PATH; EINVAL for objects that cannot be synchronized: pipes, sockets,
 * eventfds, /dev/null. */
#include "a_sys.h"

#define SYS_socket 41
#define SYS_socketpair 53
#define SYS_eventfd2 290

static i64 fdatasync_(i64 fd) { return syscall1(SYS_fdatasync, fd); }

int test_main(void) {
    expect("minus1", fdatasync_(-1), -EBADF);
    expect("atfdcwd", fdatasync_(AT_FDCWD), -EBADF);
    expect("unopened", fdatasync_(777), -EBADF);
    i64 fd = a_open("/tmp", O_RDONLY | O_DIRECTORY, 0);
    expect("close", a_close(fd), 0);
    expect("closed", fdatasync_(fd), -EBADF);
    i64 op = a_open("/tmp", O_PATH, 0);
    expect("opath", fdatasync_(op), -EBADF);
    a_close(op);

    i32 pp[2];
    expect("pipe2", syscall2(SYS_pipe2, pp, 0), 0);
    expect("pipe_read_end", fdatasync_(pp[0]), -EINVAL);
    expect("pipe_write_end", fdatasync_(pp[1]), -EINVAL);
    i64 s = syscall3(SYS_socket, 1, 1, 0);
    expect("socket", fdatasync_(s), -EINVAL);
    i32 sp[2];
    expect("socketpair", syscall4(SYS_socketpair, 1, 2, 0, sp), 0);
    expect("socketpair_end", fdatasync_(sp[0]), -EINVAL);
    i64 e = syscall2(SYS_eventfd2, 0, 0);
    expect("eventfd", fdatasync_(e), -EINVAL);
    i64 n = a_open("/dev/null", O_RDWR, 0);
    expect("devnull", fdatasync_(n), -EINVAL);
    /* the standard descriptors of a test are files (ok) or not; a fresh
     * pipe duplicated onto fd 0 is EINVAL */
    expect("dup2_pipe_to_0", syscall2(SYS_dup2, pp[0], 0), 0);
    expect("fd0_pipe", fdatasync_(0), -EINVAL);
    return finish();
}
