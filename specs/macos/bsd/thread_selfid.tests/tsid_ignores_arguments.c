/* thread_selfid, BSD 372: arguments change nothing, and other calls in
 * between do not change the id. */
#include "poly_mtest.h"

int test_main(void) {
    sysret a = bsd0(SYS_thread_selfid);
    bsd1(SYS_close, bsd3(SYS_open, ".", O_RDONLY, 0).ret);
    sysret b = bsd6(SYS_thread_selfid, 1, 2, 3, 4, 5, 6);
    check_ok("thread_selfid with arguments succeeds", b);
    check("and gives the same id", a.ret == b.ret, 0);
    return finish();
}
