/* setpriority(PRIO_PGRP) sets every thread of every member, not just
 * each process's main thread: a member with two threads has both set.
 * When one of its threads would be lowered (no privilege) that thread
 * keeps its value, the call is EACCES, and the other thread is set.
 * poly-diff: syscalls=off (processes and threads; ids are arguments)
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

static volatile i32 *sh;
static volatile i32 done = -1;

/* In the child: sh[2] = thread id, sh[1] = 1 when it runs; on sh[0] == 1
 * it raises its own value to sh[3]; sh[1] = 2; it ends on sh[0] == 2. */
static int thread(void *arg) {
    (void)arg;
    sh[2] = (i32)syscall0(SYS_gettid);
    sh[1] = 1;
    a_spin_until(&sh[0], 1, A_SPIN_LIMIT);
    sh[4] = (i32)setprio(PRIO_PROCESS, 0, sh[3]);
    sh[1] = 2;
    a_spin_until(&sh[0], 2, A_SPIN_LIMIT);
    return 0;
}

static int body(void) {
    i64 n0 = 20 - getprio(PRIO_PROCESS, 0);
    sh = a_shared();
    sh[0] = 0;
    sh[1] = 0;
    sh[3] = (i32)(n0 + 6);
    i64 pid = syscall2(SYS_fork, 0, 0);
    if (pid == 0) {
        syscall2(SYS_setpgid, 0, 0);
        a_clone(THREAD_FLAGS, a_stacks[0] + A_STACK_SIZE, 0, (i32 *)&done, 0, thread, 0);
        a_spin_until(&sh[0], 2, A_SPIN_LIMIT);
        a_spin_until(&done, 0, A_SPIN_LIMIT);
        syscall1(SYS_exit_group, 0);
    }
    check("fork_ok", pid > 0, pid);
    check("thread_running", a_spin_until(&sh[1], 1, A_SPIN_LIMIT), sh[1]);
    i64 tid = sh[2];
    expect("own_group", syscall1(SYS_getpgid, pid), pid);

    expect("set_group", setprio(PRIO_PGRP, pid, n0 + 4), 0);
    expect("main_thread_set", getprio(PRIO_PROCESS, pid), g_of(n0 + 4));
    expect("second_thread_set", getprio(PRIO_PROCESS, tid), g_of(n0 + 4));
    expect("caller_untouched", getprio(PRIO_PROCESS, 0), g_of(n0));

    sh[0] = 1;
    check("thread_raised", a_spin_until(&sh[1], 2, A_SPIN_LIMIT), sh[1]);
    expect("thread_raise_ok", sh[4], 0);
    expect("second_thread_own", getprio(PRIO_PROCESS, tid), g_of(n0 + 6));
    int lowers = g_of(n0 + 5) > g_of(n0 + 6);
    expect("partly_lowering", setprio(PRIO_PGRP, pid, n0 + 5), lowers ? -EACCES : 0);
    expect("main_thread_raised", getprio(PRIO_PROCESS, pid), g_of(n0 + 5));
    expect("second_thread_kept", getprio(PRIO_PROCESS, tid), g_of(n0 + 6));
    expect("group_best", getprio(PRIO_PGRP, pid), g_of(n0 + 5));
    sh[0] = 2;
    i32 st = 0;
    expect("reaped", syscall4(SYS_wait4, pid, &st, 0, 0), pid);
    return finish();
}

int test_main(void) {
    as_user();
    return a_in_session(body);
}
