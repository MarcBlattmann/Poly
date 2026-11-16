/* msync, BSD 65, ranges: ENOMEM for pages that are not mapped, EINVAL for an
 * address that is not page aligned and for a length of 0. */
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
    sysret m = anon(2 * ps, PROT_READ | PROT_WRITE);
    check_err("pages that are not mapped: ENOMEM", bsd3(SYS_msync, 0x100000, ps, MS_SYNC), ENOMEM);
    check_err("an unaligned address: EINVAL", bsd3(SYS_msync, m.ret + 1, ps, MS_SYNC), EINVAL);
    check_err("a length of 0: EINVAL", bsd3(SYS_msync, m.ret, 0, MS_SYNC), EINVAL);
    check_ok("the whole mapping", bsd3(SYS_msync, m.ret, 2 * ps, MS_ASYNC));
    return finish();
}
