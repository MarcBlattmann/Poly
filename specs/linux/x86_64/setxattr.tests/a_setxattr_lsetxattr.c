/* lsetxattr (189): does not follow a final symbolic link, and a link can
 * hold no user.* attribute (EPERM, also for a dangling or looping link and
 * for "user."); an unknown namespace on a link is EOPNOTSUPP. Links in
 * earlier components and a trailing slash are followed. On files and
 * directories it is setxattr. Runs with no capabilities, so trusted.* on a
 * link is EPERM too. */
#include "a_sys.h"

#define SYS_capget 125
#define SYS_capset 126
#define SYS_lsetxattr 189
#define SYS_getxattr 191
#define SYS_lgetxattr 192
#define ENODATA 61
#define D "/tmp/a_setxattr_lsetxattr"

static char buf[64];

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
    expect("setup_sub", a_mkdir(D "/sub", 0755), 0);
    expect("setup_file", a_make_file(D "/f", "", 0644), 0);
    expect("setup_file2", a_make_file(D "/sub/g", "", 0644), 0);
    expect("setup_link", syscall3(SYS_symlinkat, "f", AT_FDCWD, D "/lf"), 0);
    expect("setup_dangling", syscall3(SYS_symlinkat, "nope", AT_FDCWD, D "/dl"), 0);
    expect("setup_loop", syscall3(SYS_symlinkat, "loop", AT_FDCWD, D "/loop"), 0);
    expect("setup_dirlink", syscall3(SYS_symlinkat, "sub", AT_FDCWD, D "/ls"), 0);
    expect("link_user_eperm", lset(D "/lf", "user.a"), -EPERM);
    expect("target_untouched", syscall4(SYS_getxattr, D "/f", "user.a", buf, sizeof buf), -ENODATA);
    expect("link_user_dot_eperm", lset(D "/lf", "user."), -EPERM);
    expect("link_unknown_ns_eopnotsupp", lset(D "/lf", "foo.a"), -EOPNOTSUPP);
    expect("link_trusted_eperm", lset(D "/lf", "trusted.a"), -EPERM);
    expect("dangling_eperm", lset(D "/dl", "user.a"), -EPERM);
    expect("loop_eperm", lset(D "/loop", "user.a"), -EPERM);
    expect("missing_enoent", lset(D "/nope", "user.a"), -ENOENT);
    expect("file", lset(D "/f", "user.f"), 0);
    expect("file_value", syscall4(SYS_getxattr, D "/f", "user.f", buf, sizeof buf), 1);
    expect("dir", lset(D "/sub", "user.d"), 0);
    expect("dir_value", syscall4(SYS_getxattr, D "/sub", "user.d", buf, sizeof buf), 1);
    expect("through_dir_link", lset(D "/ls/g", "user.g"), 0);
    expect("through_dir_link_value", syscall4(SYS_getxattr, D "/sub/g", "user.g", buf, sizeof buf), 1);
    expect("trailing_slash_follows", lset(D "/ls/", "user.s"), 0);
    expect("trailing_slash_value", syscall4(SYS_getxattr, D "/sub", "user.s", buf, sizeof buf), 1);
    expect("dir_link_itself_eperm", lset(D "/ls", "user.s2"), -EPERM);
    expect("file_link_slash_enotdir", lset(D "/lf/", "user.s"), -ENOTDIR);
    expect("link_still_bare", syscall4(SYS_lgetxattr, D "/lf", "user.a", buf, sizeof buf), -ENODATA);
    a_unlink(D "/lf");
    a_unlink(D "/dl");
    a_unlink(D "/loop");
    a_unlink(D "/ls");
    expect("cleanup_g", a_unlink(D "/sub/g"), 0);
    expect("cleanup_f", a_unlink(D "/f"), 0);
    expect("cleanup_sub", a_rmdir(D "/sub"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
