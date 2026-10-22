/* With CAP_SETGID any gid is taken; every call returns the previous
 * filesystem gid; the real, effective and saved gids are not touched.
 * poly-diff: needs=root (only root holds CAP_SETGID for arbitrary gids)
 */
#include "poly_ipc.h"

int test_main(void) {
    i64 v = syscall1(SYS_setfsgid, 500);
    check("first_returns_egid", v == syscall0(SYS_getegid), v);
    v = syscall1(SYS_setfsgid, 77);
    check("second_returns_500", v == 500, v);
    v = syscall1(SYS_setfsgid, -1);
    check("query_77", v == 77, v);
    v = syscall1(SYS_setfsgid, 0);
    check("back_to_0_returns_77", v == 77, v);
    check("now_0", syscall1(SYS_setfsgid, -1) == 0, 0);
    syscall1(SYS_setfsgid, 123456);
    unsigned r, e, s;
    syscall3(SYS_getresgid, &r, &e, &s);
    check("resgid_untouched", r == 0 && e == 0 && s == 0, (i64)e);
    check("getegid_untouched", syscall0(SYS_getegid) == 0, syscall0(SYS_getegid));
    check("query_123456", syscall1(SYS_setfsgid, -1) == 123456, 0);
    return finish();
}
