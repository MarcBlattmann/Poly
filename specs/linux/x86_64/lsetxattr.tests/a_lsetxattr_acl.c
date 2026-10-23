/* system.* names through lsetxattr. The path is looked up first (a
 * missing file is ENOENT whatever the value), then the ACL value is
 * checked: a length that is not 4 + 8 * n bytes is EINVAL, a version other
 * than 2 EOPNOTSUPP, on links and files alike (also on a read-only file).
 * A well-formed access or default ACL on a symbolic link is EOPNOTSUPP (a
 * link holds no ACL), so are an empty and a header-only value; an unknown
 * system.* name is EOPNOTSUPP. On a regular file a default ACL is EACCES.
 * ACL layout: version 2 (u32), then
 * entries of tag (u16), perm (u16), id (u32); tags USER_OBJ 1, GROUP_OBJ
 * 4, OTHER 0x20 (linux/posix_acl_xattr.h, linux/posix_acl.h). */
#include "a_sys.h"

#define SYS_lsetxattr 189
#define D "/tmp/a_lsetxattr_acl"

static u8 acl[28];

static void put_entry(int at, int tag, int perm) {
    acl[at] = (u8)tag, acl[at + 1] = (u8)(tag >> 8);
    acl[at + 2] = (u8)perm, acl[at + 3] = 0;
    acl[at + 4] = acl[at + 5] = acl[at + 6] = acl[at + 7] = 0xff;
}

static i64 lset(const char *p, const char *n, const void *v, i64 size) {
    return syscall5(SYS_lsetxattr, p, n, v, size, 0);
}

int test_main(void) {
    acl[0] = 2;
    put_entry(4, 1, 6);
    put_entry(12, 4, 4);
    put_entry(20, 0x20, 4);
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_file", a_make_file(D "/f", "", 0644), 0);
    expect("setup_ro", a_make_file(D "/ro", "", 0444), 0);
    expect("setup_link", syscall3(SYS_symlinkat, "f", AT_FDCWD, D "/l"), 0);
    expect("link_access_eopnotsupp", lset(D "/l", "system.posix_acl_access", acl, sizeof acl), -EOPNOTSUPP);
    expect("link_default_eopnotsupp", lset(D "/l", "system.posix_acl_default", acl, sizeof acl), -EOPNOTSUPP);
    expect("link_bad_acl_einval", lset(D "/l", "system.posix_acl_access", "v", 1), -EINVAL);
    expect("link_six_bytes_einval", lset(D "/l", "system.posix_acl_access", "\2\0\0\0\1\0", 6), -EINVAL);
    expect("link_bad_version_eopnotsupp", lset(D "/l", "system.posix_acl_access", "\1\0\0\0", 4), -EOPNOTSUPP);
    expect("link_header_only_eopnotsupp", lset(D "/l", "system.posix_acl_access", "\2\0\0\0", 4), -EOPNOTSUPP);
    expect("link_empty_eopnotsupp", lset(D "/l", "system.posix_acl_access", 0, 0), -EOPNOTSUPP);
    expect("link_unknown_system_eopnotsupp", lset(D "/l", "system.x", "v", 1), -EOPNOTSUPP);
    expect("missing_bad_acl_enoent", lset(D "/nope", "system.posix_acl_access", "v", 1), -ENOENT);
    expect("missing_bad_flags_einval", syscall5(SYS_lsetxattr, D "/nope", "system.posix_acl_access", "v", 1, 4), -EINVAL);
    expect("missing_good_acl_enoent", lset(D "/nope", "system.posix_acl_access", acl, sizeof acl), -ENOENT);
    expect("file_default_eacces", lset(D "/f", "system.posix_acl_default", acl, sizeof acl), -EACCES);
    expect("file_bad_acl_einval", lset(D "/f", "system.posix_acl_access", "v", 1), -EINVAL);
    expect("file_bad_version_eopnotsupp", lset(D "/f", "system.posix_acl_access", "\1\0\0\0", 4), -EOPNOTSUPP);
    expect("ro_bad_acl_einval", lset(D "/ro", "system.posix_acl_access", "v", 1), -EINVAL);
    expect("ro_unknown_system_eopnotsupp", lset(D "/ro", "system.x", "v", 1), -EOPNOTSUPP);
    expect("cleanup_l", a_unlink(D "/l"), 0);
    expect("cleanup_f", a_unlink(D "/f"), 0);
    expect("cleanup_ro", a_unlink(D "/ro"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
