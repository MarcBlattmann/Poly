/* The policy argument is a 32-bit int: the upper half of the register
 * is ignored (1<<32 | SCHED_RR is SCHED_RR, minimum 1; 0x7fff00000000 |
 * SCHED_BATCH is SCHED_BATCH, minimum 0); 0x80000001 is a negative int
 * and EINVAL.
 */
#include "poly_ipc.h"

/* asm/unistd_64.h */
#define SYS_sched_get_priority_min 147
/* linux/sched.h */
#define SCHED_FIFO 1
#define SCHED_RR 2
#define SCHED_BATCH 3

static i64 pmin(i64 policy) { return syscall1(SYS_sched_get_priority_min, policy); }

int test_main(void) {
    i64 r = pmin((1L << 32) | SCHED_RR);
    check("high_bits_rr_1", r == 1, r);
    r = pmin(0x7fff00000000L | SCHED_BATCH);
    check("high_bits_batch_0", r == 0, r);
    r = pmin((i64)0xffffffff00000000UL | SCHED_FIFO);
    check("all_high_bits_fifo_1", r == 1, r);
    r = pmin(0x80000001L);
    check("bit31_negative_einval", r == -EINVAL, r);
    r = pmin((5L << 32) | 9);
    check("high_bits_policy_9_einval", r == -EINVAL, r);
    return finish();
}
