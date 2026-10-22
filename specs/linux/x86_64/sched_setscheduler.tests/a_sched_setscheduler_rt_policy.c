/* Real-time policies need privilege. After lowering RLIMIT_RTPRIO to 0, a
 * caller without CAP_SYS_NICE gets EPERM (1) for SCHED_FIFO and SCHED_RR
 * with any valid priority (1..99) and stays SCHED_OTHER; a privileged
 * caller (uid 0 here) gets 0 and becomes real-time with that priority.
 * Invalid priorities are EINVAL (22) either way. */
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_sched_setparam 142
#define SYS_sched_getparam 143
#define SYS_sched_setscheduler 144
#define SYS_sched_getscheduler 145

/* linux/sched.h */
#define SCHED_OTHER 0
#define SCHED_FIFO 1
#define SCHED_RR 2
#define SCHED_BATCH 3
#define SCHED_IDLE 5
#define SCHED_DEADLINE 6
#define SCHED_RESET_ON_FORK 0x40000000

static A_UNUSED i64 setsch(i64 pid, i64 policy, i32 prio) {
    i32 p = prio;
    return syscall3(SYS_sched_setscheduler, pid, policy, &p);
}

static A_UNUSED i64 getsch(i64 pid) { return syscall1(SYS_sched_getscheduler, pid); }

static A_UNUSED i64 getprio(i64 pid) {
    i32 p = -1;
    i64 r = syscall2(SYS_sched_getparam, pid, &p);
    return r < 0 ? r : p;
}

int test_main(void) {
    struct { u64 cur, max; } lim = {0, 0};
    expect("rtprio_0", syscall2(SYS_setrlimit, RLIMIT_RTPRIO, &lim), 0);
    int priv = syscall0(SYS_geteuid) == 0;
    i64 want = priv ? 0 : -EPERM;
    expect("fifo_0_einval", setsch(0, SCHED_FIFO, 0), -EINVAL);
    expect("fifo_1", setsch(0, SCHED_FIFO, 1), want);
    expect("fifo_1_policy", getsch(0), priv ? SCHED_FIFO : SCHED_OTHER);
    expect("fifo_1_prio", getprio(0), priv ? 1 : 0);
    expect("back_1", setsch(0, SCHED_OTHER, 0), 0);
    expect("fifo_50", setsch(0, SCHED_FIFO, 50), want);
    expect("back_2", setsch(0, SCHED_OTHER, 0), 0);
    expect("rr_99", setsch(0, SCHED_RR, 99), want);
    expect("rr_99_policy", getsch(0), priv ? SCHED_RR : SCHED_OTHER);
    expect("rr_99_prio", getprio(0), priv ? 99 : 0);
    expect("rr_100_einval", setsch(0, SCHED_RR, 100), -EINVAL);
    expect("back_3", setsch(0, SCHED_OTHER, 0), 0);
    expect("end_other", getsch(0), SCHED_OTHER);
    expect("end_prio", getprio(0), 0);
    return finish();
}
