/* Attributes belong to the inode: removing through one hard link removes
 * it for every link; a renamed file keeps its attributes and they can be
 * removed under the new name; a file that is unlinked but still open can
 * be reached through /proc/self/fd/N, and removing there is seen through
 * the descriptor (fgetxattr ENODATA). */
#include "a_sys.h"

#define SYS_setxattr 188
#define SYS_getxattr 191
#define SYS_fgetxattr 193
#define SYS_removexattr 197
#define ENODATA 61
#define D "/tmp/a_removexattr_links"

static char buf[16], fdpath[64];

static i64 rm(const char *p, const char *name) { return syscall2(SYS_removexattr, p, name); }
static i64 set(const char *p, const char *name) { return syscall5(SYS_setxattr, p, name, "v", 1, 0); }
static i64 get(const char *p, const char *name) { return syscall4(SYS_getxattr, p, name, buf, sizeof buf); }

int test_main(void) {
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_file", a_make_file(D "/f", "", 0644), 0);
    expect("setup_link", syscall5(SYS_linkat, AT_FDCWD, D "/f", AT_FDCWD, D "/h", 0), 0);
    expect("set_a", set(D "/f", "user.a"), 0);
    expect("set_b", set(D "/f", "user.b"), 0);
    expect("remove_via_h", rm(D "/h", "user.a"), 0);
    expect("gone_via_f", get(D "/f", "user.a"), -ENODATA);
    expect("again_via_f", rm(D "/f", "user.a"), -ENODATA);
    expect("rename", syscall4(SYS_renameat, AT_FDCWD, D "/f", AT_FDCWD, D "/g"), 0);
    expect("renamed_keeps_b", get(D "/g", "user.b"), 1);
    expect("old_name_enoent", rm(D "/f", "user.b"), -ENOENT);
    expect("remove_via_new_name", rm(D "/g", "user.b"), 0);
    expect("gone_via_h", get(D "/h", "user.b"), -ENODATA);
    expect("setup_u", a_make_file(D "/u", "", 0644), 0);
    expect("set_u", set(D "/u", "user.u"), 0);
    i64 fd = a_open(D "/u", O_RDONLY, 0);
    check("open_u", fd >= 0, fd);
    expect("unlink_u", a_unlink(D "/u"), 0);
    str_cpy(fdpath, "/proc/self/fd/");
    a_utoa(fdpath + str_len(fdpath), (u64)fd);
    expect("remove_unlinked_via_proc", rm(fdpath, "user.u"), 0);
    expect("fd_sees_removal", syscall4(SYS_fgetxattr, fd, "user.u", buf, sizeof buf), -ENODATA);
    expect("remove_unlinked_again", rm(fdpath, "user.u"), -ENODATA);
    a_close(fd);
    expect("cleanup_g", a_unlink(D "/g"), 0);
    expect("cleanup_h", a_unlink(D "/h"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
