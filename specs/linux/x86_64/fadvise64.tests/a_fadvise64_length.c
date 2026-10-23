/* fadvise64: offset and len are signed 64-bit. A negative len is EINVAL
 * for every advice; len 0 means "to the end of the file"; a negative
 * offset is accepted; offset + len past the largest offset is fine. */
#include "a_sys.h"

#define SYS_fadvise64 221
#define D "/tmp/fadvise64_length"
#define I64_MAX 0x7fffffffffffffffL
#define I64_MIN ((i64)0x8000000000000000UL)

static i64 fadv(i64 fd, i64 off, i64 len, i64 advice) { return syscall4(SYS_fadvise64, fd, off, len, advice); }

int test_main(void) {
    static char buf[16384];
    memset(buf, 'l', sizeof buf);
    a_mkdir(D, 0755);
    i64 fd = a_open(D "/f", O_RDWR | O_CREAT | O_TRUNC, 0644);
    check("open", fd >= 0, fd);
    expect("write", a_write(fd, buf, sizeof buf), sizeof buf);
    expect("neg_len_normal", fadv(fd, 0, -1, 0), -EINVAL);
    expect("neg_len_random", fadv(fd, 0, -1, 1), -EINVAL);
    expect("neg_len_sequential", fadv(fd, 0, -4096, 2), -EINVAL);
    expect("neg_len_willneed", fadv(fd, 0, -1, 3), -EINVAL);
    expect("neg_len_dontneed", fadv(fd, 4096, -4096, 4), -EINVAL);
    expect("neg_len_noreuse", fadv(fd, 0, -1, 5), -EINVAL);
    expect("min_len", fadv(fd, 0, I64_MIN, 4), -EINVAL);
    expect("neg_len_and_neg_offset", fadv(fd, -1, -1, 0), -EINVAL);
    expect("neg_len_bad_advice", fadv(fd, 0, -1, 99), -EINVAL);
    expect("zero_len", fadv(fd, 0, 0, 4), 0);
    expect("max_len", fadv(fd, 0, I64_MAX, 4), 0);
    expect("max_len_willneed", fadv(fd, 0, I64_MAX, 3), 0);
    expect("neg_offset", fadv(fd, -1, 0, 0), 0);
    expect("neg_offset_dontneed", fadv(fd, -4096, 4096, 4), 0);
    expect("neg_offset_willneed", fadv(fd, -4096, 8192, 3), 0);
    expect("min_offset", fadv(fd, I64_MIN, 0, 4), 0);
    expect("min_offset_len", fadv(fd, I64_MIN, 100, 3), 0);
    expect("neg_offset_max_len", fadv(fd, -5, I64_MAX, 4), 0);
    expect("max_offset_max_len", fadv(fd, I64_MAX, I64_MAX, 3), 0);
    expect("max_offset_zero_len", fadv(fd, I64_MAX, 0, 4), 0);
    expect("size_unchanged", syscall3(SYS_lseek, fd, 0, SEEK_END), sizeof buf);
    a_close(fd);
    a_unlink(D "/f");
    a_rmdir(D);
    return finish();
}
