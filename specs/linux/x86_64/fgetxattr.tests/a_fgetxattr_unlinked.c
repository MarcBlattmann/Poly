/* Attributes belong to the inode, so a descriptor keeps reading them after
 * the file is renamed or unlinked, and after an open directory is removed.
 * An O_TMPFILE file (never linked) starts with no user.* attributes and
 * reads back what fsetxattr gives it. */
#include "a_sys.h"

#define SYS_setxattr 188
#define SYS_fsetxattr 190
#define SYS_fgetxattr 193
#define ENODATA 61
#define D "/tmp/a_fgetxattr_unlinked"

static char buf[64];

static i64 fget(i64 fd, const char *name) { return syscall4(SYS_fgetxattr, fd, name, buf, sizeof buf); }

int test_main(void) {
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_file", a_make_file(D "/f", "", 0644), 0);
    expect("setup_set", syscall5(SYS_setxattr, D "/f", "user.a", "abc", 3, 0), 0);
    i64 fd = a_open(D "/f", O_RDONLY, 0);
    check("setup_open", fd >= 0, fd);
    expect("rename", syscall2(SYS_rename, D "/f", D "/g"), 0);
    expect("after_rename", fget(fd, "user.a"), 3);
    expect("unlink", a_unlink(D "/g"), 0);
    expect("after_unlink", fget(fd, "user.a"), 3);
    check("after_unlink_value", mem_eq(buf, "abc", 3), 0);
    expect("after_unlink_missing", fget(fd, "user.b"), -ENODATA);
    a_close(fd);
    expect("setup_dir", a_mkdir(D "/d", 0755), 0);
    expect("setup_set_dir", syscall5(SYS_setxattr, D "/d", "user.d", "dir", 3, 0), 0);
    fd = a_open(D "/d", O_RDONLY | O_DIRECTORY, 0);
    check("setup_open_dir", fd >= 0, fd);
    expect("rmdir", a_rmdir(D "/d"), 0);
    expect("removed_dir", fget(fd, "user.d"), 3);
    check("removed_dir_value", mem_eq(buf, "dir", 3), 0);
    a_close(fd);
    fd = a_open(D, O_TMPFILE | O_RDWR, 0600);
    check("setup_tmpfile", fd >= 0, fd);
    expect("tmpfile_none", fget(fd, "user.t"), -ENODATA);
    expect("tmpfile_set", syscall5(SYS_fsetxattr, fd, "user.t", "tmp", 3, 0), 0);
    expect("tmpfile_read", fget(fd, "user.t"), 3);
    check("tmpfile_value", mem_eq(buf, "tmp", 3), 0);
    a_close(fd);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
