/* fremovexattr on a descriptor of a regular file or directory: removing
 * an existing user.* attribute returns 0, fgetxattr is ENODATA after and
 * a second removal ENODATA too; a name never set is ENODATA. Other
 * attributes stay with their values; names are exact and case-sensitive
 * ("user.k" does not touch "user.K" or "user.k0"). flistxattr shrinks by
 * the name and its NUL, and the name can be created again (also with
 * XATTR_CREATE). The file's contents are untouched. */
#include "a_sys.h"

#define SYS_fsetxattr 190
#define SYS_fgetxattr 193
#define SYS_flistxattr 196
#define SYS_fremovexattr 199
#define ENODATA 61
#define D "/tmp/a_fremovexattr_basic"

static char buf[256];

static i64 frm(i64 fd, const char *name) { return syscall2(SYS_fremovexattr, fd, name); }
static i64 fset(i64 fd, const char *name, const char *v, i64 flags) {
    return syscall5(SYS_fsetxattr, fd, name, v, str_len(v), flags);
}
static i64 fget(i64 fd, const char *name) { return syscall4(SYS_fgetxattr, fd, name, buf, sizeof buf); }

int test_main(void) {
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_file", a_make_file(D "/f", "data", 0644), 0);
    i64 fd = a_open(D "/f", O_RDONLY, 0);
    check("setup_open", fd >= 0, fd);
    expect("set_k", fset(fd, "user.k", "one", 0), 0);
    expect("set_K", fset(fd, "user.K", "two!", 0), 0);
    expect("set_k0", fset(fd, "user.k0", "three", 0), 0);
    expect("list_before", syscall3(SYS_flistxattr, fd, buf, sizeof buf), 22);
    expect("remove_k", frm(fd, "user.k"), 0);
    expect("get_k_gone", fget(fd, "user.k"), -ENODATA);
    expect("remove_k_again", frm(fd, "user.k"), -ENODATA);
    expect("get_K_stays", fget(fd, "user.K"), 4);
    check("K_value", mem_eq(buf, "two!", 4), 0);
    expect("get_k0_stays", fget(fd, "user.k0"), 5);
    check("k0_value", mem_eq(buf, "three", 5), 0);
    expect("list_after", syscall3(SYS_flistxattr, fd, buf, sizeof buf), 15);
    expect("remove_never_set", frm(fd, "user.never"), -ENODATA);
    expect("remove_longer_name", frm(fd, "user.k0x"), -ENODATA);
    expect("remove_k0", frm(fd, "user.k0"), 0);
    expect("set_k_again_create", fset(fd, "user.k", "new", 1), 0);
    expect("get_k_new", fget(fd, "user.k"), 3);
    check("k_new_value", mem_eq(buf, "new", 3), 0);
    expect("remove_K", frm(fd, "user.K"), 0);
    expect("remove_k_last", frm(fd, "user.k"), 0);
    expect("list_empty", syscall3(SYS_flistxattr, fd, buf, sizeof buf), 0);
    expect("contents_unchanged", a_read(fd, buf, sizeof buf), 4);
    check("contents_value", mem_eq(buf, "data", 4), 0);
    a_close(fd);
    i64 dfd = a_open(D, O_RDONLY | O_DIRECTORY, 0);
    check("setup_open_dir", dfd >= 0, dfd);
    expect("dir_set", fset(dfd, "user.d", "x", 0), 0);
    expect("dir_remove", frm(dfd, "user.d"), 0);
    expect("dir_get_gone", fget(dfd, "user.d"), -ENODATA);
    expect("dir_remove_again", frm(dfd, "user.d"), -ENODATA);
    a_close(dfd);
    expect("cleanup_f", a_unlink(D "/f"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
