/* Every successful setregid sets the filesystem gid to the (new)
 * effective gid, even setregid(-1, -1) after setfsgid moved it away and
 * a call that changes only the real gid.
 * poly-diff: needs=root (setfsgid to another gid needs CAP_SETGID)
 */
#include "poly_ipc.h"

int test_main(void) {
    syscall3(SYS_setresgid, 1, 2, 3);
    syscall1(SYS_setfsgid, 9);
    check("setfsgid_moved", syscall1(SYS_setfsgid, -1) == 9, syscall1(SYS_setfsgid, -1));
    i64 rc = syscall2(SYS_setregid, -1, -1);
    check("noop_ok", rc == 0, rc);
    check("noop_resets_fsgid", syscall1(SYS_setfsgid, -1) == 2, syscall1(SYS_setfsgid, -1));
    syscall1(SYS_setfsgid, 9);
    rc = syscall2(SYS_setregid, 6, -1);
    check("real_only_ok", rc == 0, rc);
    check("real_only_resets_fsgid", syscall1(SYS_setfsgid, -1) == 2, syscall1(SYS_setfsgid, -1));
    rc = syscall2(SYS_setregid, -1, 8);
    check("effective_ok", rc == 0, rc);
    check("fsgid_follows_new_effective", syscall1(SYS_setfsgid, -1) == 8, syscall1(SYS_setfsgid, -1));
    syscall1(SYS_setfsgid, 9);
    syscall3(SYS_setresuid, -1, 1000, -1);
    rc = syscall2(SYS_setregid, -1, 99);
    check("refused_eperm", rc == -EPERM, rc);
    check("refused_keeps_fsgid", syscall1(SYS_setfsgid, -1) == 9, syscall1(SYS_setfsgid, -1));
    return finish();
}
