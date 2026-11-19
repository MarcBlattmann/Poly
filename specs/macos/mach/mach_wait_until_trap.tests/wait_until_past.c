/* mach_wait_until_trap, Mach trap 90 (deadline): a deadline that has passed returns KERN_SUCCESS at once (deadline 1; a deadline a second ago).
 * The deadline is in mach_absolute_time units, which arm64 reads from the
 * virtual counter (24 MHz, mach_timebase_info 125/3): on x86_64 this test
 * checks nothing yet. */
#include "poly_mtest.h"

#if defined(POLY_ARM64)
int test_main(void) {
    u64 t0 = poly_mach_now();
    check("deadline 1: KERN_SUCCESS", (u32)poly_mach6(MACH_mach_wait_until_trap, 1, 0, 0, 0, 0, 0) == KERN_SUCCESS, 0);
    check("a second ago: KERN_SUCCESS", (u32)poly_mach6(MACH_mach_wait_until_trap, (i64)(poly_mach_now() - 24000000), 0, 0, 0, 0, 0) == KERN_SUCCESS, 0);
    u64 t1 = poly_mach_now();
    check("both returned within 5 ms", t1 - t0 < 24000 * 5, (i64)(t1 - t0));
    return finish();
}
#else
int test_main(void) { return finish(); }
#endif
