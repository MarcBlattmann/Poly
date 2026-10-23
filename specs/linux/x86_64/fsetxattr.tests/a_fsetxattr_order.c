/* Which error wins when several apply. The arguments are checked before
 * the descriptor: bad flags EINVAL, then the name (EFAULT, ERANGE), then
 * size E2BIG, then an unreadable value EFAULT, and only then EBADF (also
 * for an O_PATH descriptor). After EBADF come system.* checks (a bad ACL
 * value on a bad descriptor is EBADF, on a read-only file EINVAL), then
 * trusted.* EPERM, then write permission EACCES (user.* and unknown
 * prefixes, "user." too), then unknown prefix EOPNOTSUPP and "user."
 * EINVAL, and EEXIST / ENODATA last. (Capabilities are dropped first.) */
#include "a_sys.h"

#define SYS_capget 125
#define SYS_capset 126
#define SYS_fsetxattr 190
#define ENODATA 61
#define D "/tmp/a_fsetxattr_order"

static i64 drop_caps(void) {
    u32 h[2] = { 0x20080522, 0 }, d[6] = { 0 }, e[6];
    i64 r = syscall2(SYS_capset, h, d);
    if (r < 0) return r;
    r = syscall2(SYS_capget, h, e);
    if (r < 0) return r;
    return (e[0] | e[1] | e[3] | e[4]) ? -1 : 0;
}

static i64 fset(i64 fd, const void *n, const void *v, i64 size, i64 flags) {
    return syscall5(SYS_fsetxattr, fd, n, v, size, flags);
}

int test_main(void) {
    expect("drop_caps", drop_caps(), 0);
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_file", a_make_file(D "/f", "", 0644), 0);
    expect("setup_ro", a_make_file(D "/ro", "", 0444), 0);
    expect("badfd_badflags", fset(-1, "user.a", "v", 1, 4), -EINVAL);
    expect("badfd_badname", fset(-1, (void *)1, "v", 1, 0), -EFAULT);
    expect("badfd_emptyname", fset(-1, "", "v", 1, 0), -ERANGE);
    expect("badfd_e2big", fset(-1, "user.a", "v", 65537, 0), -E2BIG);
    expect("badfd_badvalue", fset(-1, "user.a", 0, 1, 0), -EFAULT);
    expect("badfd_foo", fset(-1, "foo.a", "v", 1, 0), -EBADF);
    expect("badfd_userdot", fset(-1, "user.", "v", 1, 0), -EBADF);
    expect("badfd_trusted", fset(-1, "trusted.a", "v", 1, 0), -EBADF);
    expect("badfd_bad_acl", fset(-1, "system.posix_acl_access", "v", 1, 0), -EBADF);
    expect("badfd_replace", fset(-1, "user.a", "v", 1, 2), -EBADF);
    expect("badflags_emptyname", fset(-1, "", "v", 1, 4), -EINVAL);
    expect("emptyname_e2big", fset(-1, "", "v", 65537, 0), -ERANGE);
    expect("e2big_badvalue", fset(-1, "user.a", 0, 65537, 0), -E2BIG);
    i64 op = a_open(D "/f", O_PATH, 0);
    check("setup_opath", op >= 0, op);
    expect("opath_badflags", fset(op, "user.a", "v", 1, 4), -EINVAL);
    expect("opath_emptyname", fset(op, "", "v", 1, 0), -ERANGE);
    expect("opath_badvalue", fset(op, "user.a", 0, 1, 0), -EFAULT);
    expect("opath_foo", fset(op, "foo.a", "v", 1, 0), -EBADF);
    a_close(op);
    i64 ro = a_open(D "/ro", O_RDONLY, 0);
    check("setup_open_ro", ro >= 0, ro);
    expect("ro_badflags", fset(ro, "user.a", "v", 1, 4), -EINVAL);
    expect("ro_e2big", fset(ro, "user.a", "v", 65537, 0), -E2BIG);
    expect("ro_bad_acl", fset(ro, "system.posix_acl_access", "v", 1, 0), -EINVAL);
    expect("ro_system_x", fset(ro, "system.x", "v", 1, 0), -EOPNOTSUPP);
    expect("ro_trusted", fset(ro, "trusted.a", "v", 1, 0), -EPERM);
    expect("ro_security", fset(ro, "security.a", "v", 1, 0), -EPERM);
    expect("ro_user", fset(ro, "user.a", "v", 1, 0), -EACCES);
    expect("ro_userdot", fset(ro, "user.", "v", 1, 0), -EACCES);
    expect("ro_foo", fset(ro, "foo.a", "v", 1, 0), -EACCES);
    expect("ro_replace_missing", fset(ro, "user.m", "v", 1, 2), -EACCES);
    a_close(ro);
    i64 fd = a_open(D "/f", O_RDWR, 0);
    check("setup_open_f", fd >= 0, fd);
    expect("rw_foo_create", fset(fd, "foo.a", "v", 1, 1), -EOPNOTSUPP);
    expect("rw_userdot_replace", fset(fd, "user.", "v", 1, 2), -EINVAL);
    expect("rw_trusted_replace", fset(fd, "trusted.m", "v", 1, 2), -EPERM);
    expect("rw_replace_missing", fset(fd, "user.m", "v", 1, 2), -ENODATA);
    a_close(fd);
    a_unlink(D "/f");
    a_unlink(D "/ro");
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
