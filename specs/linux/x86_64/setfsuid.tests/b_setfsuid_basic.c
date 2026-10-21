/* setfsuid and setfsgid: return the previous filesystem id, always (they
 * never fail with an error code); as root any value is taken; -1 only
 * reads it; the filesystem id follows each change of the effective id;
 * without privilege a value that is not one of the real, effective,
 * saved or current fs ids is ignored (the old value is still returned). */
#include "poly_ipc.h"

int test_main(void) {
    i64 r = syscall1(SYS_setfsuid, -1);
    check("query_returns_current", r == 0, r);
    r = syscall1(SYS_setfsuid, 500);
    check("set_returns_previous", r == 0, r);
    r = syscall1(SYS_setfsuid, -1);
    check("now_500", r == 500, r);
    unsigned a, b, c;
    syscall3(SYS_getresuid, &a, &b, &c);
    check("other_ids_untouched", a == 0 && b == 0 && c == 0, (i64)b);
    syscall3(SYS_setresuid, -1, 0, -1);
    r = syscall1(SYS_setfsuid, -1);
    check("euid_change_resets_fsuid", r == 0, r);

    r = syscall1(SYS_setfsgid, 77);
    check("setfsgid_returns_previous", r == 0, r);
    check("setfsgid_now_77", syscall1(SYS_setfsgid, -1) == 77, 0);
    syscall3(SYS_setresgid, -1, 0, -1);
    check("egid_change_resets_fsgid", syscall1(SYS_setfsgid, -1) == 0, 0);

    /* Effective uid 200 (no capabilities), saved 0 so root can come back. */
    syscall3(SYS_setresuid, 100, 200, 0);
    r = syscall1(SYS_setfsuid, 999);
    check("unprivileged_other_ignored_returns_old", r == 200, r);
    check("unprivileged_other_not_set", syscall1(SYS_setfsuid, -1) == 200, 0);
    r = syscall1(SYS_setfsuid, 100);
    check("unprivileged_to_real", r == 200 && syscall1(SYS_setfsuid, -1) == 100, r);
    r = syscall1(SYS_setfsuid, 0);
    check("unprivileged_to_saved", r == 100 && syscall1(SYS_setfsuid, -1) == 0, r);
    syscall3(SYS_setresuid, 0, 0, 0);
    check("back_to_root", syscall0(SYS_geteuid) == 0 && syscall1(SYS_setfsuid, -1) == 0, 0);
    return finish();
}
