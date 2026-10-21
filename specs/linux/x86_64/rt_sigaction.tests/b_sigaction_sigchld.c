/* SIGCHLD actions with child processes: a SA_SIGINFO handler sees
 * CLD_EXITED / CLD_KILLED with the child's pid and status; SIG_IGN and
 * SA_NOCLDWAIT make children reap themselves, so wait4 fails with ECHILD.
 * poly-diff: syscalls=off (two processes; the order of their calls is scheduling)
 */
#include "poly_ipc.h"

static volatile int chld_code, chld_status, chld_pid, chld_signo, chld_calls;

static void on_chld(int sig, struct k_siginfo *info, void *ctx) {
    (void)ctx;
    chld_calls++;
    chld_signo = sig;
    chld_code = info->si_code;
    chld_pid = info->u.kill.pid;
    chld_status = (int)(info->u.kill.value & 0xffffffff);
}

int test_main(void) {
    /* Handler with SA_SIGINFO; block SIGCHLD until the child is reaped. */
    sig_action(SIGCHLD, (u64)on_chld, SA_SIGINFO, 0);
    sig_mask(SIG_BLOCK, SIGBIT(SIGCHLD), 0);
    i64 pid = fork();
    if (pid == 0) exit_group(7);
    int st = wait_status(pid);
    check("child_exit_status_7", W_EXITED(st) && W_EXITCODE(st) == 7, st);
    check("sigchld_pending_after_exit", (sig_pending() & SIGBIT(SIGCHLD)) != 0, (i64)sig_pending());
    sig_mask(SIG_UNBLOCK, SIGBIT(SIGCHLD), 0);
    check("sigchld_handler_ran", chld_calls == 1 && chld_signo == SIGCHLD, chld_calls);
    check("sigchld_code_exited", chld_code == CLD_EXITED, chld_code);
    check("sigchld_pid_is_child", chld_pid == pid, 0);
    check("sigchld_status_is_exit_code", chld_status == 7, chld_status);

    /* Killed child: CLD_KILLED with the signal number as status. */
    sig_mask(SIG_BLOCK, SIGBIT(SIGCHLD), 0);
    int pfd[2];
    syscall2(SYS_pipe2, pfd, 0);
    pid = fork();
    if (pid == 0) {
        char c;
        close(pfd[1]);
        read(pfd[0], &c, 1); /* blocks until killed */
        exit_group(1);
    }
    close(pfd[0]);
    syscall2(SYS_kill, pid, SIGTERM);
    st = wait_status(pid);
    check("killed_child_status", W_SIGNALED(st) && W_TERMSIG(st) == SIGTERM, st);
    sig_mask(SIG_UNBLOCK, SIGBIT(SIGCHLD), 0);
    check("sigchld_code_killed", chld_code == CLD_KILLED, chld_code);
    check("sigchld_status_is_signal", chld_status == SIGTERM, chld_status);
    close(pfd[1]);

    /* SIG_IGN on SIGCHLD: no zombie; wait4 waits for the child to end and fails. */
    sig_action(SIGCHLD, SIG_IGN, 0, 0);
    pid = fork();
    if (pid == 0) exit_group(3);
    int status = 0;
    i64 r = syscall4(SYS_wait4, pid, &status, 0, 0);
    check("ignored_sigchld_wait_echild", r == -ECHILD, r);
    r = syscall4(SYS_wait4, -1, &status, WNOHANG, 0);
    check("ignored_sigchld_no_children_left", r == -ECHILD, r);

    /* SA_NOCLDWAIT with a handler: no zombie either. */
    chld_calls = 0;
    sig_action(SIGCHLD, (u64)on_chld, SA_SIGINFO | SA_NOCLDWAIT, 0);
    pid = fork();
    if (pid == 0) exit_group(4);
    r = syscall4(SYS_wait4, pid, &status, 0, 0);
    check("nocldwait_wait_echild", r == -ECHILD, r);

    /* Back to SIG_DFL: zombies again. */
    sig_action(SIGCHLD, SIG_DFL, 0, 0);
    pid = fork();
    if (pid == 0) exit_group(5);
    check("default_sigchld_reaps_status", wait_exit(pid) == 5, 0);
    return finish();
}
