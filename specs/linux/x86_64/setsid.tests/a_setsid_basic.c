/* setsid in a child (not a group leader): returns the child's pid, which
 * becomes its session id and process group; a second setsid is EPERM. */
#include "a_sys.h"

/* The checks run in a child that leads a new session and process group
 * (a_in_session), so they do not depend on how the test was started. */
static int body(void) {
    a_block_sigchld();
    volatile i32 *sh = a_shared();
    i64 pid = a_fork_sync();
    if (pid == 0) {
        i64 me = syscall0(SYS_getpid);
        sh[0] = syscall0(SYS_setsid) == me;
        sh[1] = syscall1(SYS_getsid, 0) == me;
        sh[2] = syscall0(SYS_getpgrp) == me;
        sh[3] = (i32)syscall0(SYS_setsid);
        syscall1(SYS_exit_group, 0);
    }
    i32 st;
    syscall4(SYS_wait4, pid, &st, 0, 0);
    expect("returns_pid", sh[0], 1);
    expect("session_is_pid", sh[1], 1);
    expect("group_is_pid", sh[2], 1);
    expect("second_setsid_eperm", sh[3], -EPERM);
    return finish();
}

int test_main(void) { return a_in_session(body); }
