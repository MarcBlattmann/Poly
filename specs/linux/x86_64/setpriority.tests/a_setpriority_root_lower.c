/* With CAP_SYS_NICE (root) setpriority may lower the nice value: below
 * the start, to -20, and values below -20 (-21, -100, INT_MIN) are -20.
 * Without privilege, RLIMIT_NICE allows lowering down to nice
 * 20 - limit: with a limit of 25, nice -5 is allowed and -6 is EACCES;
 * any value from 19 down to that floor may be set again and again.
 * poly-diff: needs=root (lowering a nice value needs CAP_SYS_NICE, raising RLIMIT_NICE's hard limit too)
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
    i64 uid = syscall0(SYS_getuid);
    expect("is_root", uid, 0);
    if (uid != 0) return finish();
    i64 n0 = 20 - getprio(PRIO_PROCESS, 0);
    expect("lower_3", setprio(PRIO_PROCESS, 0, n0 - 3), 0);
    expect("get_lower_3", getprio(PRIO_PROCESS, 0), 20 - (n0 - 3 < -20 ? -20 : n0 - 3));
    expect("lower_to_min", setprio(PRIO_PROCESS, 0, -20), 0);
    expect("get_min", getprio(PRIO_PROCESS, 0), 40);
    expect("raise_10", setprio(PRIO_PROCESS, 0, 10), 0);
    expect("set_minus_21", setprio(PRIO_PROCESS, 0, -21), 0);
    expect("get_minus_21", getprio(PRIO_PROCESS, 0), 40);
    expect("raise_0", setprio(PRIO_PROCESS, 0, 0), 0);
    expect("set_minus_100", setprio(PRIO_PROCESS, 0, -100), 0);
    expect("get_minus_100", getprio(PRIO_PROCESS, 0), 40);
    expect("raise_5", setprio(PRIO_PROCESS, 0, 5), 0);
    expect("set_int_min", setprio(PRIO_PROCESS, 0, -0x80000000L), 0);
    expect("get_int_min", getprio(PRIO_PROCESS, 0), 40);
    expect("back_to_0", setprio(PRIO_PROCESS, 0, 0), 0);

    /* RLIMIT_NICE 25 without privilege: the floor is nice -5. */
    volatile i32 *sh = a_shared();
    i64 pid = a_fork_sync();
    if (pid == 0) {
        struct { u64 cur, max; } rl = {25, 25};
        sh[0] = (i32)syscall2(SYS_setrlimit, RLIMIT_NICE, &rl);
        syscall2(SYS_setgroups, 0, 0);
        syscall3(SYS_setresgid, 4000123, 4000123, 4000123);
        sh[1] = (i32)syscall3(SYS_setresuid, 4000123, 4000123, 4000123);
        sh[2] = (i32)setprio(PRIO_PROCESS, 0, -5);
        sh[3] = (i32)getprio(PRIO_PROCESS, 0);
        sh[4] = (i32)setprio(PRIO_PROCESS, 0, -6);
        sh[5] = (i32)setprio(PRIO_PROCESS, 0, -20);
        sh[6] = (i32)setprio(PRIO_PROCESS, 0, 12);
        sh[7] = (i32)setprio(PRIO_PROCESS, 0, -3);
        sh[8] = (i32)getprio(PRIO_PROCESS, 0);
        syscall1(SYS_exit_group, 0);
    }
    i32 st;
    syscall4(SYS_wait4, pid, &st, 0, 0);
    expect("rlimit_set", sh[0], 0);
    expect("dropped", sh[1], 0);
    expect("lower_to_floor", sh[2], 0);
    expect("get_floor", sh[3], 25);
    expect("below_floor", sh[4], -EACCES);
    expect("min_below_floor", sh[5], -EACCES);
    expect("raise_12", sh[6], 0);
    expect("lower_again", sh[7], 0);
    expect("get_minus_3", sh[8], 23);
    return finish();
}

int test_main(void) { return a_in_session(body); }
