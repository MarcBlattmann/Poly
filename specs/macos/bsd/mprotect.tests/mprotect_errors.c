/* mprotect, BSD 74, errors: EINVAL for an address that is not page aligned
 * and for a negative length; ENOMEM for pages that are not mapped, also
 * when the range runs from mapped into unmapped pages. */
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
    check_err("an unaligned address: EINVAL", bsd3(SYS_mprotect, m.ret + 1, ps, PROT_READ), EINVAL);
    check_err("a negative length: EINVAL", bsd3(SYS_mprotect, m.ret, -ps, PROT_READ), EINVAL);
    check_err("pages that are not mapped: ENOMEM", bsd3(SYS_mprotect, 0x100000, ps, PROT_READ), ENOMEM);
    check_err("a range that leaves the mapping: ENOMEM", bsd3(SYS_mprotect, m.ret + ps, 3 * ps, PROT_READ), ENOMEM);
    bsd2(SYS_munmap, m.ret, 2 * ps);
    check_err("an unmapped mapping: ENOMEM", bsd3(SYS_mprotect, m.ret, ps, PROT_READ), ENOMEM);
    return finish();
}
