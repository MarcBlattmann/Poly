/* A child that has exited but is not reaped yet still has its id: tkill
 * to it returns 0 (signal 0 and a real signal, which does nothing), while
 * an invalid signal number is still EINVAL. After wait4 it is ESRCH. The
 * exit status the parent collects is the child's own.
 * poly-diff: syscalls=off (two processes)
 */
#include "poly_ipc.h"

/* asm/unistd_64.h; linux/wait.h */
#define SYS_waitid 247
#define P_PID 1
#define WEXITED 0x00000004
#define WNOWAIT 0x01000000

int test_main(void) {
    i64 c = fork();
    if (c == 0) syscall1(SYS_exit, 7);
    check("fork", c > 0, c);
    /* Wait until the child is a zombie: waitid with WNOWAIT leaves it. */
    struct k_siginfo si;
    memset(&si, 0, sizeof si);
    i64 r = syscall5(SYS_waitid, P_PID, c, &si, WEXITED | WNOWAIT, 0);
    check("waitid_nowait", r == 0, r);
    check("child_exited", si.si_signo == SIGCHLD && si.si_code == CLD_EXITED, si.si_code);

    r = syscall2(SYS_tkill, c, 0);
    check("zombie_signal_zero", r == 0, r);
    r = syscall2(SYS_tkill, c, SIGKILL);
    check("zombie_sigkill", r == 0, r);
    r = syscall2(SYS_tkill, c, SIGUSR1);
    check("zombie_sigusr1", r == 0, r);
    r = syscall2(SYS_tkill, c, 65);
    check("zombie_bad_signal_einval", r == -EINVAL, r);

    int st = wait_status(c);
    check("status_unchanged", st != -1 && W_EXITED(st) && W_EXITCODE(st) == 7, st);
    r = syscall2(SYS_tkill, c, 0);
    check("reaped_esrch", r == -ESRCH, r);
    return finish();
}
