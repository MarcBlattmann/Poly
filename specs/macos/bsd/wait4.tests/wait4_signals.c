/* wait4(2), BSD 7, children that do not exit: killed by a signal the
 * status is the signal number (15, 9); a child stopped by SIGSTOP is only
 * reported with WUNTRACED, as 0x117f (stop signal 17 in bits 8..15, 0x7f in
 * the low byte); WNOHANG returns 0 while the child runs. */
#include "poly_mtest.h"

static i64 spinner(void) {
    sysret f = bsd0(SYS_fork);
    if (f.ret2 == 1) {
        for (;;) bsd1(SYS_sigsuspend, 0);
    }
    return f.ret;
}

int test_main(void) {
    int st = -1;
    i64 pid = spinner();
    sysret r = bsd4(SYS_wait4, pid, &st, WNOHANG, 0);
    check("WNOHANG while the child runs: 0", r.err == 0 && r.ret == 0, r.err ? r.err : r.ret);
    bsd2(SYS_kill, pid, SIGTERM);
    r = bsd4(SYS_wait4, pid, &st, 0, 0);
    check("killed by SIGTERM: status 15", r.ret == pid && st == SIGTERM, st);
    pid = spinner();
    bsd2(SYS_kill, pid, SIGKILL);
    r = bsd4(SYS_wait4, pid, &st, 0, 0);
    check("killed by SIGKILL: status 9", r.ret == pid && st == SIGKILL, st);
    pid = spinner();
    bsd2(SYS_kill, pid, SIGSTOP);
    r = bsd4(SYS_wait4, pid, &st, WUNTRACED, 0);
    check("stopped by SIGSTOP with WUNTRACED: 0x117f", r.ret == pid && st == 0x117f, st);
    check("WIFSTOPPED and the stop signal", W_STOPPED(st) && W_STOPSIG(st) == SIGSTOP, st);
    bsd2(SYS_kill, pid, SIGKILL);
    r = bsd4(SYS_wait4, pid, &st, 0, 0);
    check("a stopped child can still be killed: status 9", r.ret == pid && st == SIGKILL, st);
    return finish();
}
