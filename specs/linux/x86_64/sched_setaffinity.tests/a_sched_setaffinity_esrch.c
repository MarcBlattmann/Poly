/* A pid no thread has is ESRCH (3): negative pids (-1, INT_MIN) included,
 * and a child that has been reaped. The lookup comes before the mask is
 * checked: a missing pid with an empty mask is ESRCH, not EINVAL. */
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
    u64 start = mask_of(0);
    u64 v = start;
    expect("pid_minus_one", syscall3(SYS_sched_setaffinity, -1, 8, &v), -ESRCH);
    expect("pid_int_min", syscall3(SYS_sched_setaffinity, (i64)(i32)0x80000000, 8, &v), -ESRCH);
    expect("pid_int_max", syscall3(SYS_sched_setaffinity, 0x7fffffff, 8, &v), -ESRCH);
    expect("pid_big", syscall3(SYS_sched_setaffinity, 0x3fffffff, 8, &v), -ESRCH);
    expect("pid_4194304", syscall3(SYS_sched_setaffinity, 4194304, 8, &v), -ESRCH);
    v = 0;
    expect("missing_empty_mask", syscall3(SYS_sched_setaffinity, 0x3fffffff, 8, &v), -ESRCH);
    expect("missing_len0", syscall3(SYS_sched_setaffinity, 0x7fffffff, 0, &v), -ESRCH);
    expect("negative_empty_mask", syscall3(SYS_sched_setaffinity, -1, 8, &v), -ESRCH);

    a_block_sigchld();
    i64 pid = a_fork_sync();
    if (pid == 0) syscall1(SYS_exit, 0);
    i32 st = -1;
    expect("reap", syscall4(SYS_wait4, pid, &st, 0, 0), pid);
    v = start;
    expect("reaped_child", syscall3(SYS_sched_setaffinity, pid, 8, &v), -ESRCH);
    check("own_mask_untouched", mask_of(0) == start, 0);
    return finish();
}
