/* io_setup(1, &ctx) with ctx 0: returns 0 and writes a non-zero,
 * page-aligned handle that is the address of a readable and writable
 * ring in the caller's memory. io_destroy of that handle succeeds once;
 * a second io_destroy is EINVAL. Two contexts get different handles.
 */
#include "poly_test.h"
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_io_setup 206
#define SYS_io_destroy 207

static i64 io_setup(u64 nr, void *ctxp) { return syscall2(SYS_io_setup, nr, ctxp); }
static i64 io_destroy(u64 ctx) { return syscall1(SYS_io_destroy, ctx); }

int test_main(void) {
    u64 ctx = 0;
    i64 r = io_setup(1, &ctx);
    check("setup_ok", r == 0, r);
    if (r != 0) return finish();
    check("handle_nonzero", ctx != 0, 0);
    check("handle_page_aligned", (ctx & 4095) == 0, (i64)(ctx & 4095));
    check("handle_user_half", ctx < 0x800000000000ULL, 0);

    volatile u32 *ring = (volatile u32 *)ctx;
    u32 magic = ring[4];
    check("ring_readable", magic == 0xa10a10a1u, magic);
    u32 saved = ring[2];
    ring[2] = saved;
    check("ring_writable", ring[2] == saved, ring[2]);

    u64 ctx2 = 0;
    r = io_setup(1, &ctx2);
    check("second_setup_ok", r == 0, r);
    check("second_handle_differs", ctx2 != 0 && ctx2 != ctx, 0);

    r = io_destroy(ctx);
    check("destroy_ok", r == 0, r);
    r = io_destroy(ctx);
    check("destroy_again_einval", r == -EINVAL, r);
    r = io_destroy(ctx2);
    check("destroy_second_ok", r == 0, r);
    r = io_destroy(0);
    check("destroy_zero_einval", r == -EINVAL, r);
    return finish();
}
