/* setgid: with CAP_SETGID it sets real, effective and saved gid; without
 * it only the effective gid, to the real or saved gid (EPERM otherwise).
 * The gid is an unsigned int: 0xffffffff is EINVAL here (it does not
 * mean "keep" for setgid). setregid follows setreuid's rules. */
#include "poly_ipc.h"

int test_main(void) {
    i64 rc = syscall1(SYS_setgid, 1234);
    check("root_setgid_ok", rc == 0, rc);
    unsigned a, b, c;
    syscall3(SYS_getresgid, &a, &b, &c);
    check("root_sets_all_three", a == 1234 && b == 1234 && c == 1234, (i64)a);
    rc = syscall1(SYS_setgid, 0xffffffffL);
    check("minus_one_einval", rc == -EINVAL, rc);

    syscall3(SYS_setresgid, 100, 200, 300);
    syscall3(SYS_setresuid, -1, 1000, -1); /* drop CAP_SETGID from the effective set */
    rc = syscall1(SYS_setgid, 300);
    syscall3(SYS_getresgid, &a, &b, &c);
    check("unprivileged_to_saved_euid_only", rc == 0 && a == 100 && b == 300 && c == 300, (i64)b);
    rc = syscall1(SYS_setgid, 100);
    syscall3(SYS_getresgid, &a, &b, &c);
    check("unprivileged_to_real", rc == 0 && b == 100 && c == 300, (i64)b);
    rc = syscall1(SYS_setgid, 200);
    check("unprivileged_other_eperm", rc == -EPERM, rc);
    rc = syscall2(SYS_setregid, 300, -1);
    check("setregid_rgid_to_saved_eperm", rc == -EPERM, rc);
    rc = syscall2(SYS_setregid, -1, 300);
    syscall3(SYS_getresgid, &a, &b, &c);
    check("setregid_egid_to_saved", rc == 0 && a == 100 && b == 300 && c == 300, (i64)b);

    syscall3(SYS_setresuid, -1, 0, -1);
    rc = syscall2(SYS_setregid, 7, 8);
    syscall3(SYS_getresgid, &a, &b, &c);
    check("root_setregid_saved_follows", rc == 0 && a == 7 && b == 8 && c == 8, (i64)c);
    syscall3(SYS_setresgid, 0, 0, 0);
    return finish();
}
