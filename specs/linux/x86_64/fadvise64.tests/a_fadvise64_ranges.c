/* fadvise64: any range succeeds on a regular file, including one past the
 * end, a range on an empty file, and offsets and lengths that are not page
 * aligned. The file's size does not change. */
#include "a_sys.h"

#define SYS_fadvise64 221
#define D "/tmp/fadvise64_ranges"

static i64 fadv(i64 fd, i64 off, i64 len, i64 advice) { return syscall4(SYS_fadvise64, fd, off, len, advice); }

int test_main(void) {
    static char buf[3 * 4096 + 100];
    memset(buf, 'r', sizeof buf);
    a_mkdir(D, 0755);
    i64 e = a_open(D "/empty", O_RDWR | O_CREAT | O_TRUNC, 0644);
    check("open_empty", e >= 0, e);
    int ok = 1;
    for (i64 a = 0; a <= 5; a++) ok &= fadv(e, 0, 0, a) == 0 && fadv(e, 4096, 4096, a) == 0;
    check("empty_all_advice", ok, 0);
    struct a_stat st;
    expect("empty_fstat", syscall2(SYS_fstat, e, &st), 0);
    expect("empty_size_unchanged", st.st_size, 0);
    a_close(e);
    i64 fd = a_open(D "/f", O_RDWR | O_CREAT | O_TRUNC, 0644);
    check("open", fd >= 0, fd);
    expect("write", a_write(fd, buf, sizeof buf), sizeof buf);
    expect("past_eof_willneed", fadv(fd, 1L << 30, 4096, 3), 0);
    expect("past_eof_dontneed", fadv(fd, 1L << 40, 0, 4), 0);
    expect("partial_page_dontneed", fadv(fd, 100, 200, 4), 0);
    expect("unaligned_willneed", fadv(fd, 4095, 4097, 3), 0);
    expect("straddling_eof", fadv(fd, 3 * 4096, 8192, 4), 0);
    expect("one_byte", fadv(fd, 5, 1, 2), 0);
    expect("whole_by_len", fadv(fd, 0, sizeof buf, 5), 0);
    expect("tail_from_offset", fadv(fd, 4096, 0, 1), 0);
    expect("fstat", syscall2(SYS_fstat, fd, &st), 0);
    expect("size_unchanged", st.st_size, sizeof buf);
    a_close(fd);
    a_unlink(D "/f");
    a_unlink(D "/empty");
    a_rmdir(D);
    return finish();
}
