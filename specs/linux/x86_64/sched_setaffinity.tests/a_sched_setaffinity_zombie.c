/* A child that has exited but is not yet reaped is still found: setting
 * its mask returns 0 and sched_getaffinity reports the new mask; an empty
 * mask is EINVAL. Once reaped, its pid is ESRCH. */
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_sched_setaffinity 203
#define SYS_sched_getaffinity 204

static u64 mask_of(i64 pid) {
    u64 m[16];
    memset(m, 0, sizeof m);
    i64 r = syscall3(SYS_sched_getaffinity, pid, sizeof m, m);
    return r == 8 ? m[0] : 0;
}

int test_main(void) {
    a_block_sigchld();
    u64 start = mask_of(0);
    u64 lo = start & -start;
    i64 pid = a_fork_sync();
    if (pid == 0) syscall1(SYS_exit, 0);
    check("forked", pid > 0, pid);

    u64 v = lo;
    expect("zombie_set", syscall3(SYS_sched_setaffinity, pid, 8, &v), 0);
    check("zombie_mask", mask_of(pid) == lo, 0);
    v = 0;
    expect("zombie_empty", syscall3(SYS_sched_setaffinity, pid, 8, &v), -EINVAL);
    expect("zombie_null", syscall3(SYS_sched_setaffinity, pid, 8, 0), -EFAULT);
    check("zombie_mask_kept", mask_of(pid) == lo, 0);

    i32 st = -1;
    expect("reap", syscall4(SYS_wait4, pid, &st, 0, 0), pid);
    check("exit_status", A_WIFEXITED(st) && A_WEXITSTATUS(st) == 0, st);
    v = lo;
    expect("reaped", syscall3(SYS_sched_setaffinity, pid, 8, &v), -ESRCH);
    check("parent_unchanged", mask_of(0) == start, 0);
    return finish();
}
