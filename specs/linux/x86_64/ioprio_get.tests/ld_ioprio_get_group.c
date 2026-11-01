/* ioprio_get(IOPRIO_WHO_PGRP, 0) reports the best priority of the
 * caller's process group. Each member counts with its own value, except
 * that class none (0) counts as best effort with level (nice + 20) / 5
 * (level 4 at nice 0, 6 at nice 10, 7 at nice 19); the best is real time
 * over best effort over idle, and the lower level within a class. So a
 * group of one process with class none reads (2 << 13) | 4 although
 * IOPRIO_WHO_PROCESS reads 0 for it. ioprio_set(IOPRIO_WHO_PGRP, 0, v)
 * sets every member. The test runs in a new session (its own group of
 * one process, then two: a CLONE_VM child that sets its own value when
 * told to).
 */
#include "a_sys.h"

#define SYS_setpriority 141
#define SYS_ioprio_set 251
#define SYS_ioprio_get 252
/* linux/ioprio.h */
#define IOPRIO_WHO_PROCESS 1
#define IOPRIO_WHO_PGRP 2
#define PRIO(c, l) (((c) << 13) | (l))
#define PRIO_PROCESS 0 /* linux/resource.h */

static i64 set_self(i64 prio) { return syscall3(SYS_ioprio_set, IOPRIO_WHO_PROCESS, 0, prio); }
static i64 get_self(void) { return syscall2(SYS_ioprio_get, IOPRIO_WHO_PROCESS, 0); }
static i64 get_group(void) { return syscall2(SYS_ioprio_get, IOPRIO_WHO_PGRP, 0); }

static volatile i32 done, cmd;
static volatile i64 arg, res;

static int member(void *unused) {
    (void)unused;
    for (;;) {
        while (!cmd) a_relax();
        if (cmd == 1) res = set_self(arg);
        if (cmd == 2) res = get_self();
        if (cmd == 3) { cmd = 0; return 0; }
        cmd = 0;
    }
}
static i64 tell(int c, i64 v) {
    arg = v;
    cmd = c;
    while (cmd) a_relax();
    return res;
}

static int body(void) {
    expect("none_self_reads_0", set_self(0) == 0 ? get_self() : -1, 0);
    expect("none_group_of_one_reads_be4", get_group(), PRIO(2, 4));
    expect("be2_set", set_self(PRIO(2, 2)), 0);
    expect("be2_group_of_one", get_group(), PRIO(2, 2));
    expect("idle5_set", set_self(PRIO(3, 5)), 0);
    expect("idle5_group_of_one", get_group(), PRIO(3, 5));

    i64 m = a_spawn(0, &done, member, 0);
    expect("member_set_be6", tell(1, PRIO(2, 6)), 0);
    expect("group_be6_over_idle5", get_group(), PRIO(2, 6));
    expect("self_be3", set_self(PRIO(2, 3)), 0);
    expect("group_lower_level_wins", get_group(), PRIO(2, 3));
    expect("member_set_none", tell(1, 0), 0);
    expect("group_be3_over_none_as_be4", get_group(), PRIO(2, 3));
    expect("self_be7", set_self(PRIO(2, 7)), 0);
    expect("group_none_as_be4_over_be7", get_group(), PRIO(2, 4));
    expect("self_none", set_self(0), 0);
    expect("group_both_none", get_group(), PRIO(2, 4));
    expect("member_set_idle0", tell(1, PRIO(3, 0)), 0);
    expect("group_none_over_idle", get_group(), PRIO(2, 4));

    expect("set_group_be1", syscall3(SYS_ioprio_set, IOPRIO_WHO_PGRP, 0, PRIO(2, 1)), 0);
    expect("self_after_group_set", get_self(), PRIO(2, 1));
    expect("member_after_group_set", tell(2, 0), PRIO(2, 1));
    expect("set_group_bad_prio", syscall3(SYS_ioprio_set, IOPRIO_WHO_PGRP, 0, PRIO(6, 0)), -EINVAL);
    expect("member_unchanged", tell(2, 0), PRIO(2, 1));
    tell(3, 0);
    a_spin_until(&done, 0, A_SPIN_LIMIT);
    syscall4(SYS_wait4, m, 0, 0, 0); /* a zombie would still count as a member */

    /* Alone again: class none follows the nice value. */
    expect("self_none_again", set_self(0), 0);
    expect("nice10", syscall3(SYS_setpriority, PRIO_PROCESS, 0, 10), 0);
    expect("none_at_nice10_group_be6", get_group(), PRIO(2, 6));
    expect("none_at_nice10_self_0", get_self(), 0);
    expect("nice19", syscall3(SYS_setpriority, PRIO_PROCESS, 0, 19), 0);
    expect("none_at_nice19_group_be7", get_group(), PRIO(2, 7));
    expect("be0_at_nice19", set_self(PRIO(2, 0)) == 0 ? get_group() : -1, PRIO(2, 0));
    return finish();
}

int test_main(void) { return a_in_session(body); }
