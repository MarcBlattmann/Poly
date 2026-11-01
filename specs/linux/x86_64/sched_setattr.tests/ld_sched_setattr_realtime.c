/* sched_setattr as root: SCHED_FIFO and SCHED_RR with priority 1..99;
 * sched_getattr then reports the policy and priority, with sched_nice 0
 * and runtime/deadline/period 0 for a real-time task. With
 * SCHED_FLAG_RESET_ON_FORK, a child made by fork starts as SCHED_OTHER
 * with nice 0 and priority 0, without the flag. SCHED_DEADLINE with
 * runtime 10 ms, deadline 30 ms, period 100 ms is accepted (admission
 * control: the machine has bandwidth for it) and read back; a period of 0
 * means a period equal to the deadline. A SCHED_DEADLINE task without the
 * reset flag cannot fork (EAGAIN); with it, the child is SCHED_OTHER.
 * poly-diff: needs=root (real-time and deadline policies need CAP_SYS_NICE)
 */
#include "a_sys.h"

#define SYS_sched_setattr 314
#define SYS_sched_getattr 315
#define SCHED_OTHER 0
#define SCHED_FIFO 1
#define SCHED_RR 2
#define SCHED_DEADLINE 6
#define SCHED_FLAG_RESET_ON_FORK 0x01

struct sched_attr {
    u32 size, sched_policy;
    u64 sched_flags;
    i32 sched_nice;
    u32 sched_priority;
    u64 sched_runtime, sched_deadline, sched_period;
    u32 sched_util_min, sched_util_max;
};
static struct sched_attr a, g;

static i64 set(u32 policy, u64 flags, i32 nice, u32 prio, u64 rt, u64 dl, u64 per) {
    memset(&a, 0, sizeof a);
    a.size = 48;
    a.sched_policy = policy;
    a.sched_flags = flags;
    a.sched_nice = nice;
    a.sched_priority = prio;
    a.sched_runtime = rt;
    a.sched_deadline = dl;
    a.sched_period = per;
    return syscall3(SYS_sched_setattr, 0, &a, 0);
}
static i64 get(void) {
    memset(&g, 0xab, sizeof g);
    return syscall4(SYS_sched_getattr, 0, &g, sizeof g, 0);
}
/* Fork a child that reports its attributes through its exit status:
 * policy * 16 + flags, or 255 if nice is not 0 or the priority is not
 * `prio`. */
static i64 child_attrs(u32 prio) {
    i64 pid = a_fork_sync();
    if (pid == 0) {
        if (get() != 0 || g.sched_nice != 0 || g.sched_priority != prio) syscall1(SYS_exit, 255);
        syscall1(SYS_exit, (i64)(g.sched_policy * 16 + g.sched_flags));
    }
    if (pid < 0) return pid;
    i32 status = -1;
    syscall4(SYS_wait4, pid, &status, 0, 0);
    return A_WEXITSTATUS(status);
}

int test_main(void) {
    a_block_sigchld();
    expect("fifo10", set(SCHED_FIFO, 0, 5, 10, 0, 0, 0), 0);
    expect("fifo10_get", get(), 0);
    expect("fifo10_policy", g.sched_policy, SCHED_FIFO);
    expect("fifo10_priority", g.sched_priority, 10);
    expect("fifo10_nice_reported_0", g.sched_nice, 0);
    expect("fifo10_runtime_0", (i64)g.sched_runtime, 0);
    expect("rr99", set(SCHED_RR, 0, 0, 99, 0, 0, 0), 0);
    expect("rr99_get", get(), 0);
    expect("rr99_policy", g.sched_policy, SCHED_RR);
    expect("rr99_priority", g.sched_priority, 99);
    expect("rr_child_keeps_rr_99", child_attrs(99), SCHED_RR * 16);
    expect("fifo_reset", set(SCHED_FIFO, SCHED_FLAG_RESET_ON_FORK, 0, 20, 0, 0, 0), 0);
    expect("fifo_reset_get", get(), 0);
    expect("fifo_reset_flags", (i64)g.sched_flags, 1);
    expect("fifo_reset_child_other", child_attrs(0), SCHED_OTHER * 16);
    expect("nice_minus5_reset", set(SCHED_OTHER, SCHED_FLAG_RESET_ON_FORK, -5, 0, 0, 0, 0), 0);
    expect("nice_minus5_child_nice0", child_attrs(0), SCHED_OTHER * 16);

    expect("deadline", set(SCHED_DEADLINE, 0, 0, 0, 10000000, 30000000, 100000000), 0);
    expect("deadline_get", get(), 0);
    expect("deadline_policy", g.sched_policy, SCHED_DEADLINE);
    expect("deadline_runtime", (i64)g.sched_runtime, 10000000);
    expect("deadline_deadline", (i64)g.sched_deadline, 30000000);
    expect("deadline_period", (i64)g.sched_period, 100000000);
    expect("deadline_priority_0", g.sched_priority, 0);
    expect("deadline_fork_eagain", a_fork_sync(), -EAGAIN);
    expect("deadline_period0", set(SCHED_DEADLINE, 0, 0, 0, 10000000, 30000000, 0), 0);
    expect("deadline_period0_get", get(), 0);
    expect("deadline_period_is_deadline", (i64)g.sched_period, 30000000);
    expect("deadline_reset", set(SCHED_DEADLINE, SCHED_FLAG_RESET_ON_FORK, 0, 0, 10000000, 30000000, 100000000), 0);
    expect("deadline_reset_child_other", child_attrs(0), SCHED_OTHER * 16);
    expect("back_to_other", set(SCHED_OTHER, 0, 0, 0, 0, 0, 0), 0);
    expect("other_get", get(), 0);
    expect("other_policy", g.sched_policy, SCHED_OTHER);
    expect("other_deadline_0", (i64)g.sched_deadline, 0);
    return finish();
}
