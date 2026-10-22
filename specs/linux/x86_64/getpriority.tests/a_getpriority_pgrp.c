/* getpriority(PRIO_PGRP, pgid) returns the highest value (lowest nice)
 * of all members of the process group; who 0 is the caller's group. A
 * member with a higher nice value does not lower it; when the caller
 * raises its own nice value above its children's, the best child
 * decides. A process that moved to a group of its own (setpgid) is
 * counted only there. Once all other members are gone, the caller's
 * own value is returned.
 * poly-diff: syscalls=off (several processes; pids are arguments)
 */
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_getpriority 140
#define SYS_setpriority 141
/* linux/resource.h */
#define PRIO_PROCESS 0
#define PRIO_PGRP 1

static i64 getprio(i64 which, i64 who) { return syscall2(SYS_getpriority, which, who); }
static i64 setprio(i64 which, i64 who, i64 nice) { return syscall3(SYS_setpriority, which, who, nice); }

static volatile i32 *sh;

/* A child with nice n0 + raise, in its own group if `own` is set; it
 * reports ready in sh[slot] and waits for sh[0] to open. */
static i64 child(int slot, i64 nice, int own) {
    i64 pid = syscall2(SYS_fork, 0, 0);
    if (pid == 0) {
        if (own) syscall2(SYS_setpgid, 0, 0);
        setprio(PRIO_PROCESS, 0, nice);
        sh[slot] = 1;
        a_spin_until(&sh[0], 1, A_SPIN_LIMIT);
        syscall1(SYS_exit_group, 0);
    }
    return pid;
}

static int body(void) {
    i64 g0 = getprio(PRIO_PROCESS, 0);
    i64 n0 = 20 - g0;
    i64 me = syscall0(SYS_getpid);
    expect("alone", getprio(PRIO_PGRP, 0), g0);
    expect("alone_by_id", getprio(PRIO_PGRP, me), g0);

    sh = a_shared();
    sh[0] = 0;
    i64 a = child(1, n0 + 3, 0);
    i64 b = child(2, n0 + 6, 0);
    i64 c = child(3, n0 + 1, 1);
    int ready = a_spin_until(&sh[1], 1, A_SPIN_LIMIT) && a_spin_until(&sh[2], 1, A_SPIN_LIMIT) &&
                a_spin_until(&sh[3], 1, A_SPIN_LIMIT);
    check("children_ready", ready && a > 0 && b > 0 && c > 0, ready);

    expect("children_worse_do_not_lower", getprio(PRIO_PGRP, 0), g0);
    expect("set_self_9", setprio(PRIO_PROCESS, 0, n0 + 9), 0);
    expect("best_child_decides", getprio(PRIO_PGRP, 0), g0 - 3);
    expect("best_child_decides_by_id", getprio(PRIO_PGRP, me), g0 - 3);
    expect("other_group", getprio(PRIO_PGRP, c), g0 - 1);
    expect("other_member_value", getprio(PRIO_PROCESS, c), g0 - 1);

    sh[0] = 1;
    i32 st;
    syscall4(SYS_wait4, a, &st, 0, 0);
    expect("after_a_gone", getprio(PRIO_PGRP, 0), g0 - 6);
    syscall4(SYS_wait4, b, &st, 0, 0);
    expect("after_b_gone", getprio(PRIO_PGRP, 0), g0 - 9);
    syscall4(SYS_wait4, c, &st, 0, 0);
    expect("other_group_gone", getprio(PRIO_PGRP, c), -ESRCH);
    return finish();
}

int test_main(void) { return a_in_session(body); }
