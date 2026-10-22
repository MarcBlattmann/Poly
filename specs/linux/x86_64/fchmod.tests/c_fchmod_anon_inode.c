/* fchmod on a descriptor of an anonymous inode (eventfd, epoll,
 * timerfd) is refused with EOPNOTSUPP, whoever the caller is; a bad
 * descriptor is still EBADF. */
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_timerfd_create 283
#define SYS_eventfd2 290
#define SYS_epoll_create1 291

int test_main(void) {
    i64 e = syscall2(SYS_eventfd2, 0, 0);
    check("eventfd", e >= 0, e);
    expect("fchmod_eventfd", syscall2(SYS_fchmod, e, 0600), -EOPNOTSUPP);
    expect("fchmod_eventfd_0", syscall2(SYS_fchmod, e, 0), -EOPNOTSUPP);
    i64 ep = syscall1(SYS_epoll_create1, 0);
    check("epoll", ep >= 0, ep);
    expect("fchmod_epoll", syscall2(SYS_fchmod, ep, 0644), -EOPNOTSUPP);
    i64 t = syscall2(SYS_timerfd_create, CLOCK_MONOTONIC, 0);
    check("timerfd", t >= 0, t);
    expect("fchmod_timerfd", syscall2(SYS_fchmod, t, 0600), -EOPNOTSUPP);
    a_close(e);
    expect("fchmod_closed_eventfd", syscall2(SYS_fchmod, e, 0600), -EBADF);
    a_close(ep);
    a_close(t);
    return finish();
}
