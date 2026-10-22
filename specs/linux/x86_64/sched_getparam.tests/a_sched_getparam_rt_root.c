/* Under SCHED_FIFO and SCHED_RR sched_getparam returns the real-time
 * priority (1..99) last set by sched_setscheduler or sched_setparam; a
 * failed sched_setparam leaves it as it was. Back under SCHED_OTHER it
 * is 0 again.
 * poly-diff: needs=root (only a privileged process may become real-time)
 */
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_sched_setparam 142
#define SYS_sched_getparam 143
#define SYS_sched_setscheduler 144

/* linux/sched.h */
#define SCHED_OTHER 0
#define SCHED_FIFO 1
#define SCHED_RR 2

static i64 getp(i64 pid) {
    i32 prio = 0x55;
    i64 r = syscall2(SYS_sched_getparam, pid, &prio);
    return r < 0 ? r : prio;
}

static i64 setp(i32 v) {
    i32 prio = v;
    return syscall2(SYS_sched_setparam, 0, &prio);
}

int test_main(void) {
    i32 ten = 10, seven = 7, zero = 0;
    expect("to_fifo_10", syscall3(SYS_sched_setscheduler, 0, SCHED_FIFO, &ten), 0);
    expect("fifo_10", getp(0), 10);
    expect("fifo_10_by_pid", getp(syscall0(SYS_getpid)), 10);
    expect("set_99", setp(99), 0);
    expect("fifo_99", getp(0), 99);
    expect("set_1", setp(1), 0);
    expect("fifo_1", getp(0), 1);
    expect("set_100", setp(100), -EINVAL);
    expect("fifo_still_1", getp(0), 1);
    expect("to_rr_7", syscall3(SYS_sched_setscheduler, 0, SCHED_RR, &seven), 0);
    expect("rr_7", getp(0), 7);
    expect("set_50", setp(50), 0);
    expect("rr_50", getp(0), 50);
    expect("rr_bad_ptr", syscall2(SYS_sched_getparam, 0, 1), -EFAULT);
    expect("rr_still_50", getp(0), 50);
    expect("to_other", syscall3(SYS_sched_setscheduler, 0, SCHED_OTHER, &zero), 0);
    expect("other_0", getp(0), 0);
    return finish();
}
