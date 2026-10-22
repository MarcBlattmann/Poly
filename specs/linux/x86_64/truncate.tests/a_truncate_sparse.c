/* truncate can make a file far larger than its data: growing to 4 GiB + 5
 * succeeds, the size is exact and the new part reads as zeros right up to
 * the end; reading at the end returns 0. Shrinking back keeps the data
 * before the new end. */
#include "a_sys.h"

#define F "/tmp/a_truncate_sparse"
#define BIG ((1L << 32) + 5)

int test_main(void) {
    struct a_stat st;
    char buf[16];
    expect("setup_file", a_make_file(F, "data", 0644), 0);
    expect("grow_4g", syscall2(SYS_truncate, F, BIG), 0);
    a_stat_path(F, &st);
    expect("size_4g", st.st_size, BIG);
    i64 fd = a_open(F, O_RDONLY, 0);
    for (int i = 0; i < 16; i++) buf[i] = 'x';
    expect("pread_tail", syscall4(SYS_pread64, fd, buf, sizeof buf, BIG - 3), 3);
    check("tail_zeros", buf[0] == 0 && buf[1] == 0 && buf[2] == 0, buf[0]);
    expect("pread_middle", syscall4(SYS_pread64, fd, buf, 8, 1L << 31), 8);
    check("middle_zeros", buf[0] == 0 && buf[7] == 0, buf[0]);
    expect("pread_at_end", syscall4(SYS_pread64, fd, buf, sizeof buf, BIG), 0);
    expect("pread_head", syscall4(SYS_pread64, fd, buf, 4, 0), 4);
    check("head_data", mem_eq(buf, "data", 4), buf[0]);
    expect("seek_end", syscall3(SYS_lseek, fd, 0, SEEK_END), BIG);
    expect("shrink", syscall2(SYS_truncate, F, 2), 0);
    expect("pread_after_shrink", syscall4(SYS_pread64, fd, buf, sizeof buf, 0), 2);
    check("shrink_data", mem_eq(buf, "da", 2), buf[0]);
    a_close(fd);
    expect("cleanup_unlink", a_unlink(F), 0);
    return finish();
}
