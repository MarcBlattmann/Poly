/* The nice value setpriority sets is per thread: PRIO_PROCESS 0 in a
 * thread sets that thread only; PRIO_PROCESS with a thread's id sets
 * that thread from another one; PRIO_PROCESS with the pid sets the main
 * thread only, not the other threads of the process. A thread starts
 * with its creator's value. Lowering another thread's value is EACCES.
 * poly-diff: syscalls=off (threads; tids are arguments)
 */
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_setgroups 116
#define SYS_setresuid 117
#define SYS_setresgid 119
#define SYS_getpriority 140
#define SYS_setpriority 141
/* linux/resource.h */
#define PRIO_PROCESS 0
#define PRIO_PGRP 1
#define PRIO_USER 2

static A_UNUSED i64 getprio(i64 which, i64 who) { return syscall2(SYS_getpriority, which, who); }
static A_UNUSED i64 setprio(i64 which, i64 who, i64 nice) { return syscall3(SYS_setpriority, which, who, nice); }

/* What getpriority returns after nice n was set: nice values above 19
 * are 19, so a test that starts near 19 still agrees with itself. */
static A_UNUSED i64 g_of(i64 n) { return 20 - (n > 19 ? 19 : n); }

/* Run without privilege, whoever started the test: RLIMIT_NICE 0 (no
 * lowering allowed by the limit), and root becomes uid and gid 4000123
 * with no groups (so it has no CAP_SYS_NICE). */
static A_UNUSED void as_user(void) {
    struct { u64 cur, max; } rl = {0, 0};
    syscall2(SYS_setrlimit, RLIMIT_NICE, &rl);
    if (syscall0(SYS_getuid) == 0) {
        syscall2(SYS_setgroups, 0, 0);
        syscall3(SYS_setresgid, 4000123, 4000123, 4000123);
        syscall3(SYS_setresuid, 4000123, 4000123, 4000123);
    }
}

#define THREAD_FLAGS \
    (CLONE_VM | CLONE_FS | CLONE_FILES | CLONE_SIGHAND | CLONE_THREAD | CLONE_SYSVSEM | CLONE_CHILD_CLEARTID)

static volatile i32 done = -1;
static volatile i32 step, gate;
static volatile i64 tid, t_start, t_set, t_after;
static i64 n0;

static int thread(void *arg) {
    (void)arg;
    tid = syscall0(SYS_gettid);
    t_start = getprio(PRIO_PROCESS, 0);
    t_set = setprio(PRIO_PROCESS, 0, n0 + 2);
    step = 1;
    a_spin_until(&gate, 1, A_SPIN_LIMIT);
    t_after = getprio(PRIO_PROCESS, 0);
    step = 2;
    a_spin_until(&gate, 2, A_SPIN_LIMIT);
    return 0;
}

int test_main(void) {
    as_user();
    n0 = 20 - getprio(PRIO_PROCESS, 0);
    expect("main_raise_1", setprio(PRIO_PROCESS, 0, n0 + 1), 0);
    n0 = 20 - g_of(n0 + 1);
    long r = a_clone(THREAD_FLAGS, a_stacks[0] + A_STACK_SIZE, 0, (i32 *)&done, 0, thread, 0);
    check("clone_ok", r > 0, r);
    check("thread_step_1", a_spin_until(&step, 1, A_SPIN_LIMIT), step);
    expect("thread_inherited", t_start, g_of(n0));
    expect("thread_set_self", t_set, 0);
    expect("main_unchanged", getprio(PRIO_PROCESS, 0), g_of(n0));
    expect("thread_by_tid", getprio(PRIO_PROCESS, tid), g_of(n0 + 2));

    expect("set_thread_by_tid", setprio(PRIO_PROCESS, tid, n0 + 5), 0);
    expect("thread_after_set", getprio(PRIO_PROCESS, tid), g_of(n0 + 5));
    expect("main_still", getprio(PRIO_PROCESS, 0), g_of(n0));
    i64 pid = syscall0(SYS_getpid);
    expect("set_by_pid", setprio(PRIO_PROCESS, pid, n0 + 1), 0);
    expect("main_by_pid", getprio(PRIO_PROCESS, 0), g_of(n0 + 1));
    expect("thread_not_touched_by_pid", getprio(PRIO_PROCESS, tid), g_of(n0 + 5));
    i64 tcur = 20 - g_of(n0 + 5);
    expect("lower_thread", setprio(PRIO_PROCESS, tid, tcur - 1), -EACCES);
    gate = 1;
    check("thread_step_2", a_spin_until(&step, 2, A_SPIN_LIMIT), step);
    expect("thread_sees_own", t_after, g_of(n0 + 5));
    gate = 2;
    check("thread_ended", a_spin_until(&done, 0, A_SPIN_LIMIT), done);
    return finish();
}
