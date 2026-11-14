/* munmap(2), BSD 73: EINVAL for an unaligned address, a length of 0 or a
 * negative length; a length running past the end of the address space
 * returns 0 and unmaps nothing. */
#include "poly_mtest.h"

static i64 page_size(void) {
    i64 ps = 0;
    size_t len = sizeof ps;
    bsd6(SYS_sysctlbyname, "hw.pagesize", 11, &ps, &len, 0, 0);
    return ps;
}

int test_main(void) {
    i64 ps = page_size();
    sysret m = bsd6(SYS_mmap, 0, 2 * ps, PROT_READ | PROT_WRITE, MAP_ANON | MAP_PRIVATE, -1, 0);
    check_err("an unaligned address: EINVAL", bsd2(SYS_munmap, m.ret + 1, ps), EINVAL);
    check_err("length 0: EINVAL", bsd2(SYS_munmap, m.ret, 0), EINVAL);
    check_err("a negative length: EINVAL", bsd2(SYS_munmap, m.ret, -ps), EINVAL);
    check_ok("the mapping is still there", bsd3(SYS_mprotect, m.ret, 2 * ps, PROT_READ));
    sysret h = bsd2(SYS_munmap, m.ret, (i64)1 << 62);
    check("length 2^62 returns 0", h.err == 0 && h.ret == 0, h.err);
    check_ok("and unmapped nothing", bsd3(SYS_mprotect, m.ret, 2 * ps, PROT_READ));
    return finish();
}
