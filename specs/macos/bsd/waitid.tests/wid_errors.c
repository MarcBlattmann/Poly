#include "poly_mtest.h"
/* waitid, BSD 173: argument errors, each on a fresh exited child. No option at all is EINVAL (22);
 * an option nobody defined is EINVAL; WNOHANG alone (no WEXITED, WSTOPPED or WCONTINUED) is accepted
 * and returns 0 without reporting anything; an unwritable or NULL siginfo is EFAULT (14) and the
 * child stays waitable afterwards; an idtype that does not exist (7) acts like P_ALL: it reaps a
 * child and returns 0; a pid that is not a child is ECHILD (10), a process group that is not the
 * children's too, a negative group EINVAL. */
#define SYS_waitid 173
#define WEXITED 4
#define WNOWAIT 0x20
struct si { i32 signo, err, code, pid; u32 uid; i32 status; void *addr; i64 val; i64 band; i64 pad[7]; };
static i64 kid(int code) {
    sysret f = bsd0(SYS_fork);
    if (f.ret2 == 1) poly_exit(code);
    for (volatile int i = 0; i < 3000000; i++) {}
    return f.ret;
}
int test_main(void) {
    struct si s;
    i64 c = kid(3);
    check_err("no option is EINVAL", bsd4(SYS_waitid, 1, c, &s, 0), EINVAL);
    check_err("an undefined option is EINVAL", bsd4(SYS_waitid, 1, c, &s, WEXITED | 0x8000000), EINVAL);
    memset(&s, 0x55, sizeof s);
    sysret r = bsd4(SYS_waitid, 0, 0, &s, WNOHANG);
    check("WNOHANG alone: 0, nothing reported", r.err == 0 && r.ret == 0 && s.signo == 0x55555555, r.err);
    r = bsd4(SYS_waitid, 1, c, (void *)8, WEXITED | WNOWAIT);
    check_err("an unwritable siginfo is EFAULT", r, EFAULT);
    r = bsd4(SYS_waitid, 1, c, 0, WEXITED | WNOWAIT);
    check_err("a NULL siginfo is EFAULT", r, EFAULT);
    r = bsd4(SYS_waitid, 1, c, &s, WEXITED);
    check("the child was not lost: it is reaped now", r.err == 0 && s.pid == c && s.status == 3, r.err);
    c = kid(4);
    memset(&s, 0x55, sizeof s);
    r = bsd4(SYS_waitid, 7, 0, &s, WEXITED);
    check("an unknown idtype: 0 and the child is reported like P_ALL", r.err == 0 && r.ret == 0 && s.pid == c && s.status == 4, r.err);
    check_err("the child is reaped by it", bsd4(SYS_waitid, 1, c, &s, WEXITED), ECHILD);
    c = kid(5);
    check_err("a pid that is not a child is ECHILD", bsd4(SYS_waitid, 1, c + 100000, &s, WEXITED), ECHILD);
    check_err("a process group that is not the children's is ECHILD", bsd4(SYS_waitid, 2, 1, &s, WEXITED), ECHILD);
    r = bsd4(SYS_waitid, 1, c, &s, WEXITED);
    check("and the real child is still there", r.err == 0 && s.status == 5, r.err);
    return finish();
}
