/* getsid: a child is in its parent's session until it calls setsid; its
 * children follow it. */
#include "a_sys.h"

/* The checks run in a child that leads a new session and process group
 * (a_in_session), so they do not depend on how the test was started. */
static int body(void) {
    a_block_sigchld();
    volatile i32 *sh = a_shared();
    i64 s = syscall1(SYS_getsid, 0);
    i64 pid = a_fork_sync();
    if (pid == 0) {
        sh[0] = syscall1(SYS_getsid, 0) == s;
        sh[1] = syscall1(SYS_getsid, syscall0(SYS_getppid)) == s;
        i64 n = syscall0(SYS_setsid);
        sh[2] = syscall1(SYS_getsid, 0) == n;
        i64 g = a_fork_sync();
        if (g == 0) {
            sh[3] = syscall1(SYS_getsid, 0) == n;
            syscall1(SYS_exit_group, 0);
        }
        i32 st;
        syscall4(SYS_wait4, g, &st, 0, 0);
        syscall1(SYS_exit_group, 0);
    }
    i32 st;
    syscall4(SYS_wait4, pid, &st, 0, 0);
    expect("child_inherits_session", sh[0], 1);
    expect("child_sees_parent_session", sh[1], 1);
    expect("after_setsid_own", sh[2], 1);
    expect("grandchild_follows", sh[3], 1);
    return finish();
}

int test_main(void) { return a_in_session(body); }
