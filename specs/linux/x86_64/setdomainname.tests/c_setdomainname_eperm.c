/* Without CAP_SYS_ADMIN (all capability sets emptied with capset first,
 * so the result is the same for root and for a normal user),
 * setdomainname of a valid name fails with EPERM (-1 from the raw call),
 * every time, and leaves the domain name unchanged: uname's domainname and
 * /proc/sys/kernel/domainname read the same as before.
 */
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_capget 125
#define SYS_capset 126
#define SYS_setdomainname 171

/* linux/capability.h: _LINUX_CAPABILITY_VERSION_3, two data records */
struct cap_header {
    u32 version;
    i32 pid;
};
struct cap_data {
    u32 effective, permitted, inheritable;
};

/* Empty all three capability sets and confirm it with capget. */
static int drop_all_caps(void) {
    struct cap_header h = { 0x20080522, 0 };
    struct cap_data d[2], e[2];
    memset(d, 0, sizeof d);
    if (syscall2(SYS_capset, &h, d) != 0) return 0;
    memset(e, 0xff, sizeof e);
    if (syscall2(SYS_capget, &h, e) != 0) return 0;
    return e[0].effective == 0 && e[1].effective == 0 && e[0].permitted == 0 && e[1].permitted == 0;
}

static i64 read_file(const char *path, char *buf, size_t n) {
    i64 fd = a_open(path, O_RDONLY, 0);
    if (fd < 0) return fd;
    i64 r = a_read(fd, buf, n);
    a_close(fd);
    return r;
}

int test_main(void) {
    struct a_utsname before, after;
    char pbefore[128], pafter[128];
    memset(&before, 0, sizeof before);
    memset(&after, 0, sizeof after);
    expect("uname_before", syscall1(SYS_uname, &before), 0);
    i64 nb = read_file("/proc/sys/kernel/domainname", pbefore, sizeof pbefore);
    check("proc_before", nb > 0, nb);

    int dropped = drop_all_caps();
    check("caps_dropped", dropped, 0);
    if (!dropped) return finish(); /* never try to change the real domain name */

    expect("valid_name_eperm", syscall2(SYS_setdomainname, "poly-eperm", 10), -EPERM);
    expect("again_eperm", syscall2(SYS_setdomainname, "poly-eperm", 10), -EPERM);
    expect("one_byte_eperm", syscall2(SYS_setdomainname, "p", 1), -EPERM);
    expect("same_name_eperm", syscall2(SYS_setdomainname, before.domainname, str_len(before.domainname)), -EPERM);

    expect("uname_after", syscall1(SYS_uname, &after), 0);
    check("domainname_unchanged", mem_eq(before.domainname, after.domainname, sizeof before.domainname), 0);
    i64 na = read_file("/proc/sys/kernel/domainname", pafter, sizeof pafter);
    check("proc_unchanged", na == nb && nb > 0 && mem_eq(pbefore, pafter, (size_t)nb), na);
    return finish();
}
