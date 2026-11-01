/* fadvise64(fd, offset, len, advice) on a regular file: every advice
 * POSIX_FADV_NORMAL 0, RANDOM 1, SEQUENTIAL 2, WILLNEED 3, DONTNEED 4 and
 * NOREUSE 5 returns 0, for any offset/len (len 0 = to the end of the
 * file). Other advice values (-1, 6, 7, 100) are EINVAL. advice is a
 * 32-bit int: bits 32..63 are ignored. The call changes neither the file
 * offset nor the data: DONTNEED on freshly written (dirty) data does not
 * lose it.
 */
#include "a_sys.h"

#define SYS_fadvise64 221
/* linux/fadvise.h (x86-64 values) */
#define POSIX_FADV_NORMAL 0
#define POSIX_FADV_RANDOM 1
#define POSIX_FADV_SEQUENTIAL 2
#define POSIX_FADV_WILLNEED 3
#define POSIX_FADV_DONTNEED 4
#define POSIX_FADV_NOREUSE 5

static const char *path = "/tmp/ld_fadvise64_advice";

static i64 fadv(i64 fd, i64 off, i64 len, i64 adv) { return syscall4(SYS_fadvise64, fd, off, len, adv); }

int test_main(void) {
    i64 fd = a_open(path, O_RDWR | O_CREAT | O_TRUNC, 0644);
    expect("open", fd >= 0, 1);
    char buf[10000];
    for (int i = 0; i < (int)sizeof buf; i++) buf[i] = (char)('a' + i % 26);
    expect("write", a_write(fd, buf, sizeof buf), sizeof buf);

    expect("normal_whole", fadv(fd, 0, 0, POSIX_FADV_NORMAL), 0);
    expect("random_whole", fadv(fd, 0, 0, POSIX_FADV_RANDOM), 0);
    expect("sequential_whole", fadv(fd, 0, 0, POSIX_FADV_SEQUENTIAL), 0);
    expect("willneed_range", fadv(fd, 4096, 4096, POSIX_FADV_WILLNEED), 0);
    expect("dontneed_whole", fadv(fd, 0, 0, POSIX_FADV_DONTNEED), 0);
    expect("noreuse_range", fadv(fd, 100, 50, POSIX_FADV_NOREUSE), 0);
    expect("willneed_past_eof", fadv(fd, 1 << 20, 4096, POSIX_FADV_WILLNEED), 0);
    expect("dontneed_past_eof", fadv(fd, 1 << 20, 0, POSIX_FADV_DONTNEED), 0);
    expect("negative_offset", fadv(fd, -4096, 100, POSIX_FADV_DONTNEED), 0);
    expect("huge_offset_len", fadv(fd, 0x7fffffffffffffffL, 0x7fffffffffffffffL, POSIX_FADV_DONTNEED), 0);
    expect("offset_plus_len_overflows", fadv(fd, 0x7fffffffffffffffL, 10, POSIX_FADV_WILLNEED), 0);

    expect("advice_minus1", fadv(fd, 0, 0, -1), -EINVAL);
    expect("advice_6", fadv(fd, 0, 0, 6), -EINVAL);
    expect("advice_7", fadv(fd, 0, 0, 7), -EINVAL);
    expect("advice_100", fadv(fd, 0, 0, 100), -EINVAL);
    expect("advice_high_bits_ignored", fadv(fd, 0, 0, (1L << 32) | POSIX_FADV_DONTNEED), 0);
    expect("advice_high_bits_6", fadv(fd, 0, 0, (1L << 32) | 6), -EINVAL);

    /* Nothing moved, nothing lost. */
    expect("offset_unchanged", syscall3(SYS_lseek, fd, 0, SEEK_CUR), sizeof buf);
    char back[10000];
    memset(back, 0, sizeof back);
    expect("pread_all", syscall4(SYS_pread64, fd, back, sizeof back, 0), sizeof back);
    check("data_intact", mem_eq(back, buf, sizeof buf), 0);
    struct a_stat st;
    syscall2(SYS_fstat, fd, &st);
    expect("size_unchanged", st.st_size, sizeof buf);

    expect("close", a_close(fd), 0);
    expect("unlink", a_unlink(path), 0);
    return finish();
}
