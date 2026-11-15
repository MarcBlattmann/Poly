/* getpid(2), BSD 20, and getppid(2), BSD 39: the parent's id is positive
 * and not the process's own. */
#include "poly_mtest.h"

int test_main(void) {
    sysret p = bsd0(SYS_getpid), pp = bsd0(SYS_getppid);
    check_ok("getppid succeeds", pp);
    check("the parent pid is positive", pp.ret > 0, pp.ret);
    check("the parent pid is not the pid", pp.ret != p.ret, 0);
    return finish();
}
