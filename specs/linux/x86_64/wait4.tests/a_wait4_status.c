/* wait4: the status word for normal exits (code << 8) and for deaths by
 * signal (the signal number); the rusage argument is filled; a reaped
 * child is gone. */
#include "a_sys.h"

int test_main(void) {
    a_block_sigchld();
    i32 st = -1;
    i64 pid = a_fork_sync();
    if (pid == 0) syscall1(SYS_exit_group, 0);
    expect("reap_exit_0", syscall4(SYS_wait4, pid, &st, 0, 0), pid);
    expect("status_exit_0", st, 0);
    pid = a_fork_sync();
    if (pid == 0) syscall1(SYS_exit_group, 200);
    expect("reap_exit_200", syscall4(SYS_wait4, pid, &st, 0, 0), pid);
    expect("status_exit_200", st, 200 << 8);
    pid = a_fork_sync();
    if (pid == 0) {
        u64 none = 0;
        syscall4(SYS_rt_sigprocmask, SIG_SETMASK, &none, 0, 8);
        syscall2(SYS_kill, syscall0(SYS_getpid), SIGTERM);
        syscall1(SYS_exit_group, 1);
    }
    expect("reap_killed", syscall4(SYS_wait4, pid, &st, 0, 0), pid);
    expect("status_sigterm", st, SIGTERM);
    check("wifsignaled", A_WIFSIGNALED(st) && A_WTERMSIG(st) == SIGTERM, st);
    struct a_rusage ru;
    memset(&ru, 0xff, sizeof ru);
    pid = a_fork_sync();
    if (pid == 0) syscall1(SYS_exit_group, 9);
    expect("reap_with_rusage", syscall4(SYS_wait4, pid, &st, 0, &ru), pid);
    check("rusage_filled", ru.ru_utime.tv_sec >= 0 && ru.ru_utime.tv_usec >= 0 && ru.ru_utime.tv_usec < 1000000 &&
                               ru.ru_stime.tv_usec >= 0 && ru.ru_stime.tv_usec < 1000000, ru.ru_utime.tv_usec);
    pid = a_fork_sync();
    if (pid == 0) syscall1(SYS_exit_group, 0);
    expect("null_status_ok", syscall4(SYS_wait4, pid, 0, 0, 0), pid);
    expect("reaped_child_gone_echild", syscall4(SYS_wait4, pid, &st, 0, 0), -ECHILD);
    return finish();
}
