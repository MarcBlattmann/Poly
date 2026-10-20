/* setpgid: a session leader cannot change its group (EPERM); a process
 * cannot move into a group of another session (EPERM). */
#include "a_sys.h"

/* The checks run in a child that leads a new session and process group
 * (a_in_session), so they do not depend on how the test was started. */
static int body(void) {
    a_block_sigchld();
    volatile i32 *sh = a_shared();
    i64 parent_group = syscall0(SYS_getpgrp);
    i64 pid = a_fork_sync();
    if (pid == 0) {
        sh[0] = syscall0(SYS_setsid) == syscall0(SYS_getpid);
        sh[1] = (i32)syscall2(SYS_setpgid, 0, 0);
        sh[2] = (i32)syscall2(SYS_setpgid, 0, parent_group);
        /* A child of the session leader cannot join the old session's group. */
        i64 g = a_fork_sync();
        if (g == 0) {
            sh[3] = (i32)syscall2(SYS_setpgid, 0, parent_group);
            sh[4] = (i32)syscall2(SYS_setpgid, 0, 0);
            syscall1(SYS_exit_group, 0);
        }
        i32 s;
        syscall4(SYS_wait4, g, &s, 0, 0);
        syscall1(SYS_exit_group, 0);
    }
    i32 st;
    syscall4(SYS_wait4, pid, &st, 0, 0);
    expect("setsid_ok", sh[0], 1);
    expect("session_leader_setpgid_eperm", sh[1], -EPERM);
    expect("leader_to_other_session_eperm", sh[2], -EPERM);
    expect("other_session_group_eperm", sh[3], -EPERM);
    expect("new_group_in_own_session_ok", sh[4], 0);
    return finish();
}

int test_main(void) { return a_in_session(body); }
