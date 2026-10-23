/* Another process of the same user is read by its pid. A forked child
 * starts with its parent's mask; a mask set on the child (by the parent,
 * by pid) shows when read by pid, and the child sees it with pid 0; the
 * parent's own mask is unchanged. A child that has exited but is not yet
 * reaped can still be read. */
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_sched_setaffinity 203
#define SYS_sched_getaffinity 204

static u64 mask_of(i64 pid) {
    u64 m[2] = {0, 0};
    i64 r = syscall3(SYS_sched_getaffinity, pid, sizeof m, m);
    return r == 8 ? m[0] : 0;
}

static volatile i32 done, go, stage;
static volatile u64 child_saw;

static int child(void *arg) {
    (void)arg;
    stage = 1;
    while (!go) a_relax();
    child_saw = mask_of(0);
    stage = 2;
    while (go != 2) a_relax();
    return 0;
}

int test_main(void) {
    a_block_sigchld();
    u64 start = mask_of(0);
    u64 lo = start & -start;

    long c = a_spawn(0, &done, child, 0);
    check("spawned", c > 0, c);
    a_spin_until(&stage, 1, A_SPIN_LIMIT);
    check("child_inherits", mask_of(c) == start, 0);
    u64 v = lo;
    expect("set_child", syscall3(SYS_sched_setaffinity, c, 8, &v), 0);
    check("child_by_pid", mask_of(c) == lo, 0);
    check("parent_unchanged", mask_of(0) == start, 0);
    go = 1;
    a_spin_until(&stage, 2, A_SPIN_LIMIT);
    check("child_sees_own", child_saw == lo, 0);
    go = 2;
    a_spin_until(&done, 0, A_SPIN_LIMIT);

    /* A zombie: exited, not reaped. */
    i64 z = a_fork_sync();
    if (z == 0) syscall1(SYS_exit, 0);
    check("forked", z > 0, z);
    check("zombie_readable", mask_of(z) == start, 0);
    i32 st = -1;
    expect("reap_zombie", syscall4(SYS_wait4, z, &st, 0, 0), z);
    expect("reap_spawned", syscall4(SYS_wait4, c, &st, 0, 0), c);
    return finish();
}
