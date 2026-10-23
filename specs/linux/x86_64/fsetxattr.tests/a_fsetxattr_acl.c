/* POSIX ACLs through fsetxattr. Value checks: a length that is not
 * 4 + 8 * n bytes is EINVAL, a version other than 2 is EOPNOTSUPP, even on
 * a read-only file. A default ACL on a regular file is EACCES. The owner
 * may set an access ACL on its own 0444 file (no write permission needed);
 * an ACL with only the three base entries (user, group, other) sets the
 * mode's permission bits (rwx r-x --- gives 0750) and is not kept as an
 * attribute (reading it back is ENODATA). An empty or header-only access
 * ACL is accepted. A default ACL on a directory is kept and reads back
 * with the same 28 bytes. XATTR_CREATE / XATTR_REPLACE have no effect on
 * ACL names. ACL layout (linux/posix_acl_xattr.h): u32 version 2, then
 * entries of u16 tag, u16 perm, u32 id; tags USER_OBJ 1, GROUP_OBJ 4,
 * OTHER 0x20 (linux/posix_acl.h). */
#include "a_sys.h"

#define SYS_fsetxattr 190
#define SYS_fgetxattr 193
#define ENODATA 61
#define D "/tmp/a_fsetxattr_acl"

static u8 acl[28];
static u8 buf[64];

static void put_entry(int at, int tag, int perm) {
    acl[at] = (u8)tag, acl[at + 1] = (u8)(tag >> 8);
    acl[at + 2] = (u8)perm, acl[at + 3] = 0;
    acl[at + 4] = acl[at + 5] = acl[at + 6] = acl[at + 7] = 0xff;
}

static i64 fset(i64 fd, const char *n, const void *v, i64 size, i64 flags) {
    return syscall5(SYS_fsetxattr, fd, n, v, size, flags);
}

static i64 mode_of(i64 fd) {
    struct a_stat st;
    i64 r = syscall2(SYS_fstat, fd, &st);
    return r < 0 ? r : (i64)(st.st_mode & 07777);
}

int test_main(void) {
    acl[0] = 2;
    put_entry(4, 1, 7);
    put_entry(12, 4, 5);
    put_entry(20, 0x20, 0);
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_sub", a_mkdir(D "/sub", 0755), 0);
    expect("setup_f", a_make_file(D "/f", "", 0644), 0);
    expect("setup_ro", a_make_file(D "/ro", "", 0444), 0);
    i64 fd = a_open(D "/f", O_RDWR, 0);
    check("setup_open_f", fd >= 0, fd);
    expect("len_1_einval", fset(fd, "system.posix_acl_access", "v", 1, 0), -EINVAL);
    expect("len_6_einval", fset(fd, "system.posix_acl_access", "\2\0\0\0\1\0", 6, 0), -EINVAL);
    expect("version_1", fset(fd, "system.posix_acl_access", "\1\0\0\0", 4, 0), -EOPNOTSUPP);
    expect("default_on_file", fset(fd, "system.posix_acl_default", acl, 28, 0), -EACCES);
    expect("header_only", fset(fd, "system.posix_acl_access", "\2\0\0\0", 4, 0), 0);
    expect("empty", fset(fd, "system.posix_acl_access", 0, 0, 0), 0);
    expect("mode_kept", mode_of(fd), 0644);
    a_close(fd);
    fd = a_open(D "/ro", O_RDONLY, 0);
    check("setup_open_ro", fd >= 0, fd);
    expect("ro_len_1_einval", fset(fd, "system.posix_acl_access", "v", 1, 0), -EINVAL);
    expect("ro_owner_sets_acl", fset(fd, "system.posix_acl_access", acl, 28, 0), 0);
    expect("ro_mode_now_750", mode_of(fd), 0750);
    expect("base_acl_not_kept", syscall4(SYS_fgetxattr, fd, "system.posix_acl_access", buf, sizeof buf), -ENODATA);
    expect("create_flag_ignored", fset(fd, "system.posix_acl_access", acl, 28, 1), 0);
    expect("create_flag_again", fset(fd, "system.posix_acl_access", acl, 28, 1), 0);
    expect("replace_flag_ignored", fset(fd, "system.posix_acl_access", acl, 28, 2), 0);
    a_close(fd);
    fd = a_open(D "/sub", O_RDONLY | O_DIRECTORY, 0);
    check("setup_open_sub", fd >= 0, fd);
    expect("dir_default", fset(fd, "system.posix_acl_default", acl, 28, 0), 0);
    expect("dir_default_get", syscall4(SYS_fgetxattr, fd, "system.posix_acl_default", buf, sizeof buf), 28);
    check("dir_default_bytes", mem_eq(buf, acl, 28), 0);
    expect("dir_mode_kept", mode_of(fd), 0755);
    a_close(fd);
    a_rmdir(D "/sub");
    a_unlink(D "/f");
    a_unlink(D "/ro");
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
