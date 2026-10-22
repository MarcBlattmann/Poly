/* SCHED_RESET_ON_FORK (0x40000000) is part of the value: after
 * sched_setscheduler(policy | SCHED_RESET_ON_FORK) the result is
 * policy | 0x40000000, for SCHED_OTHER, SCHED_BATCH and (in a child)
 * SCHED_IDLE. Another process reading this one by pid sees the flag too.
 * A failed sched_setscheduler leaves the value as it was. */
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_sched_setscheduler 144
#define SYS_sched_getscheduler 145

/* linux/sched.h */
#define SCHED_OTHER 0
#define SCHED_BATCH 3
#define SCHED_IDLE 5
#define SCHED_RESET_ON_FORK 0x40000000

int test_main(void) {
    a_block_sigchld();
    i32 zero = 0;
    i32 one = 1;
    expect("set_other_rof", syscall3(SYS_sched_setscheduler, 0, SCHED_OTHER | SCHED_RESET_ON_FORK, &zero), 0);
    expect("other_rof", syscall1(SYS_sched_getscheduler, 0), 0x40000000);
    expect("set_batch_rof", syscall3(SYS_sched_setscheduler, 0, SCHED_BATCH | SCHED_RESET_ON_FORK, &zero), 0);
    expect("batch_rof", syscall1(SYS_sched_getscheduler, 0), 0x40000003);
    expect("bad_prio", syscall3(SYS_sched_setscheduler, 0, SCHED_OTHER | SCHED_RESET_ON_FORK, &one),
           -EINVAL);
    expect("after_bad_prio", syscall1(SYS_sched_getscheduler, 0), 0x40000003);
    expect("bad_policy", syscall3(SYS_sched_setscheduler, 0, 4 | SCHED_RESET_ON_FORK, &zero), -EINVAL);
    expect("after_bad_policy", syscall1(SYS_sched_getscheduler, 0), 0x40000003);
    i64 ppid = syscall0(SYS_getpid);
    i64 child = a_fork_sync();
    if (child == 0) {
        expect("child_reads_parent", syscall1(SYS_sched_getscheduler, ppid), 0x40000003);
        expect("child_set_idle_rof",
               syscall3(SYS_sched_setscheduler, 0, SCHED_IDLE | SCHED_RESET_ON_FORK, &zero), 0);
        expect("child_idle_rof", syscall1(SYS_sched_getscheduler, 0), 0x40000005);
        syscall1(SYS_exit_group, failed ? 1 : 0);
    }
    expect("zombie_idle_rof", syscall1(SYS_sched_getscheduler, child), 0x40000005);
    i32 st = -1;
    expect("reap", syscall4(SYS_wait4, child, &st, 0, 0), child);
    check("child_ok", A_WIFEXITED(st) && A_WEXITSTATUS(st) == 0, st);
    expect("parent_batch_rof", syscall1(SYS_sched_getscheduler, 0), 0x40000003);
    return finish();
}
