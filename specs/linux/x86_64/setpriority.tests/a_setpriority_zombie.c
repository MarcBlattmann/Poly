/* setpriority(PRIO_PROCESS) on a child that has exited but has not been
 * reaped (a zombie) finds it: raising succeeds and getpriority returns
 * the new value, lowering is EACCES; once reaped, the pid is ESRCH.
 * poly-diff: syscalls=off (the pid is an argument)
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
    a_block_sigchld();
    i64 n0 = 20 - getprio(PRIO_PROCESS, 0);
    i64 pid = syscall2(SYS_fork, 0, 0);
    if (pid == 0) {
        setprio(PRIO_PROCESS, 0, n0 + 2);
        syscall1(SYS_exit_group, 0);
    }
    check("fork_ok", pid > 0, pid);
    u8 info[128];
    expect("exited", syscall5(SYS_waitid, P_PID, pid, info, WEXITED | WNOWAIT, 0), 0);
    expect("zombie_value", getprio(PRIO_PROCESS, pid), g_of(n0 + 2));
    expect("raise_zombie", setprio(PRIO_PROCESS, pid, n0 + 5), 0);
    expect("zombie_raised", getprio(PRIO_PROCESS, pid), g_of(n0 + 5));
    i64 z = 20 - g_of(n0 + 5);
    expect("lower_zombie", setprio(PRIO_PROCESS, pid, z - 1), -EACCES);
    expect("zombie_kept", getprio(PRIO_PROCESS, pid), g_of(n0 + 5));
    i32 st = 0;
    expect("reaped", syscall4(SYS_wait4, pid, &st, 0, 0), pid);
    expect("reaped_esrch", setprio(PRIO_PROCESS, pid, n0 + 5), -ESRCH);
    return finish();
}

int test_main(void) {
    as_user();
    return a_in_session(body);
}
