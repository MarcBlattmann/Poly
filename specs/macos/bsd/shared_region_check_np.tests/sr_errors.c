/* shared_region_check_np, BSD 294, errors: EFAULT for a pointer that cannot
 * be written (address 8), after which calls work as before; a NULL pointer
 * returns 0 and stores nothing, but afterwards every call in that process
 * fails with EINVAL (observed: a good call right after it, and again after a
 * second one). */
#include "poly_mtest.h"

int test_main(void) {
    u64 a = 0, b = 0;
    check_ok("a good call", bsd1(SYS_shared_region_check_np, &a));
    check_err("the pointer at address 8: EFAULT", bsd1(SYS_shared_region_check_np, (void *)8), EFAULT);
    check_ok("a good call after the failed one", bsd1(SYS_shared_region_check_np, &b));
    check("with the same address", a == b && a != 0, b);
    sysret r = bsd1(SYS_shared_region_check_np, 0);
    check("a NULL pointer: returns 0", r.err == 0 && r.ret == 0, r.err);
    check_err("a good call after the NULL one: EINVAL", bsd1(SYS_shared_region_check_np, &b), EINVAL);
    check_err("and again: EINVAL", bsd1(SYS_shared_region_check_np, &b), EINVAL);
    return finish();
}
