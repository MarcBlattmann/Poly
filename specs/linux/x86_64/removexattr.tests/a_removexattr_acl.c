/* removexattr of POSIX ACLs (system.posix_acl_access/_default): removing
 * an access ACL returns 0, leaves the mode bits as they were (the group
 * bits keep showing the old mask) and getxattr is ENODATA after. Removing
 * an ACL that is not there also returns 0 (not ENODATA): a second removal,
 * a default ACL on a plain file, an ACL on a FIFO. Removing a directory's
 * default ACL means new children get no ACL. Only ownership counts, not
 * write permission: the owner removes the ACL of its own 0444 file. */
#include "a_sys.h"

#define SYS_setxattr 188
#define SYS_getxattr 191
#define SYS_listxattr 194
#define SYS_removexattr 197
#define SYS_mknodat 259
#define ENODATA 61
#define D "/tmp/a_removexattr_acl"
#define ACCESS "system.posix_acl_access"
#define DEFAULT "system.posix_acl_default"

/* linux/posix_acl_xattr.h: version 2 header, then {tag, perm, id} entries;
 * linux/posix_acl.h: ACL_USER_OBJ 1, ACL_USER 2, ACL_GROUP_OBJ 4,
 * ACL_MASK 0x10, ACL_OTHER 0x20; ACL_UNDEFINED_ID -1. */
struct ace { u16 tag, perm; u32 id; };
static struct { u32 version; struct ace e[5]; } acl = {
    2,
    { { 1, 7, 0xffffffff }, { 2, 7, 4242 }, { 4, 4, 0xffffffff }, { 0x10, 5, 0xffffffff }, { 0x20, 4, 0xffffffff } },
};
static char buf[128];

static i64 rm(const char *p, const char *name) { return syscall2(SYS_removexattr, p, name); }
static i64 get(const char *p, const char *name) { return syscall4(SYS_getxattr, p, name, buf, sizeof buf); }
static i64 setacl(const char *p, const char *name) { return syscall5(SYS_setxattr, p, name, &acl, sizeof acl, 0); }
static i64 mode_of(const char *p) {
    struct a_stat st;
    if (a_stat_path(p, &st) != 0) return -1;
    return st.st_mode & 07777;
}

int test_main(void) {
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_file", a_make_file(D "/f", "", 0644), 0);
    expect("set_access", setacl(D "/f", ACCESS), 0);
    expect("mode_with_acl", mode_of(D "/f"), 0754);
    expect("get_access", get(D "/f", ACCESS), 44);
    expect("remove_access", rm(D "/f", ACCESS), 0);
    expect("mode_after_remove", mode_of(D "/f"), 0754);
    expect("get_access_gone", get(D "/f", ACCESS), -ENODATA);
    expect("list_empty", syscall3(SYS_listxattr, D "/f", buf, sizeof buf), 0);
    expect("remove_access_again_ok", rm(D "/f", ACCESS), 0);
    expect("remove_default_on_file_ok", rm(D "/f", DEFAULT), 0);
    expect("setup_ro", a_make_file(D "/ro", "", 0644), 0);
    expect("set_ro_access", setacl(D "/ro", ACCESS), 0);
    expect("setup_ro_mode", syscall3(SYS_fchmodat, AT_FDCWD, D "/ro", 0444), 0);
    expect("remove_ro_access_ok", rm(D "/ro", ACCESS), 0);
    expect("ro_access_gone", get(D "/ro", ACCESS), -ENODATA);
    expect("ro_mode_kept", mode_of(D "/ro"), 0444);
    expect("setup_dir", a_mkdir(D "/dd", 0755), 0);
    expect("set_default", setacl(D "/dd", DEFAULT), 0);
    expect("child_with_default", a_make_file(D "/dd/c1", "", 0644), 0);
    expect("child_got_acl", get(D "/dd/c1", ACCESS), 44);
    expect("remove_default", rm(D "/dd", DEFAULT), 0);
    expect("get_default_gone", get(D "/dd", DEFAULT), -ENODATA);
    expect("remove_default_again_ok", rm(D "/dd", DEFAULT), 0);
    expect("child_without_default", a_make_file(D "/dd/c2", "", 0644), 0);
    expect("child_no_acl", get(D "/dd/c2", ACCESS), -ENODATA);
    expect("child_mode_umask", mode_of(D "/dd/c2"), 0644);
    expect("old_child_keeps_acl", get(D "/dd/c1", ACCESS), 44);
    expect("setup_fifo", syscall4(SYS_mknodat, AT_FDCWD, D "/fifo", 0010644, 0), 0);
    expect("remove_fifo_access_ok", rm(D "/fifo", ACCESS), 0);
    a_unlink(D "/dd/c1");
    a_unlink(D "/dd/c2");
    a_rmdir(D "/dd");
    a_unlink(D "/fifo");
    a_unlink(D "/ro");
    expect("cleanup_f", a_unlink(D "/f"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
