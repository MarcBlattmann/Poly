/* munmap(2), BSD 73: an unmapped page is gone (mprotect(2) of it is
 * ENOMEM); unmapping it again succeeds; MAP_FIXED can map there again. */
#include "poly_mtest.h"

static i64 page_size(void) {
    i64 ps = 0;
    size_t len = sizeof ps;
    bsd6(SYS_sysctlbyname, "hw.pagesize", 11, &ps, &len, 0, 0);
    return ps;
}

int test_main(void) {
    i64 ps = page_size();
    sysret m = bsd6(SYS_mmap, 0, ps, PROT_READ | PROT_WRITE, MAP_ANON | MAP_PRIVATE, -1, 0);
    check_ok("mprotect of the mapped page", bsd3(SYS_mprotect, m.ret, ps, PROT_READ));
    sysret r = bsd2(SYS_munmap, m.ret, ps);
    check("munmap returns 0", r.err == 0 && r.ret == 0, r.err);
    check_err("then mprotect: ENOMEM", bsd3(SYS_mprotect, m.ret, ps, PROT_READ), ENOMEM);
    check_ok("munmap of the unmapped page again", bsd2(SYS_munmap, m.ret, ps));
    sysret f = bsd6(SYS_mmap, m.ret, ps, PROT_READ | PROT_WRITE, MAP_ANON | MAP_PRIVATE | MAP_FIXED, -1, 0);
    check("MAP_FIXED maps the same address again", f.err == 0 && f.ret == m.ret, f.err);
    check("and the new page is zero filled", ((unsigned char *)f.ret)[0] == 0, 0);
    return finish();
}
