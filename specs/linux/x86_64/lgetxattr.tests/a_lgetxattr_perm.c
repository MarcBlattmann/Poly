/* lgetxattr permission checks, with all capabilities dropped first (so
 * root is checked like anyone else): on files and directories user.*
 * names and unknown prefixes need read permission by the mode (0200,
 * 0000 and a 0044 file owned by the caller are EACCES; 0444 and 0400
 * work), and EACCES comes before ENODATA and before EINVAL for "user.".
 * trusted.*, security.* and system.* names are not checked against the
 * mode. A symbolic link to an unreadable file is the link: user.* is
 * ENODATA, unknown prefixes EOPNOTSUPP. A directory without search
 * permission on the way is EACCES, also for a final link and for
 * trusted.*; a directory without read permission is EACCES itself but
 * its entries are reachable. */
#include "a_sys.h"

#define SYS_capget 125
#define SYS_capset 126
#define SYS_setxattr 188
#define SYS_lgetxattr 192
#define ENODATA 61
#define D "/tmp/a_lgetxattr_perm"

static char buf[64];

static i64 drop_caps(void) {
    u32 h[2] = { 0x20080522, 0 }, d[6] = { 0 }, e[6];
    i64 r = syscall2(SYS_capset, h, d);
    if (r < 0) return r;
    r = syscall2(SYS_capget, h, e);
    if (r < 0) return r;
    return (e[0] | e[1] | e[3] | e[4]) ? -1 : 0;
}

static i64 lget(const char *p, const char *name) { return syscall4(SYS_lgetxattr, p, name, buf, sizeof buf); }

static i64 mode(const char *p, i64 m) { return syscall3(SYS_fchmodat, AT_FDCWD, p, m); }

int test_main(void) {
    i64 dropped = drop_caps();
    expect("drop_caps", dropped, 0);
    if (dropped != 0) return finish();
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_file", a_make_file(D "/f", "", 0644), 0);
    expect("setup_set", syscall5(SYS_setxattr, D "/f", "user.a", "v", 1, 0), 0);
    expect("setup_link", syscall2(SYS_symlink, "f", D "/l"), 0);
    expect("setup_sub", a_mkdir(D "/sub", 0755), 0);
    expect("setup_sub_file", a_make_file(D "/sub/g", "", 0644), 0);
    expect("setup_set_g", syscall5(SYS_setxattr, D "/sub/g", "user.g", "g", 1, 0), 0);
    expect("setup_sub_link", syscall2(SYS_symlink, "g", D "/sub/l"), 0);
    expect("setup_set_sub", syscall5(SYS_setxattr, D "/sub", "user.d", "dd", 2, 0), 0);
    expect("setup_ro", mode(D "/f", 0444), 0);
    expect("read_only_ok", lget(D "/f", "user.a"), 1);
    expect("setup_wo", mode(D "/f", 0200), 0);
    expect("write_only_eacces", lget(D "/f", "user.a"), -EACCES);
    expect("write_only_size0_eacces", syscall4(SYS_lgetxattr, D "/f", "user.a", 0, 0), -EACCES);
    expect("missing_name_eacces", lget(D "/f", "user.zz"), -EACCES);
    expect("user_dot_eacces", lget(D "/f", "user."), -EACCES);
    expect("unknown_prefix_eacces", lget(D "/f", "foo.a"), -EACCES);
    expect("trusted_enodata", lget(D "/f", "trusted.a"), -ENODATA);
    expect("security_enodata", lget(D "/f", "security.a"), -ENODATA);
    expect("security_dot_einval", lget(D "/f", "security."), -EINVAL);
    expect("acl_enodata", lget(D "/f", "system.posix_acl_access"), -ENODATA);
    expect("system_x_eopnotsupp", lget(D "/f", "system.x"), -EOPNOTSUPP);
    expect("link_to_wo_user", lget(D "/l", "user.a"), -ENODATA);
    expect("link_to_wo_unknown", lget(D "/l", "foo.a"), -EOPNOTSUPP);
    expect("setup_none", mode(D "/f", 0), 0);
    expect("mode_0000_eacces", lget(D "/f", "user.a"), -EACCES);
    expect("link_to_none_user", lget(D "/l", "user.a"), -ENODATA);
    expect("link_to_none_unknown", lget(D "/l", "foo.a"), -EOPNOTSUPP);
    expect("setup_owner_read", mode(D "/f", 0400), 0);
    expect("owner_read_ok", lget(D "/f", "user.a"), 1);
    expect("setup_group_other_read", mode(D "/f", 0044), 0);
    expect("owner_bits_decide_eacces", lget(D "/f", "user.a"), -EACCES);
    expect("setup_dir_noread", mode(D "/sub", 0311), 0);
    expect("dir_noread_eacces", lget(D "/sub", "user.d"), -EACCES);
    expect("dir_noread_slash_eacces", lget(D "/sub/", "user.d"), -EACCES);
    expect("dir_noread_unknown_eacces", lget(D "/sub", "foo.a"), -EACCES);
    expect("dir_noread_entry_ok", lget(D "/sub/g", "user.g"), 1);
    expect("dir_noread_link_enodata", lget(D "/sub/l", "user.g"), -ENODATA);
    expect("setup_dir_nosearch", mode(D "/sub", 0644), 0);
    expect("dir_nosearch_itself_ok", lget(D "/sub", "user.d"), 2);
    expect("nosearch_entry_eacces", lget(D "/sub/g", "user.g"), -EACCES);
    expect("nosearch_link_eacces", lget(D "/sub/l", "user.g"), -EACCES);
    expect("nosearch_trusted_eacces", lget(D "/sub/l", "trusted.a"), -EACCES);
    expect("nosearch_unknown_eacces", lget(D "/sub/l", "foo.a"), -EACCES);
    expect("nosearch_missing_eacces", lget(D "/sub/nope", "user.g"), -EACCES);
    expect("cleanup_sub_mode", mode(D "/sub", 0755), 0);
    expect("cleanup_sub_link", a_unlink(D "/sub/l"), 0);
    expect("cleanup_sub_file", a_unlink(D "/sub/g"), 0);
    expect("cleanup_sub", a_rmdir(D "/sub"), 0);
    expect("cleanup_l", a_unlink(D "/l"), 0);
    expect("cleanup_f", a_unlink(D "/f"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
