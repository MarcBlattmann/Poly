/* setfsgid(fsgid) always returns the previous filesystem gid and never an
 * error. -1 (0xffffffff; the argument is 32 bits, so 0x1ffffffff and
 * 0xffffffffffffffff too) changes nothing and so reads the current value,
 * which for a program started normally is the effective gid. Any user may
 * set it to the own real, effective or saved gid: the call returns the
 * old value and the next query shows the new one. The filesystem gid is
 * not reported by getresgid or getegid.
 */
#include "a_sys.h"

#define SYS_getresgid 120
#define SYS_setfsgid 123

int test_main(void) {
    u32 r, e, s;
    syscall3(SYS_getresgid, &r, &e, &s);
    i64 v = syscall1(SYS_setfsgid, -1);
    check("query_is_egid", v == e, v);
    check("query_again_same", syscall1(SYS_setfsgid, -1) == e, 0);
    check("query_32bit_minus1", syscall1(SYS_setfsgid, 0x1ffffffffL) == e, 0);
    check("query_64bit_minus1", syscall1(SYS_setfsgid, 0xffffffffffffffffL) == e, 0);
    check("returns_nonnegative", v >= 0, v);

    v = syscall1(SYS_setfsgid, r);
    check("set_to_rgid_returns_old", v == e, v);
    check("now_rgid", syscall1(SYS_setfsgid, -1) == r, 0);
    v = syscall1(SYS_setfsgid, s);
    check("set_to_sgid_returns_old", v == r, v);
    check("now_sgid", syscall1(SYS_setfsgid, -1) == s, 0);
    v = syscall1(SYS_setfsgid, (1L << 32) | e);
    check("high_bits_ignored", v == s && syscall1(SYS_setfsgid, -1) == e, v);

    u32 r2, e2, s2;
    syscall3(SYS_getresgid, &r2, &e2, &s2);
    check("resgid_unchanged", r2 == r && e2 == e && s2 == s, 0);
    check("getegid_unchanged", syscall0(SYS_getegid) == e, 0);
    return finish();
}
