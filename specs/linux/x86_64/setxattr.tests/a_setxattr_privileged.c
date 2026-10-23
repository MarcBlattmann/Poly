/* setxattr of trusted.* and security.* without CAP_SYS_ADMIN (all
 * capabilities dropped first, so the same for root): EPERM on the caller's
 * own writable file, for "trusted." and "security." too, and EPERM rather
 * than EACCES on a read-only file; through a descriptor as well. Path
 * lookup errors still come first. user.* keeps working. */
#include "a_sys.h"

#define SYS_capget 125
#define SYS_capset 126
#define SYS_setxattr 188
#define SYS_fsetxattr 190
#define SYS_getxattr 191
#define ENODATA 61
#define D "/tmp/a_setxattr_privileged"

static char buf[64];

static i64 drop_caps(void) {
    u32 h[2] = { 0x20080522, 0 }, d[6] = { 0 }, e[6];
    i64 r = syscall2(SYS_capset, h, d);
    if (r < 0) return r;
    r = syscall2(SYS_capget, h, e);
    if (r < 0) return r;
    return (e[0] | e[1] | e[3] | e[4]) ? -1 : 0;
}

static i64 set(const char *p, const char *name, i64 flags) {
    return syscall5(SYS_setxattr, p, name, "v", 1, flags);
}

int test_main(void) {
    i64 dropped = drop_caps();
    expect("drop_caps", dropped, 0);
    if (dropped != 0) return finish();
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_file", a_make_file(D "/f", "", 0644), 0);
    expect("setup_ro", a_make_file(D "/ro", "", 0444), 0);
    expect("trusted_eperm", set(D "/f", "trusted.a", 0), -EPERM);
    expect("trusted_create_eperm", set(D "/f", "trusted.a", 1), -EPERM);
    expect("trusted_replace_eperm", set(D "/f", "trusted.a", 2), -EPERM);
    expect("trusted_dot_eperm", set(D "/f", "trusted.", 0), -EPERM);
    expect("security_eperm", set(D "/f", "security.a", 0), -EPERM);
    expect("security_dot_eperm", set(D "/f", "security.", 0), -EPERM);
    expect("trusted_ro_eperm", set(D "/ro", "trusted.a", 0), -EPERM);
    expect("security_ro_eperm", set(D "/ro", "security.a", 0), -EPERM);
    expect("trusted_dir_eperm", set(D, "trusted.a", 0), -EPERM);
    i64 fd = a_open(D "/f", O_RDWR, 0);
    expect("fset_trusted_eperm", syscall5(SYS_fsetxattr, fd, "trusted.a", "v", 1, 0), -EPERM);
    expect("fset_security_eperm", syscall5(SYS_fsetxattr, fd, "security.a", "v", 1, 0), -EPERM);
    a_close(fd);
    expect("trusted_missing_enoent", set(D "/nope", "trusted.a", 0), -ENOENT);
    expect("trusted_enotdir", set(D "/f/x", "trusted.a", 0), -ENOTDIR);
    expect("user_still_ok", set(D "/f", "user.a", 0), 0);
    expect("user_value", syscall4(SYS_getxattr, D "/f", "user.a", buf, sizeof buf), 1);
    expect("cleanup_f", a_unlink(D "/f"), 0);
    expect("cleanup_ro", a_unlink(D "/ro"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
