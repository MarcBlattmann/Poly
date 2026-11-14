/* sysctl(2), BSD 202, hw.pagesize {CTL_HW, HW_PAGESIZE} and hw.ncpu: 4-byte
 * integers; a larger buffer gets 4 bytes, a 2-byte one ENOMEM. The page
 * size is a value (16384 on arm64). */
#include "poly_mtest.h"

int test_main(void) {
    int mib[2] = {CTL_HW, HW_PAGESIZE};
    i64 ps = 0;
    size_t len = sizeof ps;
    sysret r = bsd6(SYS_sysctl, mib, 2, &ps, &len, 0, 0);
    check("hw.pagesize succeeds", r.err == 0, r.err);
    check("into 8 bytes it writes 4", len == 4, len);
    value("hw.pagesize", ps);
    i32 ps32 = 0;
    len = 4;
    bsd6(SYS_sysctl, mib, 2, &ps32, &len, 0, 0);
    check("the same in 4 bytes", ps32 == ps, ps32);
    check("a power of two of at least 4096", ps32 >= 4096 && (ps32 & (ps32 - 1)) == 0, ps32);
    len = 2;
    check_err("a 2-byte buffer: ENOMEM", bsd6(SYS_sysctl, mib, 2, &ps32, &len, 0, 0), ENOMEM);
    int ncpu[2] = {CTL_HW, HW_NCPU};
    i32 n = 0;
    len = 4;
    r = bsd6(SYS_sysctl, ncpu, 2, &n, &len, 0, 0);
    check("hw.ncpu is positive", r.err == 0 && n > 0, r.err);
    return finish();
}
