/* A nonzero size smaller than the number of groups is EINVAL and nothing
 * is written. With no supplementary groups any size is fine and returns 0.
 * Sizes larger than the count, up to INT_MAX, work with a buffer that
 * only holds the count.
 */
#include "poly_ipc.h"

static unsigned buf[65540];

int test_main(void) {
    i64 n = syscall2(SYS_getgroups, 0, 0);
    for (int i = 0; i < 65540; i++) buf[i] = 0x5a5a5a5a;
    /* Size count - 1: EINVAL with two or more groups; with one group it is
     * size 0, the count query; with none, use size 1 instead. */
    i64 r = syscall2(SYS_getgroups, n > 0 ? n - 1 : 1, buf);
    check("one_short", n >= 2 ? r == -EINVAL : r == n, r);
    check("one_short_nothing_written", buf[0] == 0x5a5a5a5a, (i64)buf[0]);
    r = syscall2(SYS_getgroups, 1, buf);
    check("size_one", n >= 2 ? r == -EINVAL : r == n, r);
    check("size_one_writes_only_if_fits", n == 1 || buf[0] == 0x5a5a5a5a, (i64)buf[0]);
    check("size_one_second_slot_untouched", buf[1] == 0x5a5a5a5a, (i64)buf[1]);
    for (int i = 0; i < 65540; i++) buf[i] = 0x5a5a5a5a;
    r = syscall2(SYS_getgroups, 0x7fffffff, buf);
    check("int_max_size_ok", r == n, r);
    check("int_max_size_tail_untouched", buf[n] == 0x5a5a5a5a, (i64)buf[n]);
    r = syscall2(SYS_getgroups, 65536, buf);
    check("ngroups_max_size_ok", r == n, r);
    r = syscall2(SYS_getgroups, 65537, buf);
    check("above_ngroups_max_size_ok", r == n, r);
    return finish();
}
