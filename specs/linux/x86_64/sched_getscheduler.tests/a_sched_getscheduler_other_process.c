/* No permission is needed to read another process: pid 1 and the test's
 * parent (whoever owns them) are found and report a valid policy. A child
 * reads its parent's change made before the fork. */
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_sched_setscheduler 144
#define SYS_sched_getscheduler 145

/* linux/sched.h */
#define SCHED_BATCH 3

static int valid(i64 v) {
    i64 p = v & ~0x40000000L;
    return v >= 0 && (p == 0 || p == 1 || p == 2 || p == 3 || p == 5 || p == 6 || p == 7);
}

int test_main(void) {
    a_block_sigchld();
    i64 r1 = syscall1(SYS_sched_getscheduler, 1);
    check("pid1_valid", valid(r1), r1);
    i64 pp = syscall0(SYS_getppid);
    if (pp > 0) {
        i64 rp = syscall1(SYS_sched_getscheduler, pp);
        check("parent_valid", valid(rp), rp);
    } else {
        check("parent_valid", 1, 0);
    }
    i32 zero = 0;
    expect("to_batch", syscall3(SYS_sched_setscheduler, 0, SCHED_BATCH, &zero), 0);
    i64 me = syscall0(SYS_getpid);
    i64 c = a_fork_sync();
    if (c == 0) {
        i64 r = syscall1(SYS_sched_getscheduler, me);
        i64 p1 = syscall1(SYS_sched_getscheduler, 1);
        syscall1(SYS_exit_group, (r == SCHED_BATCH && valid(p1)) ? 0 : 1);
    }
    i32 st = -1;
    expect("reap", syscall4(SYS_wait4, c, &st, 0, 0), c);
    check("child_read_parent", A_WIFEXITED(st) && A_WEXITSTATUS(st) == 0, st);
    return finish();
}
