/* setreuid as root in one process: (-1, -1) is a no-op; (-1, x) with x
 * equal to the real uid leaves the saved uid alone; setting the effective
 * uid away from root and back is possible while the saved uid stays 0
 * (setreuid(-1, 0) uses the real uid 0); 0xffffffff means "keep". */
#include "poly_ipc.h"

int test_main(void) {
    unsigned r, e, s;
    i64 rc = syscall2(SYS_setreuid, -1, -1);
    syscall3(SYS_getresuid, &r, &e, &s);
    check("noop_ok", rc == 0 && r == 0 && e == 0 && s == 0, rc);
    rc = syscall2(SYS_setreuid, -1, 0);
    syscall3(SYS_getresuid, &r, &e, &s);
    check("euid_equal_ruid_saved_kept", rc == 0 && s == 0, (i64)s);
    syscall3(SYS_setresuid, 0, 0, 0);
    rc = syscall2(SYS_setreuid, 0xffffffffL, 0xffffffffL);
    check("all_ones_keep", rc == 0, rc);
    /* Effective to 1000 with saved 0 kept through setresuid, then back. */
    syscall3(SYS_setresuid, -1, 1000, -1);
    rc = syscall2(SYS_setreuid, -1, 0);
    syscall3(SYS_getresuid, &r, &e, &s);
    check("back_through_real_uid", rc == 0 && e == 0, (i64)e);
    return finish();
}
