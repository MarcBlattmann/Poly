/* Permission checks of lsetxattr on what the path names (no capabilities,
 * so root sees the same): user.* needs write permission on the file by its
 * mode (EACCES otherwise: a 0444 file, a 0555 directory; a write-only
 * 0200 file is enough); unknown prefixes also need write permission
 * first (EACCES), then EOPNOTSUPP. trusted.* and security.* without
 * CAP_SYS_ADMIN are EPERM, even on a read-only file. The owner may set
 * user.* on its own sticky directory. A link pointing at a read-only
 * file is the link, not the file (EPERM, EOPNOTSUPP). */
#include "a_sys.h"

#define SYS_capget 125
#define SYS_capset 126
#define SYS_lsetxattr 189
#define SYS_getxattr 191
#define ENODATA 61
#define D "/tmp/a_lsetxattr_perm"

static char buf[16];

static i64 drop_caps(void) {
    u32 h[2] = { 0x20080522, 0 }, d[6] = { 0 }, e[6];
    i64 r = syscall2(SYS_capset, h, d);
    if (r < 0) return r;
    r = syscall2(SYS_capget, h, e);
    if (r < 0) return r;
    return (e[0] | e[1] | e[3] | e[4]) ? -1 : 0;
}

static i64 lset(const char *p, const char *n) { return syscall5(SYS_lsetxattr, p, n, "v", 1, 0); }

int test_main(void) {
    expect("drop_caps", drop_caps(), 0);
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_ro", a_make_file(D "/ro", "", 0444), 0);
    expect("setup_wo", a_make_file(D "/wo", "", 0200), 0);
    expect("setup_none", a_make_file(D "/none", "", 0644), 0);
    expect("setup_none_mode", syscall3(SYS_fchmodat, AT_FDCWD, D "/none", 0), 0);
    expect("setup_rodir", a_mkdir(D "/rodir", 0555), 0);
    expect("setup_sticky", a_mkdir(D "/sticky", 0755), 0);
    expect("setup_sticky_mode", syscall3(SYS_fchmodat, AT_FDCWD, D "/sticky", 01777), 0);
    expect("setup_link", syscall3(SYS_symlinkat, "ro", AT_FDCWD, D "/lro"), 0);
    expect("ro_eacces", lset(D "/ro", "user.a"), -EACCES);
    expect("ro_untouched", syscall4(SYS_getxattr, D "/ro", "user.a", buf, sizeof buf), -ENODATA);
    expect("ro_unknown_eacces", lset(D "/ro", "foo.a"), -EACCES);
    expect("ro_trusted_eperm", lset(D "/ro", "trusted.a"), -EPERM);
    expect("ro_security_eperm", lset(D "/ro", "security.a"), -EPERM);
    expect("none_eacces", lset(D "/none", "user.a"), -EACCES);
    expect("write_only_ok", lset(D "/wo", "user.a"), 0);
    expect("rodir_eacces", lset(D "/rodir", "user.a"), -EACCES);
    expect("own_sticky_ok", lset(D "/sticky", "user.a"), 0);
    expect("link_to_ro_eperm", lset(D "/lro", "user.a"), -EPERM);
    expect("link_to_ro_unknown_eopnotsupp", lset(D "/lro", "foo.a"), -EOPNOTSUPP);
    expect("link_slash_to_ro_enotdir", lset(D "/lro/", "user.a"), -ENOTDIR);
    expect("chmod_ro_writable", syscall3(SYS_fchmodat, AT_FDCWD, D "/ro", 0644), 0);
    expect("now_writable_ok", lset(D "/ro", "user.a"), 0);
    expect("chmod_rodir", syscall3(SYS_fchmodat, AT_FDCWD, D "/rodir", 0755), 0);
    expect("cleanup_rodir", a_rmdir(D "/rodir"), 0);
    expect("cleanup_sticky", a_rmdir(D "/sticky"), 0);
    expect("cleanup_lro", a_unlink(D "/lro"), 0);
    expect("cleanup_ro", a_unlink(D "/ro"), 0);
    expect("cleanup_wo", a_unlink(D "/wo"), 0);
    expect("cleanup_none", a_unlink(D "/none"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
