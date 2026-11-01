/* A process in pause can be stopped (SIGSTOP: the parent's wait4 with
 * WUNTRACED sees it stopped) and continued (SIGCONT: waitid/wait4 with
 * WCONTINUED sees it continued) without leaving pause: afterwards it is
 * still running (wait4 WNOHANG returns 0). A signal whose default action
 * terminates it (SIGTERM, no handler) ends it in pause: the parent sees it
 * killed by SIGTERM.
 * poly-diff: syscalls=off (two processes run at once)
 */
#include "a_sys.h"

#define SYS_pause 34

int test_main(void) {
    a_block_sigchld();
    volatile i32 *sh = a_shared();
    sh[0] = 0;
    i64 pid = syscall5(SYS_clone, SIGCHLD, 0, 0, 0, 0);
    if (pid == 0) {
        sh[0] = 1;
        i64 r = syscall0(SYS_pause);
        syscall1(SYS_exit_group, r == -EINTR ? 3 : 4);
    }
    a_spin_until(sh, 1, A_SPIN_LIMIT);
    a_spin_ticks(60000000);
    i32 st = 0;
    syscall2(SYS_kill, pid, SIGSTOP);
    expect("wait_stopped", syscall4(SYS_wait4, pid, &st, WUNTRACED, 0), pid);
    check("status_stopped_sigstop", (st & 0xff) == 0x7f && ((st >> 8) & 0xff) == SIGSTOP, st);
    syscall2(SYS_kill, pid, SIGCONT);
    st = 0;
    expect("wait_continued", syscall4(SYS_wait4, pid, &st, WCONTINUED, 0), pid);
    expect("status_continued", st, 0xffff);
    a_spin_ticks(60000000);
    expect("still_running", syscall4(SYS_wait4, pid, &st, WNOHANG, 0), 0);
    syscall2(SYS_kill, pid, SIGTERM);
    st = 0;
    expect("wait_killed", syscall4(SYS_wait4, pid, &st, 0, 0), pid);
    check("killed_by_sigterm", A_WIFSIGNALED(st) && A_WTERMSIG(st) == SIGTERM, st);
    return finish();
}
