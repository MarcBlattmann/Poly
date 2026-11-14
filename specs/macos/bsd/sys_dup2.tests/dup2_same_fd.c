/* dup2, BSD 90: with both descriptors the same, an open one is returned
 * unchanged (and keeps its close-on-exec flag), a closed one is EBADF. */
#include "poly_mtest.h"

int test_main(void) {
    sysret a = bsd3(SYS_open, ".", O_RDONLY, 0);
    sysret r = bsd2(SYS_dup2, a.ret, a.ret);
    check("an open descriptor: returned", r.err == 0 && r.ret == a.ret, r.ret);
    bsd3(SYS_fcntl, a.ret, F_SETFD, FD_CLOEXEC);
    bsd2(SYS_dup2, a.ret, a.ret);
    check("close-on-exec is kept", bsd3(SYS_fcntl, a.ret, F_GETFD, 0).ret == FD_CLOEXEC, 0);
    check_err("a descriptor that is not open: EBADF", bsd2(SYS_dup2, 55, 55), EBADF);
    return finish();
}
