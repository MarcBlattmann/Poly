/* sched_setattr policies. SCHED_BATCH (3) and SCHED_IDLE (5) are open to
 * every user (sched_getattr then reports them); leaving SCHED_IDLE for
 * another fair policy needs CAP_SYS_NICE (EPERM). Unknown policies (4, 8,
 * 0xffffffff) are EINVAL. SCHED_FIFO (1) and SCHED_RR (2) need a
 * priority 1..99 (0 or 100 is EINVAL for every user, checked before the
 * permission) and, with RLIMIT_RTPRIO 0, CAP_SYS_NICE (EPERM). A failed
 * call changes nothing. Each case runs in a child of its own.
 */
#include "a_sys.h"

#define SYS_sched_setattr 314
#define SYS_sched_getattr 315
/* linux/sched.h */
#define SCHED_OTHER 0
#define SCHED_FIFO 1
#define SCHED_RR 2
#define SCHED_BATCH 3
#define SCHED_IDLE 5

struct sched_attr {
    u32 size, sched_policy;
    u64 sched_flags;
    i32 sched_nice;
    u32 sched_priority;
    u64 sched_runtime, sched_deadline, sched_period;
    u32 sched_util_min, sched_util_max;
};
static struct sched_attr a;
static int root;

static i64 set(u32 policy, i32 nice, u32 prio) {
    memset(&a, 0, sizeof a);
    a.size = 48;
    a.sched_policy = policy;
    a.sched_nice = nice;
    a.sched_priority = prio;
    return syscall3(SYS_sched_setattr, 0, &a, 0);
}
static i64 policy_now(void) {
    memset(&a, 0xab, sizeof a);
    if (syscall4(SYS_sched_getattr, 0, &a, sizeof a, 0) != 0) return -99;
    return a.sched_policy;
}
static void in_child(const char *name, void (*body)(void)) {
    i64 pid = a_fork_sync();
    if (pid == 0) {
        body();
        syscall1(SYS_exit, failed ? 1 : 0);
    }
    i32 status = -1;
    syscall4(SYS_wait4, pid, &status, 0, 0);
    expect(name, status, 0);
}

static void batch(void) {
    expect("batch_set", set(SCHED_BATCH, 0, 0), 0);
    expect("batch_policy", policy_now(), SCHED_BATCH);
    expect("batch_back_to_other", set(SCHED_OTHER, 0, 0), 0);
    expect("other_policy", policy_now(), SCHED_OTHER);
}
static void idle(void) {
    expect("idle_set", set(SCHED_IDLE, 0, 0), 0);
    expect("idle_policy", policy_now(), SCHED_IDLE);
    expect("idle_to_other", set(SCHED_OTHER, 0, 0), root ? 0 : -EPERM);
    expect("after_idle_to_other", policy_now(), root ? SCHED_OTHER : SCHED_IDLE);
}
static void unknown(void) {
    expect("policy4_einval", set(4, 0, 0), -EINVAL);
    expect("policy8_einval", set(8, 0, 0), -EINVAL);
    expect("policy_all_ones_einval", set(0xffffffff, 0, 0), -EINVAL);
    expect("policy_reset_bit_einval", set(SCHED_BATCH | 0x40000000, 0, 0), -EINVAL);
    expect("still_other", policy_now(), SCHED_OTHER);
}
static void realtime(void) {
    expect("fifo_prio0_einval", set(SCHED_FIFO, 0, 0), -EINVAL);
    expect("fifo_prio100_einval", set(SCHED_FIFO, 0, 100), -EINVAL);
    expect("rr_prio0_einval", set(SCHED_RR, 0, 0), -EINVAL);
    expect("rr_prio1000_einval", set(SCHED_RR, 0, 1000), -EINVAL);
    expect("fifo_prio10", set(SCHED_FIFO, 0, 10), root ? 0 : -EPERM);
    expect("after_fifo", policy_now(), root ? SCHED_FIFO : SCHED_OTHER);
}

int test_main(void) {
    a_block_sigchld();
    root = syscall0(SYS_geteuid) == 0;
    expect("start_other", policy_now(), SCHED_OTHER);
    in_child("batch_child", batch);
    in_child("idle_child", idle);
    in_child("unknown_child", unknown);
    in_child("realtime_child", realtime);
    expect("parent_still_other", policy_now(), SCHED_OTHER);
    return finish();
}
