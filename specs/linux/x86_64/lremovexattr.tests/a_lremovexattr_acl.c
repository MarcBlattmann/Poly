/* lremovexattr of POSIX ACLs (system.posix_acl_access/_default) on files
 * and directories: removing an access ACL returns 0, leaves the mode bits
 * as they were and getxattr is ENODATA after; removing it again returns 0
 * (not ENODATA). Ownership is enough, write permission is not needed (the
 * owner's 0444 file). A directory's default ACL is removed the same way.
 * A symbolic link to a file with an ACL is the link: EOPNOTSUPP, and the
 * file keeps its ACL. */
#include "a_sys.h"

#define SYS_setxattr 188
#define SYS_getxattr 191
#define SYS_lremovexattr 198
#define ENODATA 61
#define D "/tmp/a_lremovexattr_acl"
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

static i64 lrm(const char *p, const char *name) { return syscall2(SYS_lremovexattr, p, name); }
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
    expect("setup_link", syscall3(SYS_symlinkat, "f", AT_FDCWD, D "/l"), 0);
    expect("set_access", setacl(D "/f", ACCESS), 0);
    expect("mode_with_acl", mode_of(D "/f"), 0754);
    expect("link_access_eopnotsupp", lrm(D "/l", ACCESS), -EOPNOTSUPP);
    expect("file_keeps_acl", get(D "/f", ACCESS), 44);
    expect("remove_access", lrm(D "/f", ACCESS), 0);
    expect("mode_after_remove", mode_of(D "/f"), 0754);
    expect("get_access_gone", get(D "/f", ACCESS), -ENODATA);
    expect("remove_access_again_ok", lrm(D "/f", ACCESS), 0);
    expect("remove_default_on_file_ok", lrm(D "/f", DEFAULT), 0);
    expect("setup_ro", a_make_file(D "/ro", "", 0644), 0);
    expect("set_ro_access", setacl(D "/ro", ACCESS), 0);
    expect("setup_ro_mode", syscall3(SYS_fchmodat, AT_FDCWD, D "/ro", 0444), 0);
    expect("remove_ro_access_ok", lrm(D "/ro", ACCESS), 0);
    expect("ro_access_gone", get(D "/ro", ACCESS), -ENODATA);
    expect("ro_mode_kept", mode_of(D "/ro"), 0444);
    expect("setup_dir", a_mkdir(D "/dd", 0755), 0);
    expect("set_default", setacl(D "/dd", DEFAULT), 0);
    expect("get_default", get(D "/dd", DEFAULT), 44);
    expect("remove_default", lrm(D "/dd", DEFAULT), 0);
    expect("get_default_gone", get(D "/dd", DEFAULT), -ENODATA);
    expect("remove_default_again_ok", lrm(D "/dd", DEFAULT), 0);
    expect("child_without_default", a_make_file(D "/dd/c", "", 0644), 0);
    expect("child_no_acl", get(D "/dd/c", ACCESS), -ENODATA);
    a_unlink(D "/dd/c");
    a_rmdir(D "/dd");
    a_unlink(D "/ro");
    a_unlink(D "/l");
    expect("cleanup_f", a_unlink(D "/f"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
