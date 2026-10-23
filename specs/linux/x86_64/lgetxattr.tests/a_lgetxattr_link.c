/* lgetxattr does not follow a final symbolic link: it reads the link
 * itself, which has no attributes a caller without capabilities can see
 * (all capabilities are dropped first). On a live link (relative or
 * absolute target, to a file or a directory), a dangling link and a
 * looping link: user.* (also "user.") trusted.*, security.* names are
 * ENODATA; "security." is EINVAL; system.posix_acl_access,
 * system.posix_acl_default, other system.* names and unknown prefixes are
 * EOPNOTSUPP; an empty name ERANGE. getxattr on the same path reads the
 * target. With size 0 or a bad buffer the link's user.* is still ENODATA.
 * A second hard link to the symbolic link is the link too. */
#include "a_sys.h"

#define SYS_capget 125
#define SYS_capset 126
#define SYS_setxattr 188
#define SYS_getxattr 191
#define SYS_lgetxattr 192
#define ENODATA 61
#define D "/tmp/a_lgetxattr_link"
#define BAD ((void *)8)

static char buf[64];

static i64 drop_caps(void) {
    u32 h[2] = { 0x20080522, 0 }, d[6] = { 0 }, e[6];
    i64 r = syscall2(SYS_capset, h, d);
    if (r < 0) return r;
    r = syscall2(SYS_capget, h, e);
    if (r < 0) return r;
    return (e[0] | e[1] | e[3] | e[4]) ? -1 : 0;
}

static i64 get(const char *p, const char *name) { return syscall4(SYS_getxattr, p, name, buf, sizeof buf); }
static i64 lget(const char *p, const char *name) { return syscall4(SYS_lgetxattr, p, name, buf, sizeof buf); }

static void link_names(const char *tag, const char *p) {
    static const char *const names[] = { "user.a", "user.", "trusted.a", "security.a", "security.capability", 0 };
    static const char *const unsupported[] = { "system.posix_acl_access", "system.posix_acl_default", "system.x",
                                               "foo.a", "plain", 0 };
    char label[80];
    for (int i = 0; names[i]; i++) {
        str_cpy(label, tag);
        str_cpy(label + str_len(label), "_enodata_");
        str_cpy(label + str_len(label), names[i]);
        expect(label, lget(p, names[i]), -ENODATA);
    }
    for (int i = 0; unsupported[i]; i++) {
        str_cpy(label, tag);
        str_cpy(label + str_len(label), "_eopnotsupp_");
        str_cpy(label + str_len(label), unsupported[i]);
        expect(label, lget(p, unsupported[i]), -EOPNOTSUPP);
    }
    str_cpy(label, tag);
    str_cpy(label + str_len(label), "_security_dot_einval");
    expect(label, lget(p, "security."), -EINVAL);
    str_cpy(label, tag);
    str_cpy(label + str_len(label), "_empty_name_erange");
    expect(label, lget(p, ""), -ERANGE);
}

int test_main(void) {
    i64 dropped = drop_caps();
    expect("drop_caps", dropped, 0);
    if (dropped != 0) return finish();
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_file", a_make_file(D "/f", "", 0644), 0);
    expect("setup_dir", a_mkdir(D "/d", 0755), 0);
    expect("setup_set_f", syscall5(SYS_setxattr, D "/f", "user.a", "target", 6, 0), 0);
    expect("setup_set_d", syscall5(SYS_setxattr, D "/d", "user.a", "dir", 3, 0), 0);
    expect("setup_link", syscall2(SYS_symlink, "f", D "/l"), 0);
    expect("setup_abs_link", syscall2(SYS_symlink, D "/f", D "/abs"), 0);
    expect("setup_dir_link", syscall2(SYS_symlink, "d", D "/ld"), 0);
    expect("setup_dangling", syscall2(SYS_symlink, "/nonexistent/x", D "/dangling"), 0);
    expect("setup_loop", syscall2(SYS_symlink, "loop", D "/loop"), 0);
    expect("getxattr_follows", get(D "/l", "user.a"), 6);
    check("getxattr_target_value", mem_eq(buf, "target", 6), 0);
    expect("getxattr_dir_follows", get(D "/ld", "user.a"), 3);
    link_names("link", D "/l");
    link_names("abs", D "/abs");
    link_names("dirlink", D "/ld");
    link_names("dangling", D "/dangling");
    link_names("loop", D "/loop");
    expect("link_size0", syscall4(SYS_lgetxattr, D "/l", "user.a", 0, 0), -ENODATA);
    expect("link_size1", syscall4(SYS_lgetxattr, D "/l", "user.a", buf, 1), -ENODATA);
    expect("link_bad_buffer", syscall4(SYS_lgetxattr, D "/l", "user.a", BAD, 10), -ENODATA);
    expect("link_acl_bad_buffer", syscall4(SYS_lgetxattr, D "/l", "system.posix_acl_access", BAD, 10), -EOPNOTSUPP);
    expect("setup_hardlink", syscall5(SYS_linkat, AT_FDCWD, D "/l", AT_FDCWD, D "/l2", 0), 0);
    expect("hardlink_is_link", lget(D "/l2", "user.a"), -ENODATA);
    expect("hardlink_get_follows", get(D "/l2", "user.a"), 6);
    expect("target_still_reads", lget(D "/f", "user.a"), 6);
    expect("cleanup_l2", a_unlink(D "/l2"), 0);
    expect("cleanup_l", a_unlink(D "/l"), 0);
    expect("cleanup_abs", a_unlink(D "/abs"), 0);
    expect("cleanup_ld", a_unlink(D "/ld"), 0);
    expect("cleanup_dangling", a_unlink(D "/dangling"), 0);
    expect("cleanup_loop", a_unlink(D "/loop"), 0);
    expect("cleanup_d", a_rmdir(D "/d"), 0);
    expect("cleanup_f", a_unlink(D "/f"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
