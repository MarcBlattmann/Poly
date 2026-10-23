/* removexattr permission checks, with all capabilities dropped first (so
 * root is checked like anyone else): removing a user.* attribute needs
 * write permission on the file by its mode: a 0444 file is EACCES and
 * keeps the attribute, whether or not the name exists; a 0200 file works;
 * a 0555 directory is EACCES. EACCES also comes before EOPNOTSUPP for an
 * unknown prefix and EINVAL for "user."; unknown system.* names stay
 * EOPNOTSUPP and trusted.* and security.* EPERM on a read-only file. The
 * owner may remove attributes of its own sticky directory. */
#include "a_sys.h"

#define SYS_capget 125
#define SYS_capset 126
#define SYS_setxattr 188
#define SYS_getxattr 191
#define SYS_removexattr 197
#define ENODATA 61
#define D "/tmp/a_removexattr_perm"

static char buf[16];

static i64 drop_caps(void) {
    u32 h[2] = { 0x20080522, 0 }, d[6] = { 0 }, e[6];
    i64 r = syscall2(SYS_capset, h, d);
    if (r < 0) return r;
    r = syscall2(SYS_capget, h, e);
    if (r < 0) return r;
    return (e[0] | e[1] | e[3] | e[4]) ? -1 : 0;
}

static i64 rm(const char *p, const char *name) { return syscall2(SYS_removexattr, p, name); }
static i64 set(const char *p, const char *name) { return syscall5(SYS_setxattr, p, name, "v", 1, 0); }
static i64 mode(const char *p, i64 m) { return syscall3(SYS_fchmodat, AT_FDCWD, p, m); }

int test_main(void) {
    expect("drop_caps", drop_caps(), 0);
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_ro", a_make_file(D "/ro", "", 0644), 0);
    expect("setup_ro_set", set(D "/ro", "user.a"), 0);
    expect("setup_ro_mode", mode(D "/ro", 0444), 0);
    expect("setup_wo", a_make_file(D "/wo", "", 0644), 0);
    expect("setup_wo_set", set(D "/wo", "user.a"), 0);
    expect("setup_wo_mode", mode(D "/wo", 0200), 0);
    expect("setup_rodir", a_mkdir(D "/rodir", 0755), 0);
    expect("setup_rodir_set", set(D "/rodir", "user.a"), 0);
    expect("setup_rodir_mode", mode(D "/rodir", 0555), 0);
    expect("ro_existing_eacces", rm(D "/ro", "user.a"), -EACCES);
    expect("ro_attr_kept", syscall4(SYS_getxattr, D "/ro", "user.a", buf, sizeof buf), 1);
    expect("ro_missing_eacces", rm(D "/ro", "user.b"), -EACCES);
    expect("ro_unknown_prefix_eacces", rm(D "/ro", "foo.a"), -EACCES);
    expect("ro_user_dot_eacces", rm(D "/ro", "user."), -EACCES);
    expect("ro_no_dot_eacces", rm(D "/ro", "user"), -EACCES);
    expect("ro_system_eopnotsupp", rm(D "/ro", "system.x"), -EOPNOTSUPP);
    expect("ro_trusted_eperm", rm(D "/ro", "trusted.a"), -EPERM);
    expect("ro_security_eperm", rm(D "/ro", "security.a"), -EPERM);
    expect("wo_ok", rm(D "/wo", "user.a"), 0);
    expect("wo_missing_enodata", rm(D "/wo", "user.a"), -ENODATA);
    expect("rodir_eacces", rm(D "/rodir", "user.a"), -EACCES);
    expect("setup_ro_writable", mode(D "/ro", 0644), 0);
    expect("ro_now_writable_ok", rm(D "/ro", "user.a"), 0);
    expect("setup_sticky", mode(D "/rodir", 01777), 0);
    expect("own_sticky_ok", rm(D "/rodir", "user.a"), 0);
    a_rmdir(D "/rodir");
    a_unlink(D "/ro");
    a_unlink(D "/wo");
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
