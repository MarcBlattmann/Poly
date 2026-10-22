/* fchown on descriptors of anonymous inodes (eventfd, epoll, timerfd,
 * inotify) fails with EOPNOTSUPP whatever the ids: -1 for both, the
 * caller's own ids, or foreign ones. */
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_timerfd_create 283
#define SYS_eventfd2 290
#define SYS_epoll_create1 291
#define SYS_inotify_init1 294

/* asm-generic/errno.h */
#define EOPNOTSUPP 95

int test_main(void) {
    i64 uid = syscall0(SYS_geteuid), gid = syscall0(SYS_getegid);
    i64 ev = syscall2(SYS_eventfd2, 0, 0);
    check("eventfd", ev >= 0, ev);
    expect("eventfd_minus_one", syscall3(SYS_fchown, ev, -1, -1), -EOPNOTSUPP);
    expect("eventfd_own_ids", syscall3(SYS_fchown, ev, uid, gid), -EOPNOTSUPP);
    expect("eventfd_foreign_uid", syscall3(SYS_fchown, ev, uid + 1, -1), -EOPNOTSUPP);
    i64 ep = syscall1(SYS_epoll_create1, 0);
    check("epoll", ep >= 0, ep);
    expect("epoll_minus_one", syscall3(SYS_fchown, ep, -1, -1), -EOPNOTSUPP);
    expect("epoll_own_ids", syscall3(SYS_fchown, ep, uid, gid), -EOPNOTSUPP);
    i64 tf = syscall2(SYS_timerfd_create, CLOCK_MONOTONIC, 0);
    check("timerfd", tf >= 0, tf);
    expect("timerfd_minus_one", syscall3(SYS_fchown, tf, -1, -1), -EOPNOTSUPP);
    i64 in = syscall1(SYS_inotify_init1, 0);
    check("inotify", in >= 0, in);
    expect("inotify_minus_one", syscall3(SYS_fchown, in, -1, -1), -EOPNOTSUPP);
    a_close(ev);
    a_close(ep);
    a_close(tf);
    a_close(in);
    expect("closed_eventfd_ebadf", syscall3(SYS_fchown, ev, -1, -1), -EBADF);
    return finish();
}
