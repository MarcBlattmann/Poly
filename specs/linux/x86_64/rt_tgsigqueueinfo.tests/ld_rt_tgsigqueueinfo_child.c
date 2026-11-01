/* rt_tgsigqueueinfo to another process: a child's main thread is
 * (tgid = tid = child's pid). With SI_QUEUE the child, which has the
 * signal blocked and waits for it in rt_sigtimedwait, receives the
 * caller's siginfo (code and value as given). The child's tid with the
 * caller's pid as tgid is ESRCH. Once the child has been reaped it is
 * ESRCH.
 * poly-diff: syscalls=off (the child runs while the parent signals it)
 */
#include "poly_ipc.h"

int test_main(void) {
    int sig = SIGRTMIN + 9;
    sig_mask(SIG_BLOCK, SIGBIT(sig), 0); /* inherited: no early delivery */
    i64 c = fork();
    if (c == 0) {
        u64 set = SIGBIT(sig);
        struct k_siginfo got;
        memset(&got, 0, sizeof got);
        i64 r = syscall4(SYS_rt_sigtimedwait, &set, &got, 0, 8);
        int code = 0;
        if (r != sig) code |= 1;
        if (got.si_code != SI_QUEUE) code |= 2;
        if (got.u.raw[1] != 0xabcdef) code |= 4;
        if (got.si_errno != 9) code |= 8;
        syscall1(SYS_exit, code);
    }
    check("child_started", c > 0, c);
    struct k_siginfo si;
    memset(&si, 0, sizeof si);
    si.si_code = SI_QUEUE;
    si.si_errno = 9;
    si.u.raw[1] = 0xabcdef;
    i64 pid = getpid();
    i64 r = syscall4(SYS_rt_tgsigqueueinfo, pid, c, sig, &si);
    check("child_in_wrong_group_esrch", r == -ESRCH, r);
    r = syscall4(SYS_rt_tgsigqueueinfo, c, c, 0, &si);
    check("signal_0_to_child", r == 0, r);
    r = syscall4(SYS_rt_tgsigqueueinfo, c, c, sig, &si);
    check("queued_to_child", r == 0, r);
    int s = wait_status(c);
    check("child_got_signal", W_EXITED(s) && (W_EXITCODE(s) & 1) == 0, s);
    check("child_got_code", W_EXITED(s) && (W_EXITCODE(s) & 2) == 0, s);
    check("child_got_value", W_EXITED(s) && (W_EXITCODE(s) & 4) == 0, s);
    check("child_got_errno", W_EXITED(s) && (W_EXITCODE(s) & 8) == 0, s);
    r = syscall4(SYS_rt_tgsigqueueinfo, c, c, sig, &si);
    check("reaped_child_esrch", r == -ESRCH, r);
    return finish();
}
