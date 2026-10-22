/* Without CAP_SETGID a gid that is none of the caller's real, effective
 * or saved gids is refused with EPERM, for the real gid, the effective gid
 * or both, and nothing changes. A process started as root first drops to
 * uid 65534 (with setresuid) so it has no capabilities; any other user
 * runs the checks as it is. */
#include "poly_ipc.h"

int test_main(void) {
    if (syscall0(SYS_geteuid) == 0) syscall3(SYS_setresuid, 65534, 65534, 65534);
    unsigned r0, e0, s0, r, e, s;
    syscall3(SYS_getresgid, &r0, &e0, &s0);
    unsigned other = r0 + 1;
    while (other == e0 || other == s0 || other == 0xFFFFFFFFu) other++;
    i64 rc = syscall2(SYS_setregid, other, -1);
    check("real_new_gid_eperm", rc == -EPERM, rc);
    rc = syscall2(SYS_setregid, -1, other);
    check("effective_new_gid_eperm", rc == -EPERM, rc);
    rc = syscall2(SYS_setregid, other, other);
    check("both_new_gid_eperm", rc == -EPERM, rc);
    rc = syscall2(SYS_setregid, r0, other);
    check("real_ok_effective_new_eperm", rc == -EPERM, rc);
    rc = syscall2(SYS_setregid, other, e0);
    check("real_new_effective_ok_eperm", rc == -EPERM, rc);
    syscall3(SYS_getresgid, &r, &e, &s);
    check("nothing_changed", r == r0 && e == e0 && s == s0, (i64)r);
    check("fsgid_unchanged", syscall1(SYS_setfsgid, -1) == e0, syscall1(SYS_setfsgid, -1));
    return finish();
}
