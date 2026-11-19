/* sysctlbyname, BSD 274, numbers: hw.memsize is 8 bytes and the same as
 * the number {CTL_HW, HW_MEMSIZE}; hw.logicalcpu equals hw.ncpu; hw.physicalcpu
 * is positive; hw.byteorder is 1234; hw.pagesize32 equals hw.pagesize;
 * kern.maxfilesperproc is positive; vm.loadavg is 24 bytes and a smaller buffer
 * is ENOMEM. Values: hw.tbfrequency (24000000 on arm64), hw.cachelinesize
 * (128), kern.argmax (1048576). */
#include "poly_mtest.h"

static __attribute__((unused)) sysret mib2(int a, int b, void *old, size_t *len) {
    int m[2] = {a, b};
    return bsd6(SYS_sysctl, m, 2, old, len, 0, 0);
}
static __attribute__((unused)) sysret byname(const char *name, void *old, size_t *len) { return bsd6(SYS_sysctlbyname, name, str_len(name), old, len, 0, 0); }
static __attribute__((unused)) char sbuf[4096];
static __attribute__((unused)) int starts_with(const char *s, const char *prefix) {
    while (*prefix) if (*s++ != *prefix++) return 0;
    return 1;
}

int test_main(void) {
    u64 a = 0, b = 0;
    size_t len = 8;
    sysret r = byname("hw.memsize", &a, &len);
    check("hw.memsize: 8 bytes", r.err == 0 && len == 8, len);
    len = 8;
    mib2(CTL_HW, HW_MEMSIZE, &b, &len);
    check("the same as by number", a == b && a != 0, (i64)a);
    i32 x = 0, y = 0;
    len = 4;
    byname("hw.logicalcpu", &x, &len);
    len = 4;
    byname("hw.ncpu", &y, &len);
    check("hw.logicalcpu equals hw.ncpu", x == y && x > 0, x);
    len = 4;
    r = byname("hw.physicalcpu", &x, &len);
    check("hw.physicalcpu is positive", r.err == 0 && x > 0, x);
    len = 4;
    byname("hw.byteorder", &x, &len);
    check("hw.byteorder is 1234", x == 1234, x);
    i64 ps = 0;
    len = 8;
    byname("hw.pagesize", &ps, &len);
    len = 4;
    byname("hw.pagesize32", &x, &len);
    check("hw.pagesize32 equals hw.pagesize", x == ps && x > 0, x);
    len = 4;
    r = byname("kern.maxfilesperproc", &x, &len);
    check("kern.maxfilesperproc is positive", r.err == 0 && x > 0, x);
    len = 0;
    r = byname("vm.loadavg", 0, &len);
    check("vm.loadavg: 24 bytes", r.err == 0 && len == 24, len);
    len = 4;
    check_err("vm.loadavg into 4 bytes: ENOMEM", byname("vm.loadavg", &x, &len), ENOMEM);
    i64 v = 0;
    len = 8;
    byname("hw.tbfrequency", &v, &len);
    value("hw.tbfrequency", v);
    len = 8;
    byname("hw.cachelinesize", &v, &len);
    value("hw.cachelinesize", v);
    len = 4;
    byname("kern.argmax", &x, &len);
    value("kern.argmax", x);
    return finish();
}
