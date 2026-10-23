/* tkill with a thread id of 0 or below is EINVAL, whatever the signal
 * (also an invalid one). The tid is an int: the upper 32 bits of the
 * register are ignored, so 2^32 + own tid reaches the caller and 2^32 alone
 * is tid 0.
 */
#include "poly_ipc.h"

static volatile int hits;
static void on_usr1(int sig) {
    (void)sig;
    hits++;
}

int test_main(void) {
    i64 tid = gettid();
    i64 r = syscall2(SYS_tkill, 0, 0);
    check("tid_zero_einval", r == -EINVAL, r);
    r = syscall2(SYS_tkill, 0, SIGUSR1);
    check("tid_zero_with_signal_einval", r == -EINVAL, r);
    r = syscall2(SYS_tkill, -1, 0);
    check("tid_minus_one_einval", r == -EINVAL, r);
    r = syscall2(SYS_tkill, -1, 65);
    check("tid_minus_one_bad_signal_einval", r == -EINVAL, r);
    r = syscall2(SYS_tkill, (int)0x80000000, 0);
    check("tid_int_min_einval", r == -EINVAL, r);
    r = syscall2(SYS_tkill, -(i64)tid, 0);
    check("negative_own_tid_einval", r == -EINVAL, r);
    r = syscall2(SYS_tkill, 0xffffffffL, 0);
    check("tid_all_ones_32_is_minus_one", r == -EINVAL, r);
    r = syscall2(SYS_tkill, 1L << 32, 0);
    check("tid_upper_bits_only_is_zero", r == -EINVAL, r);

    sig_handler(SIGUSR1, on_usr1, 0);
    r = syscall2(SYS_tkill, (1L << 32) + tid, SIGUSR1);
    check("tid_upper_bits_ignored", r == 0, r);
    check("upper_bits_tid_delivered", hits == 1, hits);
    check("no_signal_pending", sig_pending() == 0, (i64)sig_pending());
    return finish();
}
