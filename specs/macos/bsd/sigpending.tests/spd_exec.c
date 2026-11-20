#include "poly_mtest.h"
/* sigpending, BSD 52: what an exec does to pending and blocked signals. The signal mask survives
 * execve (blocked stays blocked); a signal that was pending before the exec is still pending
 * afterwards. The re-executed child reports its mask and its pending set. */
static u32 pending(void) { u32 p = 0xffffffff; bsd1(SYS_sigpending, &p); return p; }
int test_main(void) {
    if (is_child("child")) {
        put("child mask="); put_num(poly_sigmask());
        put(" pending="); put_num(pending());
        put("\n");
        poly_exit(0);
    }
    u32 m = SIGBIT(SIGUSR1) | SIGBIT(SIGHUP);
    bsd3(SYS_sigprocmask, SIG_BLOCK, &m, 0);
    bsd2(SYS_kill, bsd0(SYS_getpid).ret, SIGUSR1);
    check("pending before the exec", pending() == SIGBIT(SIGUSR1), pending());
    i64 c = poly_start_self("child", 0, 0);
    int st = poly_wait(c);
    check("the child ran", W_EXITED(st) && W_EXITSTATUS(st) == 0, st);
    return finish();
}
