/* sched_setparam on a child process of the same user: priority 0 works,
 * other values are EINVAL. Once the child has exited but is not yet
 * reaped (a zombie) the same holds; after it is reaped its pid is ESRCH. */
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_sched_setparam 142
#define SYS_sched_getparam 143
#define SYS_sched_setscheduler 144
#define SYS_sched_getscheduler 145

static volatile i32 done, go;

static int child(void *arg) {
    (void)arg;
    a_spin_until(&go, 1, A_SPIN_LIMIT);
    return 0;
}

int test_main(void) {
    a_block_sigchld();
    i32 zero = 0, one = 1;
    go = 0;
    long pid = a_spawn(0, &done, child, 0);
    expect("live_prio_0", syscall2(SYS_sched_setparam, pid, &zero), 0);
    expect("live_prio_1", syscall2(SYS_sched_setparam, pid, &one), -EINVAL);
    expect("live_null", syscall2(SYS_sched_setparam, pid, 0), -EINVAL);
    expect("live_bad_ptr", syscall2(SYS_sched_setparam, pid, 1), -EFAULT);
    go = 1;
    a_spin_until(&done, 0, A_SPIN_LIMIT);
    char info[128];
    expect("wait_nowait", syscall5(SYS_waitid, P_PID, pid, info, WEXITED | WNOWAIT, 0), 0);
    expect("zombie_prio_0", syscall2(SYS_sched_setparam, pid, &zero), 0);
    expect("zombie_prio_1", syscall2(SYS_sched_setparam, pid, &one), -EINVAL);
    i32 st;
    expect("reap", syscall4(SYS_wait4, pid, &st, 0, 0), pid);
    expect("reaped", syscall2(SYS_sched_setparam, pid, &zero), -ESRCH);
    expect("reaped_bad_prio", syscall2(SYS_sched_setparam, pid, &one), -ESRCH);
    return finish();
}
