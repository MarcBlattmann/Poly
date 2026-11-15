/* vfork, BSD 66, on arm64: not a working system call. A process that makes
 * the call is killed by SIGSYS (signal 12), as for any unimplemented call;
 * libSystem implements vfork with fork there. The test makes the call in
 * a child and looks at its wait status. */
#include "poly_mtest.h"

int test_main(void) {
    sysret f = bsd0(SYS_fork);
    if (f.ret2 == 1) {
        sysret r = bsd0(SYS_vfork);
        poly_exit(r.err ? 100 + (int)r.err : 50);
    }
    int st = poly_wait(f.ret);
    check("the caller is killed by SIGSYS", st == SIGSYS, st);
    return finish();
}
