/* sched_setattr flags (linux/sched.h). SCHED_FLAG_RESET_ON_FORK (1) is
 * set and read back by sched_getattr; clearing it later needs
 * CAP_SYS_NICE. SCHED_FLAG_KEEP_POLICY (8) keeps the current policy and
 * applies the rest (sched_nice; a priority invalid for the kept policy is
 * EINVAL). SCHED_FLAG_KEEP_PARAMS (0x10) keeps the policy, nice value and
 * priority as they are and applies only the other flags. Flags outside
 * 0x7f are EINVAL; the utilisation clamp flags (0x20, 0x40) with a size
 * of 48 (no clamp fields) are EINVAL. Each case runs in a child.
 */
#include "a_sys.h"

#define SYS_sched_setattr 314
#define SYS_sched_getattr 315
#define SCHED_OTHER 0
#define SCHED_RR 2
#define SCHED_BATCH 3
#define SCHED_FLAG_RESET_ON_FORK 0x01
#define SCHED_FLAG_KEEP_POLICY 0x08
#define SCHED_FLAG_KEEP_PARAMS 0x10
#define SCHED_FLAG_UTIL_CLAMP_MIN 0x20
#define SCHED_FLAG_UTIL_CLAMP_MAX 0x40

struct sched_attr {
    u32 size, sched_policy;
    u64 sched_flags;
    i32 sched_nice;
    u32 sched_priority;
    u64 sched_runtime, sched_deadline, sched_period;
    u32 sched_util_min, sched_util_max;
};
static struct sched_attr a, g;
static int root;

static i64 set(u32 size, u32 policy, u64 flags, i32 nice, u32 prio) {
    memset(&a, 0, sizeof a);
    a.size = size;
    a.sched_policy = policy;
    a.sched_flags = flags;
    a.sched_nice = nice;
    a.sched_priority = prio;
    return syscall3(SYS_sched_setattr, 0, &a, 0);
}
static void now_is(const char *what, u32 policy, u64 flags, i32 nice) {
    char n[64];
    memset(&g, 0xab, sizeof g);
    expect(str_cat2(n, what, "_get"), syscall4(SYS_sched_getattr, 0, &g, sizeof g, 0), 0);
    expect(str_cat2(n, what, "_policy"), g.sched_policy, policy);
    expect(str_cat2(n, what, "_flags"), (i64)g.sched_flags, (i64)flags);
    expect(str_cat2(n, what, "_nice"), g.sched_nice, nice);
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

static void reset(void) {
    expect("reset_set", set(48, SCHED_OTHER, SCHED_FLAG_RESET_ON_FORK, 2, 0), 0);
    now_is("reset", SCHED_OTHER, 1, 2);
    expect("reset_clear", set(48, SCHED_OTHER, 0, 2, 0), root ? 0 : -EPERM);
    now_is("reset_after_clear", SCHED_OTHER, root ? 0 : 1, 2);
}
static void keep_policy(void) {
    expect("batch", set(48, SCHED_BATCH, 0, 1, 0), 0);
    expect("keep_policy_other_nice4", set(48, SCHED_OTHER, SCHED_FLAG_KEEP_POLICY, 4, 0), 0);
    now_is("keep_policy", SCHED_BATCH, 0, 4);
    expect("keep_policy_rr_prio7_einval", set(48, SCHED_RR, SCHED_FLAG_KEEP_POLICY, 6, 7), -EINVAL);
    expect("keep_all_prio7_einval",
           set(48, SCHED_RR, SCHED_FLAG_KEEP_POLICY | SCHED_FLAG_KEEP_PARAMS, 6, 7), -EINVAL);
    now_is("keep_policy_after_einval", SCHED_BATCH, 0, 4);
}
static void keep_params(void) {
    expect("nice3", set(48, SCHED_OTHER, 0, 3, 0), 0);
    expect("keep_params_batch_nice9", set(48, SCHED_BATCH, SCHED_FLAG_KEEP_PARAMS, 9, 0), 0);
    now_is("keep_params", SCHED_OTHER, 0, 3);
    expect("keep_params_with_reset",
           set(48, SCHED_BATCH, SCHED_FLAG_KEEP_PARAMS | SCHED_FLAG_RESET_ON_FORK, 9, 0), 0);
    now_is("keep_params_reset", SCHED_OTHER, 1, 3);
}
static void bad_flags(void) {
    expect("flag_0x80_einval", set(48, SCHED_OTHER, 0x80, 0, 0), -EINVAL);
    expect("flag_0x100_einval", set(48, SCHED_OTHER, 0x100, 0, 0), -EINVAL);
    expect("flag_bit63_einval", set(48, SCHED_OTHER, 1UL << 63, 0, 0), -EINVAL);
    expect("clamp_min_size48_einval", set(48, SCHED_OTHER, SCHED_FLAG_UTIL_CLAMP_MIN, 0, 0), -EINVAL);
    expect("clamp_max_size48_einval", set(48, SCHED_OTHER, SCHED_FLAG_UTIL_CLAMP_MAX, 0, 0), -EINVAL);
    now_is("bad_flags_unchanged", SCHED_OTHER, 0, 0);
}

int test_main(void) {
    a_block_sigchld();
    root = syscall0(SYS_geteuid) == 0;
    in_child("reset_child", reset);
    in_child("keep_policy_child", keep_policy);
    in_child("keep_params_child", keep_params);
    in_child("bad_flags_child", bad_flags);
    now_is("parent", SCHED_OTHER, 0, 0);
    return finish();
}
