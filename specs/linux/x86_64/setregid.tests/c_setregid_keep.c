/* setregid(-1, -1) succeeds and changes no gid; setting the real and
 * effective gid to the values they already have succeeds for any caller
 * and leaves all three gids as they were (they are all equal here, as for
 * any process started normally). */
#include "poly_ipc.h"

int test_main(void) {
    unsigned r0, e0, s0, r, e, s;
    syscall3(SYS_getresgid, &r0, &e0, &s0);
    i64 rc = syscall2(SYS_setregid, -1, -1);
    check("both_minus_one_ok", rc == 0, rc);
    syscall3(SYS_getresgid, &r, &e, &s);
    check("both_minus_one_keeps", r == r0 && e == e0 && s == s0, (i64)r);
    rc = syscall2(SYS_setregid, r0, e0);
    check("same_values_ok", rc == 0, rc);
    syscall3(SYS_getresgid, &r, &e, &s);
    check("same_values_keep", r == r0 && e == e0 && s == s0, (i64)s);
    rc = syscall2(SYS_setregid, r0, -1);
    check("real_only_same_ok", rc == 0, rc);
    rc = syscall2(SYS_setregid, -1, e0);
    check("effective_only_same_ok", rc == 0, rc);
    check("getgid_unchanged", syscall0(SYS_getgid) == r0, syscall0(SYS_getgid));
    check("getegid_unchanged", syscall0(SYS_getegid) == e0, syscall0(SYS_getegid));
    return finish();
}
