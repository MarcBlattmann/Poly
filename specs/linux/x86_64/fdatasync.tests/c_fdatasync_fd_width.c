/* The descriptor argument is a 32-bit unsigned number: the upper half of
 * the register is ignored, for fdatasync as for fsync; 0xffffffff is an
 * unused descriptor. */
#include "a_sys.h"

#define F "/tmp/c_fdatasync_fd_width"

int test_main(void) {
    i64 fd = a_open(F, O_RDWR | O_CREAT | O_TRUNC, 0644);
    expect("plain", syscall1(SYS_fdatasync, fd), 0);
    expect("bit_32", syscall1(SYS_fdatasync, fd | (1L << 32)), 0);
    expect("upper_all_ones", syscall1(SYS_fdatasync, fd | (i64)0xffffffff00000000UL), 0);
    expect("fsync_bit_32", syscall1(SYS_fsync, fd | (1L << 32)), 0);
    i32 p[2];
    expect("pipe2", syscall2(SYS_pipe2, p, 0), 0);
    expect("pipe_bit_32", syscall1(SYS_fdatasync, p[0] | (1L << 32)), -EINVAL);
    expect("low_all_ones", syscall1(SYS_fdatasync, 0xffffffffL), -EBADF);
    expect("upper_only", syscall1(SYS_fdatasync, 77L | (1L << 40)), -EBADF);
    a_close(p[0]);
    a_close(p[1]);
    a_close(fd);
    expect("cleanup", a_unlink(F), 0);
    return finish();
}
