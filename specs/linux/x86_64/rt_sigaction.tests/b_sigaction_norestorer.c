/* On x86-64 a handler needs SA_RESTORER: a signal caught by a handler
 * installed without it kills the process with SIGSEGV (the frame cannot be
 * set up), while SIG_IGN and SIG_DFL need no restorer. A handler installed
 * with SA_RESTORER and a NULL restorer faults when it returns.
 * poly-diff: syscalls=off (child processes)
 */
#include "poly_ipc.h"

static volatile int hits;
static void on_usr1(int sig) {
    (void)sig;
    hits++;
}

int test_main(void) {
    i64 pid = fork();
    if (pid == 0) {
        struct k_sigaction sa = { (u64)on_usr1, 0, 0, 0 };
        if (syscall4(SYS_rt_sigaction, SIGUSR1, &sa, 0, 8) != 0) exit_group(10);
        raise_sig(SIGUSR1);
        exit_group(hits ? 11 : 12);
    }
    int st = wait_status(pid);
    check("no_restorer_install_succeeds_then_sigsegv", W_SIGNALED(st) && W_TERMSIG(st) == SIGSEGV, st);

    pid = fork();
    if (pid == 0) {
        struct k_sigaction sa = { (u64)on_usr1, SA_RESTORER, 0, 0 };
        syscall4(SYS_rt_sigaction, SIGUSR1, &sa, 0, 8);
        raise_sig(SIGUSR1);
        exit_group(13);
    }
    st = wait_status(pid);
    check("null_restorer_return_sigsegv", W_SIGNALED(st) && W_TERMSIG(st) == SIGSEGV, st);

    pid = fork();
    if (pid == 0) {
        struct k_sigaction sa = { SIG_IGN, 0, 0, 0 };
        syscall4(SYS_rt_sigaction, SIGUSR1, &sa, 0, 8);
        raise_sig(SIGUSR1);
        exit_group(14);
    }
    check("ignore_needs_no_restorer", wait_exit(pid) == 14, 0);

    pid = fork();
    if (pid == 0) {
        struct k_sigaction sa = { (u64)on_usr1, SA_RESTORER, (u64)poly_sigreturn, 0 };
        syscall4(SYS_rt_sigaction, SIGUSR1, &sa, 0, 8);
        raise_sig(SIGUSR1);
        exit_group(hits == 1 ? 15 : 16);
    }
    check("with_restorer_handler_returns", wait_exit(pid) == 15, 0);
    return finish();
}
