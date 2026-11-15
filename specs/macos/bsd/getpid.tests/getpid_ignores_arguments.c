/* getpid(2), BSD 20: arguments in the argument registers change nothing. */
#include "poly_mtest.h"

int test_main(void) {
    sysret a = bsd0(SYS_getpid);
    sysret b = bsd6(SYS_getpid, 1, 2, 3, 4, 5, 6);
    check_ok("getpid with arguments succeeds", b);
    check("and gives the same pid", a.ret == b.ret, 0);
    return finish();
}
