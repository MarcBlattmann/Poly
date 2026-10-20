/* nanosleep: sleeps at least the requested time (measured with
 * CLOCK_MONOTONIC), returns 0; a zero sleep returns at once. */
#include "a_sys.h"

static i64 now_ns(void) {
    struct a_timespec t;
    syscall2(SYS_clock_gettime, CLOCK_MONOTONIC, &t);
    return t.tv_sec * 1000000000L + t.tv_nsec;
}

int test_main(void) {
    struct a_timespec req = {0, 30000000}, rem = {77, 77};
    i64 t0 = now_ns();
    expect("sleep_30ms", syscall2(SYS_nanosleep, &req, &rem), 0);
    i64 t1 = now_ns();
    check("slept_at_least_30ms", t1 - t0 >= 30000000, t1 - t0);
    check("not_absurdly_long", t1 - t0 < 5000000000L, t1 - t0);
    struct a_timespec zero = {0, 0};
    expect("sleep_zero", syscall2(SYS_nanosleep, &zero, 0), 0);
    struct a_timespec one_sec = {1, 0};
    t0 = now_ns();
    expect("sleep_1s_seconds_field", syscall2(SYS_nanosleep, &one_sec, 0), 0);
    t1 = now_ns();
    check("slept_at_least_1s", t1 - t0 >= 1000000000L, t1 - t0);
    struct a_timespec mixed = {0, 999999999};
    t0 = now_ns();
    expect("sleep_max_nsec", syscall2(SYS_nanosleep, &mixed, 0), 0);
    check("slept_almost_1s", now_ns() - t0 >= 999999999L, 0);
    return finish();
}
