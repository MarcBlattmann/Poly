/* close: many descriptors, closed in any order; numbers are always handed
 * out lowest first. */
#include "a_sys.h"

#define F "/tmp/a_close_many"

int test_main(void) {
    i64 fds[20];
    expect("setup_file", a_make_file(F, "", 0644), 0);
    int sequential = 1;
    for (int i = 0; i < 20; i++) {
        fds[i] = a_open(F, O_RDONLY, 0);
        sequential &= fds[i] == 3 + i;
    }
    check("twenty_opens_are_3_to_22", sequential, 0);
    /* Close the odd ones, then reopen: the holes fill from the bottom. */
    int closed = 1;
    for (int i = 1; i < 20; i += 2) closed &= a_close(fds[i]) == 0;
    check("close_odd_ones", closed, 0);
    int refill = 1;
    for (int i = 1; i < 20; i += 2) refill &= a_open(F, O_RDONLY, 0) == 3 + i;
    check("holes_refilled_lowest_first", refill, 0);
    int all = 1;
    for (int i = 19; i >= 0; i--) all &= a_close(3 + i) == 0;
    check("close_all_backwards", all, 0);
    expect("all_closed_next_is_3", a_open(F, O_RDONLY, 0), 3);
    expect("close_3", a_close(3), 0);
    int bad = 1;
    for (int i = 3; i < 23; i++) bad &= a_close(i) == -EBADF;
    check("closed_ones_ebadf", bad, 0);
    expect("cleanup_unlink", a_unlink(F), 0);
    return finish();
}
