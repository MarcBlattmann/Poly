/* Descriptors fremovexattr refuses with EBADF: -1, other negative numbers
 * (-99, -101), AT_FDCWD (-100: unlike fsetxattr, it does not act on the
 * working directory, whose attribute stays), 0xffffff9c (AT_FDCWD in the
 * low 32 bits), a closed descriptor, a never-opened one, 0x7fffffff, and
 * O_PATH descriptors (of a file, a directory, and a symbolic link opened
 * with O_PATH | O_NOFOLLOW). The fd argument is an int: only its low 32
 * bits count, so fd + 2^32 and fd | 0xffffffff00000000 are the same
 * descriptor. Nothing is removed by a refused call. */
#include "a_sys.h"

#define SYS_setxattr 188
#define SYS_fsetxattr 190
#define SYS_getxattr 191
#define SYS_fremovexattr 199
#define ENODATA 61
#define D "/tmp/a_fremovexattr_badfd"

static char buf[64];

static i64 frm(i64 fd, const char *name) { return syscall2(SYS_fremovexattr, fd, name); }
static i64 get(const char *p, const char *name) { return syscall4(SYS_getxattr, p, name, buf, sizeof buf); }

int test_main(void) {
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_sub", a_mkdir(D "/sub", 0755), 0);
    expect("setup_file", a_make_file(D "/f", "", 0644), 0);
    expect("setup_link", syscall3(SYS_symlinkat, "f", AT_FDCWD, D "/l"), 0);
    expect("setup_set_f", syscall5(SYS_setxattr, D "/f", "user.a", "v", 1, 0), 0);
    expect("setup_set_sub", syscall5(SYS_setxattr, D "/sub", "user.a", "v", 1, 0), 0);
    expect("minus_one", frm(-1, "user.a"), -EBADF);
    expect("minus_99", frm(-99, "user.a"), -EBADF);
    expect("minus_101", frm(-101, "user.a"), -EBADF);
    expect("int_max", frm(0x7fffffff, "user.a"), -EBADF);
    expect("never_opened", frm(900, "user.a"), -EBADF);
    expect("chdir_sub", syscall1(SYS_chdir, D "/sub"), 0);
    expect("at_fdcwd", frm(AT_FDCWD, "user.a"), -EBADF);
    expect("at_fdcwd_low_bits", frm(0xffffff9cLL, "user.a"), -EBADF);
    expect("chdir_root", syscall1(SYS_chdir, "/"), 0);
    expect("cwd_attr_stays", get(D "/sub", "user.a"), 1);
    i64 fd = a_open(D "/f", O_RDONLY, 0);
    check("setup_open", fd >= 0, fd);
    expect("set_hi", syscall5(SYS_fsetxattr, fd, "user.hi", "v", 1, 0), 0);
    expect("high_bits_ignored", frm(fd + 0x100000000LL, "user.hi"), 0);
    expect("high_bits_get", get(D "/f", "user.hi"), -ENODATA);
    expect("set_hi2", syscall5(SYS_fsetxattr, fd, "user.hi", "v", 1, 0), 0);
    expect("all_high_bits_ignored", frm(fd | (i64)0xffffffff00000000LL, "user.hi"), 0);
    expect("close", a_close(fd), 0);
    expect("closed_fd", frm(fd, "user.a"), -EBADF);
    expect("closed_fd_high_bits", frm(fd + 0x100000000LL, "user.a"), -EBADF);
    fd = a_open(D "/f", O_PATH, 0);
    check("setup_opath_file", fd >= 0, fd);
    expect("opath_file", frm(fd, "user.a"), -EBADF);
    expect("opath_trusted", frm(fd, "trusted.a"), -EBADF);
    expect("opath_acl", frm(fd, "system.posix_acl_access"), -EBADF);
    a_close(fd);
    fd = a_open(D "/sub", O_PATH | O_DIRECTORY, 0);
    check("setup_opath_dir", fd >= 0, fd);
    expect("opath_dir", frm(fd, "user.a"), -EBADF);
    a_close(fd);
    fd = a_open(D "/l", O_PATH | O_NOFOLLOW, 0);
    check("setup_opath_link", fd >= 0, fd);
    expect("opath_link", frm(fd, "user.a"), -EBADF);
    a_close(fd);
    expect("file_attr_stays", get(D "/f", "user.a"), 1);
    expect("sub_attr_stays", get(D "/sub", "user.a"), 1);
    a_unlink(D "/l");
    a_unlink(D "/f");
    a_rmdir(D "/sub");
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
