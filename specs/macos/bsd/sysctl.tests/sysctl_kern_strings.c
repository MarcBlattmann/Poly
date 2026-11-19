/* sysctl(2), BSD 202, strings and a time: KERN_VERSION (4) starts "Darwin
 * Kernel Version " and ends with a NUL; KERN_HOSTNAME (10) is not empty;
 * KERN_OSVERSION (65) is the build ("26A428" on the oracle, a value);
 * KERN_BOOTTIME (21) is a struct timeval of 16 bytes (seconds, microseconds)
 * after 2023 and not in the future. */
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
    size_t len = sizeof sbuf;
    sysret r = mib2(CTL_KERN, KERN_VERSION, sbuf, &len);
    check("kern.version succeeds", r.err == 0 && len > 22 && sbuf[len - 1] == 0, r.err);
    check("it starts with \"Darwin Kernel Version \"", starts_with(sbuf, "Darwin Kernel Version "), sbuf[0]);
    len = sizeof sbuf;
    r = mib2(CTL_KERN, KERN_HOSTNAME, sbuf, &len);
    check("kern.hostname is not empty", r.err == 0 && len > 1, len);
    len = sizeof sbuf;
    r = mib2(CTL_KERN, KERN_OSVERSION, sbuf, &len);
    check("kern.osversion is a short string", r.err == 0 && len > 3 && len < 16, len);
    value_str("kern.osversion", sbuf);
    struct poly_timespec bt = {0, -1};
    len = 16;
    r = mib2(CTL_KERN, KERN_BOOTTIME, &bt, &len);
    check("kern.boottime is 16 bytes", r.err == 0 && len == 16, len);
    check("seconds after 2023, microseconds below a million", bt.tv_sec > 1700000000 && bt.tv_nsec >= 0 && bt.tv_nsec < 1000000, bt.tv_sec);
    len = 8;
    r = mib2(CTL_KERN, KERN_BOOTTIME, &bt, &len);
    check("a buffer of 8 bytes: ENOMEM", r.err == ENOMEM, r.err);
    return finish();
}
