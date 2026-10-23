/* getxattr permission checks, with all capabilities dropped first (so root
 * is checked like anyone else): user.* names and unknown prefixes need
 * read permission on the file by its mode (a 0200 or 0000 file is EACCES,
 * a 0444 file works), also on a directory; EACCES comes before ENODATA,
 * before EINVAL for "user." and before EOPNOTSUPP. A directory without
 * search permission on the way is EACCES. trusted.*, security.* and
 * system.* names are not checked against the mode: on an unreadable file
 * they give what they give on a readable one (ENODATA, EINVAL for
 * "security.", EOPNOTSUPP for unknown system.* names). */
#include "a_sys.h"

#define SYS_capget 125
#define SYS_capset 126
#define SYS_setxattr 188
#define SYS_getxattr 191
#define ENODATA 61
#define D "/tmp/a_getxattr_perm"

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

static i64 mode(const char *p, i64 m) { return syscall3(SYS_fchmodat, AT_FDCWD, p, m); }

int test_main(void) {
    i64 dropped = drop_caps();
    expect("drop_caps", dropped, 0);
    if (dropped != 0) return finish();
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_file", a_make_file(D "/f", "", 0644), 0);
    expect("setup_set", syscall5(SYS_setxattr, D "/f", "user.a", "v", 1, 0), 0);
    expect("setup_sub", a_mkdir(D "/sub", 0755), 0);
    expect("setup_set_sub", syscall5(SYS_setxattr, D "/sub", "user.d", "dd", 2, 0), 0);
    expect("setup_ro", mode(D "/f", 0444), 0);
    expect("read_only_ok", get(D "/f", "user.a"), 1);
    expect("setup_wo", mode(D "/f", 0200), 0);
    expect("write_only_eacces", get(D "/f", "user.a"), -EACCES);
    expect("write_only_size0_eacces", syscall4(SYS_getxattr, D "/f", "user.a", 0, 0), -EACCES);
    expect("missing_name_eacces", get(D "/f", "user.zz"), -EACCES);
    expect("user_dot_eacces", get(D "/f", "user."), -EACCES);
    expect("unknown_prefix_eacces", get(D "/f", "foo.a"), -EACCES);
    expect("no_dot_eacces", get(D "/f", "user"), -EACCES);
    expect("trusted_enodata", get(D "/f", "trusted.a"), -ENODATA);
    expect("security_enodata", get(D "/f", "security.a"), -ENODATA);
    expect("security_dot_einval", get(D "/f", "security."), -EINVAL);
    expect("acl_enodata", get(D "/f", "system.posix_acl_access"), -ENODATA);
    expect("system_x_eopnotsupp", get(D "/f", "system.x"), -EOPNOTSUPP);
    expect("setup_none", mode(D "/f", 0), 0);
    expect("mode_0000_eacces", get(D "/f", "user.a"), -EACCES);
    expect("setup_owner_read", mode(D "/f", 0400), 0);
    expect("owner_read_ok", get(D "/f", "user.a"), 1);
    expect("setup_group_other_read", mode(D "/f", 0044), 0);
    expect("owner_bits_decide_eacces", get(D "/f", "user.a"), -EACCES);
    expect("setup_dir_noread", mode(D "/sub", 0300), 0);
    expect("dir_noread_eacces", get(D "/sub", "user.d"), -EACCES);
    expect("dir_noread_dot_eacces", get(D "/sub/.", "user.d"), -EACCES);
    expect("setup_dir_nosearch", mode(D "/sub", 0600), 0);
    expect("dir_nosearch_itself_ok", get(D "/sub", "user.d"), 2);
    expect("setup_inner", mode(D, 0600), 0);
    expect("no_search_on_way_eacces", get(D "/f", "user.a"), -EACCES);
    expect("no_search_trusted_eacces", get(D "/f", "trusted.a"), -EACCES);
    expect("cleanup_inner", mode(D, 0755), 0);
    expect("cleanup_sub", a_rmdir(D "/sub"), 0);
    expect("cleanup_f", a_unlink(D "/f"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
