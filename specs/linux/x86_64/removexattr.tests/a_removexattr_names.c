/* removexattr name checks, with all capabilities dropped first (so root
 * gets the same answers): an empty name is ERANGE, a name of 255 bytes
 * works, 256 bytes or more ERANGE; a NULL or unmapped name EFAULT. Names
 * without a known prefix ("plain", "user" without dot, "USER.x", ".user.x",
 * "foo.bar") are EOPNOTSUPP on a writable file; "user." is EINVAL;
 * unknown system.* names ("system.x", "system.") EOPNOTSUPP; trusted.* and
 * security.* (also "trusted." and "security.") EPERM without
 * CAP_SYS_ADMIN. Odd bytes after "user." are an ordinary name. */
#include "a_sys.h"

#define SYS_capget 125
#define SYS_capset 126
#define SYS_setxattr 188
#define SYS_removexattr 197
#define ENODATA 61
#define D "/tmp/a_removexattr_names"
#define F D "/f"

static char name[400];

static i64 drop_caps(void) {
    u32 h[2] = { 0x20080522, 0 }, d[6] = { 0 }, e[6];
    i64 r = syscall2(SYS_capset, h, d);
    if (r < 0) return r;
    r = syscall2(SYS_capget, h, e);
    if (r < 0) return r;
    return (e[0] | e[1] | e[3] | e[4]) ? -1 : 0;
}

static i64 rm(const void *name_) { return syscall2(SYS_removexattr, F, name_); }
static i64 set(const char *n) { return syscall5(SYS_setxattr, F, n, "v", 1, 0); }

static const char *mkname(int len) {
    str_cpy(name, "user.");
    for (int i = 5; i < len; i++) name[i] = 'n';
    name[len] = 0;
    return name;
}

int test_main(void) {
    expect("drop_caps", drop_caps(), 0);
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_file", a_make_file(F, "", 0644), 0);
    expect("empty_erange", rm(""), -ERANGE);
    expect("null_efault", rm((const void *)0), -EFAULT);
    expect("unmapped_efault", rm((const void *)8), -EFAULT);
    expect("set_255", set(mkname(255)), 0);
    expect("remove_255", rm(mkname(255)), 0);
    expect("remove_255_again", rm(mkname(255)), -ENODATA);
    expect("remove_256_erange", rm(mkname(256)), -ERANGE);
    expect("remove_399_erange", rm(mkname(399)), -ERANGE);
    expect("plain_eopnotsupp", rm("plain"), -EOPNOTSUPP);
    expect("user_no_dot_eopnotsupp", rm("user"), -EOPNOTSUPP);
    expect("upper_user_eopnotsupp", rm("USER.x"), -EOPNOTSUPP);
    expect("dot_user_eopnotsupp", rm(".user.x"), -EOPNOTSUPP);
    expect("foo_bar_eopnotsupp", rm("foo.bar"), -EOPNOTSUPP);
    expect("user_dot_einval", rm("user."), -EINVAL);
    expect("system_x_eopnotsupp", rm("system.x"), -EOPNOTSUPP);
    expect("system_dot_eopnotsupp", rm("system."), -EOPNOTSUPP);
    expect("trusted_eperm", rm("trusted.a"), -EPERM);
    expect("trusted_dot_eperm", rm("trusted."), -EPERM);
    expect("security_eperm", rm("security.a"), -EPERM);
    expect("security_dot_eperm", rm("security."), -EPERM);
    expect("set_odd", set("user. /\x01\xff"), 0);
    expect("remove_odd", rm("user. /\x01\xff"), 0);
    expect("set_dotdot", set("user.."), 0);
    expect("remove_dotdot", rm("user.."), 0);
    expect("remove_dotdot_again", rm("user.."), -ENODATA);
    expect("cleanup_f", a_unlink(F), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
