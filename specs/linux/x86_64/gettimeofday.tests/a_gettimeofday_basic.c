/* gettimeofday: seconds and microseconds of CLOCK_REALTIME (within a
 * second of clock_gettime), microseconds below 1e6, after 2020; NULL
 * pointers allowed; the time zone reads as zeros. */
#include "a_sys.h"

int test_main(void) {
    struct a_timeval tv = {-1, -1};
    struct a_timezone tz = {-1, -1};
    struct a_timespec a, b;
    syscall2(SYS_clock_gettime, CLOCK_REALTIME, &a);
    expect("gettimeofday", syscall2(SYS_gettimeofday, &tv, &tz), 0);
    syscall2(SYS_clock_gettime, CLOCK_REALTIME, &b);
    check("usec_in_range", tv.tv_usec >= 0 && tv.tv_usec < 1000000, tv.tv_usec);
    check("between_clock_readings", tv.tv_sec >= a.tv_sec && tv.tv_sec <= b.tv_sec, tv.tv_sec - a.tv_sec);
    check("after_2020", tv.tv_sec > 1577836800L, tv.tv_sec);
    expect("timezone_minuteswest_0", tz.tz_minuteswest, 0);
    expect("timezone_dsttime_0", tz.tz_dsttime, 0);
    expect("both_null_ok", syscall2(SYS_gettimeofday, 0, 0), 0);
    expect("tz_only_ok", syscall2(SYS_gettimeofday, 0, &tz), 0);
    return finish();
}
