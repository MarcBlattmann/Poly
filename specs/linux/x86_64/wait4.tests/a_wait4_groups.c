/* wait4: pid 0 waits for children in the caller's process group, -pgid
 * for children in that group; children whose exit signal is not SIGCHLD
 * are found only with __WCLONE or __WALL. */
#include "a_sys.h"

static volatile i32 done;

static int quiet(void *arg) {
    (void)arg;
    return 4;
}

/* The checks run in a child that leads a new session and process group
 * (a_in_session), so they do not depend on how the test was started. */
static int body(void) {
    a_block_sigchld();
    i32 st;
    i64 pid = a_fork_sync();
    if (pid == 0) {
        syscall2(SYS_setpgid, 0, 0);
        syscall1(SYS_exit_group, 6);
    }
    expect("other_group_not_matched_by_0", syscall4(SYS_wait4, 0, &st, WNOHANG, 0), -ECHILD);
    expect("matched_by_minus_pgid", syscall4(SYS_wait4, -pid, &st, 0, 0), pid);
    expect("its_status", st, 6 << 8);
    pid = a_fork_sync();
    if (pid == 0) syscall1(SYS_exit_group, 8);
    expect("same_group_matched_by_0", syscall4(SYS_wait4, 0, &st, 0, 0), pid);
    /* A "clone" child: exit signal 0. */
    done = -1;
    long c = a_clone(CLONE_VM | CLONE_CHILD_CLEARTID, a_stacks[0] + A_STACK_SIZE, 0, (i32 *)&done, 0, quiet, 0);
    a_spin_until(&done, 0, A_SPIN_LIMIT);
    expect("clone_child_not_seen_plainly", syscall4(SYS_wait4, c, &st, WNOHANG, 0), -ECHILD);
    expect("clone_child_with_wclone", syscall4(SYS_wait4, c, &st, __WCLONE, 0), c);
    expect("clone_child_status", st, 4 << 8);
    done = -1;
    c = a_clone(CLONE_VM | CLONE_CHILD_CLEARTID, a_stacks[0] + A_STACK_SIZE, 0, (i32 *)&done, 0, quiet, 0);
    a_spin_until(&done, 0, A_SPIN_LIMIT);
    expect("clone_child_with_wall", syscall4(SYS_wait4, -1, &st, __WALL, 0), c);
    return finish();
}

int test_main(void) { return a_in_session(body); }
