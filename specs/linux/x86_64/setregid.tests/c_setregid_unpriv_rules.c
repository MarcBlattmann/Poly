/* Without CAP_SETGID (effective uid 1000 here): the real gid may become
 * the current real or effective gid but not the saved gid; the effective
 * gid may become the current real, effective or saved gid. Real and
 * effective may be swapped. The saved gid then follows the usual rule.
 * poly-diff: needs=root (needs distinct real, effective and saved gids)
 */
#include "poly_ipc.h"

static unsigned r, e, s;
static void get(void) { syscall3(SYS_getresgid, &r, &e, &s); }

/* real 1, effective 2, saved 3, then lose CAP_SETGID. */
static void reset(void) {
    syscall3(SYS_setresuid, -1, 0, -1);
    syscall3(SYS_setresgid, 1, 2, 3);
    syscall3(SYS_setresuid, -1, 1000, -1);
}

int test_main(void) {
    reset();
    i64 rc = syscall2(SYS_setregid, 3, -1);
    get();
    check("real_to_saved_eperm", rc == -EPERM, rc);
    check("real_to_saved_changes_nothing", r == 1 && e == 2 && s == 3, (i64)r);

    rc = syscall2(SYS_setregid, 2, -1);
    get();
    check("real_to_effective_ok", rc == 0 && r == 2 && e == 2 && s == 2, (i64)r);

    reset();
    rc = syscall2(SYS_setregid, -1, 3);
    get();
    check("effective_to_saved_ok", rc == 0 && r == 1 && e == 3 && s == 3, (i64)e);

    reset();
    rc = syscall2(SYS_setregid, -1, 1);
    get();
    check("effective_to_real_ok", rc == 0 && r == 1 && e == 1 && s == 3, (i64)e);

    reset();
    rc = syscall2(SYS_setregid, 2, 1);
    get();
    check("swap_ok", rc == 0 && r == 2 && e == 1 && s == 1, (i64)r);

    reset();
    rc = syscall2(SYS_setregid, 1, 2);
    get();
    check("same_pair_sets_saved", rc == 0 && r == 1 && e == 2 && s == 2, (i64)s);

    reset();
    rc = syscall2(SYS_setregid, 3, 3);
    check("both_to_saved_eperm", rc == -EPERM, rc);
    rc = syscall2(SYS_setregid, 99, -1);
    check("new_real_eperm", rc == -EPERM, rc);
    rc = syscall2(SYS_setregid, -1, 99);
    check("new_effective_eperm", rc == -EPERM, rc);
    get();
    check("eperm_changes_nothing", r == 1 && e == 2 && s == 3, (i64)s);
    return finish();
}
