/* mlock, BSD 203, and fork: a locked page is copied to the child like any
 * other, and the child can unlock it (munlock returns 0). */
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
    char *p = (char *)m.ret;
    p[0] = 5;
    check_ok("mlock", bsd2(SYS_mlock, m.ret, ps));
    sysret f = bsd0(SYS_fork);
    if (f.ret2 == 1) {
        int ok = p[0] == 5 && bsd2(SYS_munlock, m.ret, ps).err == 0;
        p[0] = 6;
        poly_exit(ok ? 0 : 1);
    }
    int st = poly_wait(f.ret);
    check("the child saw the content and could munlock", W_EXITED(st) && W_EXITSTATUS(st) == 0, st);
    check("its write did not reach the parent", p[0] == 5, p[0]);
    return finish();
}
