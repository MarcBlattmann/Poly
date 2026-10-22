/* getgroups(0, list) returns the number of supplementary groups and does
 * not look at the buffer: NULL, a bad pointer and a real buffer all give
 * the same count, which never changes between calls and is at most 65536.
 * The buffer is not written when the size is 0.
 */
#include "poly_ipc.h"

int test_main(void) {
    i64 n = syscall2(SYS_getgroups, 0, 0);
    check("count_nonnegative", n >= 0 && n <= 65536, n);
    unsigned buf[4] = { 0xdeadbeef, 0xdeadbeef, 0xdeadbeef, 0xdeadbeef };
    i64 r = syscall2(SYS_getgroups, 0, buf);
    check("count_with_buffer_same", r == n, r);
    check("size_zero_buffer_untouched", buf[0] == 0xdeadbeef && buf[3] == 0xdeadbeef, (i64)buf[0]);
    r = syscall2(SYS_getgroups, 0, 8);
    check("count_with_bad_pointer_same", r == n, r);
    r = syscall2(SYS_getgroups, 0, -1L);
    check("count_with_kernel_pointer_same", r == n, r);
    int same = 1;
    for (int i = 0; i < 100; i++)
        if (syscall2(SYS_getgroups, 0, 0) != n) same = 0;
    check("count_stable", same, 0);
    return finish();
}
