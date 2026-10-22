/* getpriority(PRIO_PROCESS, pid) of a child that has exited but not
 * been reaped (a zombie) still returns the nice value it had; once it
 * is reaped the pid names nothing (ESRCH).
 * poly-diff: syscalls=off (two processes; pids are arguments)
 */
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_getpriority 140
#define SYS_setpriority 141
/* linux/resource.h */
#define PRIO_PROCESS 0

static i64 getprio(i64 which, i64 who) { return syscall2(SYS_getpriority, which, who); }
static i64 setprio(i64 which, i64 who, i64 nice) { return syscall3(SYS_setpriority, which, who, nice); }

static int body(void) {
    i64 g0 = getprio(PRIO_PROCESS, 0);
    i64 n0 = 20 - g0;
    i64 pid = syscall2(SYS_fork, 0, 0);
    if (pid == 0) {
        setprio(PRIO_PROCESS, 0, n0 + 5);
        syscall1(SYS_exit_group, 0);
    }
    check("fork_ok", pid > 0, pid);
    /* Wait until it has exited, leaving it a zombie. */
    char info[128];
    expect("waitid_nowait", syscall5(SYS_waitid, P_PID, pid, info, WEXITED | WNOWAIT, 0), 0);
    expect("zombie_value", getprio(PRIO_PROCESS, pid), g0 - 5);
    expect("parent_value", getprio(PRIO_PROCESS, 0), g0);
    i32 st = -1;
    expect("reap", syscall4(SYS_wait4, pid, &st, 0, 0), pid);
    expect("reaped_esrch", getprio(PRIO_PROCESS, pid), -ESRCH);
    return finish();
}

int test_main(void) { return a_in_session(body); }
