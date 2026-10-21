/* getresuid and the single-value calls: getuid, geteuid report the real
 * and effective uid (getresuid all three, in order real, effective,
 * saved); getgid, getegid and getresgid the same for groups; each pointer
 * is written as an unsigned int; a bad pointer is EFAULT (and nothing is
 * changed); credentials are inherited across fork. */
#include "poly_ipc.h"

int test_main(void) {
    unsigned r = 7, e = 7, s = 7;
    i64 rc = syscall3(SYS_getresuid, &r, &e, &s);
    check("getresuid_ok", rc == 0, rc);
    check("root_all_zero", r == 0 && e == 0 && s == 0, (i64)r);
    check("getuid_matches_real", syscall0(SYS_getuid) == r, 0);
    check("geteuid_matches_effective", syscall0(SYS_geteuid) == e, 0);
    syscall3(SYS_setresuid, 11, 22, 0);
    syscall3(SYS_getresuid, &r, &e, &s);
    check("order_real_effective_saved", r == 11 && e == 22 && s == 0, (i64)(r * 10000 + e * 100 + s));
    check("getuid_real", syscall0(SYS_getuid) == 11, 0);
    check("geteuid_effective", syscall0(SYS_geteuid) == 22, 0);
    syscall3(SYS_setresuid, 0, 0, 0);

    unsigned g1 = 7, g2 = 7, g3 = 7;
    syscall3(SYS_setresgid, 33, 44, 55);
    rc = syscall3(SYS_getresgid, &g1, &g2, &g3);
    check("getresgid_order", rc == 0 && g1 == 33 && g2 == 44 && g3 == 55, (i64)g1);
    check("getgid_real", syscall0(SYS_getgid) == 33, 0);
    check("getegid_effective", syscall0(SYS_getegid) == 44, 0);
    syscall3(SYS_setresgid, 0, 0, 0);

    struct {
        unsigned v;
        unsigned guard;
    } w = { 7, 0x55555555 };
    syscall3(SYS_getresuid, &w.v, &e, &s);
    check("writes_four_bytes", w.v == 0 && w.guard == 0x55555555, (i64)w.guard);
    rc = syscall3(SYS_getresuid, &r, 8, &s);
    check("bad_pointer_efault", rc == -EFAULT, rc);
    rc = syscall3(SYS_getresgid, &r, &e, 8);
    check("getresgid_bad_pointer_efault", rc == -EFAULT, rc);
    return finish();
}
