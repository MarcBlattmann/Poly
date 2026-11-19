/* mprotect, BSD 74 (address, length, protection): changes the protection
 * of the pages that [address, address + length) touches (a length of 1
 * covers a whole page), keeps their content, and a protection can be given
 * back (RW, RO, PROT_NONE and RW again). */
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
    sysret m = anon(4 * ps, PROT_READ | PROT_WRITE);
    char *p = (char *)m.ret;
    p[0] = 1;
    p[ps] = 2;
    sysret r = bsd3(SYS_mprotect, m.ret, ps, PROT_READ);
    check("RW to RO returns 0", r.err == 0 && r.ret == 0, r.err);
    check("the page still reads back", p[0] == 1, p[0]);
    check_ok("RO to RW again", bsd3(SYS_mprotect, m.ret, ps, PROT_READ | PROT_WRITE));
    p[0] = 5;
    check("and it can be written", p[0] == 5, p[0]);
    check_ok("a page to PROT_NONE", bsd3(SYS_mprotect, m.ret + ps, ps, PROT_NONE));
    check_ok("PROT_NONE back to RW", bsd3(SYS_mprotect, m.ret + ps, ps, PROT_READ | PROT_WRITE));
    check("the content of the page survived", p[ps] == 2, p[ps]);
    check_ok("a length of 1 covers the page", bsd3(SYS_mprotect, m.ret + 2 * ps, 1, PROT_READ));
    check_ok("a range of all four pages", bsd3(SYS_mprotect, m.ret, 4 * ps, PROT_READ | PROT_WRITE));
    check_ok("a length of 0", bsd3(SYS_mprotect, m.ret, 0, PROT_READ));
    return finish();
}
