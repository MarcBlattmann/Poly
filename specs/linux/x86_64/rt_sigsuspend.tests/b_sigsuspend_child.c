/* rt_sigsuspend woken by another process: the parent blocks SIGUSR1,
 * forks, and suspends with an empty mask; the child sends SIGUSR1 (before
 * or during the suspend, both end it). The handler sees the child as
 * sender (SA_SIGINFO), and a second suspend with SIGUSR1 blocked in its
 * mask is ended by SIGUSR2 instead, leaving SIGUSR1 pending.
 * poly-diff: syscalls=off (two processes)
 */
#include "poly_ipc.h"

static volatile int got, sender;

static void on_info(int sig, struct k_siginfo *info, void *ctx) {
    (void)ctx;
    got = sig;
    sender = info->u.kill.pid;
}

int test_main(void) {
    sig_action(SIGUSR1, (u64)on_info, SA_SIGINFO, 0);
    sig_action(SIGUSR2, (u64)on_info, SA_SIGINFO, 0);
    sig_mask(SIG_SETMASK, SIGBIT(SIGUSR1) | SIGBIT(SIGUSR2), 0);
    int go[2];
    syscall2(SYS_pipe2, go, 0);
    i64 parent = getpid();
    i64 pid = fork();
    if (pid == 0) {
        char c;
        syscall2(SYS_kill, parent, SIGUSR1);
        read(go[0], &c, 1);
        syscall2(SYS_kill, parent, SIGUSR1);
        syscall2(SYS_kill, parent, SIGUSR2);
        exit_group(0);
    }
    u64 m = 0;
    i64 r = syscall2(SYS_rt_sigsuspend, &m, 8);
    check("woken_eintr", r == -EINTR, r);
    check("woken_by_sigusr1", got == SIGUSR1, got);
    check("sender_is_child", sender == pid, 0);

    got = 0;
    write(go[1], "x", 1);
    m = SIGBIT(SIGUSR1);
    r = syscall2(SYS_rt_sigsuspend, &m, 8);
    check("second_woken_eintr", r == -EINTR, r);
    check("second_woken_by_sigusr2", got == SIGUSR2, got);
    check("exit_child", wait_exit(pid) == 0, 0);
    check("sigusr1_left_pending", sig_pending() == SIGBIT(SIGUSR1), (i64)sig_pending());
    return finish();
}
