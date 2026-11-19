/* mach_wait_until_trap, Mach trap 90: three waits of 10 ms one after the other take at least 30 ms in all; a deadline already reached by the first wait makes the next one return at once.
 * The deadline is in mach_absolute_time units, which arm64 reads from the
 * virtual counter (24 MHz, mach_timebase_info 125/3): on x86_64 this test
 * checks nothing yet. */
#include "poly_mtest.h"

#if defined(POLY_ARM64)
int test_main(void) {
    u64 start = poly_mach_now();
    for (int i = 1; i <= 3; i++) poly_mach6(MACH_mach_wait_until_trap, (i64)(start + 24000 * 10 * i), 0, 0, 0, 0, 0);
    u64 t = poly_mach_now();
    check("at least 30 ms went by", t - start >= 24000 * 30, (i64)(t - start));
    u64 t2 = poly_mach_now();
    poly_mach6(MACH_mach_wait_until_trap, (i64)start, 0, 0, 0, 0, 0);
    check("an old deadline returns at once", poly_mach_now() - t2 < 24000 * 5, 0);
    return finish();
}
#else
int test_main(void) { return finish(); }
#endif
