/* mincore, BSD 78, errors: EFAULT for a vector that cannot be written; a
 * length of 0 is fine. Values: pages that are not mapped and an address that is
 * not page aligned (both returned 0 on the oracle). */
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
    *(char *)m.ret = 1;
    unsigned char vec[4];
    check_err("a vector at address 8: EFAULT", bsd3(SYS_mincore, m.ret, ps, (void *)8), EFAULT);
    check_ok("a length of 0", bsd3(SYS_mincore, m.ret, 0, vec));
    value("pages that are not mapped: errno", bsd3(SYS_mincore, 0x100000, ps, vec).err);
    value("an unaligned address: errno", bsd3(SYS_mincore, m.ret + 1, ps, vec).err);
    check_ok("a good call afterwards", bsd3(SYS_mincore, m.ret, ps, vec));
    return finish();
}
