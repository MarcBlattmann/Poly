/* posix_spawn, BSD 244, what the child inherits: open descriptors unless
 * close-on-exec, ignored signals, the signal mask; caught signals are reset
 * to their default action (as for execve). Child lines as in execve. */
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
    char *argv[] = {poly_argv[0], "child", 0};
    char *env[] = {0};
    i32 pid = 0;
    sysret r = bsd6(SYS_posix_spawn, &pid, poly_argv[0], 0, argv, env, 0);
    check_ok("posix_spawn", r);
    int st = poly_wait(pid);
    check("the child ran", W_EXITED(st) && W_EXITSTATUS(st) == 0, st);
    return finish();
}
