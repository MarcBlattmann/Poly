/* close(2), BSD 6: the lowest closed descriptor is the next one open(2)
 * returns, also descriptor 0. */
#include "poly_mtest.h"

int test_main(void) {
    sysret a = bsd3(SYS_open, ".", O_RDONLY, 0);
    sysret b = bsd3(SYS_open, ".", O_RDONLY, 0);
    check("two opens give 3 and 4", a.ret == 3 && b.ret == 4, a.ret * 100 + b.ret);
    bsd1(SYS_close, a.ret);
    sysret c = bsd3(SYS_open, ".", O_RDONLY, 0);
    check("after closing 3, open gives 3", c.ret == 3, c.ret);
    check_ok("close stdin", bsd1(SYS_close, 0));
    sysret d = bsd3(SYS_open, ".", O_RDONLY, 0);
    check("after closing 0, open gives 0", d.ret == 0, d.ret);
    return finish();
}
