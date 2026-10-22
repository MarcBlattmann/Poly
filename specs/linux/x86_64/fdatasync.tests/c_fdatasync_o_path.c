/* fdatasync on O_PATH descriptors: EBADF, for files, directories,
 * symbolic links and devices alike (before any EINVAL for the kind). */
#include "a_sys.h"

#define D "/tmp/c_fdatasync_o_path"

int test_main(void) {
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_file", a_make_file(D "/f", "x", 0644), 0);
    expect("setup_link", syscall3(SYS_symlinkat, "f", AT_FDCWD, D "/l"), 0);
    i64 fd = a_open(D "/f", O_PATH, 0);
    check("file_open", fd >= 0, fd);
    expect("file", syscall1(SYS_fdatasync, fd), -EBADF);
    a_close(fd);
    fd = a_open(D, O_PATH | O_DIRECTORY, 0);
    check("dir_open", fd >= 0, fd);
    expect("dir", syscall1(SYS_fdatasync, fd), -EBADF);
    a_close(fd);
    fd = a_open(D "/l", O_PATH | O_NOFOLLOW, 0);
    check("symlink_open", fd >= 0, fd);
    expect("symlink", syscall1(SYS_fdatasync, fd), -EBADF);
    a_close(fd);
    fd = a_open("/dev/null", O_PATH, 0);
    check("dev_null_open", fd >= 0, fd);
    expect("dev_null", syscall1(SYS_fdatasync, fd), -EBADF);
    a_close(fd);
    expect("cleanup_link", a_unlink(D "/l"), 0);
    expect("cleanup_file", a_unlink(D "/f"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
