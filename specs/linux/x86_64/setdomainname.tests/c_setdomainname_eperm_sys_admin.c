/* Only CAP_SYS_ADMIN (21) matters: with every other capability kept
 * and just CAP_SYS_ADMIN removed from all sets (a normal user has none
 * to keep), setdomainname fails with EPERM and the name stays the same.
 * Being root (euid 0) alone is not enough.
 */
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_capget 125
#define SYS_capset 126
#define SYS_setdomainname 171

/* linux/capability.h */
#define CAP_SYS_ADMIN 21
struct cap_header {
    u32 version;
    i32 pid;
};
struct cap_data {
    u32 effective, permitted, inheritable;
};

int test_main(void) {
    struct a_utsname before, after;
    memset(&before, 0, sizeof before);
    memset(&after, 0, sizeof after);
    syscall1(SYS_uname, &before);

    struct cap_header h = { 0x20080522, 0 };
    struct cap_data d[2], e[2];
    memset(d, 0, sizeof d);
    expect("capget", syscall2(SYS_capget, &h, d), 0);
    d[0].effective &= ~(1U << CAP_SYS_ADMIN);
    d[0].permitted &= ~(1U << CAP_SYS_ADMIN);
    d[0].inheritable &= ~(1U << CAP_SYS_ADMIN);
    expect("capset", syscall2(SYS_capset, &h, d), 0);
    memset(e, 0xff, sizeof e);
    syscall2(SYS_capget, &h, e);
    int gone = !(e[0].effective & (1U << CAP_SYS_ADMIN)) && !(e[0].permitted & (1U << CAP_SYS_ADMIN));
    check("sys_admin_gone", gone, 0);
    if (!gone) return finish(); /* never try to change the real domain name */

    expect("set_eperm", syscall2(SYS_setdomainname, "poly-no-admin", 13), -EPERM);
    expect("len_65_eperm", syscall2(SYS_setdomainname, "poly-no-admin", 65), -EPERM);
    syscall1(SYS_uname, &after);
    check("domainname_unchanged", mem_eq(before.domainname, after.domainname, sizeof before.domainname), 0);
    return finish();
}
