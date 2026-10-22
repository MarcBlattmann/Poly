/* fdatasync on directory descriptors: 0, also for / and /dev, a fresh
 * directory, one with entries and one removed while open. */
#include "a_sys.h"

#define D "/tmp/c_fdatasync_directories"

int test_main(void) {
    i64 fd = a_open("/", O_RDONLY | O_DIRECTORY, 0);
    expect("root", syscall1(SYS_fdatasync, fd), 0);
    a_close(fd);
    fd = a_open("/tmp", O_RDONLY | O_DIRECTORY, 0);
    expect("tmp", syscall1(SYS_fdatasync, fd), 0);
    a_close(fd);
    fd = a_open("/dev", O_RDONLY | O_DIRECTORY, 0);
    expect("dev", syscall1(SYS_fdatasync, fd), 0);
    a_close(fd);
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    fd = a_open(D, O_RDONLY | O_DIRECTORY, 0);
    expect("fresh", syscall1(SYS_fdatasync, fd), 0);
    expect("setup_entry", a_make_file(D "/e", "", 0644), 0);
    expect("with_entry", syscall1(SYS_fdatasync, fd), 0);
    expect("unlink_entry", a_unlink(D "/e"), 0);
    expect("rmdir_while_open", a_rmdir(D), 0);
    expect("removed", syscall1(SYS_fdatasync, fd), 0);
    a_close(fd);
    return finish();
}
