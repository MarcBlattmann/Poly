/* time: seconds since the epoch, the same as CLOCK_REALTIME's seconds;
 * also stored at the pointer when one is given. */
#include "a_sys.h"

int test_main(void) {
    struct a_timespec a, b;
    i64 stored = -1;
    syscall2(SYS_clock_gettime, CLOCK_REALTIME, &a);
    i64 t = syscall1(SYS_time, &stored);
    syscall2(SYS_clock_gettime, CLOCK_REALTIME, &b);
    check("between_clock_readings", t >= a.tv_sec && t <= b.tv_sec, t - a.tv_sec);
    expect("stored_equals_returned", stored, t);
    check("after_2020", t > 1577836800L, t);
    i64 t2 = syscall1(SYS_time, 0);
    check("null_ok_not_backwards", t2 >= t, t2 - t);
    return finish();
}
