/* setpriority(PRIO_PROCESS, pid) sets another process's nice value when
 * both run as the same user: a parent raises its child's value and the
 * child raises its parent's; each change is seen by the other process
 * and touches only the target. Lowering the other's value is EACCES.
 * poly-diff: syscalls=off (two processes; pids are arguments)
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

static int body(void) {
    i64 n0 = 20 - getprio(PRIO_PROCESS, 0);
    volatile i32 *sh = a_shared();
    sh[0] = 0;
    sh[1] = 0;
    i64 pid = syscall2(SYS_fork, 0, 0);
    if (pid == 0) {
        sh[0] = 1;
        a_spin_until(&sh[1], 1, A_SPIN_LIMIT);
        i64 pp = syscall0(SYS_getppid);
        sh[2] = (i32)getprio(PRIO_PROCESS, 0);
        sh[3] = (i32)setprio(PRIO_PROCESS, pp, n0 + 4);
        sh[4] = (i32)setprio(PRIO_PROCESS, pp, n0 + 3);
        sh[0] = 2;
        a_spin_until(&sh[1], 2, A_SPIN_LIMIT);
        syscall1(SYS_exit_group, 0);
    }
    check("fork_ok", pid > 0, pid);
    check("child_ready", a_spin_until(&sh[0], 1, A_SPIN_LIMIT), sh[0]);
    expect("raise_child", setprio(PRIO_PROCESS, pid, n0 + 6), 0);
    expect("child_value", getprio(PRIO_PROCESS, pid), g_of(n0 + 6));
    expect("parent_untouched", getprio(PRIO_PROCESS, 0), g_of(n0));
    i64 c = 20 - g_of(n0 + 6);
    expect("lower_child", setprio(PRIO_PROCESS, pid, c - 1), -EACCES);
    expect("child_unchanged", getprio(PRIO_PROCESS, pid), g_of(n0 + 6));
    sh[1] = 1;
    check("child_done", a_spin_until(&sh[0], 2, A_SPIN_LIMIT), sh[0]);
    expect("child_sees_own", sh[2], g_of(n0 + 6));
    expect("child_raises_parent", sh[3], 0);
    i64 p = 20 - g_of(n0 + 4);
    expect("child_lowers_parent", sh[4], p > 20 - g_of(n0 + 3) ? -EACCES : 0);
    expect("parent_value", getprio(PRIO_PROCESS, 0), g_of(n0 + 4));
    expect("child_untouched", getprio(PRIO_PROCESS, pid), g_of(n0 + 6));
    sh[1] = 2;
    i32 st = 0;
    syscall4(SYS_wait4, pid, &st, 0, 0);
    return finish();
}

int test_main(void) {
    as_user();
    return a_in_session(body);
}
