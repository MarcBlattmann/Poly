/* waitid with WEXITED: fills a siginfo (SIGCHLD, CLD_EXITED, pid, uid,
 * exit code) and returns 0; WNOWAIT leaves the child waitable; P_ALL
 * takes any child. */
#include "a_sys.h"

static i64 waitid(i64 type, i64 id, struct a_siginfo *si, i64 options) {
    return syscall5(SYS_waitid, type, id, si, options, 0);
}

int test_main(void) {
    a_block_sigchld();
    struct a_siginfo si;
    i64 pid = a_fork_sync();
    if (pid == 0) syscall1(SYS_exit_group, 7);
    memset(&si, 0xff, sizeof si);
    expect("peek_wnowait", waitid(P_PID, pid, &si, WEXITED | WNOWAIT), 0);
    expect("si_signo", si.si_signo, SIGCHLD);
    expect("si_code_exited", si.si_code, CLD_EXITED);
    expect("si_pid", si.si_pid, pid);
    expect("si_uid", si.si_uid, 0);
    expect("si_status_exit_code", si.si_status, 7);
    expect("si_errno_0", si.si_errno, 0);
    expect("still_waitable", waitid(P_PID, pid, &si, WEXITED), 0);
    expect("reaped_pid", si.si_pid, pid);
    expect("gone_echild", waitid(P_PID, pid, &si, WEXITED), -ECHILD);
    pid = a_fork_sync();
    if (pid == 0) syscall1(SYS_exit_group, 0);
    memset(&si, 0, sizeof si);
    expect("p_all", waitid(P_ALL, 0, &si, WEXITED), 0);
    check("p_all_found_child", si.si_pid == pid && si.si_status == 0, si.si_pid);
    /* The rusage argument is filled when given. */
    pid = a_fork_sync();
    if (pid == 0) syscall1(SYS_exit_group, 1);
    struct a_rusage ru;
    memset(&ru, 0xff, sizeof ru);
    expect("with_rusage", syscall5(SYS_waitid, P_PID, pid, &si, WEXITED, &ru), 0);
    check("rusage_filled", ru.ru_utime.tv_usec >= 0 && ru.ru_utime.tv_usec < 1000000, ru.ru_utime.tv_usec);
    return finish();
}
