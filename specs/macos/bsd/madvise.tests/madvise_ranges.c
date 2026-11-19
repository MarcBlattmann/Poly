/* madvise, BSD 75, ranges: ENOMEM for pages that are not mapped (an address
 * below the first mapping; the hole where a mapping was unmapped is a value,
 * EINVAL on the oracle); a length of 0 is fine; a range that runs past the end of the mapping, and an
 * address that is not page aligned, are values (EINVAL and accepted on the
 * oracle). */
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
    check_err("pages that are not mapped: ENOMEM", bsd3(SYS_madvise, 0x100000, ps, MADV_NORMAL), ENOMEM);
    check_ok("a length of 0", bsd3(SYS_madvise, m.ret, 0, MADV_NORMAL));
    check_ok("the whole mapping", bsd3(SYS_madvise, m.ret, 2 * ps, MADV_NORMAL));
    value("a range of 5 pages over a mapping of 2: errno", bsd3(SYS_madvise, m.ret, 5 * ps, MADV_NORMAL).err);
    value("an unaligned address: errno", bsd3(SYS_madvise, m.ret + 1, ps, MADV_NORMAL).err);
    bsd2(SYS_munmap, m.ret, 2 * ps);
    value("a hole where a mapping was unmapped: errno", bsd3(SYS_madvise, m.ret, ps, MADV_NORMAL).err);
    return finish();
}
