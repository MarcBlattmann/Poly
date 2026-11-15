/* sysctlbyname, BSD 274: ENOENT for unknown names, the empty name, a node
 * ("kern") and a length that cuts the name short; ENAMETOOLONG for a very
 * long name; EFAULT; EPERM for setting kern.ostype; ENOMEM for a buffer too
 * small, the length set to 0. */
#include "poly_mtest.h"

static char longname[2000];

int test_main(void) {
    char buf[64];
    size_t len = sizeof buf;
    check_err("kern.nosuchthing: ENOENT", bsd6(SYS_sysctlbyname, "kern.nosuchthing", 16, buf, &len, 0, 0), ENOENT);
    len = sizeof buf;
    check_err("the empty name: ENOENT", bsd6(SYS_sysctlbyname, "", 0, buf, &len, 0, 0), ENOENT);
    len = sizeof buf;
    check_err("the node kern: ENOENT", bsd6(SYS_sysctlbyname, "kern", 4, buf, &len, 0, 0), ENOENT);
    len = sizeof buf;
    check_err("length 5 of kern.ostype: ENOENT", bsd6(SYS_sysctlbyname, "kern.ostype", 5, buf, &len, 0, 0), ENOENT);
    for (int i = 0; i < 1999; i++) longname[i] = 'a';
    len = sizeof buf;
    check_err("a 1999-byte name: ENAMETOOLONG", bsd6(SYS_sysctlbyname, longname, 1999, buf, &len, 0, 0), ENAMETOOLONG);
    len = sizeof buf;
    check_err("the name at address 8: EFAULT", bsd6(SYS_sysctlbyname, (void *)8, 11, buf, &len, 0, 0), EFAULT);
    check_err("setting kern.ostype: EPERM", bsd6(SYS_sysctlbyname, "kern.ostype", 11, 0, 0, "Linux", 6), EPERM);
    len = 2;
    check_err("a 2-byte buffer: ENOMEM", bsd6(SYS_sysctlbyname, "kern.ostype", 11, buf, &len, 0, 0), ENOMEM);
    check("the length becomes 0", len == 0, len);
    return finish();
}
