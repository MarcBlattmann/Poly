/* Pipes: the read end is EINVAL, the write end EBADF (not open for
 * reading, checked first). Nothing is consumed from the pipe. */
#include "a_sys.h"

#define SYS_readahead 187

static i64 ra(i64 fd, i64 off, i64 count) { return syscall3(SYS_readahead, fd, off, count); }


int test_main(void) {
    int p[2];
    char c = 0;
    expect("pipe2", syscall2(SYS_pipe2, p, 0), 0);
    expect("write", a_write(p[1], "q", 1), 1);
    expect("read_end", ra(p[0], 0, 1), -EINVAL);
    expect("read_end_count_0", ra(p[0], 0, 0), -EINVAL);
    expect("read_end_bad_count", ra(p[0], 0, -1), -EINVAL);
    expect("write_end", ra(p[1], 0, 1), -EBADF);
    expect("write_end_bad_count", ra(p[1], 0, -1), -EBADF);
    expect("byte_still_there", a_read(p[0], &c, 1), 1);
    check("byte_value", c == 'q', c);
    a_close(p[0]);
    expect("write_end_after_reader_closed", ra(p[1], 0, 1), -EBADF);
    a_close(p[1]);
    return finish();
}
