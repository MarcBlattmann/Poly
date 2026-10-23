/* Values that are not the exact start address of a live ring are EINVAL
 * and leave every context alive: small numbers, all ones, a page inside
 * a multi-page ring, the handle plus one, the handle with bit 32 flipped
 * or bit 63 set, a stack page, and an ordinary page holding a copy of a
 * ring header.
 */
#include "poly_test.h"
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_io_setup 206
#define SYS_io_destroy 207

static i64 io_setup(u64 nr, void *ctxp) { return syscall2(SYS_io_setup, nr, ctxp); }
static i64 io_destroy(u64 ctx) { return syscall1(SYS_io_destroy, ctx); }

int test_main(void) {
    u64 a = 0, b = 0, x = 0;
    i64 r = io_setup(1, &a);
    check("setup_a_ok", r == 0, r);
    r = io_setup(1000, &b);
    check("setup_b_ok", r == 0, r);
    if (!a || !b) return finish();

    r = io_destroy(1);
    check("one_einval", r == -EINVAL, r);
    r = io_destroy(4096);
    check("page_4096_einval", r == -EINVAL, r);
    r = io_destroy(~0UL);
    check("all_ones_einval", r == -EINVAL, r);
    r = io_destroy(a + 1);
    check("handle_plus_1_einval", r == -EINVAL, r);
    r = io_destroy(a + 32);
    check("handle_plus_32_einval", r == -EINVAL, r);
    r = io_destroy(b + 4096);
    check("second_ring_page_einval", r == -EINVAL, r);
    r = io_destroy(b + 15 * 4096);
    check("last_ring_page_einval", r == -EINVAL, r);
    r = io_destroy(a ^ (1UL << 32));
    check("bit32_flipped_einval", r == -EINVAL, r);
    r = io_destroy(a | (1UL << 63));
    check("bit63_set_einval", r == -EINVAL, r);
    r = io_destroy(((u64)&x) & ~4095UL);
    check("stack_page_einval", r == -EINVAL, r);

    u32 *m = (u32 *)syscall6(SYS_mmap, 0, 4096, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    check("mmap_ok", (i64)m > 0, (i64)m);
    if ((i64)m > 0) {
        for (int i = 0; i < 8; i++) m[i] = ((volatile u32 *)a)[i];
        r = io_destroy((u64)m);
        check("header_copy_einval", r == -EINVAL, r);
        syscall2(SYS_munmap, m, 4096);
    }

    r = io_destroy(a);
    check("a_still_alive", r == 0, r);
    r = io_destroy(b);
    check("b_still_alive", r == 0, r);
    return finish();
}
