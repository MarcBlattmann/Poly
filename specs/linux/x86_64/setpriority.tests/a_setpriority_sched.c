/* The nice value setpriority sets is the one sched_getattr reports as
 * sched_nice (SCHED_OTHER and SCHED_BATCH), and setpriority works under
 * SCHED_BATCH (3) and SCHED_IDLE (5) too: it returns 0 and getpriority
 * returns the new value; a lowering is EACCES under any policy. The
 * policy is not changed by setpriority.
 */
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_setgroups 116
#define SYS_setresuid 117
#define SYS_setresgid 119
#define SYS_getpriority 140
#define SYS_setpriority 141
/* linux/resource.h */
#define PRIO_PROCESS 0
#define PRIO_PGRP 1
#define PRIO_USER 2

static A_UNUSED i64 getprio(i64 which, i64 who) { return syscall2(SYS_getpriority, which, who); }
static A_UNUSED i64 setprio(i64 which, i64 who, i64 nice) { return syscall3(SYS_setpriority, which, who, nice); }

/* What getpriority returns after nice n was set: nice values above 19
 * are 19, so a test that starts near 19 still agrees with itself. */
static A_UNUSED i64 g_of(i64 n) { return 20 - (n > 19 ? 19 : n); }

/* Run without privilege, whoever started the test: RLIMIT_NICE 0 (no
 * lowering allowed by the limit), and root becomes uid and gid 4000123
 * with no groups (so it has no CAP_SYS_NICE). */
static A_UNUSED void as_user(void) {
    struct { u64 cur, max; } rl = {0, 0};
    syscall2(SYS_setrlimit, RLIMIT_NICE, &rl);
    if (syscall0(SYS_getuid) == 0) {
        syscall2(SYS_setgroups, 0, 0);
        syscall3(SYS_setresgid, 4000123, 4000123, 4000123);
        syscall3(SYS_setresuid, 4000123, 4000123, 4000123);
    }
}

#define SYS_sched_setscheduler 144
#define SYS_sched_getscheduler 145
#define SYS_sched_getattr 315
/* linux/sched.h */
#define SCHED_OTHER 0
#define SCHED_BATCH 3
#define SCHED_IDLE 5

/* struct sched_attr, linux/sched/types.h (SCHED_ATTR_SIZE_VER1, 56 bytes) */
struct sched_attr {
    u32 size;
    u32 sched_policy;
    u64 sched_flags;
    i32 sched_nice;
    u32 sched_priority;
    u64 sched_runtime, sched_deadline, sched_period;
    u32 sched_util_min, sched_util_max;
};

static i64 attr_nice(void) {
    struct sched_attr a;
    memset(&a, 0, sizeof a);
    i64 r = syscall4(SYS_sched_getattr, 0, &a, sizeof a, 0);
    return r < 0 ? 1000 + r : a.sched_nice;
}

int test_main(void) {
    as_user();
    i64 n0 = 20 - getprio(PRIO_PROCESS, 0);
    expect("policy_other", syscall1(SYS_sched_getscheduler, 0), SCHED_OTHER);
    expect("raise_1", setprio(PRIO_PROCESS, 0, n0 + 1), 0);
    expect("attr_nice_1", attr_nice(), 20 - g_of(n0 + 1));

    i32 param = 0;
    expect("to_batch", syscall3(SYS_sched_setscheduler, 0, SCHED_BATCH, &param), 0);
    expect("batch_raise", setprio(PRIO_PROCESS, 0, n0 + 3), 0);
    expect("batch_get", getprio(PRIO_PROCESS, 0), g_of(n0 + 3));
    expect("batch_attr_nice", attr_nice(), 20 - g_of(n0 + 3));
    expect("batch_policy_kept", syscall1(SYS_sched_getscheduler, 0), SCHED_BATCH);
    expect("batch_lower", setprio(PRIO_PROCESS, 0, 20 - g_of(n0 + 3) - 1), -EACCES);

    expect("to_idle", syscall3(SYS_sched_setscheduler, 0, SCHED_IDLE, &param), 0);
    expect("idle_raise", setprio(PRIO_PROCESS, 0, n0 + 5), 0);
    expect("idle_get", getprio(PRIO_PROCESS, 0), g_of(n0 + 5));
    expect("idle_policy_kept", syscall1(SYS_sched_getscheduler, 0), SCHED_IDLE);
    expect("idle_lower", setprio(PRIO_PROCESS, 0, 20 - g_of(n0 + 5) - 1), -EACCES);
    expect("idle_get_kept", getprio(PRIO_PROCESS, 0), g_of(n0 + 5));
    return finish();
}
