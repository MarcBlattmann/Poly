/* removexattr error order, with all capabilities dropped first: the name
 * is checked before the path (EFAULT, ERANGE win over a missing, unmapped
 * or unsearchable path); the path walk (ENOENT, ENOTDIR, EFAULT) comes
 * before anything about the attribute namespace (an unknown prefix,
 * "user.", trusted.* on a missing path are ENOENT); unknown system.* names
 * (EOPNOTSUPP) and trusted.* (EPERM) come before the write-permission
 * check; user.* on a FIFO (EPERM) comes before it too; write permission
 * (EACCES) comes before EOPNOTSUPP for an unknown prefix, EINVAL for
 * "user." and ENODATA for a missing name. */
#include "a_sys.h"

#define SYS_capget 125
#define SYS_capset 126
#define SYS_removexattr 197
#define SYS_mknodat 259
#define D "/tmp/a_removexattr_order"
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

static i64 rm(const void *p, const void *name) { return syscall2(SYS_removexattr, p, name); }

int test_main(void) {
    expect("drop_caps", drop_caps(), 0);
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_ro", a_make_file(D "/ro", "", 0444), 0);
    expect("setup_rofifo", syscall4(SYS_mknodat, AT_FDCWD, D "/rofifo", 0010444, 0), 0);
    expect("setup_nox", a_mkdir(D "/nox", 0755), 0);
    expect("setup_nox_f", a_make_file(D "/nox/f", "", 0644), 0);
    expect("setup_nox_mode", syscall3(SYS_fchmodat, AT_FDCWD, D "/nox", 0600), 0);
    str_cpy(longname, "user.");
    for (int i = 5; i < 299; i++) longname[i] = 'l';
    expect("bad_name_before_missing_path", rm(NOPE, BAD), -EFAULT);
    expect("empty_name_before_missing_path", rm(NOPE, ""), -ERANGE);
    expect("empty_name_before_bad_path", rm(BAD, ""), -ERANGE);
    expect("long_name_before_missing_path", rm(NOPE, longname), -ERANGE);
    expect("long_name_before_empty_path", rm("", longname), -ERANGE);
    expect("empty_name_before_no_search", rm(D "/nox/f", ""), -ERANGE);
    expect("path_before_unknown_prefix", rm(NOPE, "foo.bar"), -ENOENT);
    expect("path_before_user_dot", rm(NOPE, "user."), -ENOENT);
    expect("path_before_trusted", rm(NOPE, "trusted.a"), -ENOENT);
    expect("path_before_system", rm(NOPE, "system.x"), -ENOENT);
    expect("enotdir_before_unknown_prefix", rm(D "/ro/x", "foo.bar"), -ENOTDIR);
    expect("bad_path_before_trusted", rm(BAD, "trusted.a"), -EFAULT);
    expect("no_search_before_trusted", rm(D "/nox/f", "trusted.a"), -EACCES);
    expect("system_before_eacces", rm(D "/ro", "system.x"), -EOPNOTSUPP);
    expect("trusted_before_eacces", rm(D "/ro", "trusted.a"), -EPERM);
    expect("fifo_user_before_eacces", rm(D "/rofifo", "user.a"), -EPERM);
    expect("eacces_before_unknown_prefix", rm(D "/ro", "foo.bar"), -EACCES);
    expect("eacces_before_user_dot", rm(D "/ro", "user."), -EACCES);
    expect("eacces_before_enodata", rm(D "/ro", "user.missing"), -EACCES);
    expect("cleanup_nox_mode", syscall3(SYS_fchmodat, AT_FDCWD, D "/nox", 0755), 0);
    expect("cleanup_nox_f", a_unlink(D "/nox/f"), 0);
    expect("cleanup_nox", a_rmdir(D "/nox"), 0);
    expect("cleanup_rofifo", a_unlink(D "/rofifo"), 0);
    expect("cleanup_ro", a_unlink(D "/ro"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
