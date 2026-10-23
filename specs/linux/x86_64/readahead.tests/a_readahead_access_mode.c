/* readahead needs a descriptor open for reading: O_RDONLY and O_RDWR
 * work (whatever else is set), O_WRONLY is EBADF, also with O_APPEND. */
#include "a_sys.h"

#define SYS_readahead 187

static i64 ra(i64 fd, i64 off, i64 count) { return syscall3(SYS_readahead, fd, off, count); }

#define F "/tmp/a_readahead_access_mode"

int test_main(void) {
    expect("make", a_make_file(F, "some data", 0644), 0);
    i64 fd = a_open(F, O_RDONLY, 0);
    expect("rdonly", ra(fd, 0, 9), 0);
    a_close(fd);
    fd = a_open(F, O_RDWR, 0);
    expect("rdwr", ra(fd, 0, 9), 0);
    a_close(fd);
    fd = a_open(F, O_RDWR | O_APPEND, 0);
    expect("rdwr_append", ra(fd, 0, 9), 0);
    a_close(fd);
    fd = a_open(F, O_RDONLY | O_NONBLOCK | O_CLOEXEC, 0);
    expect("rdonly_nonblock_cloexec", ra(fd, 0, 9), 0);
    a_close(fd);
    fd = a_open(F, O_WRONLY, 0);
    expect("wronly", ra(fd, 0, 9), -EBADF);
    expect("wronly_count_0", ra(fd, 0, 0), -EBADF);
    a_close(fd);
    fd = a_open(F, O_WRONLY | O_APPEND, 0);
    expect("wronly_append", ra(fd, 0, 9), -EBADF);
    a_close(fd);
    /* Turning on O_NONBLOCK later does not matter either. */
    fd = a_open(F, O_RDONLY, 0);
    expect("setfl_nonblock", syscall3(SYS_fcntl, fd, F_SETFL, O_NONBLOCK), 0);
    expect("rdonly_after_setfl", ra(fd, 0, 9), 0);
    a_close(fd);
    expect("cleanup", a_unlink(F), 0);
    return finish();
}
