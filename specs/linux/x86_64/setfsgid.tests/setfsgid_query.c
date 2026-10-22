/* setfsgid(-1) only reads: it returns the current filesystem gid, which
 * starts out equal to the effective gid, and changes nothing. Any user. */
#include "poly_ipc.h"

int test_main(void) {
    unsigned r, e, s;
    syscall3(SYS_getresgid, &r, &e, &s);
    i64 v = syscall1(SYS_setfsgid, -1);
    check("query_is_egid", v == (i64)e, v);
    v = syscall1(SYS_setfsgid, -1);
    check("query_again_same", v == (i64)e, v);
    check("query_non_negative", v >= 0, v);
    unsigned r2, e2, s2;
    syscall3(SYS_getresgid, &r2, &e2, &s2);
    check("resgid_untouched", r2 == r && e2 == e && s2 == s, (i64)e2);
    check("getegid_untouched", syscall0(SYS_getegid) == (i64)e, syscall0(SYS_getegid));
    check("getgid_untouched", syscall0(SYS_getgid) == (i64)r, syscall0(SYS_getgid));
    return finish();
}
