/* futex_waitv's timeout is absolute on CLOCK_MONOTONIC or CLOCK_REALTIME:
 * now + 30 ms sleeps until then and returns ETIMEDOUT (110) with the clock
 * at or past the deadline; a small value is long past (ETIMEDOUT at once);
 * the timespec is not written. All values are compared before the
 * deadline: one entry that differs is EAGAIN at once. Without a timeout
 * the clockid is ignored.
 */
#include "a_sys.h"

#define SC6(n, a, b, c, d, e, f) syscall6((n), (i64)(a), (i64)(b), (i64)(c), (i64)(d), (i64)(e), (i64)(f))
#define SC5(n, a, b, c, d, e) syscall6((n), (i64)(a), (i64)(b), (i64)(c), (i64)(d), (i64)(e), 0)
#define SYS_futex_waitv 449
/* linux/futex.h */
#define FUTEX2_SIZE_U32 0x02
#define FUTEX2_PRIVATE 128
#define ETIMEDOUT 110

struct futex_waitv {
    u64 val;
    u64 uaddr;
    u32 flags;
    u32 reserved;
};

static u32 word[8] __attribute__((aligned(64)));
static struct futex_waitv v[3];

static i64 ns_of(struct a_timespec *t) { return t->tv_sec * 1000000000L + t->tv_nsec; }

static void timed(const char *tag, i64 clk, int n) {
    char name[64];
    struct a_timespec deadline, before, after, copy;
    syscall2(SYS_clock_gettime, clk, &before);
    deadline = before;
    deadline.tv_nsec += 30000000;
    if (deadline.tv_nsec >= 1000000000) deadline.tv_sec++, deadline.tv_nsec -= 1000000000;
    copy = deadline;
    i64 r = SC5(SYS_futex_waitv, v, n, 0, &deadline, clk);
    syscall2(SYS_clock_gettime, clk, &after);
    expect(str_cat2(name, tag, "_etimedout"), r, -ETIMEDOUT);
    check(str_cat2(name, tag, "_reached_deadline"), ns_of(&after) >= ns_of(&copy), ns_of(&copy) - ns_of(&after));
    check(str_cat2(name, tag, "_within_2s"), ns_of(&after) - ns_of(&before) < 2000000000L, 0);
    check(str_cat2(name, tag, "_timespec_unchanged"), deadline.tv_sec == copy.tv_sec && deadline.tv_nsec == copy.tv_nsec,
          0);
}

int test_main(void) {
    word[0] = 10;
    word[2] = 20;
    word[4] = 30;
    v[0] = (struct futex_waitv){10, (u64)&word[0], FUTEX2_SIZE_U32, 0};
    v[1] = (struct futex_waitv){20, (u64)&word[2], FUTEX2_SIZE_U32 | FUTEX2_PRIVATE, 0};
    v[2] = (struct futex_waitv){30, (u64)&word[4], FUTEX2_SIZE_U32, 0};
    timed("mono_one", CLOCK_MONOTONIC, 1);
    timed("mono_three", CLOCK_MONOTONIC, 3);
    timed("real_three", CLOCK_REALTIME, 3);

    struct a_timespec small = {0, 30000000}, before, after;
    syscall2(SYS_clock_gettime, CLOCK_MONOTONIC, &before);
    expect("small_value_is_past", SC5(SYS_futex_waitv, v, 3, 0, &small, CLOCK_MONOTONIC), -ETIMEDOUT);
    expect("small_value_is_past_realtime", SC5(SYS_futex_waitv, v, 3, 0, &small, CLOCK_REALTIME), -ETIMEDOUT);
    syscall2(SYS_clock_gettime, CLOCK_MONOTONIC, &after);
    check("past_returns_quickly", ns_of(&after) - ns_of(&before) < 25000000L, 0);

    word[4] = 31;
    expect("last_differs_eagain", SC5(SYS_futex_waitv, v, 3, 0, &small, CLOCK_MONOTONIC), -EAGAIN);
    expect("two_without_it_timedout", SC5(SYS_futex_waitv, v, 2, 0, &small, CLOCK_MONOTONIC), -ETIMEDOUT);
    word[4] = 30;
    word[0] = 11;
    expect("first_differs_eagain", SC5(SYS_futex_waitv, v, 3, 0, &small, CLOCK_MONOTONIC), -EAGAIN);
    expect("no_timeout_bad_clock_eagain", SC5(SYS_futex_waitv, v, 3, 0, 0, 12345), -EAGAIN);
    word[0] = 10;
    return finish();
}
