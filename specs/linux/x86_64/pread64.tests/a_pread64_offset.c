/* pread64: reads at the given offset without moving the file offset. */
#include "a_sys.h"

#define F "/tmp/a_pread64_offset"

static i64 pread(i64 fd, void *buf, size_t n, i64 off) { return syscall4(SYS_pread64, fd, buf, n, off); }

int test_main(void) {
    char buf[16];
    expect("setup_file", a_make_file(F, "abcdefghij", 0644), 0);
    i64 fd = a_open(F, O_RDONLY, 0);
    expect("read_2", a_read(fd, buf, 2), 2);
    expect("pread_at_5", pread(fd, buf, 3, 5), 3);
    check("pread_at_5_contents", mem_eq(buf, "fgh", 3), buf[0]);
    expect("offset_not_moved", syscall3(SYS_lseek, fd, 0, SEEK_CUR), 2);
    expect("pread_at_0", pread(fd, buf, 4, 0), 4);
    check("pread_at_0_contents", mem_eq(buf, "abcd", 4), buf[0]);
    expect("pread_short_at_end", pread(fd, buf, 16, 8), 2);
    expect("pread_at_size_is_0", pread(fd, buf, 16, 10), 0);
    expect("pread_far_past_end_is_0", pread(fd, buf, 16, 1L << 40), 0);
    expect("pread_zero_count", pread(fd, buf, 0, 3), 0);
    expect("pread_negative_offset_einval", pread(fd, buf, 1, -1), -EINVAL);
    expect("read_continues_at_2", a_read(fd, buf, 1), 1);
    check("read_continues_contents", buf[0] == 'c', buf[0]);
    a_close(fd);

    /* O_APPEND does not affect where pread reads. */
    fd = a_open(F, O_RDWR | O_APPEND, 0);
    expect("pread_with_append", pread(fd, buf, 2, 1), 2);
    check("pread_with_append_contents", mem_eq(buf, "bc", 2), buf[0]);
    a_close(fd);
    expect("cleanup_unlink", a_unlink(F), 0);
    return finish();
}
