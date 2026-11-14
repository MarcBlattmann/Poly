/* thread_selfid, BSD 372: a thread id is not the process id. */
#include "poly_mtest.h"

int test_main(void) {
    sysret t = bsd0(SYS_thread_selfid), p = bsd0(SYS_getpid);
    check("the thread id is not the pid", t.ret != p.ret, 0);
    return finish();
}
