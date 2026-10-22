/* fdatasync on pipes: EINVAL for either end, with or without data, and
 * after the other end is closed. */
#include "a_sys.h"

int test_main(void) {
    i32 p[2];
    expect("pipe2", syscall2(SYS_pipe2, p, 0), 0);
    expect("read_end", syscall1(SYS_fdatasync, p[0]), -EINVAL);
    expect("write_end", syscall1(SYS_fdatasync, p[1]), -EINVAL);
    expect("fill", a_write(p[1], "abc", 3), 3);
    expect("read_end_with_data", syscall1(SYS_fdatasync, p[0]), -EINVAL);
    expect("write_end_with_data", syscall1(SYS_fdatasync, p[1]), -EINVAL);
    a_close(p[1]);
    expect("read_end_widowed", syscall1(SYS_fdatasync, p[0]), -EINVAL);
    char buf[4];
    expect("data_still_there", a_read(p[0], buf, sizeof buf), 3);
    a_close(p[0]);
    expect("nonblock_pipe2", syscall2(SYS_pipe2, p, O_NONBLOCK | O_CLOEXEC), 0);
    expect("nonblock_end", syscall1(SYS_fdatasync, p[1]), -EINVAL);
    a_close(p[0]);
    a_close(p[1]);
    return finish();
}
