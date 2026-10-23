/* An open descriptor always sees the inode's current attributes: an
 * attribute set, replaced or removed by path after the open, or through
 * another descriptor of a hard link, shows at once. A shorter replacement
 * returns the new length and leaves the old bytes past it in the buffer. */
#include "a_sys.h"

#define SYS_setxattr 188
#define SYS_fsetxattr 190
#define SYS_fgetxattr 193
#define SYS_removexattr 197
#define SYS_fremovexattr 199
#define ENODATA 61
#define D "/tmp/a_fgetxattr_live"

static char buf[64];

static i64 fget(i64 fd, const char *name) { return syscall4(SYS_fgetxattr, fd, name, buf, sizeof buf); }

int test_main(void) {
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_file", a_make_file(D "/f", "", 0644), 0);
    i64 fd = a_open(D "/f", O_RDONLY, 0);
    check("setup_open", fd >= 0, fd);
    expect("none_yet", fget(fd, "user.a"), -ENODATA);
    expect("set_by_path", syscall5(SYS_setxattr, D "/f", "user.a", "abc", 3, 0), 0);
    expect("seen_after_set", fget(fd, "user.a"), 3);
    check("value_after_set", mem_eq(buf, "abc", 3), 0);
    expect("replace_by_path", syscall5(SYS_setxattr, D "/f", "user.a", "xy", 2, 0), 0);
    expect("seen_after_replace", fget(fd, "user.a"), 2);
    check("value_after_replace", mem_eq(buf, "xyc", 3), 0);
    expect("remove_by_path", syscall2(SYS_removexattr, D "/f", "user.a"), 0);
    expect("gone_after_remove", fget(fd, "user.a"), -ENODATA);
    expect("setup_link", syscall2(SYS_link, D "/f", D "/g"), 0);
    i64 fd2 = a_open(D "/g", O_RDWR, 0);
    check("setup_open_link", fd2 >= 0, fd2);
    expect("set_by_link_fd", syscall5(SYS_fsetxattr, fd2, "user.b", "link", 4, 0), 0);
    expect("seen_through_first_fd", fget(fd, "user.b"), 4);
    check("value_through_first_fd", mem_eq(buf, "link", 4), 0);
    expect("remove_by_link_fd", syscall2(SYS_fremovexattr, fd2, "user.b"), 0);
    expect("gone_through_first_fd", fget(fd, "user.b"), -ENODATA);
    a_close(fd2);
    a_close(fd);
    expect("cleanup_g", a_unlink(D "/g"), 0);
    expect("cleanup_f", a_unlink(D "/f"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
