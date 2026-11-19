/* mincore, BSD 78 (address, length, vector): one byte per page; a page
 * the process has touched has MINCORE_INCORE (1) set, and anonymous memory has
 * 0x80 (MINCORE_ANONYMOUS); the other flags (referenced, modified) are a
 * value. A length of 1 fills one byte. */
#include "poly_mtest.h"

static __attribute__((unused)) i64 page_size(void) {
    i64 ps = 0;
    size_t len = sizeof ps;
    bsd6(SYS_sysctlbyname, "hw.pagesize", 11, &ps, &len, 0, 0);
    return ps;
}
static __attribute__((unused)) sysret anon(i64 bytes, int prot) { return bsd6(SYS_mmap, 0, bytes, prot, MAP_ANON | MAP_PRIVATE, -1, 0); }

int test_main(void) {
    i64 ps = page_size();
    sysret m = anon(3 * ps, PROT_READ | PROT_WRITE);
    char *p = (char *)m.ret;
    p[0] = 1;
    p[ps] = 1;
    unsigned char vec[8];
    memset(vec, 0xee, sizeof vec);
    sysret r = bsd3(SYS_mincore, m.ret, 3 * ps, vec);
    check("returns 0", r.err == 0 && r.ret == 0, r.err);
    check("touched pages are in core", (vec[0] & MINCORE_INCORE) && (vec[1] & MINCORE_INCORE), vec[0]);
    check("all three are anonymous (0x80)", (vec[0] & 0x80) && (vec[1] & 0x80) && (vec[2] & 0x80), vec[2]);
    check("exactly three bytes are written", vec[3] == 0xee, vec[3]);
    value("vector byte of a touched page", vec[0]);
    value("vector byte of an untouched page", vec[2]);
    memset(vec, 0xee, sizeof vec);
    bsd3(SYS_mincore, m.ret, 1, vec);
    check("a length of 1 fills one byte", vec[0] != 0xee && vec[1] == 0xee, vec[1]);
    return finish();
}
