/* getpgrp follows setpgid and setsid in a child: the child's pid after
 * either. */
#include "a_sys.h"

/* The checks run in a child that leads a new session and process group
 * (a_in_session), so they do not depend on how the test was started. */
static int body(void) {
    a_block_sigchld();
    volatile i32 *sh = a_shared();
    i64 g = syscall0(SYS_getpgrp);
    i64 pid = a_fork_sync();
    if (pid == 0) {
        i64 me = syscall0(SYS_getpid);
        sh[0] = syscall0(SYS_getpgrp) == g;
        syscall2(SYS_setpgid, 0, 0);
        sh[1] = syscall0(SYS_getpgrp) == me;
        syscall2(SYS_setpgid, 0, g);
        sh[2] = syscall0(SYS_getpgrp) == g;
        syscall0(SYS_setsid);
        sh[3] = syscall0(SYS_getpgrp) == me;
        syscall1(SYS_exit_group, 0);
    }
    i32 st;
    syscall4(SYS_wait4, pid, &st, 0, 0);
    expect("inherited", sh[0], 1);
    expect("after_setpgid", sh[1], 1);
    expect("after_moving_back", sh[2], 1);
    expect("after_setsid", sh[3], 1);
    expect("parent_unchanged", syscall0(SYS_getpgrp), g);
    return finish();
}

int test_main(void) { return a_in_session(body); }
