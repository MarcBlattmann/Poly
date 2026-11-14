/* pipe, BSD 42: writing after the read end was closed raises SIGPIPE; with
 * SIGPIPE ignored the write fails with EPIPE (32); with the default action a
 * child that does it is killed by signal 13. */
#include "poly_mtest.h"

int test_main(void) {
    sysret f = bsd0(SYS_fork);
    if (f.ret2 == 1) {
        sysret p = bsd0(SYS_pipe);
        bsd1(SYS_close, p.ret);
        bsd3(SYS_write, p.ret2, "x", 1);
        poly_exit(42);
    }
    int st = poly_wait(f.ret);
    check("default action: the child is killed by SIGPIPE", st == SIGPIPE, st);
    poly_sigaction(SIGPIPE, SIG_IGN, 0, 0, 0);
    sysret p = bsd0(SYS_pipe);
    bsd1(SYS_close, p.ret);
    check_err("SIGPIPE ignored: EPIPE", bsd3(SYS_write, p.ret2, "x", 1), EPIPE);
    return finish();
}
