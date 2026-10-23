/* Only nanosleep, clock_nanosleep (relative), poll and timed futex waits
 * leave something for restart_syscall. A handler interrupting any other
 * blocking call (read of an empty pipe, pause, rt_sigsuspend, ppoll,
 * select, pselect6, epoll_wait, rt_sigtimedwait) finds nothing to
 * restart: restart_syscall there is -EINTR at once, and so is it after
 * the handler has returned. */
#include "poly_test.h"
#include "poly_ipc.h"

#define SYS_restart_syscall 219

static volatile i64 h_ret, h_ms;

static void on_alrm(int sig) {
    (void)sig;
    i64 t = now_ns();
    h_ret = syscall0(SYS_restart_syscall);
    h_ms = (now_ns() - t) / 1000000;
}

static void arm_ms(long ms) {
    struct { long isec, iusec, sec, usec; } v = { 0, 0, ms / 1000, (ms % 1000) * 1000 };
    syscall3(SYS_setitimer, 0, &v, 0);
}

static void cat(char *out, const char *a, const char *b) {
    while (*a) *out++ = *a++;
    while (*b) *out++ = *b++;
    *out = 0;
}

static void after(const char *tag, i64 r) {
    char name[64];
    cat(name, tag, "_eintr"); check(name, r == -EINTR, r);
    cat(name, tag, "_restart_in_handler_eintr"); check(name, h_ret == -EINTR, h_ret);
    cat(name, tag, "_restart_at_once"); check(name, h_ms < 100, h_ms);
    i64 again = syscall0(SYS_restart_syscall);
    cat(name, tag, "_restart_after_handler_eintr"); check(name, again == -EINTR, again);
    h_ret = 99;
}

int test_main(void) {
    sig_handler(SIGALRM, on_alrm, 0);
    int p[2];
    syscall2(SYS_pipe2, p, 0);
    char c;

    arm_ms(50);
    after("pipe_read", read(p[0], &c, 1));

    arm_ms(50);
    after("pause", syscall0(SYS_pause));

    u64 none = 0;
    arm_ms(50);
    after("rt_sigsuspend", syscall2(SYS_rt_sigsuspend, &none, 8));

    struct k_timespec ts = { 0, 400000000 };
    struct k_pollfd pf = { p[0], POLLIN, 0 };
    arm_ms(50);
    after("ppoll", syscall5(SYS_ppoll, &pf, 1, &ts, 0, 8));

    struct k_fd_set rd;
    fdset_zero(&rd);
    fdset_add(&rd, p[0]);
    struct k_timeval tv = { 0, 400000 };
    arm_ms(50);
    after("select", syscall5(SYS_select, p[0] + 1, &rd, 0, 0, &tv));

    fdset_zero(&rd);
    fdset_add(&rd, p[0]);
    ts.tv_sec = 0;
    ts.tv_nsec = 400000000;
    arm_ms(50);
    after("pselect6", syscall6(SYS_pselect6, p[0] + 1, (i64)&rd, 0, 0, (i64)&ts, 0));

    i64 ep = syscall1(SYS_epoll_create1, 0);
    struct k_epoll_event ev = { EPOLLIN, 0 };
    syscall4(SYS_epoll_ctl, ep, EPOLL_CTL_ADD, p[0], &ev);
    arm_ms(50);
    after("epoll_wait", syscall4(SYS_epoll_wait, ep, &ev, 1, 400));

    u64 usr2 = SIGBIT(SIGUSR2);
    sig_mask(SIG_BLOCK, usr2, 0);
    ts.tv_sec = 0;
    ts.tv_nsec = 400000000;
    arm_ms(50);
    after("rt_sigtimedwait", syscall4(SYS_rt_sigtimedwait, &usr2, 0, &ts, 8));
    return finish();
}
