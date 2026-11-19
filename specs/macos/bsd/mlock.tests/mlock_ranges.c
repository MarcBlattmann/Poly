/* mlock, BSD 203, ranges: a length of 0 is fine; a length of 1 locks a page;
 * ENOMEM for a length that cannot fit the address space (1 << 40). Values: pages
 * that are not mapped (accepted on the oracle) and an address that is not page
 * aligned, so that the range runs one byte into the next page (ENOMEM here:
 * that page is not mapped). */
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
    sysret m = anon(ps, PROT_READ | PROT_WRITE);
    check_ok("a length of 0", bsd2(SYS_mlock, m.ret, 0));
    check_ok("a length of 1", bsd2(SYS_mlock, m.ret, 1));
    check_err("a length of 1 << 40: ENOMEM", bsd2(SYS_mlock, m.ret, (i64)1 << 40), ENOMEM);
    value("pages that are not mapped: errno", bsd2(SYS_mlock, 0x100000, ps).err);
    value("an unaligned address: errno", bsd2(SYS_mlock, m.ret + 1, ps).err);
    return finish();
}
