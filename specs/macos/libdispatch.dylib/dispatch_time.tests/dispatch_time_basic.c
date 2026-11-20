/* dispatch_time(base, delta_ns): a monotonic time value; DISPATCH_TIME_NOW
 * is 0 and DISPATCH_TIME_FOREVER is ~0; adding to FOREVER stays FOREVER;
 * dispatch_walltime gives a time on the wall clock. */
/* poly-mac: libsystem */
#include "poly_mgcd.h"

int test_main(void) {
    check("DISPATCH_TIME_NOW is 0", DISPATCH_TIME_NOW == 0, 0);
    check("DISPATCH_TIME_FOREVER is all ones", DISPATCH_TIME_FOREVER == ~0ULL, 0);
    dispatch_time_t a = dispatch_time(DISPATCH_TIME_NOW, 0);
    dispatch_time_t b = dispatch_time(DISPATCH_TIME_NOW, 0);
    check("a time for now is not 0 (it is a clock reading)", a != 0, 0);
    check("two readings in a row: the second is not earlier", b >= a, 0);
    check("one second later is later", dispatch_time(a, NSEC_PER_SEC) > a, 0);
    value("units added by a delta of 1e9 ns (equal to 1e9 on x86_64; mach ticks on arm64: 24000000)", (long long)(dispatch_time(a, NSEC_PER_SEC) - a));
    check("adding 0 to a time keeps it", dispatch_time(a, 0) == a, 0);
    check("a time before it is earlier", dispatch_time(a, -1000) < a, 0);
    check("FOREVER plus a delta is FOREVER", dispatch_time(DISPATCH_TIME_FOREVER, 5) == DISPATCH_TIME_FOREVER, 0);
    check("FOREVER minus a delta is FOREVER", dispatch_time(DISPATCH_TIME_FOREVER, -5) == DISPATCH_TIME_FOREVER, 0);
    check("adding a huge delta saturates at FOREVER", dispatch_time(a, INT64_MAX) == DISPATCH_TIME_FOREVER, 0);
    value("NSEC_PER_SEC", NSEC_PER_SEC);
    value("NSEC_PER_MSEC", NSEC_PER_MSEC);
    value("USEC_PER_SEC", USEC_PER_SEC);
    return finish();
}
