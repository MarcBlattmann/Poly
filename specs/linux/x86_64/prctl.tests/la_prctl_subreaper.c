/* PR_SET_CHILD_SUBREAPER (36, v) marks the process as a child
 * subreaper (any non-zero v is stored as 1; 0 clears) and returns 0;
 * PR_GET_CHILD_SUBREAPER (37, &int) stores 0 or 1 (0 at the start); an
 * unwritable pointer is EFAULT. A subreaper's orphaned descendants are
 * reparented to it instead of to init: the test's child C forks G and
 * exits; G's parent becomes the test (getppid), which can wait for G.
 * A fork child does not inherit the subreaper mark.
 * poly-diff: syscalls=off (three processes run at once)
 */
#include "a_sys.h"

#define SYS_prctl 157
#define PR_SET_CHILD_SUBREAPER 36
#define PR_GET_CHILD_SUBREAPER 37

static i64 pr(u64 op, u64 a) { return syscall5(SYS_prctl, op, a, 0, 0, 0); }

int test_main(void) {
    a_block_sigchld();
    volatile i32 *sh = a_shared();
    i32 v = 77;
    expect("get_initial", pr(PR_GET_CHILD_SUBREAPER, (u64)&v), 0);
    expect("initial_0", v, 0);
    expect("set_5", pr(PR_SET_CHILD_SUBREAPER, 5), 0);
    pr(PR_GET_CHILD_SUBREAPER, (u64)&v);
    expect("stored_as_1", v, 1);
    expect("set_0", pr(PR_SET_CHILD_SUBREAPER, 0), 0);
    pr(PR_GET_CHILD_SUBREAPER, (u64)&v);
    expect("cleared", v, 0);
    expect("get_null_efault", pr(PR_GET_CHILD_SUBREAPER, 0), -EFAULT);
    expect("set_1", pr(PR_SET_CHILD_SUBREAPER, 1), 0);

    i64 c = a_fork_sync();
    if (c == 0) {
        i32 w = 77;
        pr(PR_GET_CHILD_SUBREAPER, (u64)&w);
        syscall1(SYS_exit, w);
    }
    i32 status = -1;
    syscall4(SYS_wait4, c, &status, 0, 0);
    expect("fork_child_not_subreaper", A_WEXITSTATUS(status), 0);

    i64 me = syscall0(SYS_getpid);
    c = syscall0(SYS_fork);
    if (c == 0) {
        i64 g = syscall0(SYS_fork);
        if (g == 0) {
            i64 mid = syscall0(SYS_getppid);
            sh[1] = (i32)syscall0(SYS_getpid);
            sh[0] = 1;
            while (syscall0(SYS_getppid) == mid) a_relax();
            sh[2] = syscall0(SYS_getppid) == me;
            syscall1(SYS_exit, 3);
        }
        while (sh[0] == 0) a_relax();
        syscall1(SYS_exit, 0);
    }
    syscall4(SYS_wait4, c, &status, 0, 0);
    i64 g = sh[1];
    i64 r = syscall4(SYS_wait4, g, &status, 0, 0);
    expect("orphan_waitable", r == g, 1);
    expect("orphan_exit_code", A_WEXITSTATUS(status), 3);
    expect("orphan_parent_is_subreaper", sh[2], 1);
    return finish();
}
