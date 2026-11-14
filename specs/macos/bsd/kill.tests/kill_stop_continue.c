/* kill(2), BSD 37: SIGSTOP stops a child (reported to wait4 with WUNTRACED
 * as 0x117f), SIGCONT lets it run again and is not reported as a stop; a
 * process group number as a negative pid reaches every member. */
#include "poly_mtest.h"

int test_main(void) {
    sysret f = bsd0(SYS_fork);
    if (f.ret2 == 1) {
        for (;;) bsd1(SYS_sigsuspend, 0);
    }
    int st = -1;
    check_ok("SIGSTOP", bsd2(SYS_kill, f.ret, SIGSTOP));
    sysret r = bsd4(SYS_wait4, f.ret, &st, WUNTRACED, 0);
    check("reported as stopped: 0x117f", r.ret == f.ret && st == 0x117f, st);
    r = bsd4(SYS_wait4, f.ret, &st, WUNTRACED | WNOHANG, 0);
    check("the stop is reported once: then 0", r.err == 0 && r.ret == 0, r.ret);
    check_ok("SIGCONT", bsd2(SYS_kill, f.ret, SIGCONT));
    r = bsd4(SYS_wait4, f.ret, &st, WUNTRACED | WNOHANG, 0);
    check("the continued child is running: 0", r.err == 0 && r.ret == 0, r.ret);
    bsd2(SYS_kill, f.ret, SIGKILL);
    st = poly_wait(f.ret);
    check("then killed: status 9", st == SIGKILL, st);
    return finish();
}
