/* minherit, BSD 250, errors: the inheritance values 0, 1 and 2 are accepted,
 * 3 (VM_INHERIT_DONATE_COPY), 4 and 5 are EINVAL. Values: an address that is
 * not page aligned and one that is not mapped (both accepted on the oracle). */
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
    check_ok("SHARE", bsd3(SYS_minherit, m.ret, ps, VM_INHERIT_SHARE));
    check_ok("COPY", bsd3(SYS_minherit, m.ret, ps, VM_INHERIT_COPY));
    check_ok("NONE", bsd3(SYS_minherit, m.ret, ps, VM_INHERIT_NONE));
    check_err("DONATE_COPY (3): EINVAL", bsd3(SYS_minherit, m.ret, ps, VM_INHERIT_DONATE_COPY), EINVAL);
    check_err("4: EINVAL", bsd3(SYS_minherit, m.ret, ps, 4), EINVAL);
    check_err("5: EINVAL", bsd3(SYS_minherit, m.ret, ps, 5), EINVAL);
    value("an unaligned address: errno", bsd3(SYS_minherit, m.ret + 1, ps, VM_INHERIT_COPY).err);
    value("an address that is not mapped: errno", bsd3(SYS_minherit, 0x100000, ps, VM_INHERIT_COPY).err);
    return finish();
}
