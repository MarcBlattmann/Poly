/* bsdthread_register, BSD 366: a forked child inherits the registration:
 * also EINVAL there. */
#include "poly_mtest.h"

int test_main(void) {
    sysret f = bsd0(SYS_fork);
    if (f.ret2 == 1) {
        sysret r = bsd6(SYS_bsdthread_register, 0, 0, 0, 0, 0, 0);
        poly_exit(r.err == EINVAL ? 0 : 1);
    }
    int st = poly_wait(f.ret);
    check("the child: EINVAL", W_EXITED(st) && W_EXITSTATUS(st) == 0, st);
    return finish();
}
