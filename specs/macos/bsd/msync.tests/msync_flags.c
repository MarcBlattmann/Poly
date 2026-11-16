/* msync, BSD 65 (address, length, flags): MS_SYNC (0x10), MS_ASYNC (1) and
 * 0 are accepted for a file mapping and for anonymous memory, MS_INVALIDATE
 * (2) too; MS_SYNC with MS_ASYNC is EINVAL. Value: an unknown flag bit. */
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
    sysret f = bsd3(SYS_open, "f", O_CREAT | O_RDWR, 0644);
    bsd3(SYS_write, f.ret, "abc", 3);
    sysret m = bsd6(SYS_mmap, 0, ps, PROT_READ | PROT_WRITE, MAP_SHARED, f.ret, 0);
    check_ok("MS_SYNC", bsd3(SYS_msync, m.ret, ps, MS_SYNC));
    check_ok("MS_ASYNC", bsd3(SYS_msync, m.ret, ps, MS_ASYNC));
    check_ok("flags 0", bsd3(SYS_msync, m.ret, ps, 0));
    check_ok("MS_INVALIDATE", bsd3(SYS_msync, m.ret, ps, MS_INVALIDATE));
    check_err("MS_SYNC|MS_ASYNC: EINVAL", bsd3(SYS_msync, m.ret, ps, MS_SYNC | MS_ASYNC), EINVAL);
    sysret a = anon(ps, PROT_READ | PROT_WRITE);
    check_ok("anonymous memory", bsd3(SYS_msync, a.ret, ps, MS_SYNC));
    value("flag bit 0x100: errno", bsd3(SYS_msync, m.ret, ps, 0x100).err);
    return finish();
}
