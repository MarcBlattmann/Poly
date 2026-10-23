/* Character devices, sockets and eventfds are not files readahead
 * applies to: EINVAL when open for reading, EBADF when write-only. */
#include "a_sys.h"

#define SYS_readahead 187

static i64 ra(i64 fd, i64 off, i64 count) { return syscall3(SYS_readahead, fd, off, count); }

#define SYS_socketpair 53
#define SYS_eventfd2 290
#define AF_UNIX 1
#define SOCK_STREAM 1

int test_main(void) {
    i64 n = a_open("/dev/null", O_RDONLY, 0);
    expect("dev_null", ra(n, 0, 1), -EINVAL);
    expect("dev_null_count_0", ra(n, 0, 0), -EINVAL);
    a_close(n);
    n = a_open("/dev/null", O_RDWR, 0);
    expect("dev_null_rdwr", ra(n, 0, 1), -EINVAL);
    a_close(n);
    n = a_open("/dev/null", O_WRONLY, 0);
    expect("dev_null_wronly", ra(n, 0, 1), -EBADF);
    a_close(n);
    n = a_open("/dev/zero", O_RDONLY, 0);
    expect("dev_zero", ra(n, 0, 4096), -EINVAL);
    a_close(n);
    int s[2];
    expect("socketpair", syscall4(SYS_socketpair, AF_UNIX, SOCK_STREAM, 0, s), 0);
    expect("socket", ra(s[0], 0, 1), -EINVAL);
    expect("socket_bad_count", ra(s[1], 0, -1), -EINVAL);
    a_close(s[0]);
    a_close(s[1]);
    i64 e = syscall2(SYS_eventfd2, 0, 0);
    check("eventfd", e >= 0, e);
    expect("eventfd_ra", ra(e, 0, 8), -EINVAL);
    a_close(e);
    return finish();
}
