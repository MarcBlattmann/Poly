/* tkill(tid, sig) sends sig to the one thread tid. Sent to the caller's
 * own thread id (which is the pid in a single-threaded process), a handled
 * signal runs its handler before tkill returns, with si_code SI_TKILL (-6),
 * si_pid = the caller's pid, si_uid = the caller's real uid, si_errno 0.
 * Signal 0 checks only: nothing is sent. A blocked signal stays pending
 * (rt_sigpending shows it) and rt_sigtimedwait takes it with the same
 * siginfo. A standard signal sent twice while blocked is pending once; a
 * real-time signal is queued once per call. A blocked signal stays pending
 * even when its action is SIG_IGN.
 */
#include "poly_ipc.h"

static struct k_siginfo seen;
static volatile int hits;

static void on_info(int sig, struct k_siginfo *info, void *ctx) {
    (void)sig;
    (void)ctx;
    seen = *info;
    hits++;
}

static i64 tkill(i64 tid, i64 sig) { return syscall2(SYS_tkill, tid, sig); }

int test_main(void) {
    i64 pid = getpid(), tid = gettid();
    i64 uid = syscall0(SYS_getuid);
    check("tid_is_pid", tid == pid, 0);

    sig_action(SIGUSR1, (u64)on_info, SA_SIGINFO, 0);
    memset(&seen, 0x5a, sizeof seen);
    i64 r = tkill(tid, SIGUSR1);
    check("tkill_returns_0", r == 0, r);
    check("handler_ran_before_return", hits == 1, hits);
    check("si_signo", seen.si_signo == SIGUSR1, seen.si_signo);
    check("si_errno_0", seen.si_errno == 0, seen.si_errno);
    check("si_code_si_tkill", seen.si_code == SI_TKILL, seen.si_code);
    check("si_pid_caller", seen.u.kill.pid == pid, 0);
    check("si_uid_real_uid", seen.u.kill.uid == (unsigned)uid, 0);

    r = tkill(tid, 0);
    check("signal_0_returns_0", r == 0, r);
    check("signal_0_sends_nothing", hits == 1 && sig_pending() == 0, hits);

    /* Blocked: pending until taken. */
    sig_mask(SIG_BLOCK, SIGBIT(SIGUSR2) | SIGBIT(SIGRTMIN + 3), 0);
    r = tkill(tid, SIGUSR2);
    check("blocked_returns_0", r == 0, r);
    check("blocked_is_pending", sig_pending() == SIGBIT(SIGUSR2), (i64)sig_pending());
    r = tkill(tid, SIGUSR2);
    check("second_standard_returns_0", r == 0, r);
    r = tkill(tid, SIGRTMIN + 3);
    tkill(tid, SIGRTMIN + 3);
    check("rt_pending", sig_pending() == (SIGBIT(SIGUSR2) | SIGBIT(SIGRTMIN + 3)), (i64)sig_pending());

    u64 set = SIGBIT(SIGUSR2) | SIGBIT(SIGRTMIN + 3);
    struct k_timespec zero = { 0, 0 };
    struct k_siginfo got;
    memset(&got, 0, sizeof got);
    r = syscall4(SYS_rt_sigtimedwait, &set, &got, &zero, 8);
    check("timedwait_lowest_first", r == SIGUSR2, r);
    check("timedwait_code_si_tkill", got.si_code == SI_TKILL, got.si_code);
    check("timedwait_pid", got.u.kill.pid == pid, 0);
    check("timedwait_uid", got.u.kill.uid == (unsigned)uid, 0);
    r = syscall4(SYS_rt_sigtimedwait, &set, &got, &zero, 8);
    check("rt_first_copy", r == SIGRTMIN + 3, r);
    r = syscall4(SYS_rt_sigtimedwait, &set, &got, &zero, 8);
    check("rt_second_copy", r == SIGRTMIN + 3, r);
    r = syscall4(SYS_rt_sigtimedwait, &set, &got, &zero, 8);
    check("standard_was_pending_once", r == -EAGAIN, r);

    /* Blocked and ignored: still queued. */
    sig_action(SIGUSR2, SIG_IGN, 0, 0);
    r = tkill(tid, SIGUSR2);
    check("ignored_blocked_returns_0", r == 0, r);
    check("ignored_blocked_pending", sig_pending() == SIGBIT(SIGUSR2), (i64)sig_pending());
    /* Unblocking an ignored signal discards it. */
    sig_mask(SIG_UNBLOCK, SIGBIT(SIGUSR2), 0);
    check("ignored_discarded_on_unblock", sig_pending() == 0, (i64)sig_pending());
    r = tkill(tid, SIGUSR2);
    check("ignored_unblocked_returns_0", r == 0, r);
    check("ignored_unblocked_not_pending", sig_pending() == 0, (i64)sig_pending());
    check("handler_count_final", hits == 1, hits);
    return finish();
}
