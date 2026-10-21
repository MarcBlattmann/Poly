/* Timers on each accepted clock expire: CLOCK_REALTIME with an absolute
 * deadline taken from clock_gettime(CLOCK_REALTIME), CLOCK_BOOTTIME and
 * CLOCK_MONOTONIC relative; a timerfd is reported by epoll as EPOLLIN
 * when it has expired; it is not writable (never POLLOUT). */
#include "poly_ipc.h"

int test_main(void) {
    int rt = (int)syscall2(SYS_timerfd_create, CLOCK_REALTIME, 0);
    struct k_timespec now;
    syscall2(SYS_clock_gettime, CLOCK_REALTIME, &now);
    struct k_itimerspec at = { { 0, 0 }, now };
    at.value.tv_nsec += 20000000;
    if (at.value.tv_nsec >= 1000000000) at.value.tv_sec++, at.value.tv_nsec -= 1000000000;
    syscall4(SYS_timerfd_settime, rt, TFD_TIMER_ABSTIME, &at, 0);
    u64 n = 0;
    i64 r = read(rt, &n, 8);
    check("realtime_absolute_expires", r == 8 && n == 1, r);

    int bt = (int)syscall2(SYS_timerfd_create, CLOCK_BOOTTIME, 0);
    struct k_itimerspec rel = { { 0, 0 }, { 0, 10000000 } };
    syscall4(SYS_timerfd_settime, bt, 0, &rel, 0);
    r = read(bt, &n, 8);
    check("boottime_relative_expires", r == 8 && n == 1, r);

    int mt = (int)syscall2(SYS_timerfd_create, CLOCK_MONOTONIC, TFD_NONBLOCK);
    int e = (int)syscall1(SYS_epoll_create1, 0);
    struct k_epoll_event ev = { EPOLLIN | EPOLLOUT, 3 }, out;
    syscall4(SYS_epoll_ctl, e, EPOLL_CTL_ADD, mt, &ev);
    r = syscall4(SYS_epoll_wait, e, &out, 1, 0);
    check("unarmed_not_ready", r == 0, r);
    syscall4(SYS_timerfd_settime, mt, 0, &rel, 0);
    r = syscall4(SYS_epoll_wait, e, &out, 1, 2000);
    check("expired_epollin_only", r == 1 && out.events == EPOLLIN && out.data == 3, out.events);
    read(mt, &n, 8);
    r = syscall4(SYS_epoll_wait, e, &out, 1, 0);
    check("read_clears_readiness", r == 0, r);
    return finish();
}
