/* FUTEX_WAIT with a timeout, interrupted by a handled signal: the wait
 * returns -EINTR, and restart_syscall in the handler waits again on the
 * same word: it compares the word with the original value first (changed:
 * -EAGAIN at once), else sleeps until the ORIGINAL deadline and returns
 * -ETIMEDOUT. That consumes what there was to restart: a second call is
 * -EINTR. FUTEX_WAIT_BITSET with an absolute timeout behaves the same.
 * A FUTEX_WAIT without timeout leaves nothing to restart (-EINTR). */
#include "poly_test.h"
#include "poly_ipc.h"

#define SYS_restart_syscall 219

static volatile unsigned word;
static volatile int change;
static volatile i64 h_ret, h_ret2, h_end;

static void on_alrm(int sig) {
    (void)sig;
    if (change) word = 5;
    h_ret = syscall0(SYS_restart_syscall);
    h_end = now_ns();
    h_ret2 = syscall0(SYS_restart_syscall);
}

static void arm_ms(long ms) {
    struct { long isec, iusec, sec, usec; } v = { 0, 0, ms / 1000, (ms % 1000) * 1000 };
    syscall3(SYS_setitimer, 0, &v, 0);
}

int test_main(void) {
    sig_handler(SIGALRM, on_alrm, 0);
    struct k_timespec rel = { 0, 400000000 };

    word = 0;
    change = 0;
    arm_ms(100);
    i64 t0 = now_ns();
    i64 r = futex(&word, FUTEX_WAIT, 0, &rel, 0, 0);
    i64 ms = (h_end - t0) / 1000000;
    check("wait_eintr", r == -EINTR, r);
    check("restart_etimedout", h_ret == -ETIMEDOUT, h_ret);
    check("restart_ends_at_original_deadline", ms >= 399 && ms < 560, ms);
    check("second_restart_eintr", h_ret2 == -EINTR, h_ret2);

    word = 0;
    change = 1;
    arm_ms(100);
    t0 = now_ns();
    r = futex(&word, FUTEX_WAIT | FUTEX_PRIVATE_FLAG, 0, &rel, 0, 0);
    ms = (h_end - t0) / 1000000;
    check("private_wait_eintr", r == -EINTR, r);
    check("changed_word_restart_eagain", h_ret == -EAGAIN, h_ret);
    check("changed_word_restart_at_once", ms < 300, ms);
    check("changed_second_restart_eintr", h_ret2 == -EINTR, h_ret2);

    word = 0;
    change = 0;
    struct k_timespec until;
    t0 = now_ns();
    syscall2(SYS_clock_gettime, CLOCK_MONOTONIC, &until);
    until.tv_nsec += 400000000;
    if (until.tv_nsec >= 1000000000) {
        until.tv_nsec -= 1000000000;
        until.tv_sec++;
    }
    arm_ms(100);
    r = futex(&word, FUTEX_WAIT_BITSET, 0, &until, 0, FUTEX_BITSET_MATCH_ANY);
    ms = (h_end - t0) / 1000000;
    check("bitset_wait_eintr", r == -EINTR, r);
    check("bitset_restart_etimedout", h_ret == -ETIMEDOUT, h_ret);
    check("bitset_restart_ends_at_deadline", ms >= 399 && ms < 560, ms);

    word = 0;
    change = 1; /* harmless: nothing is restarted */
    arm_ms(100);
    t0 = now_ns();
    r = futex(&word, FUTEX_WAIT, 0, 0, 0, 0);
    ms = (h_end - t0) / 1000000;
    check("untimed_wait_eintr", r == -EINTR, r);
    check("untimed_restart_eintr", h_ret == -EINTR, h_ret);
    check("untimed_restart_at_once", ms < 300, ms);
    return finish();
}
