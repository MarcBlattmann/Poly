/* Once all uids are non-zero (setresuid(1000, 1000, 1000)) the capability
 * is gone for good: any gid outside the real/effective/saved set is
 * refused with EPERM, also 0, while moves within the set still work.
 * poly-diff: needs=root (starts from root to reach distinct gids)
 */
#include "poly_ipc.h"

int test_main(void) {
    unsigned r, e, s;
    syscall3(SYS_setresgid, 1, 2, 3);
    syscall3(SYS_setresuid, 1000, 1000, 1000);
    i64 rc = syscall2(SYS_setregid, 0, 0);
    check("back_to_zero_eperm", rc == -EPERM, rc);
    rc = syscall2(SYS_setregid, -1, 0);
    check("effective_zero_eperm", rc == -EPERM, rc);
    rc = syscall2(SYS_setregid, -1, 3);
    syscall3(SYS_getresgid, &r, &e, &s);
    check("effective_to_saved_ok", rc == 0 && r == 1 && e == 3 && s == 3, (i64)e);
    rc = syscall2(SYS_setregid, 3, 1);
    syscall3(SYS_getresgid, &r, &e, &s);
    check("swap_ok", rc == 0 && r == 3 && e == 1 && s == 1, (i64)r);
    rc = syscall2(SYS_setregid, 2, -1);
    check("lost_gid_2_eperm", rc == -EPERM, rc);
    return finish();
}
