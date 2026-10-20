/* alarm: a pending alarm is not inherited by a forked child (its alarm
 * returns 0), and survives execve. */
#include "a_sys.h"

int test_main(void) {
    u64 set = A_SIGBIT(SIGALRM);
    if (a_argc() > 2 && str_eq(a_argv()[1], "again")) {
        passed = (int)a_atou(a_argv()[2]);
        i64 left = syscall1(SYS_alarm, 0);
        check("survives_execve", left >= 20 && left <= 30, left);
        return finish();
    }
    a_block_sigchld();
    syscall4(SYS_rt_sigprocmask, SIG_BLOCK, &set, 0, 8);
    syscall1(SYS_alarm, 30);
    volatile i32 *sh = a_shared();
    i64 pid = a_fork_sync();
    if (pid == 0) {
        sh[0] = (i32)syscall1(SYS_alarm, 0);
        syscall1(SYS_exit_group, 0);
    }
    i32 st;
    syscall4(SYS_wait4, pid, &st, 0, 0);
    expect("child_has_no_alarm", sh[0], 0);
    i64 left = syscall1(SYS_alarm, 30);
    check("parent_alarm_kept", left >= 29 && left <= 30, left);
    if (failed) return finish();
    char count[24];
    a_utoa(count, (u64)passed);
    char *argv[] = {"x", "again", count, 0};
    char *envp[] = {0};
    syscall3(SYS_execve, a_self_path(), argv, envp);
    expect("execve_returned", 1, 0);
    return finish();
}
