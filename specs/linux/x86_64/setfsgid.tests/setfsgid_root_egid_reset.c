/* Every change of the effective gid (setresgid, setregid, setgid,
 * setfsgid aside) sets the filesystem gid to the new effective gid;
 * changes of the uids (setresuid, setfsuid) do not touch it.
 * poly-diff: needs=root (setting other gids needs CAP_SETGID)
 */
#include "poly_ipc.h"

int test_main(void) {
    syscall1(SYS_setfsgid, 900);
    syscall3(SYS_setresgid, -1, 5, -1);
    check("setresgid_egid_resets", syscall1(SYS_setfsgid, -1) == 5, syscall1(SYS_setfsgid, -1));
    syscall1(SYS_setfsgid, 901);
    syscall2(SYS_setregid, -1, 6);
    check("setregid_egid_resets", syscall1(SYS_setfsgid, -1) == 6, syscall1(SYS_setfsgid, -1));
    syscall1(SYS_setfsgid, 902);
    syscall1(SYS_setgid, 7);
    check("setgid_resets", syscall1(SYS_setfsgid, -1) == 7, syscall1(SYS_setfsgid, -1));
    syscall1(SYS_setfsgid, 903);
    syscall3(SYS_setresuid, -1, 1000, 0);
    check("uid_change_keeps", syscall1(SYS_setfsgid, -1) == 903, syscall1(SYS_setfsgid, -1));
    syscall1(SYS_setfsuid, 1001);
    check("setfsuid_keeps", syscall1(SYS_setfsgid, -1) == 903, syscall1(SYS_setfsgid, -1));
    syscall3(SYS_setresuid, -1, 0, -1);
    check("uid_back_keeps", syscall1(SYS_setfsgid, -1) == 903, syscall1(SYS_setfsgid, -1));
    unsigned r, e, s;
    syscall3(SYS_getresgid, &r, &e, &s);
    check("gids_as_set", r == 7 && e == 7 && s == 7, (i64)e);
    return finish();
}
