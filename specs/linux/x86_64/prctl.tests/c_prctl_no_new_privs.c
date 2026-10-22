/* PR_SET_NO_NEW_PRIVS / PR_GET_NO_NEW_PRIVS: GET returns 0 or 1 and
 * needs arguments 2 to 5 all 0 (else EINVAL). SET needs arg2 exactly 1
 * and arguments 3 to 5 all 0 (else EINVAL); any user may set it, and
 * once set it cannot be cleared (SET 0 is EINVAL). Setting it again is
 * fine. A fork child starts with it set. The flag is checked in a fork
 * child so the test process keeps a clear flag.
 * poly-diff: syscalls=off (two processes)
 */
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_prctl 157
/* linux/prctl.h */
#define PR_SET_NO_NEW_PRIVS 38
#define PR_GET_NO_NEW_PRIVS 39

static i64 pr(i64 o, i64 a, i64 b, i64 c, i64 d) { return syscall5(SYS_prctl, o, a, b, c, d); }

static int child(void) {
    expect("set 1", pr(PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0), 0);
    expect("get 1", pr(PR_GET_NO_NEW_PRIVS, 0, 0, 0, 0), 1);
    expect("set 0 after set", pr(PR_SET_NO_NEW_PRIVS, 0, 0, 0, 0), -EINVAL);
    expect("set 1 again", pr(PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0), 0);
    expect("set 1, arg3 set again", pr(PR_SET_NO_NEW_PRIVS, 1, 1, 0, 0), -EINVAL);
    expect("still 1", pr(PR_GET_NO_NEW_PRIVS, 0, 0, 0, 0), 1);
    long g = a_fork_sync();
    if (g == 0) syscall1(SYS_exit, (int)pr(PR_GET_NO_NEW_PRIVS, 0, 0, 0, 0));
    int st = -1;
    syscall4(SYS_wait4, g, &st, 0, 0);
    check("fork child inherits it", !A_WIFSIGNALED(st) && A_WEXITSTATUS(st) == 1, st);
    return failed ? 1 : 0;
}

int test_main(void) {
    expect("starts 0", pr(PR_GET_NO_NEW_PRIVS, 0, 0, 0, 0), 0);
    expect("get, arg2 1", pr(PR_GET_NO_NEW_PRIVS, 1, 0, 0, 0), -EINVAL);
    expect("get, arg5 1", pr(PR_GET_NO_NEW_PRIVS, 0, 0, 0, 1), -EINVAL);
    expect("set 0", pr(PR_SET_NO_NEW_PRIVS, 0, 0, 0, 0), -EINVAL);
    expect("set 2", pr(PR_SET_NO_NEW_PRIVS, 2, 0, 0, 0), -EINVAL);
    expect("set 1<<32 | 1", pr(PR_SET_NO_NEW_PRIVS, (1L << 32) | 1, 0, 0, 0), -EINVAL);
    expect("set 1, arg3 1", pr(PR_SET_NO_NEW_PRIVS, 1, 1, 0, 0), -EINVAL);
    expect("set 1, arg4 1", pr(PR_SET_NO_NEW_PRIVS, 1, 0, 1, 0), -EINVAL);
    expect("set 1, arg5 1", pr(PR_SET_NO_NEW_PRIVS, 1, 0, 0, 1), -EINVAL);
    expect("failed sets leave 0", pr(PR_GET_NO_NEW_PRIVS, 0, 0, 0, 0), 0);

    long c = a_fork_sync();
    if (c == 0) exit_group(child());
    int st = -1;
    syscall4(SYS_wait4, c, &st, 0, 0);
    expect("child checks passed", st, 0);
    expect("parent still 0", pr(PR_GET_NO_NEW_PRIVS, 0, 0, 0, 0), 0);
    return finish();
}
