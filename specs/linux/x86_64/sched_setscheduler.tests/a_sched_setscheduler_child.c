/* sched_setscheduler on another process of the same user: it sets that
 * process's policy and not the caller's. A child that has exited but is
 * not yet reaped (a zombie) is still found; once reaped its pid is ESRCH
 * (3), as are ids no thread has. */
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_sched_setparam 142
#define SYS_sched_getparam 143
#define SYS_sched_setscheduler 144
#define SYS_sched_getscheduler 145

/* linux/sched.h */
#define SCHED_OTHER 0
#define SCHED_FIFO 1
#define SCHED_RR 2
#define SCHED_BATCH 3
#define SCHED_IDLE 5
#define SCHED_DEADLINE 6
#define SCHED_RESET_ON_FORK 0x40000000

static A_UNUSED i64 setsch(i64 pid, i64 policy, i32 prio) {
    i32 p = prio;
    return syscall3(SYS_sched_setscheduler, pid, policy, &p);
}

static A_UNUSED i64 getsch(i64 pid) { return syscall1(SYS_sched_getscheduler, pid); }

static A_UNUSED i64 getprio(i64 pid) {
    i32 p = -1;
    i64 r = syscall2(SYS_sched_getparam, pid, &p);
    return r < 0 ? r : p;
}

static volatile i32 done, go;
static volatile i64 seen;

static int child(void *arg) {
    (void)arg;
    a_spin_until(&go, 1, A_SPIN_LIMIT);
    seen = getsch(0);
    go = 2;
    a_spin_until(&go, 3, A_SPIN_LIMIT);
    return 0;
}

int test_main(void) {
    a_block_sigchld();
    go = 0;
    long pid = a_spawn(0, &done, child, 0);
    expect("child_batch", setsch(pid, SCHED_BATCH, 0), 0);
    expect("child_is_batch", getsch(pid), SCHED_BATCH);
    expect("self_other", getsch(0), SCHED_OTHER);
    expect("child_bad_prio", setsch(pid, SCHED_OTHER, 1), -EINVAL);
    expect("child_bad_policy", setsch(pid, 8, 0), -EINVAL);
    go = 1;
    check("child_ran", a_spin_until(&go, 2, A_SPIN_LIMIT), go);
    expect("child_saw_batch", seen, SCHED_BATCH);
    go = 3;
    a_spin_until(&done, 0, A_SPIN_LIMIT);
    char info[128];
    expect("wait_nowait", syscall5(SYS_waitid, P_PID, pid, info, WEXITED | WNOWAIT, 0), 0);
    expect("zombie_other", setsch(pid, SCHED_OTHER, 0), 0);
    expect("zombie_is_other", getsch(pid), SCHED_OTHER);
    expect("zombie_bad_prio", setsch(pid, SCHED_BATCH, 1), -EINVAL);
    i32 st;
    expect("reap", syscall4(SYS_wait4, pid, &st, 0, 0), pid);
    expect("reaped", setsch(pid, SCHED_OTHER, 0), -ESRCH);
    expect("reaped_bad_policy", setsch(pid, 8, 0), -ESRCH);
    expect("nopid_max", setsch(0x7fffffff, SCHED_OTHER, 0), -ESRCH);
    expect("nopid_4194304", setsch(4194304, SCHED_OTHER, 0), -ESRCH);
    return finish();
}
