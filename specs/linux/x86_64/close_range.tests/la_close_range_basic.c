/* close_range(first, last, 0) closes every open descriptor from first
 * to last inclusive and returns 0. Descriptors outside the range stay
 * open; gaps (numbers not open) are fine; a range with nothing open, or
 * starting above every open descriptor, returns 0; last = ~0U closes all
 * from first up; first == last closes one. Closed numbers are reused by
 * the next open (lowest free first).
 */
#include "a_sys.h"

#define SYS_close_range 436

static i64 cr(u64 first, u64 last, u64 flags) { return syscall3(SYS_close_range, first, last, flags); }
static int is_open(int fd) { return syscall2(SYS_fcntl, fd, F_GETFD) >= 0; }

/* Bit i set when descriptor i (3..15) is open. */
static i64 open_set(void) {
    i64 s = 0;
    for (int fd = 3; fd < 16; fd++)
        if (is_open(fd)) s |= 1L << fd;
    return s;
}

int test_main(void) {
    i64 null = a_open("/dev/null", O_RDONLY, 0);
    expect("first_fd_3", null, 3);
    for (int i = 4; i <= 12; i++) syscall2(SYS_dup2, null, i);
    expect("open_3_to_12", open_set(), 0x1ff8);

    expect("close_5_7", cr(5, 7, 0), 0);
    expect("after_5_7", open_set(), 0x1f18);
    expect("close_over_gap_4_9", cr(4, 9, 0), 0);
    expect("after_4_9", open_set(), 0x1c08);
    expect("close_nothing_open_5_8", cr(5, 8, 0), 0);
    expect("close_above_all", cr(100, 200, 0), 0);
    expect("close_single_10", cr(10, 10, 0), 0);
    expect("after_single", open_set(), 0x1808);
    expect("close_same_again", cr(10, 10, 0), 0);
    expect("reuse_lowest", syscall1(SYS_dup, null), 4);
    expect("close_to_max", cr(11, ~0U, 0), 0);
    expect("after_to_max", open_set(), 0x18);
    expect("stdio_untouched", is_open(0) + is_open(1) + is_open(2), 3);
    expect("whole_range_from_3", cr(3, ~0U, 0), 0);
    expect("all_closed", open_set(), 0);
    expect("empty_table_ok", cr(3, ~0U, 0), 0);
    return finish();
}
