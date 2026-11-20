// poly-mac: libobjc
// objc_autoreleasePoolPop with a null token is not accepted: the process dies by a signal
// (observed in a child whose stderr is discarded); a pop with the token of the only pool is fine.
#include "poly_mobjc.h"
static i64 start_quiet(const char *mode) {
    char *argv[3] = {poly_argv[0], (char *)mode, 0};
    char *noenv[1] = {0};
    sysret f = bsd0(SYS_fork);
    if (f.ret2 == 1) {
        sysret fd = bsd3(SYS_open, "/dev/null", O_WRONLY, 0);
        bsd3(SYS_dup2, fd.ret, 2, 0);
        bsd3(SYS_execve, poly_argv[0], argv, noenv);
        poly_exit(126);
    }
    return f.ret;
}
int test_main(void) {
    if (is_child("null")) {
        objc_autoreleasePoolPop(0);
        return 0;
    }
    int st = poly_wait(start_quiet("null"));
    check("the child was killed by a signal", !W_EXITED(st), st);
    value("signal", W_TERMSIG(st));
    void *p = objc_autoreleasePoolPush();
    objc_autoreleasePoolPop(p);
    value("a normal pop afterwards", 1);
    return finish();
}
