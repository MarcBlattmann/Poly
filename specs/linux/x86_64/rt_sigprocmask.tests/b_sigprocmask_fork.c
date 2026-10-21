/* Across fork the child inherits the signal mask and the actions, but not
 * the pending signals; a signal the child blocks stays pending in it.
 * poly-diff: syscalls=off (child processes)
 */
#include "poly_ipc.h"

static volatile int hits;
static void on_usr2(int sig) {
    (void)sig;
    hits++;
}

int test_main(void) {
    sig_handler(SIGUSR2, on_usr2, 0);
    sig_mask(SIG_SETMASK, SIGBIT(SIGUSR1) | SIGBIT(SIGTERM), 0);
    raise_sig(SIGUSR1);
    i64 pid = fork();
    if (pid == 0) {
        int code = 0;
        if (sig_current_mask() != (SIGBIT(SIGUSR1) | SIGBIT(SIGTERM))) code |= 1;
        if (sig_pending() != 0) code |= 2;
        raise_sig(SIGUSR2);
        if (hits != 1) code |= 4;
        raise_sig(SIGTERM);
        if (sig_pending() != SIGBIT(SIGTERM)) code |= 8;
        exit_group(code);
    }
    int code = wait_exit(pid);
    check("child_inherits_mask", code >= 0 && (code & 1) == 0, code);
    check("child_has_no_pending", code >= 0 && (code & 2) == 0, code);
    check("child_inherits_handler", code >= 0 && (code & 4) == 0, code);
    check("child_blocked_signal_pending", code >= 0 && (code & 8) == 0, code);
    check("parent_still_pending", sig_pending() == SIGBIT(SIGUSR1), (i64)sig_pending());

    /* A blocked fatal signal sent to the child is acted on once unblocked. */
    int pfd[2];
    syscall2(SYS_pipe2, pfd, 0);
    pid = fork();
    if (pid == 0) {
        char c;
        sig_action(SIGTERM, SIG_DFL, 0, 0);
        close(pfd[1]);
        read(pfd[0], &c, 1); /* the parent sends SIGTERM, then closes the pipe */
        if (sig_pending() != SIGBIT(SIGTERM)) exit_group(20);
        sig_mask(SIG_UNBLOCK, SIGBIT(SIGTERM), 0);
        exit_group(21);
    }
    close(pfd[0]);
    syscall2(SYS_kill, pid, SIGTERM);
    close(pfd[1]);
    int st = wait_status(pid);
    check("unblocked_fatal_signal_kills", W_SIGNALED(st) && W_TERMSIG(st) == SIGTERM, st);
    return finish();
}
