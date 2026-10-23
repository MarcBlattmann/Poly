/* lremovexattr does not follow a final symbolic link: it acts on the link
 * itself, which can hold no user.* attribute, so any user.* name (also
 * "user.") is EPERM on a link to a file, a link to a directory, a relative
 * link, a dangling link and a link to itself. The target's attribute of
 * the same name stays. removexattr through the same link does remove the
 * target's attribute; after that lremovexattr on the target itself is
 * ENODATA. */
#include "a_sys.h"

#define SYS_setxattr 188
#define SYS_getxattr 191
#define SYS_removexattr 197
#define SYS_lremovexattr 198
#define ENODATA 61
#define D "/tmp/a_lremovexattr_link"

static char buf[16];

static i64 lrm(const char *p, const char *name) { return syscall2(SYS_lremovexattr, p, name); }
static i64 set(const char *p, const char *name) { return syscall5(SYS_setxattr, p, name, "v", 1, 0); }
static i64 get(const char *p, const char *name) { return syscall4(SYS_getxattr, p, name, buf, sizeof buf); }
static i64 sym(const char *target, const char *link) { return syscall3(SYS_symlinkat, target, AT_FDCWD, link); }

int test_main(void) {
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_file", a_make_file(D "/t", "", 0644), 0);
    expect("setup_dir", a_mkdir(D "/dir", 0755), 0);
    expect("setup_lnk", sym(D "/t", D "/lnk"), 0);
    expect("setup_rel", sym("t", D "/rel"), 0);
    expect("setup_dlnk", sym(D "/dir", D "/dlnk"), 0);
    expect("setup_dang", sym(D "/nope", D "/dang"), 0);
    expect("setup_loop", sym(D "/loop", D "/loop"), 0);
    expect("set_t", set(D "/t", "user.a"), 0);
    expect("set_dir", set(D "/dir", "user.a"), 0);
    expect("link_eperm", lrm(D "/lnk", "user.a"), -EPERM);
    expect("link_missing_eperm", lrm(D "/lnk", "user.missing"), -EPERM);
    expect("link_user_dot_eperm", lrm(D "/lnk", "user."), -EPERM);
    expect("target_kept", get(D "/t", "user.a"), 1);
    expect("rel_link_eperm", lrm(D "/rel", "user.a"), -EPERM);
    expect("dir_link_eperm", lrm(D "/dlnk", "user.a"), -EPERM);
    expect("dir_kept", get(D "/dir", "user.a"), 1);
    expect("dangling_eperm", lrm(D "/dang", "user.a"), -EPERM);
    expect("loop_eperm", lrm(D "/loop", "user.a"), -EPERM);
    expect("removexattr_follows", syscall2(SYS_removexattr, D "/lnk", "user.a"), 0);
    expect("target_gone", get(D "/t", "user.a"), -ENODATA);
    expect("target_itself_enodata", lrm(D "/t", "user.a"), -ENODATA);
    expect("link_still_eperm", lrm(D "/lnk", "user.a"), -EPERM);
    expect("dir_itself_ok", lrm(D "/dir", "user.a"), 0);
    expect("dir_gone", get(D "/dir", "user.a"), -ENODATA);
    a_unlink(D "/lnk");
    a_unlink(D "/rel");
    a_unlink(D "/dlnk");
    a_unlink(D "/dang");
    a_unlink(D "/loop");
    expect("cleanup_t", a_unlink(D "/t"), 0);
    expect("cleanup_dir", a_rmdir(D "/dir"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
