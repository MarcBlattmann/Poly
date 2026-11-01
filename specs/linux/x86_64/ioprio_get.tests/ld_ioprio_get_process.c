/* ioprio_get(IOPRIO_WHO_PROCESS, pid) reads another process's I/O
 * priority (any process may be read). A process of the same user may
 * also set it with ioprio_set(IOPRIO_WHO_PROCESS, pid, v), which the
 * target then reads as its own. The target here is a CLONE_VM child that
 * acts only when told to; after it has been reaped its pid is ESRCH.
 * poly-diff: syscalls=names (the child's pid is an argument)
 */
#include "a_sys.h"

#define SYS_ioprio_set 251
#define SYS_ioprio_get 252
#define IOPRIO_WHO_PROCESS 1 /* linux/ioprio.h */
#define PRIO(c, l) (((c) << 13) | (l))

static volatile i32 done, cmd;
static volatile i64 arg, res;

static int target(void *unused) {
    (void)unused;
    for (;;) {
        while (!cmd) a_relax();
        if (cmd == 1) res = syscall3(SYS_ioprio_set, IOPRIO_WHO_PROCESS, 0, arg);
        if (cmd == 2) res = syscall2(SYS_ioprio_get, IOPRIO_WHO_PROCESS, 0);
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

int test_main(void) {
    a_block_sigchld();
    expect("parent_be5", syscall3(SYS_ioprio_set, IOPRIO_WHO_PROCESS, 0, PRIO(2, 5)), 0);
    i64 pid = a_spawn(0, &done, target, 0);
    expect("child_inherited", syscall2(SYS_ioprio_get, IOPRIO_WHO_PROCESS, pid), PRIO(2, 5));
    expect("child_sets_idle2", tell(1, PRIO(3, 2)), 0);
    expect("read_child_idle2", syscall2(SYS_ioprio_get, IOPRIO_WHO_PROCESS, pid), PRIO(3, 2));
    expect("parent_still_be5", syscall2(SYS_ioprio_get, IOPRIO_WHO_PROCESS, 0), PRIO(2, 5));
    expect("set_child_be1", syscall3(SYS_ioprio_set, IOPRIO_WHO_PROCESS, pid, PRIO(2, 1)), 0);
    expect("child_reads_be1", tell(2, 0), PRIO(2, 1));
    expect("read_child_be1", syscall2(SYS_ioprio_get, IOPRIO_WHO_PROCESS, pid), PRIO(2, 1));
    expect("set_child_none", syscall3(SYS_ioprio_set, IOPRIO_WHO_PROCESS, pid, 0), 0);
    expect("read_child_none", syscall2(SYS_ioprio_get, IOPRIO_WHO_PROCESS, pid), 0);
    expect("set_child_bad", syscall3(SYS_ioprio_set, IOPRIO_WHO_PROCESS, pid, PRIO(0, 2)), -EINVAL);
    expect("parent_own_pid", syscall2(SYS_ioprio_get, IOPRIO_WHO_PROCESS, syscall0(SYS_getpid)), PRIO(2, 5));
    tell(3, 0);
    a_spin_until(&done, 0, A_SPIN_LIMIT);
    expect("reap", syscall4(SYS_wait4, pid, 0, 0, 0), pid);
    expect("reaped_get_esrch", syscall2(SYS_ioprio_get, IOPRIO_WHO_PROCESS, pid), -ESRCH);
    expect("reaped_set_esrch", syscall3(SYS_ioprio_set, IOPRIO_WHO_PROCESS, pid, PRIO(2, 1)), -ESRCH);
    return finish();
}
