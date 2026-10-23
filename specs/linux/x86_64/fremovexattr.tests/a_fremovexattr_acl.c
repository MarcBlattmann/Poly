/* POSIX ACLs through a descriptor. The owner removes the access ACL of
 * its own file even after fchmod to 0444 (ownership, not write
 * permission, is what counts): 0, fgetxattr then ENODATA, flistxattr no
 * longer lists it, and the mode bits stay as they were. Removing an ACL
 * that is not there is 0, not ENODATA (a second removal, a default ACL on
 * a regular file, an ACL on a 0000 file). A directory's default ACL is
 * removed the same way; afterwards new entries get no ACL. Capabilities
 * are dropped first. ACL layout: linux/posix_acl_xattr.h (u32 version 2,
 * then u16 tag, u16 perm, u32 id); tags from linux/posix_acl.h. */
#include "a_sys.h"

#define SYS_capget 125
#define SYS_capset 126
#define SYS_fsetxattr 190
#define SYS_getxattr 191
#define SYS_fgetxattr 193
#define SYS_flistxattr 196
#define SYS_fremovexattr 199
#define ENODATA 61
#define D "/tmp/a_fremovexattr_acl"
#define ACL_ACCESS "system.posix_acl_access"
#define ACL_DEFAULT "system.posix_acl_default"

static u8 acl[4 + 8 * 5];
static char buf[128];

static i64 drop_caps(void) {
    u32 h[2] = { 0x20080522, 0 }, d[6] = { 0 }, e[6];
    i64 r = syscall2(SYS_capset, h, d);
    if (r < 0) return r;
    r = syscall2(SYS_capget, h, e);
    if (r < 0) return r;
    return (e[0] | e[1] | e[3] | e[4]) ? -1 : 0;
}

static void entry(int i, u16 tag, u16 perm, u32 id) {
    u8 *e = acl + 4 + 8 * i;
    *(u16 *)e = tag;
    *(u16 *)(e + 2) = perm;
    *(u32 *)(e + 4) = id;
}

static i64 frm(i64 fd, const char *n) { return syscall2(SYS_fremovexattr, fd, n); }

int test_main(void) {
    expect("drop_caps", drop_caps(), 0);
    *(u32 *)acl = 2;
    entry(0, 1, 6, 0xffffffff);    /* USER_OBJ rw */
    entry(1, 2, 4, 54321);         /* USER 54321 r */
    entry(2, 4, 4, 0xffffffff);    /* GROUP_OBJ r */
    entry(3, 0x10, 4, 0xffffffff); /* MASK r */
    entry(4, 0x20, 4, 0xffffffff); /* OTHER r */
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_file", a_make_file(D "/f", "", 0644), 0);
    i64 fd = a_open(D "/f", O_RDONLY, 0);
    check("setup_open", fd >= 0, fd);
    expect("acl_set", syscall5(SYS_fsetxattr, fd, ACL_ACCESS, acl, sizeof acl, 0), 0);
    expect("acl_listed", syscall3(SYS_flistxattr, fd, buf, sizeof buf), 24);
    expect("chmod_0444", syscall2(SYS_fchmod, fd, 0444), 0);
    expect("acl_remove_readonly", frm(fd, ACL_ACCESS), 0);
    expect("acl_gone", syscall4(SYS_fgetxattr, fd, ACL_ACCESS, buf, sizeof buf), -ENODATA);
    expect("acl_not_listed", syscall3(SYS_flistxattr, fd, buf, sizeof buf), 0);
    struct a_stat st;
    expect("stat", syscall2(SYS_fstat, fd, &st), 0);
    expect("mode_unchanged", st.st_mode & 07777, 0444);
    expect("acl_remove_again", frm(fd, ACL_ACCESS), 0);
    expect("default_on_file", frm(fd, ACL_DEFAULT), 0);
    expect("chmod_0000", syscall2(SYS_fchmod, fd, 0), 0);
    expect("acl_remove_mode_0000", frm(fd, ACL_ACCESS), 0);
    expect("user_remove_mode_0000", frm(fd, "user.a"), -EACCES);
    expect("chmod_back", syscall2(SYS_fchmod, fd, 0644), 0);
    a_close(fd);
    expect("setup_dir", a_mkdir(D "/d", 0755), 0);
    i64 dfd = a_open(D "/d", O_RDONLY | O_DIRECTORY, 0);
    check("setup_open_dir", dfd >= 0, dfd);
    *(u32 *)acl = 2;
    entry(0, 1, 7, 0xffffffff);
    entry(1, 4, 5, 0xffffffff);
    entry(2, 0x20, 5, 0xffffffff);
    expect("default_set", syscall5(SYS_fsetxattr, dfd, ACL_DEFAULT, acl, 28, 0), 0);
    expect("default_read", syscall4(SYS_fgetxattr, dfd, ACL_DEFAULT, buf, sizeof buf), 28);
    expect("default_remove", frm(dfd, ACL_DEFAULT), 0);
    expect("default_gone", syscall4(SYS_fgetxattr, dfd, ACL_DEFAULT, buf, sizeof buf), -ENODATA);
    expect("default_remove_again", frm(dfd, ACL_DEFAULT), 0);
    expect("access_on_dir_absent", frm(dfd, ACL_ACCESS), 0);
    expect("new_subdir", a_mkdir(D "/d/s", 0755), 0);
    expect("new_subdir_no_default", syscall4(SYS_getxattr, D "/d/s", ACL_DEFAULT, buf, sizeof buf), -ENODATA);
    expect("new_subdir_no_access", syscall4(SYS_getxattr, D "/d/s", ACL_ACCESS, buf, sizeof buf), -ENODATA);
    a_close(dfd);
    a_rmdir(D "/d/s");
    a_rmdir(D "/d");
    a_unlink(D "/f");
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
