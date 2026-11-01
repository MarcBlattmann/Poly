/* Queueing through rt_tgsigqueueinfo while the signals are blocked: every
 * call with a real-time signal queues one more instance with its own
 * siginfo, taken in the order sent; a standard signal is pending at most
 * once (later calls return 0 but the first siginfo is the one kept). Of
 * several pending signals the lowest number is taken first. Pending
 * signals are the thread's own (rt_sigpending shows them).
 */
#include "poly_ipc.h"

static i64 tgsq(i64 tgid, i64 tid, i64 sig, void *info) {
    return syscall4(SYS_rt_tgsigqueueinfo, tgid, tid, sig, info);
}

int test_main(void) {
    i64 pid = getpid(), tid = gettid();
    int rt = SIGRTMIN + 5;
    u64 both = SIGBIT(rt) | SIGBIT(SIGUSR1);
    sig_mask(SIG_BLOCK, both, 0);

    struct k_siginfo si;
    memset(&si, 0, sizeof si);
    si.si_code = SI_QUEUE;
    int ok = 1;
    for (int i = 0; i < 4; i++) {
        si.u.raw[1] = 100 + i;
        if (tgsq(pid, tid, rt, &si) != 0) ok = 0;
    }
    check("rt_queued_4", ok, 0);
    for (int i = 0; i < 3; i++) {
        si.u.raw[1] = 200 + i;
        if (tgsq(pid, tid, SIGUSR1, &si) != 0) ok = 0;
    }
    check("standard_calls_return_0", ok, 0);
    check("both_pending", sig_pending() == both, (i64)sig_pending());

    struct k_timespec zero = { 0, 0 };
    struct k_siginfo got;
    memset(&got, 0, sizeof got);
    i64 r = syscall4(SYS_rt_sigtimedwait, &both, &got, &zero, 8);
    check("standard_first", r == SIGUSR1, r);
    check("standard_keeps_first_info", got.u.raw[1] == 200, (i64)got.u.raw[1]);
    check("standard_code", got.si_code == SI_QUEUE, got.si_code);
    for (int i = 0; i < 4; i++) {
        memset(&got, 0, sizeof got);
        r = syscall4(SYS_rt_sigtimedwait, &both, &got, &zero, 8);
        check(i == 0 ? "rt_1" : i == 1 ? "rt_2" : i == 2 ? "rt_3" : "rt_4", r == rt && got.u.raw[1] == (u64)(100 + i),
              (i64)got.u.raw[1]);
    }
    r = syscall4(SYS_rt_sigtimedwait, &both, &got, &zero, 8);
    check("queue_empty", r == -EAGAIN, r);
    check("nothing_pending", sig_pending() == 0, (i64)sig_pending());

    /* Two real-time signals: lower number first, each in its own order. */
    int rt2 = SIGRTMIN + 1;
    u64 rts = SIGBIT(rt) | SIGBIT(rt2);
    sig_mask(SIG_BLOCK, rts, 0);
    si.u.raw[1] = 1;
    tgsq(pid, tid, rt, &si);
    si.u.raw[1] = 2;
    tgsq(pid, tid, rt2, &si);
    si.u.raw[1] = 3;
    tgsq(pid, tid, rt, &si);
    r = syscall4(SYS_rt_sigtimedwait, &rts, &got, &zero, 8);
    check("lower_rt_first", r == rt2 && got.u.raw[1] == 2, r);
    r = syscall4(SYS_rt_sigtimedwait, &rts, &got, &zero, 8);
    check("higher_rt_second", r == rt && got.u.raw[1] == 1, r);
    r = syscall4(SYS_rt_sigtimedwait, &rts, &got, &zero, 8);
    check("higher_rt_third", r == rt && got.u.raw[1] == 3, r);
    return finish();
}
