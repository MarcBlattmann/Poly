#include "poly_mtest.h"
/* waitid, BSD 173: stopped and continued children. WSTOPPED (8) reports a stopped child once: si_code
 * CLD_STOPPED (5), si_status the stop signal (SIGSTOP 17); asked again with WNOHANG (1) nothing is
 * reported and the siginfo is left as it was. WCONTINUED (0x10) reports a continued child: si_code
 * CLD_CONTINUED (6), si_status SIGCONT (19). WNOHANG with a child that has nothing to report returns
 * 0 and does not touch the siginfo. */
#define SYS_waitid 173
#define WEXITED 4
#define WSTOPPED 8
#define WCONTINUED 0x10
struct si { i32 signo, err, code, pid; u32 uid; i32 status; void *addr; i64 val; i64 band; i64 pad[7]; };
static void snooze(void) { for (volatile int i = 0; i < 4000000; i++) {} }
int test_main(void) {
    struct si s;
    sysret f = bsd0(SYS_fork);
    if (f.ret2 == 1) { for (;;) bsd0(SYS_getpid); }
    i64 c = f.ret;
    memset(&s, 0x55, sizeof s);
    sysret r = bsd4(SYS_waitid, 1, c, &s, WEXITED | WNOHANG);
    check("WNOHANG with a running child: result 0", r.err == 0 && r.ret == 0, r.err);
    check("the siginfo is not touched", s.signo == 0x55555555 && s.pid == 0x55555555, s.signo);
    bsd2(SYS_kill, c, SIGSTOP);
    snooze();
    memset(&s, 0x55, sizeof s);
    r = bsd4(SYS_waitid, 1, c, &s, WSTOPPED);
    check_ok("WSTOPPED reports the stop", r);
    check("CLD_STOPPED, si_status SIGSTOP, si_pid the child, si_signo SIGCHLD", s.code == 5 && s.status == SIGSTOP && s.pid == c && s.signo == SIGCHLD, s.code);
    memset(&s, 0x55, sizeof s);
    r = bsd4(SYS_waitid, 1, c, &s, WSTOPPED | WNOHANG);
    check("the stop is reported once: then 0 and untouched", r.err == 0 && r.ret == 0 && s.signo == 0x55555555, r.err);
    bsd2(SYS_kill, c, SIGCONT);
    memset(&s, 0x55, sizeof s);
    r = bsd4(SYS_waitid, 1, c, &s, WCONTINUED);
    check_ok("WCONTINUED reports the continue", r);
    check("CLD_CONTINUED, si_status SIGCONT", s.code == 6 && s.status == SIGCONT && s.pid == c, s.code);
    memset(&s, 0x55, sizeof s);
    r = bsd4(SYS_waitid, 1, c, &s, WCONTINUED | WNOHANG);
    check("the continue is reported once", r.err == 0 && r.ret == 0 && s.signo == 0x55555555, r.err);
    bsd2(SYS_kill, c, SIGKILL);
    r = bsd4(SYS_waitid, 1, c, &s, WEXITED);
    check("then it can be killed and reaped", r.err == 0 && s.code == 2 && s.status == SIGKILL, r.err);
    return finish();
}
