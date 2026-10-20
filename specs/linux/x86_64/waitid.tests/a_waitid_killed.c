/* waitid: a child killed by a signal gives CLD_KILLED with the signal in
 * si_status; P_PGID selects children by process group. */
#include "a_sys.h"

static i64 waitid(i64 type, i64 id, struct a_siginfo *si, i64 options) {
    return syscall5(SYS_waitid, type, id, si, options, 0);
}

/* The checks run in a child that leads a new session and process group
 * (a_in_session), so they do not depend on how the test was started. */
static int body(void) {
    a_block_sigchld();
    struct a_siginfo si;
    i64 pid = a_fork_sync();
    if (pid == 0) {
        u64 none = 0;
        syscall4(SYS_rt_sigprocmask, SIG_SETMASK, &none, 0, 8);
        syscall2(SYS_kill, syscall0(SYS_getpid), SIGUSR1);
        syscall1(SYS_exit_group, 1);
    }
    memset(&si, 0, sizeof si);
    expect("killed_child", waitid(P_PID, pid, &si, WEXITED), 0);
    expect("cld_killed", si.si_code, CLD_KILLED);
    expect("status_is_signal", si.si_status, SIGUSR1);
    expect("killed_pid", si.si_pid, pid);
    pid = a_fork_sync();
    if (pid == 0) {
        syscall2(SYS_setpgid, 0, 0);
        syscall1(SYS_exit_group, 12);
    }
    expect("own_group_not_in_mine", waitid(P_PGID, syscall0(SYS_getpgrp), &si, WEXITED | WNOHANG), -ECHILD);
    expect("p_pgid_of_child_group", waitid(P_PGID, pid, &si, WEXITED), 0);
    check("p_pgid_result", si.si_pid == pid && si.si_code == CLD_EXITED && si.si_status == 12, si.si_status);
    return finish();
}

int test_main(void) { return a_in_session(body); }
