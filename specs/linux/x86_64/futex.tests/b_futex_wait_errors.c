/* futex without a second task: FUTEX_WAIT returns EAGAIN when the word
 * differs from the expected value and ETIMEDOUT after a relative timeout;
 * FUTEX_WAIT_BITSET takes an absolute CLOCK_MONOTONIC time (one in the past
 * times out at once) or CLOCK_REALTIME with FUTEX_CLOCK_REALTIME; waking
 * with no waiters returns 0. Errors: misaligned address (EINVAL), bad
 * address (EFAULT), bad timeout (EINVAL), bitset 0 (EINVAL), unknown
 * operation (ENOSYS). The private flag gives the same results. */
#include "poly_ipc.h"

static volatile unsigned word __attribute__((aligned(8)));
static volatile unsigned pair[2] __attribute__((aligned(8)));

/* A time on the same clock that is already past: half of t. */
static struct k_timespec half_of(struct k_timespec t) {
    i64 ns = (t.tv_sec * 1000000000L + t.tv_nsec) / 2;
    struct k_timespec h = { ns / 1000000000L, ns % 1000000000L };
    return h;
}

int test_main(void) {
    word = 5;
    i64 r = futex(&word, FUTEX_WAIT, 4, 0, 0, 0);
    check("wait_value_mismatch_eagain", r == -EAGAIN, r);
    r = futex(&word, FUTEX_WAIT | FUTEX_PRIVATE_FLAG, 4, 0, 0, 0);
    check("private_wait_value_mismatch_eagain", r == -EAGAIN, r);

    struct k_timespec t = { 0, 20000000 };
    i64 t0 = now_ns();
    r = futex(&word, FUTEX_WAIT, 5, &t, 0, 0);
    i64 dt = now_ns() - t0;
    check("wait_relative_timeout_etimedout", r == -ETIMEDOUT, r);
    check("wait_timeout_elapsed", dt >= 19000000, dt / 1000000);
    struct k_timespec z = { 0, 0 };
    r = futex(&word, FUTEX_WAIT | FUTEX_PRIVATE_FLAG, 5, &z, 0, 0);
    check("wait_zero_timeout_etimedout", r == -ETIMEDOUT, r);

    /* Absolute deadline (WAIT_BITSET): in the past, and 20 ms ahead. The
     * past deadline is half the clock's current value: before now, but not
     * negative however short the uptime (a negative time is EINVAL). */
    struct k_timespec now;
    syscall2(SYS_clock_gettime, CLOCK_MONOTONIC, &now);
    struct k_timespec past = half_of(now);
    r = futex(&word, FUTEX_WAIT_BITSET, 5, &past, 0, FUTEX_BITSET_MATCH_ANY);
    check("bitset_past_deadline_etimedout", r == -ETIMEDOUT, r);
    struct k_timespec soon = now;
    soon.tv_nsec += 20000000;
    if (soon.tv_nsec >= 1000000000) soon.tv_sec++, soon.tv_nsec -= 1000000000;
    r = futex(&word, FUTEX_WAIT_BITSET | FUTEX_PRIVATE_FLAG, 5, &soon, 0, FUTEX_BITSET_MATCH_ANY);
    struct k_timespec after;
    syscall2(SYS_clock_gettime, CLOCK_MONOTONIC, &after);
    check("bitset_deadline_etimedout", r == -ETIMEDOUT, r);
    check("bitset_deadline_reached",
          after.tv_sec > soon.tv_sec || (after.tv_sec == soon.tv_sec && after.tv_nsec >= soon.tv_nsec), 0);
    struct k_timespec rt;
    syscall2(SYS_clock_gettime, CLOCK_REALTIME, &rt);
    rt = half_of(rt);
    r = futex(&word, FUTEX_WAIT_BITSET | FUTEX_CLOCK_REALTIME, 5, &rt, 0, FUTEX_BITSET_MATCH_ANY);
    check("bitset_realtime_past_etimedout", r == -ETIMEDOUT, r);
    r = futex(&word, FUTEX_WAIT_BITSET, 4, &past, 0, FUTEX_BITSET_MATCH_ANY);
    check("bitset_value_checked_first_eagain", r == -EAGAIN, r);

    /* Wakes with nobody waiting. */
    r = futex(&word, FUTEX_WAKE, 1, 0, 0, 0);
    check("wake_no_waiters_zero", r == 0, r);
    r = futex(&word, FUTEX_WAKE | FUTEX_PRIVATE_FLAG, 0x7fffffff, 0, 0, 0);
    check("private_wake_no_waiters_zero", r == 0, r);
    r = futex(&word, FUTEX_WAKE_BITSET, 1, 0, 0, 0x5);
    check("wake_bitset_no_waiters_zero", r == 0, r);

    /* Errors. */
    volatile unsigned *odd = (volatile unsigned *)((char *)pair + 2);
    r = futex(odd, FUTEX_WAIT, 0, 0, 0, 0);
    check("misaligned_wait_einval", r == -EINVAL, r);
    r = futex(odd, FUTEX_WAKE, 1, 0, 0, 0);
    check("misaligned_wake_einval", r == -EINVAL, r);
    r = futex((volatile unsigned *)8, FUTEX_WAIT, 0, 0, 0, 0);
    check("bad_address_wait_efault", r == -EFAULT, r);
    struct k_timespec bad = { 0, 1000000000 };
    r = futex(&word, FUTEX_WAIT, 5, &bad, 0, 0);
    check("timeout_nsec_too_big_einval", r == -EINVAL, r);
    struct k_timespec neg = { -1, 0 };
    r = futex(&word, FUTEX_WAIT, 5, &neg, 0, 0);
    check("negative_timeout_einval", r == -EINVAL, r);
    r = futex(&word, FUTEX_WAIT, 5, (void *)8, 0, 0);
    check("bad_timeout_pointer_efault", r == -EFAULT, r);
    r = futex(&word, FUTEX_WAIT_BITSET, 5, &past, 0, 0);
    check("wait_bitset_zero_einval", r == -EINVAL, r);
    r = futex(&word, FUTEX_WAKE_BITSET, 1, 0, 0, 0);
    check("wake_bitset_zero_einval", r == -EINVAL, r);
    r = futex(&word, 99, 0, 0, 0, 0);
    check("unknown_op_enosys", r == -ENOSYS, r);
    r = futex(&word, FUTEX_FD, 0, 0, 0, 0);
    check("futex_fd_removed_enosys", r == -ENOSYS, r);
    r = futex(&word, FUTEX_WAKE | FUTEX_CLOCK_REALTIME, 1, 0, 0, 0);
    check("clock_realtime_with_wake_enosys", r == -ENOSYS, r);
    check("word_unchanged", word == 5, word);
    return finish();
}
