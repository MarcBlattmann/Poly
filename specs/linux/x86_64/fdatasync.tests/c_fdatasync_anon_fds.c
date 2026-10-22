/* fdatasync on descriptors without a backing file: eventfd, epoll,
 * timerfd, signalfd and inotify are EINVAL; a memfd (in-memory regular
 * file) is 0. */
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_timerfd_create 283
#define SYS_signalfd4 289
#define SYS_eventfd2 290
#define SYS_epoll_create1 291
#define SYS_inotify_init1 294
#define SYS_memfd_create 319

int test_main(void) {
    i64 fd = syscall2(SYS_eventfd2, 0, 0);
    check("eventfd_open", fd >= 0, fd);
    expect("eventfd", syscall1(SYS_fdatasync, fd), -EINVAL);
    a_close(fd);
    fd = syscall1(SYS_epoll_create1, 0);
    check("epoll_open", fd >= 0, fd);
    expect("epoll", syscall1(SYS_fdatasync, fd), -EINVAL);
    a_close(fd);
    fd = syscall2(SYS_timerfd_create, CLOCK_MONOTONIC, 0);
    check("timerfd_open", fd >= 0, fd);
    expect("timerfd", syscall1(SYS_fdatasync, fd), -EINVAL);
    a_close(fd);
    u64 mask = A_SIGBIT(SIGUSR1);
    fd = syscall4(SYS_signalfd4, -1, &mask, 8, 0);
    check("signalfd_open", fd >= 0, fd);
    expect("signalfd", syscall1(SYS_fdatasync, fd), -EINVAL);
    a_close(fd);
    fd = syscall1(SYS_inotify_init1, 0);
    check("inotify_open", fd >= 0, fd);
    expect("inotify", syscall1(SYS_fdatasync, fd), -EINVAL);
    a_close(fd);
    fd = syscall2(SYS_memfd_create, "c_fdatasync", 0);
    check("memfd_open", fd >= 0, fd);
    expect("memfd_empty", syscall1(SYS_fdatasync, fd), 0);
    expect("memfd_write", a_write(fd, "abc", 3), 3);
    expect("memfd", syscall1(SYS_fdatasync, fd), 0);
    a_close(fd);
    return finish();
}
