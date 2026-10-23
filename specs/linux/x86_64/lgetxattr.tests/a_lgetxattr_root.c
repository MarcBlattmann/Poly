/* lgetxattr as root: with CAP_SYS_ADMIN, trusted.* attributes can be put
 * on a symbolic link itself (lsetxattr) and lgetxattr reads them back from
 * the link (size 0 gives the length, a short buffer ERANGE), while
 * getxattr through the link reads the target's own trusted.* attribute;
 * the link and the target hold separate values. A dangling link can hold
 * them too. Root also reads user.* from a 0000 file and through a
 * directory without search permission. After all capabilities are
 * dropped, the link's trusted.* attributes read as ENODATA and the 0000
 * file and the unsearchable directory are EACCES.
 * poly-diff: needs=root (trusted.* needs CAP_SYS_ADMIN, DAC override)
 */
#include "a_sys.h"

#define SYS_capget 125
#define SYS_capset 126
#define SYS_setxattr 188
#define SYS_lsetxattr 189
#define SYS_getxattr 191
#define SYS_lgetxattr 192
#define ENODATA 61
#define D "/tmp/a_lgetxattr_root"

static char buf[64];

static i64 drop_caps(void) {
    u32 h[2] = { 0x20080522, 0 }, d[6] = { 0 }, e[6];
    i64 r = syscall2(SYS_capset, h, d);
    if (r < 0) return r;
    r = syscall2(SYS_capget, h, e);
    if (r < 0) return r;
    return (e[0] | e[1] | e[3] | e[4]) ? -1 : 0;
}

static i64 lset(const char *p, const char *name, const char *v) {
    return syscall5(SYS_lsetxattr, p, name, v, str_len(v), 0);
}

static i64 get(const char *p, const char *name) { return syscall4(SYS_getxattr, p, name, buf, sizeof buf); }
static i64 lget(const char *p, const char *name) { return syscall4(SYS_lgetxattr, p, name, buf, sizeof buf); }

int test_main(void) {
    i64 uid = syscall0(SYS_geteuid);
    expect("is_root", uid, 0);
    if (uid != 0) return finish();
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_file", a_make_file(D "/f", "", 0644), 0);
    expect("setup_link", syscall2(SYS_symlink, "f", D "/l"), 0);
    expect("setup_dangling", syscall2(SYS_symlink, "nope", D "/dangling"), 0);
    expect("setup_none", a_make_file(D "/none", "", 0644), 0);
    expect("setup_sub", a_mkdir(D "/sub", 0755), 0);
    expect("setup_sub_file", a_make_file(D "/sub/g", "", 0644), 0);
    expect("setup_set_none", syscall5(SYS_setxattr, D "/none", "user.a", "n", 1, 0), 0);
    expect("setup_set_g", syscall5(SYS_setxattr, D "/sub/g", "user.a", "g", 1, 0), 0);
    expect("setup_none_mode", syscall3(SYS_fchmodat, AT_FDCWD, D "/none", 0), 0);
    expect("setup_sub_mode", syscall3(SYS_fchmodat, AT_FDCWD, D "/sub", 0), 0);
    expect("lset_link_trusted", lset(D "/l", "trusted.t", "on-link"), 0);
    expect("set_target_trusted", lset(D "/f", "trusted.t", "tgt"), 0);
    expect("lset_dangling_trusted", lset(D "/dangling", "trusted.t", "dd"), 0);
    expect("link_trusted_read", lget(D "/l", "trusted.t"), 7);
    check("link_trusted_value", mem_eq(buf, "on-link", 7), 0);
    expect("link_trusted_size0", syscall4(SYS_lgetxattr, D "/l", "trusted.t", 0, 0), 7);
    expect("link_trusted_erange", syscall4(SYS_lgetxattr, D "/l", "trusted.t", buf, 6), -ERANGE);
    expect("link_trusted_missing", lget(D "/l", "trusted.m"), -ENODATA);
    expect("target_trusted_read", lget(D "/f", "trusted.t"), 3);
    check("target_trusted_value", mem_eq(buf, "tgt", 3), 0);
    expect("getxattr_reads_target", get(D "/l", "trusted.t"), 3);
    expect("dangling_trusted_read", lget(D "/dangling", "trusted.t"), 2);
    expect("link_user_still_enodata", lget(D "/l", "user.a"), -ENODATA);
    expect("none_read", lget(D "/none", "user.a"), 1);
    expect("nosearch_read", lget(D "/sub/g", "user.a"), 1);
    i64 dropped = drop_caps();
    expect("drop_caps", dropped, 0);
    if (dropped == 0) {
        expect("dropped_link_trusted", lget(D "/l", "trusted.t"), -ENODATA);
        expect("dropped_link_trusted_size0", syscall4(SYS_lgetxattr, D "/l", "trusted.t", 0, 0), -ENODATA);
        expect("dropped_dangling_trusted", lget(D "/dangling", "trusted.t"), -ENODATA);
        expect("dropped_none_eacces", lget(D "/none", "user.a"), -EACCES);
        expect("dropped_nosearch_eacces", lget(D "/sub/g", "user.a"), -EACCES);
    }
    /* Removing entries needs write and search permission on D (root owns
     * it); the 0000 directory is opened up again first (chmod needs only
     * ownership). */
    syscall3(SYS_fchmodat, AT_FDCWD, D "/sub", 0755);
    a_unlink(D "/sub/g");
    a_rmdir(D "/sub");
    a_unlink(D "/l");
    a_unlink(D "/dangling");
    a_unlink(D "/none");
    a_unlink(D "/f");
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
