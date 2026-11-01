/* getgroups(size, list) returns the number of supplementary group ids of
 * the caller. size 0 only counts (list is not used, may be NULL or
 * invalid). size >= count writes count 32-bit gids and nothing after them.
 * 0 < size < count is EINVAL; a negative size is EINVAL. size is a 32-bit
 * int: (1<<32) is size 0, and 0xffffffff is -1. The list is the same on
 * every call. No privilege is needed.
 */
#include "a_sys.h"

#define SYS_getgroups 115

static u32 list[65540];

int test_main(void) {
    i64 n = syscall2(SYS_getgroups, 0, 0);
    check("count_nonnegative", n >= 0 && n <= 65536, n);
    i64 r = syscall2(SYS_getgroups, 0, (void *)8);
    check("size_0_ignores_list", r == n, r);
    r = syscall2(SYS_getgroups, 1L << 32, 0);
    check("size_high_bits_ignored", r == n, r);
    r = syscall2(SYS_getgroups, -1, list);
    check("size_minus1_einval", r == -EINVAL, r);
    r = syscall2(SYS_getgroups, 0xffffffffL, list);
    check("size_0xffffffff_einval", r == -EINVAL, r);
    r = syscall2(SYS_getgroups, -100000, list);
    check("size_negative_einval", r == -EINVAL, r);

    for (int i = 0; i < 65540; i++) list[i] = 0xa5a5a5a5;
    r = syscall2(SYS_getgroups, n, list);
    check("size_count_returns_count", r == n, r);
    check("nothing_after_count", list[n] == 0xa5a5a5a5, (i64)list[n]);

    for (int i = 0; i < 65540; i++) list[i] = 0xa5a5a5a5;
    r = syscall2(SYS_getgroups, 65540, list);
    check("size_larger_returns_count", r == n, r);
    check("larger_nothing_after_count", list[n] == 0xa5a5a5a5 && list[65539] == 0xa5a5a5a5, 0);

    static u32 again[65536];
    r = syscall2(SYS_getgroups, 65536, again);
    check("same_count_again", r == n, r);
    check("same_list_again", mem_eq(list, again, (size_t)n * 4), 0);

    if (n > 1) {
        r = syscall2(SYS_getgroups, n - 1, list);
        check("size_below_count_einval", r == -EINVAL, r);
    }
    if (n > 0) {
        r = syscall2(SYS_getgroups, 1, list);
        check("size_1_einval_or_count", n == 1 ? r == 1 : r == -EINVAL, r);
    }
    return finish();
}
