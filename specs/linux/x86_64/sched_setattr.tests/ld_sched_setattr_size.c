/* sched_setattr(0, attr, 0) and attr->size. 0 means 48 (VER0);
 * 48..56 are read as they are; a larger size up to 4096 works when every
 * byte after the kernel's 56 is zero (attr->size is left as given). A
 * size from 1 to 47, above 4096, or a larger one with a non-zero byte
 * after 56 is E2BIG, and the kernel writes its own size (56) into
 * attr->size. Errors and their order: NULL attr, negative pid or flags
 * other than 0 are EINVAL; then the size (E2BIG, also for a pid with no
 * task) or an unreadable attr (EFAULT); then the pid (ESRCH), before
 * the policy and sched_flags are looked at.
 */
#include "a_sys.h"

#define SYS_getpriority 140
#define SYS_sched_setattr 314
#define NO_PID 0x7fffffff
#define PRIO_PROCESS 0

struct sched_attr {
    u32 size, sched_policy;
    u64 sched_flags;
    i32 sched_nice;
    u32 sched_priority;
    u64 sched_runtime, sched_deadline, sched_period;
    u32 sched_util_min, sched_util_max;
    unsigned char extra[200];
};
static struct sched_attr a;

/* SCHED_OTHER with nice `nice` (raising nice works for any user). */
static void prepare(u32 size, i32 nice) {
    memset(&a, 0, sizeof a);
    a.size = size;
    a.sched_nice = nice;
}
static i64 set(i64 pid, u64 flags) { return syscall3(SYS_sched_setattr, pid, &a, flags); }
static i64 nice_now(void) { return 20 - syscall2(SYS_getpriority, PRIO_PROCESS, 0); }

int test_main(void) {
    prepare(48, 1);
    expect("size48", set(0, 0), 0);
    expect("size48_nice1", nice_now(), 1);
    prepare(0, 2);
    expect("size0_is_ver0", set(0, 0), 0);
    expect("size0_nice2", nice_now(), 2);
    prepare(56, 3);
    expect("size56", set(0, 0), 0);
    expect("size56_nice3", nice_now(), 3);
    prepare(120, 4);
    expect("size120_zero_tail", set(0, 0), 0);
    expect("size120_size_kept", a.size, 120);
    expect("size120_nice4", nice_now(), 4);
    prepare(256, 4);
    expect("size256_zero_tail", set(0, 0), 0);

    prepare(47, 9);
    expect("size47_e2big", set(0, 0), -E2BIG);
    expect("size47_size_written", a.size, 56);
    prepare(1, 9);
    expect("size1_e2big", set(0, 0), -E2BIG);
    expect("size1_size_written", a.size, 56);
    prepare(120, 9);
    a.extra[50] = 1;
    expect("tail_nonzero_e2big", set(0, 0), -E2BIG);
    expect("tail_nonzero_size_written", a.size, 56);
    prepare(4097, 9);
    expect("size4097_e2big", set(0, 0), -E2BIG);
    expect("size4097_size_written", a.size, 56);
    expect("nice_unchanged_after_e2big", nice_now(), 4);

    prepare(48, 9);
    expect("flags1_einval", set(0, 1), -EINVAL);
    expect("flags_upper_half_ignored", set(0, 1UL << 32), 0);
    expect("flags_upper_half_nice9", nice_now(), 9);
    prepare(48, 10);
    expect("null_einval", syscall3(SYS_sched_setattr, 0, 0, 0), -EINVAL);
    expect("pid_minus1_einval", set(-1, 0), -EINVAL);
    expect("no_pid_esrch", set(NO_PID, 0), -ESRCH);
    expect("bad_attr_efault", syscall3(SYS_sched_setattr, 0, 16, 0), -EFAULT);
    expect("null_before_flags", syscall3(SYS_sched_setattr, 0, 0, 1), -EINVAL);
    prepare(8, 10);
    expect("negative_pid_before_size", set(-1, 0), -EINVAL);
    expect("size_before_lookup", set(NO_PID, 0), -E2BIG);
    expect("fault_before_lookup", syscall3(SYS_sched_setattr, NO_PID, 16, 0), -EFAULT);
    prepare(48, 0);
    a.sched_policy = 4;
    expect("lookup_before_policy", set(NO_PID, 0), -ESRCH);
    prepare(48, 0);
    a.sched_flags = 0x80;
    expect("lookup_before_sched_flags", set(NO_PID, 0), -ESRCH);
    expect("nice_still_9", nice_now(), 9);
    return finish();
}
