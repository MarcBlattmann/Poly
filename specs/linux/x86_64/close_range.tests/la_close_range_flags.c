/* close_range flags: CLOSE_RANGE_CLOEXEC (4) does not close the range
 * but sets FD_CLOEXEC on every open descriptor in it (others keep their
 * flag); CLOSE_RANGE_UNSHARE (2) alone closes as with 0 (the table is
 * not shared here); UNSHARE | CLOEXEC sets FD_CLOEXEC. Any other flag
 * bit (1, 8, 0x80000000) is EINVAL, and first > last is EINVAL; a failed
 * call closes nothing. All three arguments are 32-bit unsigned: bits
 * above 31 are ignored (so first 2^32 + 5 means 5).
 */
#include "a_sys.h"

#define SYS_close_range 436
/* linux/close_range.h */
#define CLOSE_RANGE_UNSHARE 2
#define CLOSE_RANGE_CLOEXEC 4

static i64 cr(u64 first, u64 last, u64 flags) { return syscall3(SYS_close_range, first, last, flags); }
static i64 fdflags(int fd) { return syscall2(SYS_fcntl, fd, F_GETFD); }

static i64 open_set(void) {
    i64 s = 0;
    for (int fd = 3; fd < 16; fd++)
        if (fdflags(fd) >= 0) s |= 1L << fd;
    return s;
}

int test_main(void) {
    i64 null = a_open("/dev/null", O_RDONLY, 0);
    for (int i = 4; i <= 12; i++) syscall2(SYS_dup2, null, i);
    expect("setup", open_set(), 0x1ff8);

    expect("cloexec_5_7", cr(5, 7, CLOSE_RANGE_CLOEXEC), 0);
    expect("cloexec_kept_open", open_set(), 0x1ff8);
    expect("fd4_no_cloexec", fdflags(4), 0);
    expect("fd5_cloexec", fdflags(5), FD_CLOEXEC);
    expect("fd7_cloexec", fdflags(7), FD_CLOEXEC);
    expect("fd8_no_cloexec", fdflags(8), 0);
    expect("cloexec_to_max", cr(11, ~0U, CLOSE_RANGE_CLOEXEC), 0);
    expect("fd12_cloexec", fdflags(12), FD_CLOEXEC);

    expect("flag_1_einval", cr(8, 8, 1), -EINVAL);
    expect("flag_8_einval", cr(8, 8, 8), -EINVAL);
    expect("flag_bit31_einval", cr(8, 8, 0x80000000UL), -EINVAL);
    expect("flag_valid_plus_bad_einval", cr(8, 8, CLOSE_RANGE_CLOEXEC | 1), -EINVAL);
    expect("first_gt_last_einval", cr(9, 8, 0), -EINVAL);
    expect("first_gt_last_cloexec_einval", cr(9, 8, CLOSE_RANGE_CLOEXEC), -EINVAL);
    expect("nothing_closed_by_errors", open_set(), 0x1ff8);
    expect("fd8_flag_unchanged", fdflags(8), 0);

    expect("unshare_alone_closes", cr(8, 8, CLOSE_RANGE_UNSHARE), 0);
    expect("fd8_closed", fdflags(8), -EBADF);
    expect("unshare_cloexec", cr(9, 9, CLOSE_RANGE_UNSHARE | CLOSE_RANGE_CLOEXEC), 0);
    expect("fd9_cloexec", fdflags(9), FD_CLOEXEC);

    expect("first_upper_bits", cr((1UL << 32) | 4, 4, 0), 0);
    expect("fd4_closed", fdflags(4), -EBADF);
    expect("last_upper_bits", cr(6, (1UL << 32) | 6, 0), 0);
    expect("fd6_closed", fdflags(6), -EBADF);
    expect("flags_upper_bits", cr(10, 10, (1UL << 32) | CLOSE_RANGE_CLOEXEC), 0);
    expect("fd10_cloexec_not_closed", fdflags(10), FD_CLOEXEC);
    expect("final_set", open_set(), 0x1ea8);
    return finish();
}
