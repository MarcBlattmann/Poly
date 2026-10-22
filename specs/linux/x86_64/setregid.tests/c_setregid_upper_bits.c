/* The gid arguments are 32-bit: the upper half of each 64-bit register is
 * ignored, so 0x1_0000_0000 + gid means gid and 0x1_ffff_ffff means -1
 * (keep). Works for any caller because the values it names are the
 * caller's own gids. */
#include "poly_ipc.h"

int test_main(void) {
    unsigned r0, e0, s0, r, e, s;
    syscall3(SYS_getresgid, &r0, &e0, &s0);
    i64 rc = syscall2(SYS_setregid, 0x100000000L | r0, 0x700000000L | e0);
    check("upper_half_ignored_ok", rc == 0, rc);
    syscall3(SYS_getresgid, &r, &e, &s);
    check("upper_half_ignored_values", r == r0 && e == e0 && s == s0, (i64)r);
    rc = syscall2(SYS_setregid, 0x1FFFFFFFFL, 0xFFFFFFFFFFFFFFFFL);
    check("truncated_minus_one_keeps_ok", rc == 0, rc);
    rc = syscall2(SYS_setregid, 0xFFFFFFFFL, 0xFFFFFFFFL);
    check("u32_minus_one_keeps_ok", rc == 0, rc);
    syscall3(SYS_getresgid, &r, &e, &s);
    check("minus_one_values", r == r0 && e == e0 && s == s0, (i64)e);
    return finish();
}
