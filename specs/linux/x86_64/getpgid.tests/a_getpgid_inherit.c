/* getpgid: a forked child inherits its parent's group, also when the
 * parent has just made a new group of its own. */
#include "a_sys.h"

/* The checks run in a child that leads a new session and process group
 * (a_in_session), so they do not depend on how the test was started. */
static int body(void) {
    a_block_sigchld();
    volatile i32 *sh = a_shared();
    i64 pid = a_fork_sync();
    if (pid == 0) {
        syscall2(SYS_setpgid, 0, 0);
        i64 me = syscall0(SYS_getpid);
        i64 g = a_fork_sync();
        if (g == 0) {
            sh[0] = syscall1(SYS_getpgid, 0) == me;
            sh[1] = syscall1(SYS_getpgid, syscall0(SYS_getppid)) == me;
            syscall1(SYS_exit_group, 0);
        }
        i32 s;
        syscall4(SYS_wait4, g, &s, 0, 0);
        syscall1(SYS_exit_group, 0);
    }
    i32 st;
    syscall4(SYS_wait4, pid, &st, 0, 0);
    expect("grandchild_inherits_new_group", sh[0], 1);
    expect("grandchild_sees_parent_group", sh[1], 1);
    return finish();
}

int test_main(void) { return a_in_session(body); }
