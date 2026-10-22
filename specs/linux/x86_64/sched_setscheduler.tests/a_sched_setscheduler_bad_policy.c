/* Unknown policies are EINVAL (22) and change nothing: 4, 6
 * (SCHED_DEADLINE, settable only through sched_setattr), 8, 100, -1,
 * INT_MIN, and policies with flag bits other than SCHED_RESET_ON_FORK
 * (0x10000000, 0x20000000, 0x60000000), and SCHED_RESET_ON_FORK together
 * with an unknown policy. (Policy 7 is left out: it is SCHED_EXT where the
 * kernel has sched_ext.) */
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
    expect("to_batch", setsch(0, SCHED_BATCH, 0), 0);
    expect("pol_4", setsch(0, 4, 0), -EINVAL);
    expect("pol_deadline", setsch(0, SCHED_DEADLINE, 0), -EINVAL);
    expect("pol_8", setsch(0, 8, 0), -EINVAL);
    expect("pol_100", setsch(0, 100, 0), -EINVAL);
    expect("pol_minus1", setsch(0, -1, 0), -EINVAL);
    expect("pol_int_min", setsch(0, (i32)0x80000000u, 0), -EINVAL);
    expect("pol_0x10000000", setsch(0, 0x10000000, 0), -EINVAL);
    expect("pol_0x20000000", setsch(0, 0x20000000, 0), -EINVAL);
    expect("pol_0x60000000", setsch(0, 0x60000000, 0), -EINVAL);
    expect("pol_reset_4", setsch(0, SCHED_RESET_ON_FORK | 4, 0), -EINVAL);
    expect("pol_reset_8", setsch(0, SCHED_RESET_ON_FORK | 8, 0), -EINVAL);
    expect("pol_reset_deadline", setsch(0, SCHED_RESET_ON_FORK | SCHED_DEADLINE, 0), -EINVAL);
    expect("still_batch", getsch(0), SCHED_BATCH);
    expect("prio_0", getprio(0), 0);
    return finish();
}
