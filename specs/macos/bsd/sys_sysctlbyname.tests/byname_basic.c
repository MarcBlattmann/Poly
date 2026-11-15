/* sysctlbyname, BSD 274 (name, name length, old, oldlen, new, newlen):
 * "kern.ostype" is "Darwin" (7 bytes) whether the length counts the NUL or
 * not; a NULL buffer reports the length; hw.pagesize by name is an
 * 8-byte integer that also fits 4 bytes (sysctl(2) by number gives 4; a
 * value: 16384 on arm64). */
#include "poly_mtest.h"

int test_main(void) {
    char buf[64];
    size_t len = sizeof buf;
    sysret r = bsd6(SYS_sysctlbyname, "kern.ostype", 11, buf, &len, 0, 0);
    check("kern.ostype succeeds", r.err == 0 && r.ret == 0, r.err);
    check("it is \"Darwin\", 7 bytes", str_eq(buf, "Darwin") && len == 7, len);
    len = sizeof buf;
    r = bsd6(SYS_sysctlbyname, "kern.ostype", 12, buf, &len, 0, 0);
    check("a name length counting the NUL works too", r.err == 0 && len == 7, r.err);
    len = 0;
    r = bsd6(SYS_sysctlbyname, "kern.ostype", 11, 0, &len, 0, 0);
    check("a NULL buffer reports length 7", r.err == 0 && len == 7, len);
    i64 ps = 0;
    len = sizeof ps;
    r = bsd6(SYS_sysctlbyname, "hw.pagesize", 11, &ps, &len, 0, 0);
    check("hw.pagesize by name fills 8 bytes when there is room", r.err == 0 && len == 8, len);
    value("hw.pagesize", ps);
    i32 ps32 = 0;
    len = sizeof ps32;
    r = bsd6(SYS_sysctlbyname, "hw.pagesize", 11, &ps32, &len, 0, 0);
    check("and 4 bytes when there are 4", r.err == 0 && len == 4 && ps32 == ps, len);
    return finish();
}
