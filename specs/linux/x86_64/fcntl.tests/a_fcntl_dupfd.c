/* fcntl: F_DUPFD and F_DUPFD_CLOEXEC take the lowest free number at or
 * above the argument; the argument must be below RLIMIT_NOFILE. */
#include "a_sys.h"

static i64 fcntl(i64 fd, i64 cmd, i64 arg) { return syscall3(SYS_fcntl, fd, cmd, arg); }

int test_main(void) {
    expect("dupfd_0_is_3", fcntl(1, F_DUPFD, 0), 3);
    expect("dupfd_3_is_4", fcntl(1, F_DUPFD, 3), 4);
    expect("dupfd_10", fcntl(1, F_DUPFD, 10), 10);
    expect("dupfd_10_again_is_11", fcntl(1, F_DUPFD, 10), 11);
    expect("dupfd_9_is_9", fcntl(1, F_DUPFD, 9), 9);
    expect("dupfd_no_cloexec", fcntl(9, F_GETFD, 0), 0);
    expect("dupfd_cloexec_20", fcntl(1, F_DUPFD_CLOEXEC, 20), 20);
    expect("dupfd_cloexec_flag", fcntl(20, F_GETFD, 0), FD_CLOEXEC);
    expect("dupfd_from_cloexec_clears", fcntl(20, F_DUPFD, 0), 5);
    expect("dupfd_from_cloexec_no_flag", fcntl(5, F_GETFD, 0), 0);
    struct a_rlimit old, lim;
    syscall2(SYS_getrlimit, RLIMIT_NOFILE, &old);
    lim.rlim_cur = 32;
    lim.rlim_max = old.rlim_max;
    expect("set_limit_32", syscall2(SYS_setrlimit, RLIMIT_NOFILE, &lim), 0);
    expect("dupfd_31", fcntl(1, F_DUPFD, 31), 31);
    expect("dupfd_full_from_31_emfile", fcntl(1, F_DUPFD, 31), -EMFILE);
    expect("dupfd_32_einval", fcntl(1, F_DUPFD, 32), -EINVAL);
    expect("dupfd_cloexec_32_einval", fcntl(1, F_DUPFD_CLOEXEC, 32), -EINVAL);
    expect("dupfd_negative_einval", fcntl(1, F_DUPFD, -1), -EINVAL);
    expect("dupfd_bad_fd_ebadf", fcntl(77, F_DUPFD, 0), -EBADF);
    expect("restore_limit", syscall2(SYS_setrlimit, RLIMIT_NOFILE, &old), 0);
    int closed = 1;
    for (int fd = 3; fd < 32; fd++) {
        i64 r = a_close(fd);
        closed &= r == 0 || r == -EBADF;
    }
    check("close_all", closed, 0);
    expect("dupfd_after_close_is_3", fcntl(1, F_DUPFD, 0), 3);
    a_close(3);
    return finish();
}
