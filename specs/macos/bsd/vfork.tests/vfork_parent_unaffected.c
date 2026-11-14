/* vfork, BSD 66, on arm64: the parent of the process that made the call
 * is not affected and can wait for it (no shared memory, no suspension). */
#include "poly_mtest.h"

static int marker = 0;

int test_main(void) {
    sysret f = bsd0(SYS_fork);
    if (f.ret2 == 1) {
        marker = 1;
        bsd0(SYS_vfork);
        poly_exit(0);
    }
    int st = poly_wait(f.ret);
    check("the child was killed by SIGSYS", W_TERMSIG(st) == SIGSYS, st);
    check("the parent's memory is untouched", marker == 0, marker);
    check("and the parent still has no children", bsd4(SYS_wait4, -1, 0, WNOHANG, 0).err == ECHILD, 0);
    return finish();
}
