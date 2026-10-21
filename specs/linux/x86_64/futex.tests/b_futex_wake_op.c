/* FUTEX_WAKE_OP with no waiters: the operation is applied to the second
 * word (SET, ADD, OR, ANDN, XOR; with FUTEX_OP_OPARG_SHIFT the argument is
 * 1 << oparg; a 12-bit argument is sign-extended), the old value is
 * compared (EQ, NE, LT, LE, GT, GE), and the result is the number woken
 * (0 here). Unknown op or cmp codes: ENOSYS; a bad second address:
 * EFAULT. FUTEX_REQUEUE and FUTEX_CMP_REQUEUE with no waiters return 0;
 * CMP_REQUEUE checks the first word against val3 (EAGAIN). */
#include "poly_ipc.h"

static volatile unsigned w1 __attribute__((aligned(8)));
static volatile unsigned w2 __attribute__((aligned(8)));

static i64 wake_op(unsigned op) { return futex(&w1, FUTEX_WAKE_OP, 1, (void *)1, &w2, op); }

int test_main(void) {
    w2 = 10;
    i64 r = wake_op(FUTEX_OP(FUTEX_OP_SET, 7, FUTEX_OP_CMP_EQ, 10));
    check("op_set_returns_zero", r == 0, r);
    check("op_set_applied", w2 == 7, w2);
    wake_op(FUTEX_OP(FUTEX_OP_ADD, 5, FUTEX_OP_CMP_EQ, 0));
    check("op_add_applied", w2 == 12, w2);
    wake_op(FUTEX_OP(FUTEX_OP_OR, 0x30, FUTEX_OP_CMP_EQ, 0));
    check("op_or_applied", w2 == 0x3c, w2);
    wake_op(FUTEX_OP(FUTEX_OP_ANDN, 0x0c, FUTEX_OP_CMP_EQ, 0));
    check("op_andn_applied", w2 == 0x30, w2);
    wake_op(FUTEX_OP(FUTEX_OP_XOR, 0x11, FUTEX_OP_CMP_EQ, 0));
    check("op_xor_applied", w2 == 0x21, w2);
    wake_op(FUTEX_OP(FUTEX_OP_SET | FUTEX_OP_OPARG_SHIFT, 4, FUTEX_OP_CMP_EQ, 0));
    check("op_oparg_shift", w2 == 16, w2);
    wake_op(FUTEX_OP(FUTEX_OP_ADD, 0xfff, FUTEX_OP_CMP_EQ, 0));
    check("oparg_sign_extended", w2 == 15, w2);
    r = futex(&w1, FUTEX_WAKE_OP | FUTEX_PRIVATE_FLAG, 1, (void *)1, &w2,
              FUTEX_OP(FUTEX_OP_SET, 3, FUTEX_OP_CMP_GT, 1));
    check("private_wake_op", r == 0 && w2 == 3, w2);

    unsigned cmps[] = { FUTEX_OP_CMP_EQ, FUTEX_OP_CMP_NE, FUTEX_OP_CMP_LT,
                        FUTEX_OP_CMP_LE, FUTEX_OP_CMP_GT, FUTEX_OP_CMP_GE };
    int all_zero = 1;
    for (int i = 0; i < 6; i++) all_zero &= wake_op(FUTEX_OP(FUTEX_OP_SET, 3, cmps[i], 3)) == 0;
    check("every_cmp_accepted", all_zero, 0);

    w2 = 1;
    r = wake_op(FUTEX_OP(7, 9, FUTEX_OP_CMP_EQ, 0));
    check("unknown_op_enosys", r == -ENOSYS, r);
    check("unknown_op_leaves_word", w2 == 1, w2);
    r = wake_op(FUTEX_OP(FUTEX_OP_SET, 9, 9, 0));
    check("unknown_cmp_enosys", r == -ENOSYS, r);
    r = futex(&w1, FUTEX_WAKE_OP, 1, (void *)1, (volatile unsigned *)8, FUTEX_OP(FUTEX_OP_SET, 1, 0, 0));
    check("bad_second_address_efault", r == -EFAULT, r);

    /* Requeue with nobody to move. */
    w1 = 4;
    r = futex(&w1, FUTEX_REQUEUE, 1, (void *)1, &w2, 0);
    check("requeue_no_waiters_zero", r == 0, r);
    r = futex(&w1, FUTEX_CMP_REQUEUE, 1, (void *)1, &w2, 4);
    check("cmp_requeue_match_zero", r == 0, r);
    r = futex(&w1, FUTEX_CMP_REQUEUE, 1, (void *)1, &w2, 5);
    check("cmp_requeue_mismatch_eagain", r == -EAGAIN, r);
    r = futex(&w1, FUTEX_CMP_REQUEUE | FUTEX_PRIVATE_FLAG, 1, (void *)1, &w2, 5);
    check("private_cmp_requeue_mismatch_eagain", r == -EAGAIN, r);
    r = futex(&w1, FUTEX_CMP_REQUEUE, (unsigned)-1, (void *)1, &w2, 4);
    check("cmp_requeue_negative_wake_einval", r == -EINVAL, r);
    r = futex(&w1, FUTEX_CMP_REQUEUE, 1, (void *)-1, &w2, 4);
    check("cmp_requeue_negative_requeue_einval", r == -EINVAL, r);
    return finish();
}
