/* sysctl(2), BSD 202, kern.ostype {CTL_KERN, KERN_OSTYPE}: "Darwin" and its
 * NUL (7 bytes); a NULL buffer reports the length; a buffer too small is
 * ENOMEM, with the length set to 0 and nothing copied; kern.osrelease is
 * the Darwin version (digits and dots). */
#include "poly_mtest.h"

int test_main(void) {
    int mib[2] = {CTL_KERN, KERN_OSTYPE};
    char buf[64];
    size_t len = sizeof buf;
    sysret r = bsd6(SYS_sysctl, mib, 2, buf, &len, 0, 0);
    check("kern.ostype succeeds", r.err == 0 && r.ret == 0, r.err);
    check("it is \"Darwin\"", str_eq(buf, "Darwin"), buf[0]);
    check("length 7 with the NUL", len == 7, len);
    len = 0;
    r = bsd6(SYS_sysctl, mib, 2, 0, &len, 0, 0);
    check("a NULL buffer reports length 7", r.err == 0 && len == 7, len);
    len = 3;
    memset(buf, 'z', 8);
    r = bsd6(SYS_sysctl, mib, 2, buf, &len, 0, 0);
    check("a 3-byte buffer: ENOMEM", r.err == ENOMEM, r.err);
    check("the length becomes 0", len == 0, len);
    check("nothing is copied", buf[0] == 'z' && buf[2] == 'z', buf[0]);
    int osr[2] = {CTL_KERN, KERN_OSRELEASE};
    len = sizeof buf;
    r = bsd6(SYS_sysctl, osr, 2, buf, &len, 0, 0);
    int ok = r.err == 0 && len > 1 && buf[0] >= '0' && buf[0] <= '9' && buf[len - 1] == 0;
    for (size_t i = 0; ok && i + 1 < len; i++)
        if (!((buf[i] >= '0' && buf[i] <= '9') || buf[i] == '.')) ok = 0;
    check("kern.osrelease is digits and dots", ok, r.err);
    return finish();
}
