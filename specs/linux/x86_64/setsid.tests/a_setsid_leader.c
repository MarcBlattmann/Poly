/* setsid: a process group leader cannot start a session (EPERM); a
 * child of a new session leader inherits the session. */
#include "a_sys.h"

/* The checks run in a child that leads a new session and process group
 * (a_in_session), so they do not depend on how the test was started. */
static int body(void) {
    a_block_sigchld();
    volatile i32 *sh = a_shared();
    i64 pid = a_fork_sync();
    if (pid == 0) {
        syscall2(SYS_setpgid, 0, 0);
        sh[0] = (i32)syscall0(SYS_setsid);
        syscall1(SYS_exit_group, 0);
    }
    i32 st;
    syscall4(SYS_wait4, pid, &st, 0, 0);
    expect("group_leader_eperm", sh[0], -EPERM);
    pid = a_fork_sync();
    if (pid == 0) {
        i64 sid = syscall0(SYS_setsid);
        i64 g = a_fork_sync();
        if (g == 0) {
            sh[1] = syscall1(SYS_getsid, 0) == sid;
            sh[2] = syscall0(SYS_getpgrp) == sid;
            sh[3] = syscall0(SYS_setsid) > 0;
            syscall1(SYS_exit_group, 0);
        }
        i32 s;
        syscall4(SYS_wait4, g, &s, 0, 0);
        syscall1(SYS_exit_group, 0);
    }
    syscall4(SYS_wait4, pid, &st, 0, 0);
    expect("grandchild_in_new_session", sh[1], 1);
    expect("grandchild_in_leaders_group", sh[2], 1);
    expect("grandchild_can_start_its_own", sh[3], 1);
    return finish();
}

int test_main(void) { return a_in_session(body); }
