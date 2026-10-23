/* lremovexattr as root: with CAP_SYS_ADMIN a symbolic link can hold
 * trusted attributes (lsetxattr); lremovexattr removes the link's own
 * one (0, then ENODATA, lgetxattr on the link ENODATA) and leaves the
 * target's attribute of the same name alone, while removexattr through
 * the link removes the target's. A dangling link's trusted attribute can
 * be removed too; "trusted." on a link is EINVAL. user.* on a link is
 * EPERM for root as well. After all capabilities are dropped, trusted
 * names on the link are EPERM and the attribute stays.
 * poly-diff: needs=root (trusted attributes need CAP_SYS_ADMIN)
 */
#include "a_sys.h"

#define SYS_capget 125
#define SYS_capset 126
#define SYS_lsetxattr 189
#define SYS_lgetxattr 192
#define SYS_removexattr 197
#define SYS_lremovexattr 198
#define ENODATA 61
#define D "/tmp/a_lremovexattr_root"

static char buf[32];

static i64 drop_caps(void) {
    u32 h[2] = { 0x20080522, 0 }, d[6] = { 0 }, e[6];
    i64 r = syscall2(SYS_capset, h, d);
    if (r < 0) return r;
    r = syscall2(SYS_capget, h, e);
    if (r < 0) return r;
    return (e[0] | e[1] | e[3] | e[4]) ? -1 : 0;
}

static i64 lrm(const char *p, const char *name) { return syscall2(SYS_lremovexattr, p, name); }
static i64 lset(const char *p, const char *name, const char *v) {
    return syscall5(SYS_lsetxattr, p, name, v, str_len(v), 0);
}
static i64 lget(const char *p, const char *name) { return syscall4(SYS_lgetxattr, p, name, buf, sizeof buf); }

int test_main(void) {
    i64 uid = syscall0(SYS_geteuid);
    expect("is_root", uid, 0);
    if (uid != 0) return finish();
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_file", a_make_file(D "/f", "", 0644), 0);
    expect("setup_link", syscall3(SYS_symlinkat, "f", AT_FDCWD, D "/l"), 0);
    expect("setup_dangling", syscall3(SYS_symlinkat, "nope", AT_FDCWD, D "/dang"), 0);
    expect("lset_link", lset(D "/l", "trusted.t", "link"), 0);
    expect("lset_target", lset(D "/f", "trusted.t", "file"), 0);
    expect("lset_dangling", lset(D "/dang", "trusted.t", "dd"), 0);
    expect("lset_link_keep", lset(D "/l", "trusted.k", "keep"), 0);
    expect("remove_link_trusted", lrm(D "/l", "trusted.t"), 0);
    expect("link_trusted_gone", lget(D "/l", "trusted.t"), -ENODATA);
    expect("remove_link_trusted_again", lrm(D "/l", "trusted.t"), -ENODATA);
    expect("target_kept", lget(D "/f", "trusted.t"), 4);
    check("target_value", mem_eq(buf, "file", 4), 0);
    expect("link_other_kept", lget(D "/l", "trusted.k"), 4);
    expect("link_trusted_dot_einval", lrm(D "/l", "trusted."), -EINVAL);
    expect("link_missing_enodata", lrm(D "/l", "trusted.missing"), -ENODATA);
    expect("link_user_eperm", lrm(D "/l", "user.a"), -EPERM);
    expect("link_acl_eopnotsupp", lrm(D "/l", "system.posix_acl_access"), -EOPNOTSUPP);
    expect("remove_dangling_trusted", lrm(D "/dang", "trusted.t"), 0);
    expect("dangling_gone", lget(D "/dang", "trusted.t"), -ENODATA);
    expect("removexattr_via_link_target", syscall2(SYS_removexattr, D "/l", "trusted.t"), 0);
    expect("target_gone", lget(D "/f", "trusted.t"), -ENODATA);
    i64 dropped = drop_caps();
    expect("drop_caps", dropped, 0);
    if (dropped == 0) {
        expect("dropped_link_trusted_eperm", lrm(D "/l", "trusted.k"), -EPERM);
        expect("dropped_link_trusted_missing_eperm", lrm(D "/l", "trusted.t"), -EPERM);
    }
    a_unlink(D "/l");
    a_unlink(D "/dang");
    a_unlink(D "/f");
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
