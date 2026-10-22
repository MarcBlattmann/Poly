/* A nice value set with setpriority is inherited: a forked child and a
 * new thread start with it, and it survives execve. A child's own
 * change does not touch its parent.
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
static volatile i64 t_value;

static int thread(void *arg) {
    (void)arg;
    t_value = getprio(PRIO_PROCESS, 0);
    return 0;
}

int test_main(void) {
    if (a_argc() > 3 && str_eq(a_argv()[1], "again")) {
        passed = (int)a_atou(a_argv()[2]);
        expect("survives_execve", getprio(PRIO_PROCESS, 0), (i64)a_atou(a_argv()[3]));
        return finish();
    }
    as_user();
    a_block_sigchld();
    i64 n0 = 20 - getprio(PRIO_PROCESS, 0);
    expect("raise_2", setprio(PRIO_PROCESS, 0, n0 + 2), 0);
    volatile i32 *sh = a_shared();
    i64 pid = a_fork_sync();
    if (pid == 0) {
        sh[0] = (i32)getprio(PRIO_PROCESS, 0);
        sh[1] = (i32)setprio(PRIO_PROCESS, 0, n0 + 6);
        sh[2] = (i32)getprio(PRIO_PROCESS, 0);
        syscall1(SYS_exit_group, 0);
    }
    i32 st;
    syscall4(SYS_wait4, pid, &st, 0, 0);
    expect("child_inherited", sh[0], g_of(n0 + 2));
    expect("child_raise", sh[1], 0);
    expect("child_raised", sh[2], g_of(n0 + 6));
    expect("parent_kept", getprio(PRIO_PROCESS, 0), g_of(n0 + 2));

    expect("raise_3", setprio(PRIO_PROCESS, 0, n0 + 3), 0);
    long r = a_clone(THREAD_FLAGS, a_stacks[0] + A_STACK_SIZE, 0, (i32 *)&done, 0, thread, 0);
    check("clone_ok", r > 0, r);
    check("thread_ended", a_spin_until(&done, 0, A_SPIN_LIMIT), done);
    expect("thread_inherited", t_value, g_of(n0 + 3));

    expect("raise_4", setprio(PRIO_PROCESS, 0, n0 + 4), 0);
    if (failed) return finish();
    char count[24], want[24];
    a_utoa(count, (u64)passed);
    a_utoa(want, (u64)g_of(n0 + 4));
    char *argv[] = {"x", "again", count, want, 0};
    char *envp[] = {0};
    syscall3(SYS_execve, a_self_path(), argv, envp);
    expect("execve_returned", 1, 0);
    return finish();
}
