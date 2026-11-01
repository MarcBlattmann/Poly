/* pidfd_send_signal(pidfd, sig, NULL, 0) sends sig to the process the
 * pidfd refers to, as kill(2) would (si_code SI_USER, si_pid and si_uid
 * of the sender), and returns 0; sig 0 only checks that it may be sent.
 * A process that has exited but not been reaped (a zombie) still
 * accepts signals (0 is returned, nothing happens); once reaped, ESRCH.
 * Sending to oneself delivers to the caller's handler before the call
 * returns (the signal is unblocked).
 * poly-diff: syscalls=off (two processes run at once)
 */
#include "a_sys.h"

#define SYS_pidfd_send_signal 424
#define SYS_pidfd_open 434

static i64 pss(i64 fd, i64 sig, void *info, u64 flags) { return syscall4(SYS_pidfd_send_signal, fd, sig, info, flags); }

void la_restorer(void);
__asm__(".text\n"
        ".global la_restorer\n"
        "la_restorer:\n"
        "    mov $15, %eax\n"
        "    syscall\n"
        "    hlt\n");

static volatile i32 got_sig, got_code, got_pid, got_uid;
static void on_usr1(int sig, struct a_siginfo *si, void *uc) {
    (void)uc;
    got_sig = sig;
    got_code = si->si_code;
    got_pid = si->si_pid;
    got_uid = si->si_uid;
}

int test_main(void) {
    a_block_sigchld();
    struct a_sigaction sa = { (u64)on_usr1, SA_RESTORER | 4 /* SA_SIGINFO */, (u64)la_restorer, 0 };
    syscall4(SYS_rt_sigaction, SIGUSR1, &sa, 0, 8);

    i64 me = syscall0(SYS_getpid);
    i64 self = syscall2(SYS_pidfd_open, me, 0);
    expect("self_signal", pss(self, SIGUSR1, 0, 0), 0);
    expect("handler_ran_before_return", got_sig, SIGUSR1);
    expect("si_code_si_user", got_code, 0);
    expect("si_pid_sender", got_pid, me);
    expect("si_uid_sender", got_uid, syscall0(SYS_getuid));
    expect("self_sig_0", pss(self, 0, 0, 0), 0);

    i64 pid = syscall0(SYS_fork);
    if (pid == 0) {
        for (;;) syscall0(34 /* pause */);
    }
    i64 fd = syscall2(SYS_pidfd_open, pid, 0);
    expect("child_sig_0", pss(fd, 0, 0, 0), 0);
    expect("child_sigterm", pss(fd, SIGTERM, 0, 0), 0);
    i32 status = -1;
    struct a_siginfo si;
    memset(&si, 0, sizeof si);
    /* WNOWAIT: leave it a zombie. */
    expect("child_ended", syscall5(SYS_waitid, P_PID, pid, &si, WEXITED | WNOWAIT, 0), 0);
    expect("killed_by_sigterm", si.si_code == CLD_KILLED && si.si_status == SIGTERM, 1);
    expect("zombie_sig_0", pss(fd, 0, 0, 0), 0);
    expect("zombie_sigkill", pss(fd, SIGKILL, 0, 0), 0);
    syscall4(SYS_wait4, pid, &status, 0, 0);
    check("status_sigterm", A_WIFSIGNALED(status) && A_WTERMSIG(status) == SIGTERM, status);
    expect("reaped_esrch", pss(fd, 0, 0, 0), -ESRCH);
    expect("reaped_sigterm_esrch", pss(fd, SIGTERM, 0, 0), -ESRCH);
    return finish();
}
