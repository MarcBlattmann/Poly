/* getpgid of a child: the parent's group until the child makes its own,
 * then the child's pid; a zombie child still answers. */
#include "a_sys.h"

static volatile i32 go, moved, done;

static int child(void *arg) {
    (void)arg;
    while (!go) a_relax();
    syscall2(SYS_setpgid, 0, 0);
    moved = 1;
    while (go != 2) a_relax();
    return 0;
}

/* The checks run in a child that leads a new session and process group
 * (a_in_session), so they do not depend on how the test was started. */
static int body(void) {
    a_block_sigchld();
    long pid = a_spawn(0, &done, child, 0);
    expect("child_in_parent_group", syscall1(SYS_getpgid, pid), syscall0(SYS_getpgrp));
    go = 1;
    a_spin_until(&moved, 1, A_SPIN_LIMIT);
    expect("child_own_group", syscall1(SYS_getpgid, pid), pid);
    go = 2;
    a_spin_until(&done, 0, A_SPIN_LIMIT);
    i32 st;
    /* Peek at the zombie without reaping it. */
    struct a_siginfo si;
    syscall5(SYS_waitid, P_PID, pid, &si, WEXITED | WNOWAIT, 0);
    expect("zombie_group", syscall1(SYS_getpgid, pid), pid);
    syscall4(SYS_wait4, pid, &st, 0, 0);
    expect("reaped_esrch", syscall1(SYS_getpgid, pid), -ESRCH);
    return finish();
}

int test_main(void) { return a_in_session(body); }
