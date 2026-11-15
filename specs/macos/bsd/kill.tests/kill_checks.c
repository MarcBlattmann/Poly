/* kill(2), BSD 37 (pid, signal): signal 0 only checks that the process
 * exists and may be signalled: 0 for the caller itself, ESRCH for a pid that
 * does not exist, EPERM for a process of another user (pid 1, launchd);
 * EINVAL for signal numbers 32 and -1. */
#include "poly_mtest.h"

int test_main(void) {
    i64 me = bsd0(SYS_getpid).ret;
    check_ok("signal 0 to itself", bsd2(SYS_kill, me, 0));
    check_err("a pid that does not exist: ESRCH", bsd2(SYS_kill, 99999999, 0), ESRCH);
    check_err("pid 1: EPERM", bsd2(SYS_kill, 1, 0), EPERM);
    check_err("signal 32: EINVAL", bsd2(SYS_kill, me, 32), EINVAL);
    check_err("signal -1: EINVAL", bsd2(SYS_kill, me, -1), EINVAL);
    check_err("EINVAL comes before ESRCH: signal 32 to a missing pid", bsd2(SYS_kill, 99999999, 32), EINVAL);
    return finish();
}
