/* On both ends of a pipe there are no attributes: every user.* name
 * ("user." too) and trusted.* are ENODATA, unknown prefixes, security.*
 * ("security." too), the ACL names and system.x are EOPNOTSUPP. Name
 * errors still come first (empty ERANGE, unreadable EFAULT), and the
 * buffer is never looked at (a NULL buffer with a size gives ENODATA). */
#include "a_sys.h"

#define SYS_capget 125
#define SYS_capset 126
#define SYS_fgetxattr 193
#define ENODATA 61

static char buf[64];

static i64 drop_caps(void) {
    u32 h[2] = { 0x20080522, 0 }, d[6] = { 0 }, e[6];
    i64 r = syscall2(SYS_capset, h, d);
    if (r < 0) return r;
    r = syscall2(SYS_capget, h, e);
    if (r < 0) return r;
    return (e[0] | e[1] | e[3] | e[4]) ? -1 : 0;
}

static i64 fget(i64 fd, const void *name) { return syscall4(SYS_fgetxattr, fd, name, buf, sizeof buf); }

static void ends(const char *tag, i64 fd) {
    char n[64];
    int base = 0;
    while (tag[base]) { n[base] = tag[base]; base++; }
    static const char *names[] = { "user.a", "user.", "trusted.a", "foo.a", "security.a", "security.",
                                   "system.posix_acl_access", "system.posix_acl_default", "system.x", 0 };
    static const i64 want[] = { -ENODATA, -ENODATA, -ENODATA, -EOPNOTSUPP, -EOPNOTSUPP, -EOPNOTSUPP,
                                -EOPNOTSUPP, -EOPNOTSUPP, -EOPNOTSUPP };
    for (int i = 0; names[i]; i++) {
        int j = base;
        for (const char *s = names[i]; *s; s++) n[j++] = *s;
        n[j] = 0;
        expect(n, fget(fd, names[i]), want[i]);
    }
}

int test_main(void) {
    i64 dropped = drop_caps();
    expect("drop_caps", dropped, 0);
    if (dropped != 0) return finish();
    int p[2] = { -1, -1 };
    expect("setup_pipe", syscall2(SYS_pipe2, p, 0), 0);
    ends("read_end_", p[0]);
    ends("write_end_", p[1]);
    expect("empty_name", fget(p[0], ""), -ERANGE);
    expect("name_fault", fget(p[0], (const void *)1), -EFAULT);
    expect("null_buffer", syscall4(SYS_fgetxattr, p[0], "user.a", 0, 16), -ENODATA);
    expect("size0", syscall4(SYS_fgetxattr, p[1], "user.a", 0, 0), -ENODATA);
    a_close(p[0]);
    a_close(p[1]);
    return finish();
}
