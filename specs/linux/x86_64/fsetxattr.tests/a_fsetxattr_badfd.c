/* Descriptors fsetxattr refuses with EBADF: -1, other negative numbers
 * (-99, -101), a closed descriptor, a never-opened one, 0x7fffffff, and
 * O_PATH descriptors (of a file, a directory, and a symbolic link opened
 * with O_PATH | O_NOFOLLOW). The fd argument is an int: only its low 32
 * bits count, so fd + 2^32 is the same descriptor. Nothing is set by a
 * refused call. */
#include "a_sys.h"

#define SYS_fsetxattr 190
#define SYS_getxattr 191
#define ENODATA 61
#define D "/tmp/a_fsetxattr_badfd"

static char buf[64];

static i64 fset(i64 fd, const char *name) { return syscall5(SYS_fsetxattr, fd, name, "v", 1, 0); }

int test_main(void) {
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_file", a_make_file(D "/f", "", 0644), 0);
    expect("setup_link", syscall3(SYS_symlinkat, "f", AT_FDCWD, D "/l"), 0);
    expect("minus_one", fset(-1, "user.a"), -EBADF);
    expect("minus_99", fset(-99, "user.a"), -EBADF);
    expect("minus_101", fset(-101, "user.a"), -EBADF);
    expect("int_max", fset(0x7fffffff, "user.a"), -EBADF);
    expect("never_opened", fset(900, "user.a"), -EBADF);
    i64 fd = a_open(D "/f", O_RDONLY, 0);
    check("setup_open", fd >= 0, fd);
    expect("high_bits_ignored", fset(fd + 0x100000000LL, "user.hi"), 0);
    expect("high_bits_get", syscall4(SYS_getxattr, D "/f", "user.hi", buf, sizeof buf), 1);
    expect("close", a_close(fd), 0);
    expect("closed_fd", fset(fd, "user.c"), -EBADF);
    fd = a_open(D "/f", O_PATH, 0);
    check("setup_opath_file", fd >= 0, fd);
    expect("opath_file", fset(fd, "user.p"), -EBADF);
    expect("opath_trusted", fset(fd, "trusted.p"), -EBADF);
    a_close(fd);
    fd = a_open(D, O_PATH | O_DIRECTORY, 0);
    check("setup_opath_dir", fd >= 0, fd);
    expect("opath_dir", fset(fd, "user.p"), -EBADF);
    a_close(fd);
    fd = a_open(D "/l", O_PATH | O_NOFOLLOW, 0);
    check("setup_opath_link", fd >= 0, fd);
    expect("opath_link", fset(fd, "user.p"), -EBADF);
    a_close(fd);
    expect("nothing_set_c", syscall4(SYS_getxattr, D "/f", "user.c", buf, sizeof buf), -ENODATA);
    expect("nothing_set_p", syscall4(SYS_getxattr, D "/f", "user.p", buf, sizeof buf), -ENODATA);
    expect("nothing_set_dir", syscall4(SYS_getxattr, D, "user.p", buf, sizeof buf), -ENODATA);
    a_unlink(D "/l");
    a_unlink(D "/f");
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
