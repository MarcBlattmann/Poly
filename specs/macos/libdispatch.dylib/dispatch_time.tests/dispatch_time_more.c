/* The unit of a dispatch_time value is the mach absolute time tick: the
 * nanosecond delta is converted with the timebase. */
/* poly-mac: libsystem */
#include "poly_mgcd.h"
#include <mach/mach_time.h>

int test_main(void) {
    /* The values of dispatch_time are mach_absolute_time ticks; a delta is given in
     * nanoseconds and converted with the timebase (numer/denom, 1/1 on x86_64, 125/3 on arm64). */
    mach_timebase_info_data_t tb;
    mach_timebase_info(&tb);
    value("timebase numer", tb.numer);
    value("timebase denom", tb.denom);
    dispatch_time_t a = dispatch_time(DISPATCH_TIME_NOW, 0);
    dispatch_time_t b = dispatch_time(a, 500 * NSEC_PER_MSEC);
    dispatch_time_t c = dispatch_time(b, 500 * NSEC_PER_MSEC);
    unsigned long long second_in_ticks = NSEC_PER_SEC * tb.denom / tb.numer;
    check("two half-second steps are one second, in ticks", c - a == second_in_ticks, (long long)(c - a));
    check("a - 0.5 s", a - dispatch_time(a, -500 * (int64_t)NSEC_PER_MSEC) == 500 * NSEC_PER_MSEC * tb.denom / tb.numer, 0);
    check("a reading of now is the mach absolute time, as mach_absolute_time gives it (within 1 s)", ({ unsigned long long m = mach_absolute_time(); (m >= a ? m - a : a - m) < second_in_ticks; }), 0);
    check("the clock does not run backwards over a 20 ms sleep", ({ gcd_pause_ms(20); dispatch_time(DISPATCH_TIME_NOW, 0) - a; }) >= 19 * 1000000ULL * tb.denom / tb.numer, 0);
    return finish();
}
