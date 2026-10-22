/* getpriority: which must be PRIO_PROCESS (0), PRIO_PGRP (1) or
 * PRIO_USER (2), else EINVAL, whatever who is (also a who that names
 * nothing: EINVAL comes before ESRCH). which is a 32-bit int: the upper
 * half of the register is ignored.
 */
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_getpriority 140
/* linux/resource.h */
#define PRIO_PROCESS 0
#define PRIO_PGRP 1
#define PRIO_USER 2

static i64 getprio(i64 which, i64 who) { return syscall2(SYS_getpriority, which, who); }

int test_main(void) {
    i64 g0 = getprio(PRIO_PROCESS, 0);
    check("process_ok", g0 >= 1 && g0 <= 40, g0);
    i64 r = getprio(PRIO_PGRP, 0);
    check("pgrp_ok", r >= 1 && r <= 40, r);
    r = getprio(PRIO_USER, 0);
    check("user_ok", r >= 1 && r <= 40, r);

    expect("which_3", getprio(3, 0), -EINVAL);
    expect("which_4", getprio(4, 0), -EINVAL);
    expect("which_minus_1", getprio(-1, 0), -EINVAL);
    expect("which_int_max", getprio(0x7fffffff, 0), -EINVAL);
    expect("which_int_min", getprio(-0x80000000L, 0), -EINVAL);
    expect("which_3_bad_who", getprio(3, 0x7fffffff), -EINVAL);
    expect("which_3_who_minus_1", getprio(3, -1), -EINVAL);

    expect("which_high_bits_process", getprio(0x100000000L | PRIO_PROCESS, 0), g0);
    r = getprio(0xffffffff00000000L | PRIO_USER, 0);
    check("which_high_bits_user", r == getprio(PRIO_USER, 0), r);
    expect("which_high_bits_3", getprio(0x100000003L, 0), -EINVAL);
    expect("which_low_bits_minus_1", getprio(0xffffffffL, 0), -EINVAL);
    return finish();
}
