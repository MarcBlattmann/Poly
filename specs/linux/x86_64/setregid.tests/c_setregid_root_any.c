/* With CAP_SETGID any gids may be set: setregid(10, 20) gives real 10,
 * effective 20; the saved gid becomes the new effective gid (the real gid
 * was set) and the filesystem gid follows the effective gid. Neither
 * value needs to be one the process had. getgid and getegid agree.
 * poly-diff: needs=root (only root may set arbitrary gids)
 */
#include "poly_ipc.h"

int test_main(void) {
    unsigned r, e, s;
    i64 rc = syscall2(SYS_setregid, 10, 20);
    check("root_any_values_ok", rc == 0, rc);
    syscall3(SYS_getresgid, &r, &e, &s);
    check("real_is_10", r == 10, (i64)r);
    check("effective_is_20", e == 20, (i64)e);
    check("saved_is_new_effective", s == 20, (i64)s);
    check("fsgid_is_effective", syscall1(SYS_setfsgid, -1) == 20, syscall1(SYS_setfsgid, -1));
    check("getgid", syscall0(SYS_getgid) == 10, syscall0(SYS_getgid));
    check("getegid", syscall0(SYS_getegid) == 20, syscall0(SYS_getegid));
    /* The effective gid no longer being 0 does not matter: CAP_SETGID does. */
    rc = syscall2(SYS_setregid, 30, 40);
    check("still_privileged_ok", rc == 0, rc);
    syscall3(SYS_getresgid, &r, &e, &s);
    check("values_30_40_40", r == 30 && e == 40 && s == 40, (i64)r);
    check("uid_untouched", syscall0(SYS_getuid) == 0 && syscall0(SYS_geteuid) == 0, syscall0(SYS_geteuid));
    return finish();
}
