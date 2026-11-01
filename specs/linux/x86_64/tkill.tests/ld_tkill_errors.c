/* tkill errors and argument widths. tid <= 0 is EINVAL, checked first
 * (also with an invalid signal). A tid that names no thread is ESRCH,
 * checked before the signal number (an invalid signal to a missing thread
 * is ESRCH). An existing target with a signal outside 0..64 is EINVAL and
 * nothing is sent. tid and sig are 32-bit ints: the upper halves of the
 * registers are ignored (tid (1<<32)|own tid is the own thread, sig
 * (1<<32)|SIGUSR1 is SIGUSR1, tid 0xffffffff is -1).
 */
#include "poly_ipc.h"

static volatile int hits;
static void on_usr1(int sig) {
    (void)sig;
    hits++;
}

static i64 tkill(i64 tid, i64 sig) { return syscall2(SYS_tkill, tid, sig); }

int test_main(void) {
    i64 tid = gettid();
    sig_handler(SIGUSR1, on_usr1, 0);

    check("tid_0_einval", tkill(0, 0) == -EINVAL, tkill(0, 0));
    check("tid_minus1_einval", tkill(-1, SIGUSR1) == -EINVAL, tkill(-1, SIGUSR1));
    check("tid_negative_einval", tkill(-12345, 0) == -EINVAL, tkill(-12345, 0));
    check("tid_0_bad_signal_einval", tkill(0, 999) == -EINVAL, tkill(0, 999));
    check("tid_0xffffffff_is_minus1", tkill(0xffffffffL, 0) == -EINVAL, tkill(0xffffffffL, 0));

    /* No such thread: above any pid_max (at most 4194304). */
    check("missing_tid_esrch", tkill(0x7fffffff, 0) == -ESRCH, tkill(0x7fffffff, 0));
    check("missing_tid_signal_esrch", tkill(0x7fffffff, SIGUSR1) == -ESRCH, tkill(0x7fffffff, SIGUSR1));
    check("missing_tid_bad_signal_esrch", tkill(0x7fffffff, 999) == -ESRCH, tkill(0x7fffffff, 999));

    /* Bad signal numbers to an existing thread. */
    check("signal_65_einval", tkill(tid, 65) == -EINVAL, tkill(tid, 65));
    check("signal_minus1_einval", tkill(tid, -1) == -EINVAL, tkill(tid, -1));
    check("signal_1000_einval", tkill(tid, 1000) == -EINVAL, tkill(tid, 1000));
    check("bad_signal_sent_nothing", hits == 0 && sig_pending() == 0, hits);
    check("signal_0_still_ok", tkill(tid, 0) == 0 && sig_pending() == 0, 0);

    /* Widths. */
    i64 r = tkill((1L << 32) | tid, SIGUSR1);
    check("tid_high_bits_ignored", r == 0 && hits == 1, r);
    r = tkill(tid, (1L << 32) | SIGUSR1);
    check("signal_high_bits_ignored", r == 0 && hits == 2, r);

    /* SIGRTMAX (64) is valid. */
    sig_mask(SIG_BLOCK, SIGBIT(SIGRTMAX), 0);
    r = tkill(tid, SIGRTMAX);
    check("sigrtmax_ok", r == 0, r);
    check("sigrtmax_pending", sig_pending() == SIGBIT(SIGRTMAX), (i64)sig_pending());
    u64 set = SIGBIT(SIGRTMAX);
    struct k_timespec zero = { 0, 0 };
    r = syscall4(SYS_rt_sigtimedwait, &set, 0, &zero, 8);
    check("sigrtmax_taken", r == SIGRTMAX, r);
    return finish();
}
