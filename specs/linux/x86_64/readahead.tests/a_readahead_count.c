/* count is a size_t used as a signed 64-bit length: 0 up to 2^63-1 is
 * fine (any offset), 2^63 and above (negative) is EINVAL. */
#include "a_sys.h"

#define SYS_readahead 187

static i64 ra(i64 fd, i64 off, i64 count) { return syscall3(SYS_readahead, fd, off, count); }

#define F "/tmp/a_readahead_count"

int test_main(void) {
    expect("make", a_make_file(F, "0123456789", 0644), 0);
    i64 fd = a_open(F, O_RDONLY, 0);
    expect("count_0", ra(fd, 0, 0), 0);
    expect("count_0_far", ra(fd, 1L << 40, 0), 0);
    expect("count_10", ra(fd, 0, 10), 0);
    expect("count_1M", ra(fd, 0, 1 << 20), 0);
    expect("count_4G", ra(fd, 0, 1L << 32), 0);
    expect("count_max_pos", ra(fd, 0, 0x7fffffffffffffffL), 0);
    expect("count_max_pos_at_5", ra(fd, 5, 0x7fffffffffffffffL), 0);
    expect("count_2_63", ra(fd, 0, (i64)0x8000000000000000ULL), -EINVAL);
    expect("count_minus_1", ra(fd, 0, -1), -EINVAL);
    expect("count_minus_4096", ra(fd, 4096, -4096), -EINVAL);
    expect("count_minus_1_neg_off", ra(fd, -1, -1), -EINVAL);
    expect("offset_unchanged", syscall3(SYS_lseek, fd, 0, SEEK_CUR), 0);
    a_close(fd);
    expect("cleanup", a_unlink(F), 0);
    return finish();
}
