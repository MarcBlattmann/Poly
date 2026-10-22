/* getpriority(PRIO_PROCESS, pid) reads another process's nice value
 * (no permission is needed to read it): a parent sees the value its
 * child set, a child sees its parent's; a forked child starts with its
 * parent's nice value; the child's changes do not touch the parent.
 * poly-diff: syscalls=off (two processes; pids are arguments)
 */
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_getpriority 140
#define SYS_setpriority 141
/* linux/resource.h */
#define PRIO_PROCESS 0

static i64 getprio(i64 which, i64 who) { return syscall2(SYS_getpriority, which, who); }
static i64 setprio(i64 which, i64 who, i64 nice) { return syscall3(SYS_setpriority, which, who, nice); }

static int body(void) {
    i64 g0 = getprio(PRIO_PROCESS, 0);
    i64 n0 = 20 - g0;
    expect("parent_raise", setprio(PRIO_PROCESS, 0, n0 + 2), 0);
    volatile i32 *sh = a_shared();
    sh[0] = 0;
    sh[1] = 0;
    i64 pid = syscall2(SYS_fork, 0, 0);
    if (pid == 0) {
        sh[2] = (i32)getprio(PRIO_PROCESS, 0);
        sh[3] = (i32)getprio(PRIO_PROCESS, syscall0(SYS_getppid));
        sh[4] = (i32)setprio(PRIO_PROCESS, 0, n0 + 7);
        sh[0] = 1;
        a_spin_until(&sh[1], 1, A_SPIN_LIMIT);
        sh[5] = (i32)getprio(PRIO_PROCESS, syscall0(SYS_getppid));
        sh[0] = 2;
        a_spin_until(&sh[1], 2, A_SPIN_LIMIT);
        syscall1(SYS_exit_group, 0);
    }
    check("fork_ok", pid > 0, pid);
    check("child_ready", a_spin_until(&sh[0], 1, A_SPIN_LIMIT), sh[0]);
    expect("child_inherited", sh[2], g0 - 2);
    expect("child_sees_parent", sh[3], g0 - 2);
    expect("child_raise", sh[4], 0);
    expect("parent_sees_child", getprio(PRIO_PROCESS, pid), g0 - 7);
    expect("parent_unchanged", getprio(PRIO_PROCESS, 0), g0 - 2);
    expect("parent_raise_more", setprio(PRIO_PROCESS, 0, n0 + 4), 0);
    sh[1] = 1;
    check("child_read_again", a_spin_until(&sh[0], 2, A_SPIN_LIMIT), sh[0]);
    expect("child_sees_new_parent_value", sh[5], g0 - 4);
    expect("child_still", getprio(PRIO_PROCESS, pid), g0 - 7);
    sh[1] = 2;
    i32 st = 0;
    syscall4(SYS_wait4, pid, &st, 0, 0);
    return finish();
}

int test_main(void) { return a_in_session(body); }
