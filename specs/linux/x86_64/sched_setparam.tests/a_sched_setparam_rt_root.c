/* Under SCHED_FIFO and SCHED_RR sched_setparam changes the real-time
 * priority within 1..99 and keeps the policy; 0, 100 and negative values
 * are EINVAL (22) and change nothing. Back under SCHED_OTHER, only 0 is
 * valid again.
 * poly-diff: needs=root (only a privileged process may become real-time)
 */
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

static i64 setp(i32 v) {
    i32 prio = v;
    return syscall2(SYS_sched_setparam, 0, &prio);
}

static i64 getp(void) {
    i32 prio = -1;
    i64 r = syscall2(SYS_sched_getparam, 0, &prio);
    return r < 0 ? r : prio;
}

int test_main(void) {
    i32 ten = 10, five = 5, zero = 0;
    expect("to_fifo", syscall3(SYS_sched_setscheduler, 0, SCHED_FIFO, &ten), 0);
    expect("fifo_20", setp(20), 0);
    expect("fifo_get_20", getp(), 20);
    expect("fifo_policy", syscall1(SYS_sched_getscheduler, 0), SCHED_FIFO);
    expect("fifo_99", setp(99), 0);
    expect("fifo_get_99", getp(), 99);
    expect("fifo_1", setp(1), 0);
    expect("fifo_get_1", getp(), 1);
    expect("fifo_0", setp(0), -EINVAL);
    expect("fifo_100", setp(100), -EINVAL);
    expect("fifo_minus1", setp(-1), -EINVAL);
    expect("fifo_get_still_1", getp(), 1);
    expect("fifo_policy_kept", syscall1(SYS_sched_getscheduler, 0), SCHED_FIFO);
    expect("to_rr", syscall3(SYS_sched_setscheduler, 0, SCHED_RR, &five), 0);
    expect("rr_50", setp(50), 0);
    expect("rr_get_50", getp(), 50);
    expect("rr_policy", syscall1(SYS_sched_getscheduler, 0), SCHED_RR);
    expect("rr_0", setp(0), -EINVAL);
    expect("rr_get_still_50", getp(), 50);
    expect("to_other", syscall3(SYS_sched_setscheduler, 0, SCHED_OTHER, &zero), 0);
    expect("other_5", setp(5), -EINVAL);
    expect("other_0", setp(0), 0);
    expect("other_get_0", getp(), 0);
    return finish();
}
