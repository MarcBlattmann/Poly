/* PR_GET_DUMPABLE (3) returns the dumpable flag as the call's result
 * (1 for a normal program; arguments 2..5 are ignored).
 * PR_SET_DUMPABLE (4, v) accepts only 0 and 1 (2, 3, -1: EINVAL) and
 * returns 0. A fork child inherits the flag. Unknown options (0, 9999,
 * -1) are EINVAL.
 */
#include "a_sys.h"

#define SYS_prctl 157
#define PR_GET_DUMPABLE 3
#define PR_SET_DUMPABLE 4

static i64 pr(u64 op, u64 a) { return syscall5(SYS_prctl, op, a, 0, 0, 0); }

int test_main(void) {
    a_block_sigchld();
    expect("initial_1", pr(PR_GET_DUMPABLE, 0), 1);
    expect("get_args_ignored", syscall5(SYS_prctl, PR_GET_DUMPABLE, 5, 6, 7, 8), 1);
    expect("set_2_einval", pr(PR_SET_DUMPABLE, 2), -EINVAL);
    expect("set_3_einval", pr(PR_SET_DUMPABLE, 3), -EINVAL);
    expect("set_minus1_einval", pr(PR_SET_DUMPABLE, (u64)-1), -EINVAL);
    expect("still_1", pr(PR_GET_DUMPABLE, 0), 1);
    expect("set_0", pr(PR_SET_DUMPABLE, 0), 0);
    expect("get_0", pr(PR_GET_DUMPABLE, 0), 0);

    i64 c = a_fork_sync();
    if (c == 0) syscall1(SYS_exit, (int)pr(PR_GET_DUMPABLE, 0) + 10);
    i32 status = -1;
    syscall4(SYS_wait4, c, &status, 0, 0);
    expect("fork_child_inherits_0", A_WEXITSTATUS(status), 10);

    expect("set_1", pr(PR_SET_DUMPABLE, 1), 0);
    expect("get_1", pr(PR_GET_DUMPABLE, 0), 1);
    expect("unknown_0_einval", pr(0, 0), -EINVAL);
    expect("unknown_9999_einval", pr(9999, 0), -EINVAL);
    expect("unknown_minus1_einval", pr((u64)-1, 0), -EINVAL);
    return finish();
}
