/* getpgrp in a vfork child is the parent's group; the child's own group
 * change is not the parent's. */
#include "a_sys.h"

static volatile i64 before, after;

/* The checks run in a child that leads a new session and process group
 * (a_in_session), so they do not depend on how the test was started. */
static int body(void) {
    a_block_sigchld();
    i64 g = syscall0(SYS_getpgrp);
    i64 pid = syscall0(SYS_vfork);
    if (pid == 0) {
        before = syscall0(SYS_getpgrp);
        syscall2(SYS_setpgid, 0, 0);
        after = syscall0(SYS_getpgrp);
        syscall1(SYS_exit_group, 0);
    }
    i32 st;
    syscall4(SYS_wait4, pid, &st, 0, 0);
    expect("vfork_child_inherits", before, g);
    expect("vfork_child_own_group", after, pid);
    expect("parent_group_kept", syscall0(SYS_getpgrp), g);
    return finish();
}

int test_main(void) { return a_in_session(body); }
