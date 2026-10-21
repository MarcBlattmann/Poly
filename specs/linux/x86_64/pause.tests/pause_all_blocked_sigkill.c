/* pause: with every signal blocked, a terminating signal (SIGTERM) only
 * becomes pending and pause goes on waiting; SIGKILL, which cannot be
 * blocked, ends the process.
 * poly-diff: syscalls=off (two processes)
 */
#include "poly_ipc.h"

int test_main(void) {
    volatile i64 *sh = shared_page(4096);
    i64 pid = fork();
    if (pid == 0) {
        sig_mask(SIG_SETMASK, ~0UL, 0);
        __atomic_store_n(&sh[0], 1, __ATOMIC_SEQ_CST);
        syscall0(SYS_pause);
        sh[1] = 1;
        exit_group(3);
    }
    while (__atomic_load_n(&sh[0], __ATOMIC_SEQ_CST) == 0) sleep_ms(1);
    sleep_ms(30);
    syscall2(SYS_kill, pid, SIGTERM);
    syscall2(SYS_kill, pid, SIGUSR1);
    sleep_ms(100);
    int st = 0;
    i64 w = syscall4(SYS_wait4, pid, &st, WNOHANG, 0);
    check("blocked_sigterm_does_not_end_it", w == 0, w);
    syscall2(SYS_kill, pid, SIGKILL);
    int s = wait_status(pid);
    check("sigkill_kills", W_SIGNALED(s) && W_TERMSIG(s) == SIGKILL, s);
    check("pause_never_returned", sh[1] == 0, sh[1]);
    return finish();
}
