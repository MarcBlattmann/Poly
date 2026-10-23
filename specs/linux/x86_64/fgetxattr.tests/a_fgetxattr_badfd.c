/* EBADF for descriptors that are not open (closed, never opened,
 * 0x7fffffff), negative values other than AT_FDCWD (-1, -99, -101), and
 * O_PATH descriptors of a file, a directory, a FIFO and of a symbolic link
 * opened O_PATH|O_NOFOLLOW. Name errors come first: an unreadable name is
 * EFAULT and an empty or too long name ERANGE even with a bad descriptor;
 * everything else (unknown prefix, "user.", trusted.*, system.*, size,
 * buffer) comes after EBADF. */
#include "a_sys.h"

#define SYS_mknodat 259
#define SYS_setxattr 188
#define SYS_fgetxattr 193
#define EBADF 9
#define D "/tmp/a_fgetxattr_badfd"

static char buf[64];
static char longname[300];

static i64 fget(i64 fd, const void *name) { return syscall4(SYS_fgetxattr, fd, name, buf, sizeof buf); }

int test_main(void) {
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_file", a_make_file(D "/f", "", 0644), 0);
    expect("setup_set", syscall5(SYS_setxattr, D "/f", "user.a", "abc", 3, 0), 0);
    expect("setup_link", syscall3(SYS_symlinkat, "f", AT_FDCWD, D "/l"), 0);
    expect("setup_fifo", syscall4(SYS_mknodat, AT_FDCWD, D "/p", S_IFIFO | 0644, 0), 0);
    for (int i = 0; i < 299; i++) longname[i] = i < 5 ? "user."[i] : 'x';
    expect("minus1", fget(-1, "user.a"), -EBADF);
    expect("minus99", fget(-99, "user.a"), -EBADF);
    expect("minus101", fget(-101, "user.a"), -EBADF);
    expect("never_opened", fget(999, "user.a"), -EBADF);
    expect("max_int", fget(0x7fffffff, "user.a"), -EBADF);
    i64 fd = a_open(D "/f", O_RDONLY, 0);
    expect("open_works", fget(fd, "user.a"), 3);
    a_close(fd);
    expect("closed", fget(fd, "user.a"), -EBADF);
    fd = a_open(D "/f", O_PATH, 0);
    check("setup_opath_file", fd >= 0, fd);
    expect("opath_file", fget(fd, "user.a"), -EBADF);
    expect("opath_file_unknown_prefix", fget(fd, "foo.a"), -EBADF);
    expect("opath_file_empty_name", fget(fd, ""), -ERANGE);
    a_close(fd);
    fd = a_open(D, O_PATH | O_DIRECTORY, 0);
    check("setup_opath_dir", fd >= 0, fd);
    expect("opath_dir", fget(fd, "user.a"), -EBADF);
    a_close(fd);
    fd = a_open(D "/l", O_PATH | O_NOFOLLOW, 0);
    check("setup_opath_symlink", fd >= 0, fd);
    expect("opath_symlink", fget(fd, "user.a"), -EBADF);
    expect("opath_symlink_trusted", fget(fd, "trusted.a"), -EBADF);
    a_close(fd);
    fd = a_open(D "/p", O_PATH, 0);
    check("setup_opath_fifo", fd >= 0, fd);
    expect("opath_fifo", fget(fd, "user.a"), -EBADF);
    a_close(fd);
    expect("name_fault_first", fget(-1, (const void *)1), -EFAULT);
    expect("name_null_first", fget(999, 0), -EFAULT);
    expect("empty_name_first", fget(-1, ""), -ERANGE);
    expect("long_name_first", fget(-1, longname), -ERANGE);
    expect("then_unknown_prefix", fget(-1, "foo.a"), -EBADF);
    expect("then_user_dot", fget(-1, "user."), -EBADF);
    expect("then_trusted", fget(-1, "trusted.a"), -EBADF);
    expect("then_system_x", fget(-1, "system.x"), -EBADF);
    expect("then_acl", fget(-1, "system.posix_acl_access"), -EBADF);
    expect("then_null_buffer", syscall4(SYS_fgetxattr, -1, "user.a", 0, 16), -EBADF);
    expect("then_size0", syscall4(SYS_fgetxattr, -1, "user.a", 0, 0), -EBADF);
    expect("then_size1", syscall4(SYS_fgetxattr, -1, "user.a", buf, 1), -EBADF);
    expect("cleanup_p", a_unlink(D "/p"), 0);
    expect("cleanup_l", a_unlink(D "/l"), 0);
    expect("cleanup_f", a_unlink(D "/f"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
