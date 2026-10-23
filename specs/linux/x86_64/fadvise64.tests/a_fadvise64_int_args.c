/* fadvise64 takes fd and advice as 32-bit ints: whatever is in the upper
 * half of their registers is ignored, so fd | (1 << 32) is the same fd and
 * a negative low half is EBADF; offset and len use all 64 bits. */
#include "a_sys.h"

#define SYS_fadvise64 221
#define D "/tmp/fadvise64_int_args"

static i64 fadv(i64 fd, i64 off, i64 len, i64 advice) { return syscall4(SYS_fadvise64, fd, off, len, advice); }

int test_main(void) {
    a_mkdir(D, 0755);
    expect("make_file", a_make_file(D "/f", "int args", 0644), 0);
    i64 fd = a_open(D "/f", O_RDONLY, 0);
    check("open", fd >= 0, fd);
    expect("fd_bit32", fadv(fd | 0x100000000L, 0, 0, 0), 0);
    expect("fd_upper_all_ones", fadv(fd | (i64)0xffffffff00000000UL, 0, 0, 4), 0);
    expect("fd_low_negative", fadv(0x1ffffffffL, 0, 0, 0), -EBADF);
    expect("fd_low_unused", fadv(0x100000000L | 900, 0, 0, 0), -EBADF);
    expect("advice_bit32_dontneed", fadv(fd, 0, 0, 0x100000004L), 0);
    expect("advice_upper_noreuse", fadv(fd, 0, 0, (i64)0xffffffff00000005UL), 0);
    expect("advice_low_six", fadv(fd, 0, 0, 0x7fffffff00000006L), -EINVAL);
    expect("advice_low_negative", fadv(fd, 0, 0, 0xffffffffL), -EINVAL);
    expect("len_bit63", fadv(fd, 0, (i64)0x8000000000000001UL, 0), -EINVAL);
    expect("len_bit32_only", fadv(fd, 0, 0x100000000L, 3), 0);
    expect("len_low_half_negative", fadv(fd, 0, 0xffffffffL, 3), 0);
    expect("offset_low_half_negative", fadv(fd, 0xfffffff0L, 16, 4), 0);
    a_close(fd);
    a_unlink(D "/f");
    a_rmdir(D);
    return finish();
}
