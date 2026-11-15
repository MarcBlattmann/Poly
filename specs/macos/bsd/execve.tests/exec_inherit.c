/* execve(2), BSD 59, what survives: a descriptor stays open unless it is
 * close-on-exec, ignored signals stay ignored, caught signals go back to
 * their default action, and the signal mask is kept. Child lines: the
 * descriptors 5 (open) and 6 (close-on-exec), SIGUSR1 (ignored) and SIGUSR2
 * (caught) as seen by the new program. */
#include "poly_mtest.h"

static void caught(int sig, void *info, void *uctx) {
    (void)sig;
    (void)info;
    (void)uctx;
}

int test_main(void) {
    if (is_child("child")) {
        poly_report_child();
        poly_exit(0);
    }
    bsd3(SYS_open, "/dev/null", O_RDONLY, 0);
    bsd2(SYS_dup2, 3, 5);
    bsd2(SYS_dup2, 3, 6);
    bsd3(SYS_fcntl, 6, F_SETFD, FD_CLOEXEC);
    poly_sigaction(SIGUSR1, SIG_IGN, 0, 0, 0);
    poly_sigaction(SIGUSR2, (void *)caught, 0, SA_SIGINFO, 0);
    u32 mask = SIGBIT(SIGINFO);
    bsd3(SYS_sigprocmask, SIG_BLOCK, &mask, 0);
    i64 pid = poly_start_self("child", 0, 0);
    int st = poly_wait(pid);
    check("the program ran", W_EXITED(st) && W_EXITSTATUS(st) == 0, st);
    return finish();
}
