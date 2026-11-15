/* wait4(2), BSD 7 (pid, status pointer, options, rusage): returns the pid
 * of the child that changed state and stores the wait status, the exit
 * code in bits 8..15 (exit 7: 0x700); the child is gone afterwards. */
#include "poly_mtest.h"

int test_main(void) {
    int codes[3] = {0, 7, 255};
    for (int i = 0; i < 3; i++) {
        sysret f = bsd0(SYS_fork);
        if (f.ret2 == 1) poly_exit(codes[i]);
        int st = -1;
        sysret w = bsd4(SYS_wait4, f.ret, &st, 0, 0);
        check("wait4 returns the child's pid", w.err == 0 && w.ret == f.ret, w.err ? w.err : w.ret);
        check("a normal exit: status is code << 8", st == codes[i] << 8, st);
        check_err("the child is gone: ECHILD", bsd4(SYS_wait4, f.ret, &st, 0, 0), ECHILD);
    }
    sysret a = bsd0(SYS_fork);
    if (a.ret2 == 1) poly_exit(1);
    sysret w = bsd4(SYS_wait4, -1, 0, 0, 0);
    check("pid -1 with a NULL status pointer reaps any child", w.err == 0 && w.ret == a.ret, w.ret);
    return finish();
}
