/* pwrite64: writes at the given offset without moving the file offset,
 * grows the file, leaves holes of zeros. */
#include "a_sys.h"

#define F "/tmp/a_pwrite64_offset"

static i64 pwrite(i64 fd, const void *buf, size_t n, i64 off) { return syscall4(SYS_pwrite64, fd, buf, n, off); }

int test_main(void) {
    char buf[32];
    struct a_stat st;
    i64 fd = a_open(F, O_RDWR | O_CREAT | O_TRUNC, 0644);
    expect("create", fd, 3);
    expect("write_4", a_write(fd, "abcd", 4), 4);
    expect("pwrite_at_1", pwrite(fd, "XY", 2, 1), 2);
    expect("offset_still_4", syscall3(SYS_lseek, fd, 0, SEEK_CUR), 4);
    expect("pwrite_past_end", pwrite(fd, "end", 3, 10), 3);
    syscall2(SYS_fstat, fd, &st);
    expect("size_grew_to_13", st.st_size, 13);
    expect("offset_still_4_after_grow", syscall3(SYS_lseek, fd, 0, SEEK_CUR), 4);
    expect("pwrite_zero_count", pwrite(fd, "q", 0, 50), 0);
    syscall2(SYS_fstat, fd, &st);
    expect("zero_count_does_not_grow", st.st_size, 13);
    expect("pwrite_negative_offset_einval", pwrite(fd, "q", 1, -2), -EINVAL);
    expect("pread_back", syscall4(SYS_pread64, fd, buf, sizeof buf, 0), 13);
    check("contents_head", mem_eq(buf, "aXYd", 4), buf[1]);
    check("contents_hole", buf[4] == 0 && buf[9] == 0, buf[4]);
    check("contents_tail", mem_eq(buf + 10, "end", 3), buf[10]);
    expect("write_continues_at_4", a_write(fd, "!", 1), 1);
    syscall4(SYS_pread64, fd, buf, 1, 4);
    check("write_continued_at_4", buf[0] == '!', buf[0]);
    a_close(fd);
    expect("cleanup_unlink", a_unlink(F), 0);
    return finish();
}
