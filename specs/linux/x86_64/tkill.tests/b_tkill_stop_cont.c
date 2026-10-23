/* Job-control and fatal signals through tkill to a child: SIGSTOP stops it
 * (wait4 with WUNTRACED: status 0x137f), SIGCONT continues it (WCONTINUED:
 * status 0xffff), a SIGTERM sent while it is stopped acts once it is
 * continued, and SIGKILL ends it (status: killed by signal 9). A child that
 * sends itself a default-action SIGUSR1 dies of it before tkill returns.
 * poly-diff: syscalls=off (two processes)
 */
#include "poly_ipc.h"

#define WUNTRACED 2
#define WCONTINUED 8

static i64 wait_opts(i64 pid, int opts, int *st) { return syscall4(SYS_wait4, pid, st, opts, 0); }

int test_main(void) {
    i64 c = fork();
    if (c == 0)
        for (;;) sleep_ms(1000);
    check("fork", c > 0, c);
    int st = 0;
    i64 r = syscall2(SYS_tkill, c, SIGSTOP);
    check("sigstop", r == 0, r);
    r = wait_opts(c, WUNTRACED, &st);
    check("stopped", r == c && st == 0x137f, st);
    r = syscall2(SYS_tkill, c, SIGCONT);
    check("sigcont", r == 0, r);
    r = wait_opts(c, WCONTINUED, &st);
    check("continued", r == c && st == 0xffff, st);
    r = syscall2(SYS_tkill, c, SIGKILL);
    check("sigkill", r == 0, r);
    st = wait_status(c);
    check("killed_by_9", st != -1 && W_SIGNALED(st) && W_TERMSIG(st) == SIGKILL, st);

    c = fork();
    if (c == 0)
        for (;;) sleep_ms(1000);
    check("fork2", c > 0, c);
    syscall2(SYS_tkill, c, SIGSTOP);
    r = wait_opts(c, WUNTRACED, &st);
    check("stopped2", r == c && st == 0x137f, st);
    r = syscall2(SYS_tkill, c, SIGTERM);
    check("sigterm_while_stopped", r == 0, r);
    r = wait_opts(c, WNOHANG | WUNTRACED, &st);
    check("still_stopped", r == 0, r);
    syscall2(SYS_tkill, c, SIGCONT);
    st = wait_status(c);
    check("term_after_cont", st != -1 && W_SIGNALED(st) && W_TERMSIG(st) == SIGTERM, st);

    c = fork();
    if (c == 0) {
        syscall2(SYS_tkill, gettid(), SIGUSR1);
        syscall1(SYS_exit_group, 5);
    }
    check("fork3", c > 0, c);
    st = wait_status(c);
    check("self_default_usr1_fatal", st != -1 && W_SIGNALED(st) && W_TERMSIG(st) == SIGUSR1, st);
    return finish();
}
