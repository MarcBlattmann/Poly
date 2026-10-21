/* select timeouts: with nothing ready the call returns 0 after the
 * timeval and writes the remaining time (zero) back; with no sets it is a
 * sleep; a timeout of 1 second is cut short when data arrives and the
 * remaining time is written back (less than the start, more than zero); a
 * NULL timeout waits until ready; tv_usec of a million or more is
 * normalised, not refused; a handled signal ends the wait with EINTR.
 * poly-diff: syscalls=off (another process and a timer signal)
 */
#include "poly_ipc.h"

#define ITIMER_REAL 0
struct k_itimerval {
    i64 interval_sec, interval_usec, value_sec, value_usec;
};

static void on_alarm(int sig) { (void)sig; }

int test_main(void) {
    int p[2];
    syscall2(SYS_pipe2, p, 0);
    struct k_fd_set rd;
    fdset_zero(&rd);
    fdset_add(&rd, p[0]);
    struct k_timeval t = { 0, 30000 };
    i64 t0 = now_ns();
    i64 r = syscall5(SYS_select, p[0] + 1, &rd, 0, 0, &t);
    i64 dt = now_ns() - t0;
    check("timeout_returns_zero", r == 0, r);
    check("timeout_waited", dt >= 29000000, dt / 1000000);
    check("remaining_zero_written", t.tv_sec == 0 && t.tv_usec == 0, t.tv_usec);
    check("set_cleared_on_timeout", !fdset_has(&rd, p[0]), 0);

    struct k_timeval s = { 0, 20000 };
    t0 = now_ns();
    r = syscall5(SYS_select, 0, 0, 0, 0, &s);
    check("no_sets_sleeps", r == 0 && now_ns() - t0 >= 19000000, r);

    struct k_timeval big = { 0, 1500000 };
    fdset_zero(&rd);
    fdset_add(&rd, p[0]);
    write(p[1], "x", 1);
    r = syscall5(SYS_select, p[0] + 1, &rd, 0, 0, &big);
    check("usec_over_a_million_accepted", r == 1, r);
    char c;
    read(p[0], &c, 1);

    i64 pid = fork();
    if (pid == 0) {
        sleep_ms(30);
        write(p[1], "y", 1);
        exit_group(0);
    }
    struct k_timeval one = { 1, 0 };
    fdset_zero(&rd);
    fdset_add(&rd, p[0]);
    r = syscall5(SYS_select, p[0] + 1, &rd, 0, 0, &one);
    check("woken_by_data", r == 1 && fdset_has(&rd, p[0]), r);
    check("remaining_written_back", one.tv_sec == 0 && one.tv_usec > 0, one.tv_usec);
    wait_exit(pid);
    read(p[0], &c, 1);

    pid = fork();
    if (pid == 0) {
        sleep_ms(20);
        write(p[1], "z", 1);
        exit_group(0);
    }
    fdset_zero(&rd);
    fdset_add(&rd, p[0]);
    r = syscall5(SYS_select, p[0] + 1, &rd, 0, 0, 0);
    check("null_timeout_waits_until_ready", r == 1, r);
    wait_exit(pid);
    read(p[0], &c, 1);

    sig_handler(SIGALRM, on_alarm, SA_RESTART);
    struct k_itimerval it = { 0, 0, 0, 20000 };
    syscall3(SYS_setitimer, ITIMER_REAL, &it, 0);
    fdset_zero(&rd);
    fdset_add(&rd, p[0]);
    r = syscall5(SYS_select, p[0] + 1, &rd, 0, 0, 0);
    check("signal_eintr", r == -EINTR, r);
    return finish();
}
