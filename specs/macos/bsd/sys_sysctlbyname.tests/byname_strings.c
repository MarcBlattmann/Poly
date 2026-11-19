/* sysctlbyname, BSD 274, strings and a time: kern.version starts "Darwin Kernel
 * Version "; kern.hostname is not empty; kern.osversion and hw.machine are
 * the same as the numbers {CTL_KERN, KERN_OSVERSION} and {CTL_HW, HW_MACHINE};
 * kern.boottime is a struct timeval of 16 bytes after 2023. Values:
 * kern.osproductversion ("27.0"), hw.machine ("arm64"). */
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

static char other[256];

int test_main(void) {
    size_t len = sizeof sbuf;
    sysret r = byname("kern.version", sbuf, &len);
    check("kern.version starts with \"Darwin Kernel Version \"", r.err == 0 && starts_with(sbuf, "Darwin Kernel Version "), r.err);
    len = sizeof sbuf;
    r = byname("kern.hostname", sbuf, &len);
    check("kern.hostname is not empty", r.err == 0 && len > 1, len);
    len = sizeof sbuf;
    byname("kern.osversion", sbuf, &len);
    size_t l2 = sizeof other;
    mib2(CTL_KERN, KERN_OSVERSION, other, &l2);
    check("kern.osversion equals the number's", len == l2 && mem_eq(sbuf, other, len), len);
    len = sizeof sbuf;
    byname("hw.machine", sbuf, &len);
    l2 = sizeof other;
    mib2(CTL_HW, HW_MACHINE, other, &l2);
    check("hw.machine equals the number's", len == l2 && mem_eq(sbuf, other, len), len);
    value_str("hw.machine", sbuf);
    len = sizeof sbuf;
    r = byname("kern.osproductversion", sbuf, &len);
    check("kern.osproductversion is a string", r.err == 0 && len > 1 && sbuf[len - 1] == 0, len);
    value_str("kern.osproductversion", sbuf);
    struct poly_timespec bt = {0, 0};
    len = 16;
    r = byname("kern.boottime", &bt, &len);
    check("kern.boottime: 16 bytes, after 2023", r.err == 0 && len == 16 && bt.tv_sec > 1700000000, bt.tv_sec);
    return finish();
}
