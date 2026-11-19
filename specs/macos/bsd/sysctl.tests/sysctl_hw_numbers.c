/* sysctl(2), BSD 202, more numbers: HW_MEMSIZE (24) is an 8-byte integer,
 * more than 1 GiB; HW_BYTEORDER (4) is 1234; KERN_OSREV (3) is 199506,
 * KERN_ARGMAX (8) at least 262144 (1048576 on the oracle: a value), KERN_MAXPROC
 * (6) and KERN_MAXFILES (7) are positive; HW_MACHINE (1) is a string (a value:
 * "arm64"). Each is 4 bytes unless said, and a larger buffer gets only that many. */
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
    size_t len = 8;
    u64 mem = 0;
    sysret r = mib2(CTL_HW, HW_MEMSIZE, &mem, &len);
    check("hw.memsize succeeds with 8 bytes", r.err == 0 && len == 8, r.err);
    check("more than 1 GiB", mem > ((u64)1 << 30), (i64)(mem >> 20));
    i32 v = 0;
    len = 4;
    r = mib2(CTL_HW, HW_BYTEORDER, &v, &len);
    check("hw.byteorder is 1234", r.err == 0 && v == 1234, v);
    len = 4;
    r = mib2(CTL_KERN, KERN_OSREV, &v, &len);
    check("kern.osrev is 199506", r.err == 0 && v == 199506, v);
    len = 4;
    r = mib2(CTL_KERN, KERN_ARGMAX, &v, &len);
    check("kern.argmax is at least 262144", r.err == 0 && v >= 262144, v);
    value("kern.argmax", v);
    len = 4;
    r = mib2(CTL_KERN, KERN_MAXPROC, &v, &len);
    check("kern.maxproc is positive", r.err == 0 && v > 0, v);
    len = 4;
    r = mib2(CTL_KERN, KERN_MAXFILES, &v, &len);
    check("kern.maxfiles is positive", r.err == 0 && v > 0, v);
    len = sizeof sbuf;
    r = mib2(CTL_HW, HW_MACHINE, sbuf, &len);
    check("hw.machine is a string", r.err == 0 && len > 1 && sbuf[len - 1] == 0, len);
    value_str("hw.machine", sbuf);
    return finish();
}
