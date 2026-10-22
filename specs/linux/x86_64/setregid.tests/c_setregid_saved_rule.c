/* The saved gid is set to the new effective gid when the real gid is
 * given (even to its current value), or when the effective gid is given
 * a value other than the old real gid; setting the effective gid to the
 * real gid, or giving -1 for both, leaves the saved gid alone.
 * poly-diff: needs=root (needs distinct real, effective and saved gids)
 */
#include "poly_ipc.h"

static unsigned r, e, s;
static void get(void) { syscall3(SYS_getresgid, &r, &e, &s); }

int test_main(void) {
    syscall3(SYS_setresgid, 1, 2, 3);
    i64 rc = syscall2(SYS_setregid, -1, 4);
    get();
    check("effective_not_real_sets_saved", rc == 0 && r == 1 && e == 4 && s == 4, (i64)s);

    syscall3(SYS_setresgid, 1, 2, 3);
    rc = syscall2(SYS_setregid, -1, 1);
    get();
    check("effective_to_real_keeps_saved", rc == 0 && r == 1 && e == 1 && s == 3, (i64)s);

    syscall3(SYS_setresgid, 1, 2, 3);
    rc = syscall2(SYS_setregid, -1, 2);
    get();
    check("effective_same_not_real_sets_saved", rc == 0 && r == 1 && e == 2 && s == 2, (i64)s);

    syscall3(SYS_setresgid, 1, 2, 3);
    rc = syscall2(SYS_setregid, 5, -1);
    get();
    check("real_only_sets_saved_to_effective", rc == 0 && r == 5 && e == 2 && s == 2, (i64)s);

    syscall3(SYS_setresgid, 1, 2, 3);
    rc = syscall2(SYS_setregid, 1, -1);
    get();
    check("real_same_value_still_sets_saved", rc == 0 && r == 1 && e == 2 && s == 2, (i64)s);

    syscall3(SYS_setresgid, 1, 2, 3);
    rc = syscall2(SYS_setregid, 6, 6);
    get();
    check("both_set_saved_is_effective", rc == 0 && r == 6 && e == 6 && s == 6, (i64)s);

    syscall3(SYS_setresgid, 1, 2, 3);
    rc = syscall2(SYS_setregid, 7, 1);
    get();
    check("real_given_effective_old_real_sets_saved", rc == 0 && r == 7 && e == 1 && s == 1, (i64)s);

    syscall3(SYS_setresgid, 1, 2, 3);
    rc = syscall2(SYS_setregid, -1, -1);
    get();
    check("both_minus_one_keeps_saved", rc == 0 && r == 1 && e == 2 && s == 3, (i64)s);
    return finish();
}
