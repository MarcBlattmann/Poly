/* readv: fills the buffers in order, each completely before the next;
 * short at end of file; empty entries are skipped. */
#include "a_sys.h"

#define F "/tmp/a_readv_scatter"

int test_main(void) {
    char a[3], b[1], c[5], big[64];
    expect("setup_file", a_make_file(F, "0123456789", 0644), 0);
    i64 fd = a_open(F, O_RDONLY, 0);
    struct a_iovec iov[4] = {{a, 3}, {0, 0}, {b, 1}, {c, 5}};
    expect("readv_9", syscall3(SYS_readv, fd, iov, 4), 9);
    check("first_buffer", mem_eq(a, "012", 3), a[0]);
    check("second_buffer", b[0] == '3', b[0]);
    check("third_buffer", mem_eq(c, "45678", 5), c[0]);
    expect("offset_9", syscall3(SYS_lseek, fd, 0, SEEK_CUR), 9);
    memset(c, '-', 5);
    expect("readv_short", syscall3(SYS_readv, fd, iov, 4), 1);
    check("short_fills_first", a[0] == '9', a[0]);
    expect("readv_at_eof", syscall3(SYS_readv, fd, iov, 4), 0);
    expect("readv_zero_count", syscall3(SYS_readv, fd, iov, 0), 0);
    struct a_iovec empty[2] = {{a, 0}, {b, 0}};
    syscall3(SYS_lseek, fd, 0, SEEK_SET);
    expect("readv_all_empty", syscall3(SYS_readv, fd, empty, 2), 0);
    expect("all_empty_offset_unchanged", syscall3(SYS_lseek, fd, 0, SEEK_CUR), 0);
    struct a_iovec one = {big, sizeof big};
    expect("readv_one_buffer", syscall3(SYS_readv, fd, &one, 1), 10);
    a_close(fd);
    expect("cleanup_unlink", a_unlink(F), 0);
    return finish();
}
