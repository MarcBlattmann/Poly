/* shared_region_check_np, BSD 294 (pointer to a u64): stores the address
 * where the shared cache of system libraries is mapped in the process and
 * returns 0; the address is nonzero and page aligned. The address is a
 * value (the cache's fixed place on this system). */
#include "poly_mtest.h"

int test_main(void) {
    u64 a = 0;
    sysret r = bsd1(SYS_shared_region_check_np, &a);
    check("returns 0", r.err == 0 && r.ret == 0, r.err);
    check("the address is nonzero", a != 0, a);
    check("and 4096-byte aligned", a % 4096 == 0, a % 4096);
    value("shared cache address", (i64)a);
    return finish();
}
