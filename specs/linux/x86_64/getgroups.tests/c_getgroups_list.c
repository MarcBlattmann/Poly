/* getgroups(size, list) with size >= count writes exactly `count` gids
 * (ascending, unsigned order) to the start of the buffer, returns the
 * count and leaves the rest of the buffer alone. Two calls give the same
 * list; size equal to the count works.
 */
#include "poly_ipc.h"

static unsigned a[70000], b[70000];

int test_main(void) {
    i64 n = syscall2(SYS_getgroups, 0, 0);
    check("count_ok", n >= 0 && n <= 65536, n);
    for (int i = 0; i < 70000; i++) a[i] = b[i] = 0xa5a5a5a5;
    i64 r = syscall2(SYS_getgroups, 70000, a);
    check("returns_count", r == n, r);
    check("tail_untouched", a[n] == 0xa5a5a5a5 && a[n + 1] == 0xa5a5a5a5 && a[69999] == 0xa5a5a5a5, (i64)a[n]);
    int sorted = 1;
    for (i64 i = 1; i < n; i++)
        if (a[i - 1] > a[i]) sorted = 0;
    check("ascending", sorted, 0);
    r = syscall2(SYS_getgroups, n, b);
    check("exact_size_returns_count", r == n, r);
    check("exact_size_same_list", mem_eq(a, b, (u64)n * 4), 0);
    check("exact_size_tail_untouched", b[n] == 0xa5a5a5a5, (i64)b[n]);
    return finish();
}
