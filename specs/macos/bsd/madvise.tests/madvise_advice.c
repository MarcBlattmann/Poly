/* madvise, BSD 75 (address, length, advice): the advice values of
 * <sys/mman.h> 0 to 9 (NORMAL, RANDOM, SEQUENTIAL, WILLNEED, DONTNEED, FREE,
 * ZERO_WIRED_PAGES, FREE_REUSABLE, FREE_REUSE, CAN_REUSE) are accepted and
 * do not change what the memory holds; 12 is EINVAL. Values: advice 10
 * (MADV_PAGEOUT). */
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
    char *p = (char *)m.ret;
    p[0] = 9;
    p[ps] = 8;
    for (int advice = 0; advice <= 9; advice++) {
        sysret r = bsd3(SYS_madvise, m.ret, ps, advice);
        check("advice accepted", r.err == 0 && r.ret == 0, advice * 1000 + r.err);
    }
    check("and the memory is as it was", p[0] == 9 && p[ps] == 8, p[0]);
    check_err("advice 12: EINVAL", bsd3(SYS_madvise, m.ret, ps, 12), EINVAL);
    check_err("advice -1: EINVAL", bsd3(SYS_madvise, m.ret, ps, -1), EINVAL);
    value("advice 10 (MADV_PAGEOUT): errno", bsd3(SYS_madvise, m.ret, ps, 10).err);
    return finish();
}
