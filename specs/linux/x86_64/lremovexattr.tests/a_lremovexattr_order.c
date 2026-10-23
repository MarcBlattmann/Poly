/* lremovexattr error order, with all capabilities dropped first: the name
 * is checked before the path (EFAULT and ERANGE win over a missing,
 * unmapped or unsearchable path); the path walk (ENOENT, ENOTDIR, EFAULT,
 * EACCES for search permission) comes before anything about the namespace
 * (unknown prefix, "user.", trusted.*, system.x on a missing path are
 * ENOENT); on a symbolic link, an empty or too long name is ERANGE while
 * "user." is EPERM; on a read-only file unknown system.* EOPNOTSUPP and
 * trusted.* EPERM come before EACCES, and EACCES before EOPNOTSUPP for an
 * unknown prefix, EINVAL for "user." and ENODATA. */
#include "a_sys.h"

#define SYS_capget 125
#define SYS_capset 126
#define SYS_lremovexattr 198
#define D "/tmp/a_lremovexattr_order"
#define NOPE D "/nope"
#define BAD ((const void *)8)

static char longname[300];

static i64 drop_caps(void) {
    u32 h[2] = { 0x20080522, 0 }, d[6] = { 0 }, e[6];
    i64 r = syscall2(SYS_capset, h, d);
    if (r < 0) return r;
    r = syscall2(SYS_capget, h, e);
    if (r < 0) return r;
    return (e[0] | e[1] | e[3] | e[4]) ? -1 : 0;
}

static i64 lrm(const void *p, const void *name) { return syscall2(SYS_lremovexattr, p, name); }

int test_main(void) {
    expect("drop_caps", drop_caps(), 0);
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_ro", a_make_file(D "/ro", "", 0444), 0);
    expect("setup_link", syscall3(SYS_symlinkat, "ro", AT_FDCWD, D "/l"), 0);
    expect("setup_nox", a_mkdir(D "/nox", 0755), 0);
    expect("setup_nox_l", syscall3(SYS_symlinkat, "x", AT_FDCWD, D "/nox/l"), 0);
    expect("setup_nox_mode", syscall3(SYS_fchmodat, AT_FDCWD, D "/nox", 0600), 0);
    str_cpy(longname, "user.");
    for (int i = 5; i < 299; i++) longname[i] = 'l';
    expect("bad_name_before_missing_path", lrm(NOPE, BAD), -EFAULT);
    expect("bad_name_before_bad_path", lrm(BAD, BAD), -EFAULT);
    expect("empty_name_before_missing_path", lrm(NOPE, ""), -ERANGE);
    expect("empty_name_before_bad_path", lrm(BAD, ""), -ERANGE);
    expect("long_name_before_missing_path", lrm(NOPE, longname), -ERANGE);
    expect("long_name_before_empty_path", lrm("", longname), -ERANGE);
    expect("long_name_before_no_search", lrm(D "/nox/l", longname), -ERANGE);
    expect("no_search_user", lrm(D "/nox/l", "user.a"), -EACCES);
    expect("no_search_unknown", lrm(D "/nox/l", "foo.bar"), -EACCES);
    expect("no_search_trusted", lrm(D "/nox/l", "trusted.a"), -EACCES);
    expect("path_before_unknown_prefix", lrm(NOPE, "foo.bar"), -ENOENT);
    expect("path_before_user_dot", lrm(NOPE, "user."), -ENOENT);
    expect("path_before_trusted", lrm(NOPE, "trusted.a"), -ENOENT);
    expect("path_before_system", lrm(NOPE, "system.x"), -ENOENT);
    expect("path_before_acl", lrm(NOPE, "system.posix_acl_access"), -ENOENT);
    expect("enotdir_before_unknown_prefix", lrm(D "/ro/x", "foo.bar"), -ENOTDIR);
    expect("bad_path_before_trusted", lrm(BAD, "trusted.a"), -EFAULT);
    expect("link_empty_name_erange", lrm(D "/l", ""), -ERANGE);
    expect("link_long_name_erange", lrm(D "/l", longname), -ERANGE);
    expect("link_user_dot_eperm", lrm(D "/l", "user."), -EPERM);
    expect("system_before_eacces", lrm(D "/ro", "system.x"), -EOPNOTSUPP);
    expect("trusted_before_eacces", lrm(D "/ro", "trusted.a"), -EPERM);
    expect("security_before_eacces", lrm(D "/ro", "security.a"), -EPERM);
    expect("eacces_before_unknown_prefix", lrm(D "/ro", "foo.bar"), -EACCES);
    expect("eacces_before_user_dot", lrm(D "/ro", "user."), -EACCES);
    expect("eacces_before_enodata", lrm(D "/ro", "user.missing"), -EACCES);
    expect("cleanup_nox_mode", syscall3(SYS_fchmodat, AT_FDCWD, D "/nox", 0755), 0);
    expect("cleanup_nox_l", a_unlink(D "/nox/l"), 0);
    expect("cleanup_nox", a_rmdir(D "/nox"), 0);
    expect("cleanup_link", a_unlink(D "/l"), 0);
    expect("cleanup_ro", a_unlink(D "/ro"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
