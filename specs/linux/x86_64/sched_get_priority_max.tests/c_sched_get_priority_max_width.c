/* The policy argument is a 32-bit int: the upper half of the register
 * is ignored, so 1<<32 | SCHED_FIFO is SCHED_FIFO (99) and
 * 0xffffffff00000000 is SCHED_OTHER (0); 0x80000000 is a negative int
 * and EINVAL, and 1<<32 | 4 is still the invalid policy 4.
 */
#include "poly_ipc.h"

/* asm/unistd_64.h */
#define SYS_sched_get_priority_max 146
/* linux/sched.h */
#define SCHED_FIFO 1
#define SCHED_RR 2

static i64 pmax(i64 policy) { return syscall1(SYS_sched_get_priority_max, policy); }

int test_main(void) {
    i64 r = pmax((1L << 32) | SCHED_FIFO);
    check("high_bits_fifo_99", r == 99, r);
    r = pmax((i64)0xabcd000000000000UL | SCHED_RR);
    check("high_bits_rr_99", r == 99, r);
    r = pmax((i64)0xffffffff00000000UL);
    check("high_bits_other_0", r == 0, r);
    r = pmax((1L << 32) | 4);
    check("high_bits_policy_4_einval", r == -EINVAL, r);
    r = pmax(0x80000000L);
    check("bit31_negative_einval", r == -EINVAL, r);
    r = pmax(0xffffffffL);
    check("low_half_minus1_einval", r == -EINVAL, r);
    return finish();
}
