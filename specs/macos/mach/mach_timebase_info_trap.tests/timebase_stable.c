/* mach_timebase_info_trap, Mach trap 89: the same ratio on every call. */
#include "poly_mtest.h"

int test_main(void) {
    struct poly_timebase a = {0, 0}, b = {1, 1};
    mach1(MACH_mach_timebase_info_trap, &a);
    i64 kr = mach1(MACH_mach_timebase_info_trap, &b);
    check("KERN_SUCCESS again", kr == KERN_SUCCESS, kr);
    check("the same numer and denom", a.numer == b.numer && a.denom == b.denom, 0);
    return finish();
}
