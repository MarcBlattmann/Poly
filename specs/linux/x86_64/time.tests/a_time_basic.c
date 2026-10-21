/* time: seconds since the epoch, the seconds of CLOCK_REALTIME_COARSE;
 * also stored at the pointer when one is given.
 *
 * The coarse clock is only updated at a timer tick, so it lags
 * CLOCK_REALTIME by up to about a tick: just after a second boundary
 * time() can still say the previous second, one less than a
 * CLOCK_REALTIME reading taken before it. So time() is checked between
 * coarse readings taken before and after, and against CLOCK_REALTIME
 * with one second of slack below. */
#include "a_sys.h"

int test_main(void) {
    struct a_timespec ca, a, b, cb;
    i64 stored = -1;
    syscall2(SYS_clock_gettime, CLOCK_REALTIME_COARSE, &ca);
    syscall2(SYS_clock_gettime, CLOCK_REALTIME, &a);
    i64 t = syscall1(SYS_time, &stored);
    syscall2(SYS_clock_gettime, CLOCK_REALTIME, &b);
    syscall2(SYS_clock_gettime, CLOCK_REALTIME_COARSE, &cb);
    check("between_coarse_readings", t >= ca.tv_sec && t <= cb.tv_sec, t - ca.tv_sec);
    check("realtime_within_a_second", t >= a.tv_sec - 1 && t <= b.tv_sec, t - a.tv_sec);
    expect("stored_equals_returned", stored, t);
    check("after_2020", t > 1577836800L, t);
    i64 t2 = syscall1(SYS_time, 0);
    check("null_ok_not_backwards", t2 >= t, t2 - t);
    return finish();
}
