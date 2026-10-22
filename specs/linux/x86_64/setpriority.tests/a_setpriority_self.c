/* setpriority(PRIO_PROCESS, 0 or the caller's pid, nice) returns 0 and
 * sets the calling thread's nice value; getpriority reads it back as
 * 20 - nice; setting the value the thread already has succeeds. Without
 * privilege (RLIMIT_NICE 0, no CAP_SYS_NICE) any lower nice value is
 * EACCES and changes nothing. The test starts at whatever nice value it
 * inherited and only raises it.
 * poly-diff: syscalls=names (the pid is an argument)
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

int test_main(void) {
    as_user();
    i64 n0 = 20 - getprio(PRIO_PROCESS, 0);
    i64 pid = syscall0(SYS_getpid);
    expect("raise_1", setprio(PRIO_PROCESS, 0, n0 + 1), 0);
    expect("get_raise_1", getprio(PRIO_PROCESS, 0), g_of(n0 + 1));
    expect("raise_3_by_pid", setprio(PRIO_PROCESS, pid, n0 + 3), 0);
    expect("get_raise_3", getprio(PRIO_PROCESS, 0), g_of(n0 + 3));
    expect("get_raise_3_by_pid", getprio(PRIO_PROCESS, pid), g_of(n0 + 3));
    i64 cur = 20 - g_of(n0 + 3);
    expect("set_same", setprio(PRIO_PROCESS, 0, cur), 0);
    expect("set_same_by_pid", setprio(PRIO_PROCESS, pid, cur), 0);
    expect("lower_1", setprio(PRIO_PROCESS, 0, cur - 1), -EACCES);
    expect("lower_1_by_pid", setprio(PRIO_PROCESS, pid, cur - 1), -EACCES);
    expect("lower_to_min", setprio(PRIO_PROCESS, 0, -20), -EACCES);
    expect("lower_to_0", setprio(PRIO_PROCESS, 0, 0), cur > 0 ? -EACCES : 0);
    expect("unchanged", getprio(PRIO_PROCESS, 0), 20 - cur);
    expect("raise_to_19", setprio(PRIO_PROCESS, 0, 19), 0);
    expect("get_19", getprio(PRIO_PROCESS, 0), 1);
    expect("19_again", setprio(PRIO_PROCESS, 0, 19), 0);
    expect("lower_from_19", setprio(PRIO_PROCESS, 0, 18), -EACCES);
    expect("still_19", getprio(PRIO_PROCESS, 0), 1);
    return finish();
}
