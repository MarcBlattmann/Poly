/* minherit, BSD 250: VM_INHERIT_NONE (2) leaves the region out of the child:
 * the child's access to it is a fault at an address that is not mapped (a
 * signal; which one is a value: SIGSEGV 11), while the parent keeps it. */
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
    volatile char *p = (char *)m.ret;
    *p = 1;
    check_ok("VM_INHERIT_NONE", bsd3(SYS_minherit, m.ret, ps, VM_INHERIT_NONE));
    sysret f = bsd0(SYS_fork);
    if (f.ret2 == 1) {
        (void)*p;
        poly_exit(42);
    }
    int st = poly_wait(f.ret);
    check("the child dies when it touches the region", !W_EXITED(st), st);
    value("signal", W_TERMSIG(st));
    check("the parent still has it", *p == 1, *p);
    return finish();
}
