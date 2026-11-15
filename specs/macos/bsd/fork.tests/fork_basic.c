/* fork(2), BSD 2: returns twice. The parent gets the child's pid in the
 * result and 0 in the second register; the child gets its own pid in the
 * result and 1 in the second register. The child's pid differs from the
 * parent's and its parent is the caller. */
#include "poly_mtest.h"

int test_main(void) {
    i64 parent = bsd0(SYS_getpid).ret;
    sysret f = bsd0(SYS_fork);
    if (f.ret2 == 1) {
        i64 me = bsd0(SYS_getpid).ret;
        int ok = f.err == 0 && f.ret == me && me != parent && bsd0(SYS_getppid).ret == parent;
        poly_exit(ok ? 0 : 1);
    }
    check_ok("fork succeeds in the parent", f);
    check("the parent's second register is 0", f.ret2 == 0, f.ret2);
    check("the child's pid is positive and not the parent's", f.ret > 0 && f.ret != parent, f.ret);
    int st = poly_wait(f.ret);
    check("the child: result is its own pid, not the parent's; its ppid is the parent", W_EXITED(st) && W_EXITSTATUS(st) == 0, st);
    return finish();
}
