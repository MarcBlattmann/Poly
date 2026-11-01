/* sched_getattr(pid, ...) reads another task's attributes (no privilege
 * needed): here a CLONE_VM child that changes its own nice value and
 * policy when told to. The child's own pid works like 0 for itself, the
 * parent's attributes stay as they were, and after the child has been
 * reaped its pid is ESRCH.
 * poly-diff: syscalls=names (the child's pid is an argument)
 */
#include "a_sys.h"

#define SYS_setpriority 141
#define SYS_sched_setscheduler 144
#define SYS_sched_getattr 315
#define SCHED_BATCH 3 /* linux/sched.h */

struct sched_attr {
    u32 size, sched_policy;
    u64 sched_flags;
    i32 sched_nice;
    u32 sched_priority;
    u64 sched_runtime, sched_deadline, sched_period;
    u32 sched_util_min, sched_util_max;
};
static struct sched_attr a;
static volatile i32 done, cmd;
static volatile i64 res;

static int target(void *unused) {
    (void)unused;
    static i32 param;
    for (;;) {
        while (!cmd) a_relax();
        if (cmd == 1) res = syscall3(SYS_setpriority, 0, 0, 12);
        if (cmd == 2) res = syscall3(SYS_sched_setscheduler, 0, SCHED_BATCH, &param);
        if (cmd == 3) { cmd = 0; return 0; }
        cmd = 0;
    }
}
static i64 tell(int c) {
    cmd = c;
    while (cmd) a_relax();
    return res;
}
static i64 get(i64 pid) {
    memset(&a, 0xab, sizeof a);
    return syscall4(SYS_sched_getattr, pid, &a, sizeof a, 0);
}

int test_main(void) {
    a_block_sigchld();
    i64 pid = a_spawn(0, &done, target, 0);
    expect("child_get", get(pid), 0);
    expect("child_policy_other", a.sched_policy, 0);
    expect("child_nice0", a.sched_nice, 0);
    expect("child_size", a.size, 56);
    expect("child_nice12", tell(1), 0);
    expect("child_get2", get(pid), 0);
    expect("child_reads_nice12", a.sched_nice, 12);
    expect("child_batch", tell(2), 0);
    expect("child_get3", get(pid), 0);
    expect("child_reads_batch", a.sched_policy, SCHED_BATCH);
    expect("child_still_nice12", a.sched_nice, 12);
    expect("self_get", get(0), 0);
    expect("self_policy_other", a.sched_policy, 0);
    expect("self_nice0", a.sched_nice, 0);
    expect("own_pid_get", get(syscall0(SYS_getpid)), 0);
    expect("own_pid_nice0", a.sched_nice, 0);
    tell(3);
    a_spin_until(&done, 0, A_SPIN_LIMIT);
    expect("reap", syscall4(SYS_wait4, pid, 0, 0, 0), pid);
    expect("reaped_esrch", get(pid), -ESRCH);
    return finish();
}
