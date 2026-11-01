/* sched_getattr reports what setpriority and sched_setscheduler set:
 * sched_nice is the nice value of a SCHED_OTHER, SCHED_BATCH (3) or
 * SCHED_IDLE (5) task, sched_policy the policy without the reset flag,
 * and sched_flags has SCHED_FLAG_RESET_ON_FORK (1) while the task has
 * SCHED_RESET_ON_FORK; a child made by fork starts without it (and, the
 * policy being a fair one with nice >= 0, keeps policy and nice); only
 * a privileged caller may clear the flag again (EPERM otherwise). Each
 * case runs in a child of its own (a user cannot lower its nice value
 * again).
 */
#include "a_sys.h"

#define SYS_setpriority 141
#define SYS_sched_setscheduler 144
#define SYS_sched_getattr 315
/* linux/sched.h */
#define SCHED_OTHER 0
#define SCHED_BATCH 3
#define SCHED_IDLE 5
#define SCHED_RESET_ON_FORK 0x40000000
#define PRIO_PROCESS 0 /* linux/resource.h */

struct sched_attr {
    u32 size, sched_policy;
    u64 sched_flags;
    i32 sched_nice;
    u32 sched_priority;
    u64 sched_runtime, sched_deadline, sched_period;
    u32 sched_util_min, sched_util_max;
};
static struct sched_attr a;

static void attr_is(const char *what, u32 policy, u64 flags, i32 nice) {
    char name[64];
    memset(&a, 0xab, sizeof a);
    expect(str_cat2(name, what, "_get"), syscall4(SYS_sched_getattr, 0, &a, sizeof a, 0), 0);
    expect(str_cat2(name, what, "_policy"), a.sched_policy, policy);
    expect(str_cat2(name, what, "_flags"), (i64)a.sched_flags, (i64)flags);
    expect(str_cat2(name, what, "_nice"), a.sched_nice, nice);
    expect(str_cat2(name, what, "_priority"), a.sched_priority, 0);
}
static i64 setsched(i64 policy) {
    static i32 param;
    param = 0;
    return syscall3(SYS_sched_setscheduler, 0, policy, &param);
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

static void nice7(void) {
    expect("nice7_set", syscall3(SYS_setpriority, PRIO_PROCESS, 0, 7), 0);
    attr_is("nice7", SCHED_OTHER, 0, 7);
}
static void batch(void) {
    expect("batch_set", setsched(SCHED_BATCH), 0);
    attr_is("batch", SCHED_BATCH, 0, 0);
    expect("batch_nice3", syscall3(SYS_setpriority, PRIO_PROCESS, 0, 3), 0);
    attr_is("batch3", SCHED_BATCH, 0, 3);
}
static void idle(void) {
    expect("idle_nice4", syscall3(SYS_setpriority, PRIO_PROCESS, 0, 4), 0);
    expect("idle_set", setsched(SCHED_IDLE), 0);
    attr_is("idle", SCHED_IDLE, 0, 4);
}
static void reset(void) {
    expect("reset_set", setsched(SCHED_BATCH | SCHED_RESET_ON_FORK), 0);
    attr_is("reset", SCHED_BATCH, 1, 0);
    i64 pid = a_fork_sync();
    if (pid == 0) {
        attr_is("reset_child", SCHED_BATCH, 0, 0);
        syscall1(SYS_exit, failed ? 1 : 0);
    }
    i32 status = -1;
    syscall4(SYS_wait4, pid, &status, 0, 0);
    expect("reset_child_ok", status, 0);
    /* Clearing the flag again needs CAP_SYS_NICE. */
    int root = syscall0(SYS_geteuid) == 0;
    expect("reset_clear", setsched(SCHED_BATCH), root ? 0 : -EPERM);
    attr_is("reset_after_clear", SCHED_BATCH, root ? 0 : 1, 0);
}

int test_main(void) {
    a_block_sigchld();
    attr_is("start", SCHED_OTHER, 0, 0);
    in_child("nice7_child", nice7);
    in_child("batch_child", batch);
    in_child("idle_child", idle);
    in_child("reset_child", reset);
    attr_is("parent_unchanged", SCHED_OTHER, 0, 0);
    return finish();
}
