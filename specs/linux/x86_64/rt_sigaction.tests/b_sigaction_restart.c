/* SA_RESTART: a blocking read on an empty pipe interrupted by a handled
 * signal fails with EINTR without the flag and is restarted with it (the
 * read then returns the data the handler finally writes); poll and
 * nanosleep are never restarted, they fail with EINTR either way. The
 * signals come from a repeating ITIMER_REAL.
 * poly-diff: syscalls=off (the number of timer signals varies)
 */
#include "poly_ipc.h"

#define ITIMER_REAL 0
struct k_itimerval {
    i64 interval_sec, interval_usec, value_sec, value_usec;
};

static int pfd[2];
static volatile int ticks, feed_after;

static void on_alarm(int sig) {
    (void)sig;
    ticks++;
    if (feed_after && ticks == feed_after) write(pfd[1], "z", 1);
}

static void timer(int on) {
    struct k_itimerval it = { 0, on ? 5000 : 0, 0, on ? 5000 : 0 };
    syscall3(SYS_setitimer, ITIMER_REAL, &it, 0);
}

int test_main(void) {
    syscall2(SYS_pipe2, pfd, 0);
    char c = 0;

    /* Without SA_RESTART. */
    sig_handler(SIGALRM, on_alarm, 0);
    ticks = 0;
    feed_after = 0;
    timer(1);
    i64 r = read(pfd[0], &c, 1);
    timer(0);
    check("read_without_restart_eintr", r == -EINTR, r);

    /* With SA_RESTART: the read goes on until the third tick feeds it. */
    sig_handler(SIGALRM, on_alarm, SA_RESTART);
    ticks = 0;
    feed_after = 3;
    timer(1);
    r = read(pfd[0], &c, 1);
    timer(0);
    check("read_with_restart_gets_data", r == 1 && c == 'z', r);
    check("read_restarted_over_several_signals", ticks >= 3, ticks);

    /* poll: EINTR even with SA_RESTART. */
    ticks = 0;
    feed_after = 0;
    struct k_pollfd p = { pfd[0], 1, 0 };
    timer(1);
    r = syscall3(SYS_poll, &p, 1, 5000);
    timer(0);
    check("poll_with_restart_eintr", r == -EINTR, r);

    /* nanosleep: EINTR even with SA_RESTART, remaining time written. */
    struct k_timespec req = { 5, 0 }, rem = { 0, 0 };
    timer(1);
    r = syscall2(SYS_nanosleep, &req, &rem);
    timer(0);
    check("nanosleep_with_restart_eintr", r == -EINTR, r);
    check("nanosleep_remaining_written", rem.tv_sec >= 3 && rem.tv_sec <= 5, rem.tv_sec);
    return finish();
}
