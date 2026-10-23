/* fsetxattr (190) sets an extended attribute through an open descriptor:
 * flags 0 creates or replaces, the value is stored byte for byte (NUL
 * bytes included), a zero-length value may come with a NULL pointer, and
 * names on one inode are independent and case-sensitive. The attribute is
 * seen through the path (getxattr) and the descriptor (fgetxattr).
 * Directory descriptors work the same way. */
#include "a_sys.h"

#define SYS_fsetxattr 190
#define SYS_getxattr 191
#define SYS_fgetxattr 193
#define D "/tmp/a_fsetxattr_basic"

static char buf[64];

static i64 fset(i64 fd, const char *name, const void *v, i64 size) {
    return syscall5(SYS_fsetxattr, fd, name, v, size, 0);
}

int test_main(void) {
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_sub", a_mkdir(D "/sub", 0755), 0);
    expect("setup_file", a_make_file(D "/f", "", 0644), 0);
    i64 fd = a_open(D "/f", O_RDWR, 0);
    check("setup_open", fd >= 0, fd);
    expect("set_new", fset(fd, "user.a", "hello", 5), 0);
    expect("get_path", syscall4(SYS_getxattr, D "/f", "user.a", buf, sizeof buf), 5);
    check("get_path_bytes", mem_eq(buf, "hello", 5), 0);
    expect("replace_flags0", fset(fd, "user.a", "hi", 2), 0);
    expect("get_fd", syscall4(SYS_fgetxattr, fd, "user.a", buf, sizeof buf), 2);
    check("get_fd_bytes", mem_eq(buf, "hi", 2), 0);
    expect("binary_value", fset(fd, "user.bin", "a\0b\0", 4), 0);
    expect("binary_len", syscall4(SYS_fgetxattr, fd, "user.bin", buf, sizeof buf), 4);
    check("binary_bytes", mem_eq(buf, "a\0b\0", 4), 0);
    expect("empty_null_value", fset(fd, "user.empty", 0, 0), 0);
    expect("empty_len", syscall4(SYS_fgetxattr, fd, "user.empty", buf, sizeof buf), 0);
    expect("upper_name", fset(fd, "user.A", "X", 1), 0);
    expect("lower_kept", syscall4(SYS_fgetxattr, fd, "user.a", buf, sizeof buf), 2);
    expect("upper_len", syscall4(SYS_fgetxattr, fd, "user.A", buf, sizeof buf), 1);
    expect("prefix_name", fset(fd, "user.a0", "123", 3), 0);
    expect("prefix_independent", syscall4(SYS_fgetxattr, fd, "user.a", buf, sizeof buf), 2);
    expect("close_f", a_close(fd), 0);
    i64 dfd = a_open(D "/sub", O_RDONLY | O_DIRECTORY, 0);
    check("setup_open_dir", dfd >= 0, dfd);
    expect("dir_set", fset(dfd, "user.d", "dd", 2), 0);
    expect("dir_get", syscall4(SYS_getxattr, D "/sub", "user.d", buf, sizeof buf), 2);
    check("dir_bytes", mem_eq(buf, "dd", 2), 0);
    expect("close_dir", a_close(dfd), 0);
    expect("cleanup_f", a_unlink(D "/f"), 0);
    expect("cleanup_sub", a_rmdir(D "/sub"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
