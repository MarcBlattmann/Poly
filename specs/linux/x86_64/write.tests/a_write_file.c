/* write: a regular file grows, the offset advances, a write after a seek
 * past the end leaves a hole of zeros. */
#include "a_sys.h"

#define F "/tmp/a_write_file"

int test_main(void) {
    char buf[64];
    struct a_stat st;
    i64 fd = a_open(F, O_RDWR | O_CREAT | O_TRUNC, 0644);
    expect("create", fd, 3);
    expect("write_5", a_write(fd, "hello", 5), 5);
    expect("offset_5", syscall3(SYS_lseek, fd, 0, SEEK_CUR), 5);
    expect("write_zero_bytes", a_write(fd, "x", 0), 0);
    syscall2(SYS_fstat, fd, &st);
    expect("size_5", st.st_size, 5);
    expect("overwrite_at_1", syscall3(SYS_lseek, fd, 1, SEEK_SET), 1);
    expect("overwrite_write", a_write(fd, "EL", 2), 2);
    syscall2(SYS_fstat, fd, &st);
    expect("overwrite_keeps_size", st.st_size, 5);
    expect("seek_past_end", syscall3(SYS_lseek, fd, 20, SEEK_SET), 20);
    expect("write_after_hole", a_write(fd, "end", 3), 3);
    syscall2(SYS_fstat, fd, &st);
    expect("size_with_hole", st.st_size, 23);
    syscall3(SYS_lseek, fd, 0, SEEK_SET);
    expect("read_back", a_read(fd, buf, sizeof buf), 23);
    check("head", mem_eq(buf, "hELlo", 5), buf[0]);
    int zeros = 1;
    for (int i = 5; i < 20; i++) zeros &= buf[i] == 0;
    check("hole_reads_as_zeros", zeros, 0);
    check("tail", mem_eq(buf + 20, "end", 3), buf[20]);
    expect("close", a_close(fd), 0);
    expect("cleanup_unlink", a_unlink(F), 0);
    return finish();
}
