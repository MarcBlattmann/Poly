/* mach_timebase_info_trap, Mach trap 89: a NULL or unmapped pointer is not
 * reported: the trap still returns KERN_SUCCESS (and works afterwards). */
#include "poly_mtest.h"

int test_main(void) {
    check("NULL: KERN_SUCCESS", mach1(MACH_mach_timebase_info_trap, 0) == KERN_SUCCESS, 0);
    check("address 8: KERN_SUCCESS", mach1(MACH_mach_timebase_info_trap, 8) == KERN_SUCCESS, 0);
    struct poly_timebase tb = {0, 0};
    i64 kr = mach1(MACH_mach_timebase_info_trap, &tb);
    check("a good pointer afterwards", kr == KERN_SUCCESS && tb.denom != 0, kr);
    return finish();
}
