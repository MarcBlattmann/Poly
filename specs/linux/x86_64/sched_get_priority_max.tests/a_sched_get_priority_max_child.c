/* A child process (fork semantics) gets the same answers as its parent. */
#include "a_sys.h"

#ifndef SYS_sched_get_priority_max
#define SYS_sched_get_priority_max 146
#endif

static i64 pmax(i64 policy) { return syscall1(SYS_sched_get_priority_max, policy); }

int test_main(void) {
    a_block_sigchld();
    i64 pid = a_fork_sync();
    if (pid == 0) {
        int ok = pmax(0) == 0 && pmax(1) == 99 && pmax(2) == 99 && pmax(3) == 0 && pmax(5) == 0 &&
                 pmax(6) == 0 && pmax(4) == -EINVAL && pmax(-1) == -EINVAL;
        syscall1(SYS_exit, ok ? 0 : 1);
    }
    i32 st = -1;
    expect("reap", syscall4(SYS_wait4, pid, &st, 0, 0), pid);
    check("child_exited", A_WIFEXITED(st), st);
    expect("child_values_ok", A_WEXITSTATUS(st), 0);
    expect("parent_fifo", pmax(1), 99);
    expect("parent_other", pmax(0), 0);
    return finish();
}
