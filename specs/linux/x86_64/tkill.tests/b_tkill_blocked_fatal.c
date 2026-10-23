/* A default-action fatal signal that the target thread blocks does not
 * kill: it stays pending (tkill returns 0) and the process ends with that
 * signal as soon as it unblocks it. SIGKILL cannot be blocked and kills at
 * once.
 * poly-diff: syscalls=off (two processes)
 */
#include "poly_ipc.h"

int test_main(void) {
    i64 c = fork();
    if (c == 0) {
        sig_mask(SIG_BLOCK, SIGBIT(SIGTERM), 0);
        i64 r = syscall2(SYS_tkill, gettid(), SIGTERM);
        if (r != 0) syscall1(SYS_exit_group, 10);
        if (sig_pending() != SIGBIT(SIGTERM)) syscall1(SYS_exit_group, 11);
        sig_mask(SIG_UNBLOCK, SIGBIT(SIGTERM), 0);
        syscall1(SYS_exit_group, 12); /* not reached */
    }
    check("fork", c > 0, c);
    int st = wait_status(c);
    check("not_exited_early", st == -1 || !W_EXITED(st), W_EXITCODE(st));
    check("killed_on_unblock", st != -1 && W_SIGNALED(st) && W_TERMSIG(st) == SIGTERM, st);

    c = fork();
    if (c == 0) {
        u64 all = ~0UL;
        sig_mask(SIG_BLOCK, all, 0);
        syscall2(SYS_tkill, gettid(), SIGKILL);
        syscall1(SYS_exit_group, 13); /* not reached */
    }
    check("fork2", c > 0, c);
    st = wait_status(c);
    check("sigkill_not_blockable", st != -1 && W_SIGNALED(st) && W_TERMSIG(st) == SIGKILL, st);
    return finish();
}
