/* PR_GET_NO_NEW_PRIVS (39) returns the flag (0 at the start) as the
 * result; any of arguments 2..5 non-zero is EINVAL. PR_SET_NO_NEW_PRIVS
 * (38, 1, 0, 0, 0) sets it and returns 0 (again: 0); it cannot be
 * cleared: arg2 0 or 2, or any of arguments 3..5 non-zero, is EINVAL. A
 * fork child inherits it, and it stays set across execve.
 */
#include "a_sys.h"

#define SYS_prctl 157
#define PR_SET_NO_NEW_PRIVS 38
#define PR_GET_NO_NEW_PRIVS 39

static i64 pr5(u64 op, u64 a, u64 b, u64 c, u64 d) { return syscall5(SYS_prctl, op, a, b, c, d); }

int test_main(void) {
    if (a_argc() > 3 && str_eq(a_argv()[1], "child")) {
        passed = (int)a_atou(a_argv()[2]);
        failed = (int)a_atou(a_argv()[3]);
        expect("after_execve_still_1", pr5(PR_GET_NO_NEW_PRIVS, 0, 0, 0, 0), 1);
        return finish();
    }
    a_block_sigchld();
    expect("initial_0", pr5(PR_GET_NO_NEW_PRIVS, 0, 0, 0, 0), 0);
    expect("get_arg2_einval", pr5(PR_GET_NO_NEW_PRIVS, 1, 0, 0, 0), -EINVAL);
    expect("get_arg5_einval", pr5(PR_GET_NO_NEW_PRIVS, 0, 0, 0, 1), -EINVAL);
    expect("set_0_einval", pr5(PR_SET_NO_NEW_PRIVS, 0, 0, 0, 0), -EINVAL);
    expect("set_2_einval", pr5(PR_SET_NO_NEW_PRIVS, 2, 0, 0, 0), -EINVAL);
    expect("set_arg3_einval", pr5(PR_SET_NO_NEW_PRIVS, 1, 1, 0, 0), -EINVAL);
    expect("set_arg5_einval", pr5(PR_SET_NO_NEW_PRIVS, 1, 0, 0, 1), -EINVAL);
    expect("still_0", pr5(PR_GET_NO_NEW_PRIVS, 0, 0, 0, 0), 0);
    expect("set_1", pr5(PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0), 0);
    expect("get_1", pr5(PR_GET_NO_NEW_PRIVS, 0, 0, 0, 0), 1);
    expect("set_1_again", pr5(PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0), 0);
    expect("clear_einval", pr5(PR_SET_NO_NEW_PRIVS, 0, 0, 0, 0), -EINVAL);

    i64 c = a_fork_sync();
    if (c == 0) syscall1(SYS_exit, (int)pr5(PR_GET_NO_NEW_PRIVS, 0, 0, 0, 0) + 10);
    i32 status = -1;
    syscall4(SYS_wait4, c, &status, 0, 0);
    expect("fork_child_inherits", A_WEXITSTATUS(status), 11);

    /* The new image carries on counting. */
    const char *path = a_self_path();
    char np[24], nf[24];
    a_utoa(np, (u64)passed);
    a_utoa(nf, (u64)failed);
    char *argv[] = { (char *)path, "child", np, nf, 0 };
    char *envp[] = { 0 };
    i64 r = syscall3(SYS_execve, path, argv, envp);
    check("execve", 0, r);
    return finish();
}
