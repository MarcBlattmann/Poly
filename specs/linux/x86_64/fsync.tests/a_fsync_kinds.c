/* fsync on things that cannot be synced: pipes and character devices are
 * EINVAL. */
#include "a_sys.h"

int test_main(void) {
    i32 p[2];
    expect("pipe2", syscall2(SYS_pipe2, p, 0), 0);
    expect("fsync_pipe_read_einval", syscall1(SYS_fsync, p[0]), -EINVAL);
    expect("fsync_pipe_write_einval", syscall1(SYS_fsync, p[1]), -EINVAL);
    expect("fdatasync_pipe_einval", syscall1(SYS_fdatasync, p[1]), -EINVAL);
    a_close(p[0]);
    a_close(p[1]);
    i64 fd = a_open("/dev/null", O_WRONLY, 0);
    expect("fsync_dev_null_einval", syscall1(SYS_fsync, fd), -EINVAL);
    expect("fdatasync_dev_null_einval", syscall1(SYS_fdatasync, fd), -EINVAL);
    a_close(fd);
    return finish();
}
