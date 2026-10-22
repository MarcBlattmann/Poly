/* pid is a 32-bit int: the upper half of the 64-bit register is ignored.
 * 1<<32 means 0 (the caller), 0x1ffffffff means -1 (EINVAL), and
 * (1<<32)|0x7fffffff means 0x7fffffff (ESRCH). */
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_sched_setparam 142
#define SYS_sched_getparam 143
#define SYS_sched_setscheduler 144
#define SYS_sched_getscheduler 145

int test_main(void) {
    i32 prio = 0;
    expect("hi_zero_is_self", syscall2(SYS_sched_setparam, 0x100000000L, &prio), 0);
    expect("hi_getpid_is_self", syscall2(SYS_sched_setparam, 0x500000000L | syscall0(SYS_getpid), &prio), 0);
    expect("hi_minus1", syscall2(SYS_sched_setparam, 0x1ffffffffL, &prio), -EINVAL);
    expect("hi_int_max", syscall2(SYS_sched_setparam, 0x17fffffffL, &prio), -ESRCH);
    expect("ffff_minus1", syscall2(SYS_sched_setparam, -1L, &prio), -EINVAL);
    expect("low_neg_hi_zero", syscall2(SYS_sched_setparam, 0x00000000ffffffffL, &prio), -EINVAL);
    prio = 1;
    expect("hi_zero_bad_prio", syscall2(SYS_sched_setparam, 0x100000000L, &prio), -EINVAL);
    return finish();
}
