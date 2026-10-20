/* brk: query the break, grow it, use the new memory, shrink it again.
 * brk(2): the raw syscall returns the new break on success and the current
 * break on failure. */
#include "poly_test.h"

#define GROW (64 * 1024)

int test_main(void) {
    i64 start = syscall1(SYS_brk, 0);
    check("brk_query_returns_break", start > 0, start);
    check("brk_query_twice_is_stable", syscall1(SYS_brk, 0) == start, 0);

    i64 grown = syscall1(SYS_brk, start + GROW);
    check("brk_grow_returns_new_break", grown == start + GROW, grown - start);

    /* The new memory is zeroed and writable. */
    volatile unsigned char *p = (volatile unsigned char *)start;
    int zero = 1;
    for (int i = 0; i < GROW; i += 512) zero &= p[i] == 0;
    check("brk_new_memory_is_zero", zero, 0);
    for (int i = 0; i < GROW; i += 512) p[i] = (unsigned char)i;
    int same = 1;
    for (int i = 0; i < GROW; i += 512) same &= p[i] == (unsigned char)i;
    check("brk_new_memory_is_writable", same, 0);

    i64 shrunk = syscall1(SYS_brk, start);
    check("brk_shrink_returns_old_break", shrunk == start, shrunk - start);

    /* Below the start of the heap fails: the break stays where it is. */
    i64 low = syscall1(SYS_brk, 4096);
    check("brk_below_heap_start_fails", low == start, low == start);

    /* Absurdly large fails the same way. */
    i64 huge = syscall1(SYS_brk, 0x7ffffffff000L);
    check("brk_huge_fails", huge == start, huge == start);
    return finish();
}
