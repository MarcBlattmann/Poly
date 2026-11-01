/* rt_tgsigqueueinfo to another thread of the caller's process: with
 * SI_QUEUE the signal is pending for that thread only (not for the
 * sender) and arrives there with the caller's siginfo; with SI_USER (0) or
 * SI_TKILL the call is EPERM, because the target is not the caller's own
 * thread, even within the same process. A tgid that is not the thread's
 * group is ESRCH.
 * poly-diff: syscalls=off (two threads run at once)
 */
#include "poly_ipc.h"

static volatile unsigned go;
static volatile int thread_tid_word;
static volatile i64 t_pending, t_wait, t_code, t_value, t_more;
static char stack[65536] __attribute__((aligned(16)));

static int thread_fn(void *arg) {
    (void)arg;
    gate_wait(&go);
    t_pending = (i64)sig_pending();
    u64 set = SIGBIT(SIGRTMIN + 7);
    struct k_siginfo got;
    struct k_timespec zero = { 0, 0 };
    memset(&got, 0, sizeof got);
    t_wait = syscall4(SYS_rt_sigtimedwait, &set, &got, &zero, 8);
    t_code = got.si_code;
    t_value = (i64)got.u.raw[1];
    t_more = syscall4(SYS_rt_sigtimedwait, &set, &got, &zero, 8);
    return 0;
}

int test_main(void) {
    i64 pid = getpid();
    int sig = SIGRTMIN + 7;
    sig_mask(SIG_BLOCK, SIGBIT(sig), 0); /* inherited by the thread */
    int ptid = 0;
    i64 t = poly_clone(THREAD_FLAGS, stack + sizeof stack, &ptid, (int *)&thread_tid_word, thread_fn, 0);
    check("thread_created", t > 0, t);
    thread_tid_word = (int)t;

    struct k_siginfo si;
    memset(&si, 0, sizeof si);
    si.si_code = SI_USER;
    i64 r = syscall4(SYS_rt_tgsigqueueinfo, pid, t, sig, &si);
    check("si_user_to_sibling_eperm", r == -EPERM, r);
    si.si_code = SI_TKILL;
    r = syscall4(SYS_rt_tgsigqueueinfo, pid, t, sig, &si);
    check("si_tkill_to_sibling_eperm", r == -EPERM, r);
    si.si_code = SI_QUEUE;
    si.u.raw[1] = 4242;
    r = syscall4(SYS_rt_tgsigqueueinfo, pid + 1, t, sig, &si);
    check("wrong_tgid_esrch", r == -ESRCH, r);
    r = syscall4(SYS_rt_tgsigqueueinfo, pid, t, sig, &si);
    check("si_queue_to_sibling", r == 0, r);
    check("not_pending_for_sender", sig_pending() == 0, (i64)sig_pending());

    gate_open(&go);
    while (thread_tid_word != 0) futex((volatile unsigned *)&thread_tid_word, FUTEX_WAIT, thread_tid_word, 0, 0, 0);
    check("pending_for_sibling", t_pending == (i64)SIGBIT(sig), t_pending);
    check("sibling_takes_it", t_wait == sig, t_wait);
    check("sibling_code", t_code == SI_QUEUE, t_code);
    check("sibling_value", t_value == 4242, t_value);
    check("sent_once", t_more == -EAGAIN, t_more);
    return finish();
}
