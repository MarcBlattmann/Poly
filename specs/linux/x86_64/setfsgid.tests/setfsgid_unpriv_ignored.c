/* Without CAP_SETGID, a gid that is none of the real, effective, saved or
 * current filesystem gids is ignored: the call still returns the old
 * filesystem gid and does not fail. Any user: a root caller first moves
 * its effective uid off 0 (saved uid stays 0) to lose CAP_SETGID. */
#include "poly_ipc.h"

int test_main(void) {
    if (syscall0(SYS_geteuid) == 0) syscall3(SYS_setresuid, -1, 4321, 0);
    unsigned r, e, s;
    syscall3(SYS_getresgid, &r, &e, &s);
    unsigned other = 4000;
    while (other == r || other == e || other == s) other++;
    i64 v = syscall1(SYS_setfsgid, other);
    check("other_returns_old", v == (i64)e, v);
    check("other_not_set", syscall1(SYS_setfsgid, -1) == (i64)e, 0);
    unsigned zero_or_other = (r == 0 || e == 0 || s == 0) ? other + 1 : 0;
    while (zero_or_other != 0 && (zero_or_other == r || zero_or_other == e || zero_or_other == s)) zero_or_other++;
    v = syscall1(SYS_setfsgid, zero_or_other);
    check("second_other_returns_old", v == (i64)e, v);
    check("second_other_not_set", syscall1(SYS_setfsgid, -1) == (i64)e, 0);
    v = syscall1(SYS_setfsgid, 0xfffffffeL);
    check("big_returns_old", v == (i64)e, v);
    check("big_not_set", syscall1(SYS_setfsgid, -1) == (i64)e, 0);
    unsigned r2, e2, s2;
    syscall3(SYS_getresgid, &r2, &e2, &s2);
    check("resgid_untouched", r2 == r && e2 == e && s2 == s, (i64)e2);
    return finish();
}
