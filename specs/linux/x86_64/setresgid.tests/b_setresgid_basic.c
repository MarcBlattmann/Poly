/* setresgid: like setresuid for groups: -1 keeps a field; with CAP_SETGID
 * any values (also for a process whose uids are not 0, as long as it has
 * the capability); the effective gid sets the filesystem gid; without
 * CAP_SETGID (effective uid not 0) each field may only take the current
 * real, effective or saved gid (EPERM otherwise). */
#include "poly_ipc.h"

int test_main(void) {
    i64 rc = syscall3(SYS_setresgid, 10, 20, 30);
    check("root_any_values", rc == 0, rc);
    unsigned a, b, c;
    syscall3(SYS_getresgid, &a, &b, &c);
    check("values_set", a == 10 && b == 20 && c == 30, (i64)a);
    check("fsgid_follows_egid", syscall1(SYS_setfsgid, -1) == 20, syscall1(SYS_setfsgid, -1));
    rc = syscall3(SYS_setresgid, -1, 40, -1);
    syscall3(SYS_getresgid, &a, &b, &c);
    check("keep_fields", rc == 0 && a == 10 && b == 40 && c == 30, (i64)b);

    /* Effective uid 1000: no CAP_SETGID any more. */
    syscall3(SYS_setresuid, -1, 1000, -1);
    rc = syscall3(SYS_setresgid, 30, 10, 40);
    check("unprivileged_permutation_ok", rc == 0, rc);
    syscall3(SYS_getresgid, &a, &b, &c);
    check("permutation_values", a == 30 && b == 10 && c == 40, (i64)a);
    rc = syscall3(SYS_setresgid, -1, 99, -1);
    check("unprivileged_new_gid_eperm", rc == -EPERM, rc);
    syscall3(SYS_getresgid, &a, &b, &c);
    check("eperm_changes_nothing", b == 10, (i64)b);
    syscall3(SYS_setresuid, -1, 0, -1);
    rc = syscall3(SYS_setresgid, 0, 0, 0);
    check("root_again_resets", rc == 0, rc);
    rc = syscall1(SYS_setgid, 0);
    check("setgid_as_root", rc == 0, rc);
    return finish();
}
