#include "poly_mtest.h"
/* wait4, BSD 7: which child is waited for. pid > 0: that child only (another child that exited earlier stays);
 * pid -1: any child; pid 0: any child in the caller's process group; pid < -1: any child of that process
 * group (-pgid); a child that moved to a group of its own is not matched by 0 (ECHILD). With several exited children, each wait returns one and they are all eventually
 * returned. The rusage pointer (4th argument) is filled in for a reaped child (ru_utime and ru_stime
 * seconds are small) and an unwritable one is EFAULT without reaping. A zombie still exists until
 * waited for: kill(pid, 0) succeeds; afterwards it is ESRCH (3). */
#define SYS_setpgid 82
#define SYS_getpgid 151
static void snooze(void) { for (volatile int i = 0; i < 4000000; i++) {} }
static i64 kid(int code) {
    sysret f = bsd0(SYS_fork);
    if (f.ret2 == 1) poly_exit(code);
    return f.ret;
}
int test_main(void) {
    int st = 0;
    i64 a = kid(1), b = kid(2), c = kid(3);
    snooze();
    sysret r = bsd4(SYS_wait4, b, &st, 0, 0);
    check("pid > 0 waits for that child: b with status 2", r.err == 0 && r.ret == b && W_EXITSTATUS(st) == 2, r.err ? r.err : r.ret);
    check("a zombie exists until waited for: kill(a, 0) works", bsd2(SYS_kill, a, 0).err == 0, 0);
    r = bsd4(SYS_wait4, b, &st, 0, 0);
    check_err("b is gone now: ECHILD", r, ECHILD);
    r = bsd4(SYS_wait4, -1, &st, 0, 0);
    i64 first = r.ret;
    check("pid -1: one of the others", r.err == 0 && (first == a || first == c), r.err ? r.err : first);
    r = bsd4(SYS_wait4, -1, &st, 0, 0);
    check("and the remaining one", r.err == 0 && r.ret != first && (r.ret == a || r.ret == c), r.err ? r.err : r.ret);
    check("both were returned, none twice", (first == a && r.ret == c) || (first == c && r.ret == a), 0);
    check_err("kill of a reaped child is ESRCH", bsd2(SYS_kill, a, 0), ESRCH);
    check_err("no children left: ECHILD", bsd4(SYS_wait4, -1, &st, 0, 0), ECHILD);
    i64 pg = bsd1(SYS_getpgid, 0).ret;
    i64 d = kid(4);
    snooze();
    r = bsd4(SYS_wait4, 0, &st, 0, 0);
    check("pid 0: a child in the caller's group", r.err == 0 && r.ret == d && W_EXITSTATUS(st) == 4, r.err ? r.err : r.ret);
    d = kid(5);
    snooze();
    r = bsd4(SYS_wait4, -pg, &st, 0, 0);
    check("pid -pgid: a child of that group", r.err == 0 && r.ret == d && W_EXITSTATUS(st) == 5, r.err ? r.err : r.ret);
    sysret f = bsd0(SYS_fork);
    if (f.ret2 == 1) {
        bsd2(SYS_setpgid, 0, 0);
        for (volatile long i = 0; i < 60000000L; i++) {}
        poly_exit(6);
    }
    snooze();
    r = bsd4(SYS_wait4, 0, &st, WNOHANG, 0);
    check_err("pid 0 does not match a child that left the group: ECHILD", r, ECHILD);
    r = bsd4(SYS_wait4, -(f.ret), &st, 0, 0);
    check("pid -pgid of the child's own group finds it", r.err == 0 && r.ret == f.ret && W_EXITSTATUS(st) == 6, r.err ? r.err : r.ret);
    d = kid(7);
    snooze();
    r = bsd4(SYS_wait4, d, &st, 0, (void *)8);
    check_err("an unwritable rusage pointer is EFAULT", r, EFAULT);
    char ru[144];
    memset(ru, 0xff, sizeof ru);
    r = bsd4(SYS_wait4, d, &st, 0, ru);
    check("the child is still there after the refused call and is reaped now", r.err == 0 && r.ret == d, r.err ? r.err : r.ret);
    i64 utime_sec = *(i64 *)ru;
    check("the rusage was filled in: ru_utime seconds is small (< 5)", utime_sec >= 0 && utime_sec < 5, utime_sec);
    return finish();
}
