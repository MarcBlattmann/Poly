/* uname: sysname and machine, and every field NUL-terminated. The release
 * and node name differ between machines, so they are not printed. */
#include "poly_test.h"

#define FIELD 65 /* linux/utsname.h: __NEW_UTS_LEN + 1 */

struct utsname {
    char sysname[FIELD], nodename[FIELD], release[FIELD], version[FIELD], machine[FIELD], domainname[FIELD];
};

static int terminated(const char *f) {
    for (int i = 0; i < FIELD; i++)
        if (!f[i]) return 1;
    return 0;
}

int test_main(void) {
    struct utsname u;
    memset(&u, 0xff, sizeof u);
    i64 r = syscall1(SYS_uname, &u);
    check("uname_succeeds", r == 0, r);
    check("uname_sysname_is_linux", str_eq(u.sysname, "Linux"), 0);
    check("uname_machine_is_x86_64", str_eq(u.machine, "x86_64"), 0);
    int all = terminated(u.sysname) && terminated(u.nodename) && terminated(u.release) && terminated(u.version) &&
              terminated(u.machine) && terminated(u.domainname);
    check("uname_fields_nul_terminated", all, 0);
    check("uname_release_not_empty", u.release[0] != 0, 0);
    r = syscall1(SYS_uname, 8);
    check("uname_bad_pointer_efault", r == -EFAULT, r);
    return finish();
}
