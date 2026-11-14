/* bsdthread_register, BSD 366: libSystem registers the thread entry points
 * when a program starts, and a second registration is refused with EINVAL
 * (22), whatever the arguments are. */
#include "poly_mtest.h"

int test_main(void) {
    check_err("all arguments 0: EINVAL", bsd6(SYS_bsdthread_register, 0, 0, 0, 0, 0, 0), EINVAL);
    check_err("nonzero entry points: EINVAL", bsd6(SYS_bsdthread_register, 0x1000, 0x1000, 0, 0, 0, 0), EINVAL);
    check_err("with flags: EINVAL", bsd6(SYS_bsdthread_register, 0x1000, 0x1000, 0x1, 0, 0, 0), EINVAL);
    return finish();
}
