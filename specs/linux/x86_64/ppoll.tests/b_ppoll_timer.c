/* ppoll with a mask that blocks a timer signal: the signal that arrives
 * while waiting does not end the call (it times out with 0) and its
 * handler runs once ppoll has put the old mask back; a NULL timeout with a
 * ready descriptor returns at once; data written by a child ends a
 * blocking ppoll.
 * poly-diff: syscalls=off (a timer signal and a child process)
 */
#include "poly_ipc.h"

#define ITIMER_REAL 0
struct k_itimerval {
    i64 interval_sec, interval_usec, value_sec, value_usec;
};
static volatile int alarms;
static void on_alarm(int sig) {
    (void)sig;
    alarms++;
}

int test_main(void) {
    int p[2];
    syscall2(SYS_pipe2, p, 0);
    struct k_pollfd f = { p[0], POLLIN, 0 };
    sig_handler(SIGALRM, on_alarm, 0);
    struct k_itimerval it = { 0, 0, 0, 10000 };
    syscall3(SYS_setitimer, ITIMER_REAL, &it, 0);
    u64 block = SIGBIT(SIGALRM);
    struct k_timespec t = { 0, 60000000 };
    i64 r = syscall5(SYS_ppoll, &f, 1, &t, &block, 8);
    check("blocked_timer_does_not_interrupt", r == 0, r);
    check("handler_after_return", alarms == 1, alarms);

    i64 pid = fork();
    if (pid == 0) {
        sleep_ms(30);
        write(p[1], "x", 1);
        exit_group(0);
    }
    r = syscall5(SYS_ppoll, &f, 1, 0, 0, 8);
    check("child_data_ends_wait", r == 1 && f.revents == POLLIN, r);
    wait_exit(pid);
    r = syscall5(SYS_ppoll, &f, 1, 0, 0, 8);
    check("ready_null_timeout_returns", r == 1, r);
    return finish();
}
