/* Under SCHED_OTHER any priority other than 0 is EINVAL (22): small,
 * negative, the real-time maximum and the int extremes. Nothing changes. */
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_sched_setparam 142
#define SYS_sched_getparam 143
#define SYS_sched_setscheduler 144
#define SYS_sched_getscheduler 145

/* linux/sched.h */
#define SCHED_OTHER 0

static i64 setp(i32 v) {
    i32 prio = v;
    return syscall2(SYS_sched_setparam, 0, &prio);
}

int test_main(void) {
    expect("prio_1", setp(1), -EINVAL);
    expect("prio_minus1", setp(-1), -EINVAL);
    expect("prio_50", setp(50), -EINVAL);
    expect("prio_99", setp(99), -EINVAL);
    expect("prio_100", setp(100), -EINVAL);
    expect("prio_int_max", setp(0x7fffffff), -EINVAL);
    expect("prio_int_min", setp((i32)0x80000000), -EINVAL);
    i32 one = 1;
    expect("tid_prio_1", syscall2(SYS_sched_setparam, syscall0(SYS_gettid), &one), -EINVAL);
    expect("policy_other", syscall1(SYS_sched_getscheduler, 0), SCHED_OTHER);
    expect("prio_0_still_ok", setp(0), 0);
    return finish();
}
