/* Names on a symbolic link (not followed): user.* and "user." EPERM;
 * trusted.* and security.* EPERM without capabilities (this test drops all
 * of them first); an unknown prefix ("foo.a", "plain", "USER.a") and
 * unknown system.* names EOPNOTSUPP; a link to a read-only file is still
 * the link (mode 0777): EPERM for user.*, EOPNOTSUPP for an unknown prefix.
 * Name checks that come before the path (empty, 256 bytes: ERANGE;
 * unreadable: EFAULT) still apply. */
#include "a_sys.h"

#define SYS_capget 125
#define SYS_capset 126
#define SYS_lsetxattr 189
#define D "/tmp/a_lsetxattr_linknames"

static char longname[300];

static i64 drop_caps(void) {
    u32 h[2] = { 0x20080522, 0 }, d[6] = { 0 }, e[6];
    i64 r = syscall2(SYS_capset, h, d);
    if (r < 0) return r;
    r = syscall2(SYS_capget, h, e);
    if (r < 0) return r;
    return (e[0] | e[1] | e[3] | e[4]) ? -1 : 0;
}

static i64 lset(const char *p, const char *name) { return syscall5(SYS_lsetxattr, p, name, "v", 1, 0); }

int test_main(void) {
    expect("drop_caps", drop_caps(), 0);
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_file", a_make_file(D "/ro", "", 0444), 0);
    expect("setup_link", syscall3(SYS_symlinkat, "ro", AT_FDCWD, D "/l"), 0);
    expect("user_eperm", lset(D "/l", "user.a"), -EPERM);
    expect("user_dot_eperm", lset(D "/l", "user."), -EPERM);
    expect("trusted_eperm", lset(D "/l", "trusted.a"), -EPERM);
    expect("trusted_dot_eperm", lset(D "/l", "trusted."), -EPERM);
    expect("security_eperm", lset(D "/l", "security.a"), -EPERM);
    expect("security_dot_eperm", lset(D "/l", "security."), -EPERM);
    expect("unknown_prefix_eopnotsupp", lset(D "/l", "foo.a"), -EOPNOTSUPP);
    expect("no_prefix_eopnotsupp", lset(D "/l", "plain"), -EOPNOTSUPP);
    expect("upper_prefix_eopnotsupp", lset(D "/l", "USER.a"), -EOPNOTSUPP);
    expect("system_unknown_eopnotsupp", lset(D "/l", "system.x"), -EOPNOTSUPP);
    expect("file_itself_user_eacces", lset(D "/ro", "user.a"), -EACCES);
    expect("file_itself_unknown_eacces", lset(D "/ro", "foo.a"), -EACCES);
    expect("file_itself_security_eperm", lset(D "/ro", "security.a"), -EPERM);
    expect("empty_name_erange", lset(D "/l", ""), -ERANGE);
    for (int i = 0; i < 256; i++) longname[i] = 'x';
    longname[0] = 'u', longname[1] = 's', longname[2] = 'e', longname[3] = 'r', longname[4] = '.';
    longname[255] = 0;
    expect("name255_eperm", lset(D "/l", longname), -EPERM);
    longname[255] = 'x';
    longname[256] = 0;
    expect("name256_erange", lset(D "/l", longname), -ERANGE);
    expect("name_unreadable_efault", syscall5(SYS_lsetxattr, D "/l", 1, "v", 1, 0), -EFAULT);
    expect("cleanup_l", a_unlink(D "/l"), 0);
    expect("cleanup_ro", a_unlink(D "/ro"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
