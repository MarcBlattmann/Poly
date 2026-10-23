/* Error order: EBADF (no such fd, or not open for reading) comes first,
 * then EINVAL for the file type, then EINVAL for the count. */
#include "a_sys.h"

#define SYS_readahead 187

static i64 ra(i64 fd, i64 off, i64 count) { return syscall3(SYS_readahead, fd, off, count); }

#define F "/tmp/a_readahead_order"

int test_main(void) {
    int p[2];
    expect("make", a_make_file(F, "abc", 0644), 0);
    i64 w = a_open(F, O_WRONLY, 0);
    i64 r = a_open(F, O_RDONLY, 0);
    i64 d = a_open("/tmp", O_RDONLY | O_DIRECTORY, 0);
    expect("pipe2", syscall2(SYS_pipe2, p, 0), 0);
    expect("badfd_bad_count", ra(77, -1, -1), -EBADF);
    expect("wronly_bad_count", ra(w, -1, -1), -EBADF);
    expect("pipe_write_bad_count", ra(p[1], 0, -1), -EBADF);
    expect("dir_bad_count", ra(d, 0, -1), -EINVAL);
    expect("pipe_read_bad_count", ra(p[0], 0, -1), -EINVAL);
    expect("file_bad_count", ra(r, 0, -1), -EINVAL);
    expect("file_good", ra(r, 0, 3), 0);
    a_close(p[0]);
    a_close(p[1]);
    a_close(d);
    a_close(r);
    a_close(w);
    expect("cleanup", a_unlink(F), 0);
    return finish();
}
