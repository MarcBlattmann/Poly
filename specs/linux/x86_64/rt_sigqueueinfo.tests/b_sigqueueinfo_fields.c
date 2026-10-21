/* rt_sigqueueinfo copies the caller's siginfo: si_errno, a negative si_code
 * and the pid, uid and value fields arrive unchanged, whether taken by a
 * handler or by rt_sigtimedwait, as do the other bytes of the first 48;
 * si_signo is set from the argument. Bytes 48 to 127 must be zero:
 * otherwise E2BIG and nothing is sent. */
#include "poly_ipc.h"

#define E2BIG 7

static struct k_siginfo seen;
static volatile int hits;

static void on_info(int sig, struct k_siginfo *info, void *ctx) {
    (void)sig;
    (void)ctx;
    seen = *info;
    hits++;
}

int test_main(void) {
    i64 pid = getpid();
    struct k_siginfo si;
    memset(&si, 0, sizeof si);
    si.si_signo = SIGRTMIN + 4;
    si.si_errno = 77;
    si.si_code = -42;
    si.u.raw[0] = 0x1111111122222222UL; /* pid and uid */
    si.u.raw[1] = 0x3333333344444444UL; /* value */

    sig_action(SIGRTMIN + 4, (u64)on_info, SA_SIGINFO, 0);
    i64 r = syscall3(SYS_rt_sigqueueinfo, pid, SIGRTMIN + 4, &si);
    check("queue_returns_zero", r == 0, r);
    check("handler_ran", hits == 1, hits);
    check("si_errno_passed", seen.si_errno == 77, seen.si_errno);
    check("negative_code_passed", seen.si_code == -42, seen.si_code);
    check("pid_uid_passed", seen.u.raw[0] == si.u.raw[0], 0);
    check("value_passed", seen.u.raw[1] == si.u.raw[1], 0);
    check("rest_zero", seen.u.raw[2] == 0 && seen.u.raw[13] == 0, 0);

    /* Bytes 32 to 47 are copied as they are. */
    si.u.raw[2] = 0xa1;
    si.u.raw[3] = 0xb2;
    r = syscall3(SYS_rt_sigqueueinfo, pid, SIGRTMIN + 4, &si);
    check("bytes_below_48_accepted", r == 0 && hits == 2, r);
    check("bytes_below_48_passed", seen.u.raw[2] == 0xa1 && seen.u.raw[3] == 0xb2, 0);
    si.u.raw[2] = si.u.raw[3] = 0;

    /* Bytes from 48 on. */
    si.u.raw[4] = 1;
    r = syscall3(SYS_rt_sigqueueinfo, pid, SIGRTMIN + 4, &si);
    check("byte_48_e2big", r == -E2BIG, r);
    si.u.raw[4] = 0;
    si.u.raw[13] = 1;
    r = syscall3(SYS_rt_sigqueueinfo, pid, SIGRTMIN + 4, &si);
    check("last_bytes_e2big", r == -E2BIG, r);
    check("e2big_sent_nothing", hits == 2 && sig_pending() == 0, hits);
    si.u.raw[13] = 0;

    /* The same through rt_sigtimedwait. */
    sig_mask(SIG_BLOCK, SIGBIT(SIGRTMIN + 4), 0);
    si.si_errno = 5;
    si.u.raw[1] = 0x0102030405060708UL;
    syscall3(SYS_rt_sigqueueinfo, pid, SIGRTMIN + 4, &si);
    struct k_siginfo got;
    memset(&got, 0x5a, sizeof got);
    u64 set = SIGBIT(SIGRTMIN + 4);
    struct k_timespec zero = { 0, 0 };
    r = syscall4(SYS_rt_sigtimedwait, &set, &got, &zero, 8);
    check("timedwait_returns_signal", r == SIGRTMIN + 4, r);
    check("timedwait_signo", got.si_signo == SIGRTMIN + 4, got.si_signo);
    check("timedwait_errno_passed", got.si_errno == 5, got.si_errno);
    check("timedwait_code_passed", got.si_code == -42, got.si_code);
    check("timedwait_value_passed", got.u.raw[1] == 0x0102030405060708UL, 0);
    check("timedwait_rest_zeroed", got.u.raw[2] == 0 && got.u.raw[13] == 0, 0);
    check("handler_not_run_for_waited_signal", hits == 2, hits);
    return finish();
}
