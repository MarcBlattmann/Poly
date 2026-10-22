/* pid is a 32-bit int: the upper 32 bits of the register are ignored.
 * 0x100000000 is 0 (the caller), N | 0x500000000 is N, 0xffffffff is -1
 * (EINVAL) and 0x17fffffff is 0x7fffffff (ESRCH). */
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_sched_getparam 143

int test_main(void) {
    i32 prio = 0x55;
    expect("hi_zero", syscall2(SYS_sched_getparam, 0x100000000UL, &prio), 0);
    expect("hi_zero_val", prio, 0);
    prio = 0x55;
    expect("hi_self", syscall2(SYS_sched_getparam, syscall0(SYS_getpid) | 0x500000000UL, &prio), 0);
    expect("hi_self_val", prio, 0);
    expect("u32_minus1", syscall2(SYS_sched_getparam, 0xffffffffUL, &prio), -EINVAL);
    expect("hi_minus1", syscall2(SYS_sched_getparam, 0x1ffffffffUL, &prio), -EINVAL);
    expect("u32_int_min", syscall2(SYS_sched_getparam, 0x80000000UL, &prio), -EINVAL);
    expect("hi_max", syscall2(SYS_sched_getparam, 0x17fffffffUL, &prio), -ESRCH);
    return finish();
}
