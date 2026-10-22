/* setpriority on a thread of another user is EPERM unless the caller's
 * effective uid equals the target's real or effective uid (or the
 * caller has CAP_SYS_NICE): a caller whose real uid matches is not
 * enough. EPERM comes before EACCES (lowering another user's value is
 * EPERM too). PRIO_PGRP over a group with such a member is EPERM, but
 * the members the caller may change are set. PRIO_USER sets every
 * thread whose real uid is who. Root changes anyone's value.
 * poly-diff: needs=root (needs processes of several users)
 * poly-diff: syscalls=off (several processes; pids are arguments)
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

static volatile i32 *sh;

static void become(i64 r, i64 e) {
    syscall2(SYS_setgroups, 0, 0);
    syscall3(SYS_setresgid, r, e, e);
    sh[20] = (i32)syscall3(SYS_setresuid, r, e, e);
}

static int body(void) {
    i64 uid = syscall0(SYS_getuid);
    expect("is_root", uid, 0);
    if (uid != 0) return finish();
    i64 n0 = 20 - getprio(PRIO_PROCESS, 0);
    i64 me = syscall0(SYS_getpid);
    sh = a_shared();
    sh[0] = 0;
    sh[1] = 0;

    /* y: real uid 4000002, effective uid 4000003, in a group of its own. */
    i64 y = syscall2(SYS_fork, 0, 0);
    if (y == 0) {
        syscall2(SYS_setpgid, 0, 0);
        become(4000002, 4000003);
        sh[1] = 1;
        a_spin_until(&sh[0], 1, A_SPIN_LIMIT);
        syscall1(SYS_exit_group, 0);
    }
    check("y_ready", a_spin_until(&sh[1], 1, A_SPIN_LIMIT) && y > 0, y);
    expect("y_dropped", sh[20], 0);

    /* x: uid 4000001, in our group, against root and y. */
    i64 x = a_fork_sync();
    if (x == 0) {
        become(4000001, 4000001);
        if (sh[20] == 0) {
            sh[2] = (i32)setprio(PRIO_PROCESS, me, n0 + 2);
            sh[3] = (i32)setprio(PRIO_PROCESS, me, -20);
            sh[4] = (i32)setprio(PRIO_PROCESS, y, n0 + 2);
            sh[5] = (i32)setprio(PRIO_PGRP, 0, n0 + 2);
            sh[6] = (i32)getprio(PRIO_PROCESS, 0);
            sh[7] = (i32)setprio(PRIO_USER, 0, n0 + 4);
            sh[8] = (i32)getprio(PRIO_PROCESS, 0);
            sh[9] = (i32)setprio(PRIO_USER, 4000002, n0 + 4);
        }
        syscall1(SYS_exit_group, 0);
    }
    expect("x_dropped", sh[20], 0);
    expect("x_raise_root", sh[2], -EPERM);
    expect("x_lower_root", sh[3], -EPERM);
    expect("x_raise_y", sh[4], -EPERM);
    expect("x_pgrp_with_root", sh[5], -EPERM);
    expect("x_own_value_set", sh[6], g_of(n0 + 2));
    expect("root_unchanged", getprio(PRIO_PROCESS, 0), g_of(n0));
    expect("x_user_self", sh[7], 0);
    expect("x_user_value", sh[8], g_of(n0 + 4));
    expect("x_user_other", sh[9], -EPERM);

    /* Callers whose effective uid is y's real uid, or y's effective uid,
     * may raise it; one whose real uid is y's effective uid may not. */
    i64 c = a_fork_sync();
    if (c == 0) {
        become(4000009, 4000002);
        if (sh[20] == 0) sh[10] = (i32)setprio(PRIO_PROCESS, y, n0 + 3);
        syscall1(SYS_exit_group, 0);
    }
    expect("euid_is_real_uid", sh[10], 0);
    expect("y_after_1", getprio(PRIO_PROCESS, y), g_of(n0 + 3));
    c = a_fork_sync();
    if (c == 0) {
        become(4000009, 4000003);
        if (sh[20] == 0) {
            sh[11] = (i32)setprio(PRIO_PROCESS, y, n0 + 5);
            sh[12] = (i32)setprio(PRIO_PGRP, y, n0 + 6);
        }
        syscall1(SYS_exit_group, 0);
    }
    expect("euid_is_euid", sh[11], 0);
    expect("euid_is_euid_pgrp", sh[12], 0);
    expect("y_after_2", getprio(PRIO_PROCESS, y), g_of(n0 + 6));
    c = a_fork_sync();
    if (c == 0) {
        become(4000003, 4000005);
        if (sh[20] == 0) {
            sh[13] = (i32)setprio(PRIO_PROCESS, y, n0 + 7);
            sh[14] = (i32)setprio(PRIO_USER, 4000002, n0 + 7);
        }
        syscall1(SYS_exit_group, 0);
    }
    expect("ruid_is_euid_only", sh[13], -EPERM);
    expect("ruid_is_euid_only_user", sh[14], -EPERM);
    expect("y_after_3", getprio(PRIO_PROCESS, y), g_of(n0 + 6));

    /* Root may lower y below where it started. */
    expect("root_lowers_y", setprio(PRIO_PROCESS, y, n0 - 1), 0);
    expect("y_lowered", getprio(PRIO_PROCESS, y), g_of(n0 - 1) > 40 ? 40 : g_of(n0 - 1));
    sh[0] = 1;
    i32 st;
    syscall4(SYS_wait4, y, &st, 0, 0);
    return finish();
}

int test_main(void) { return a_in_session(body); }
