/* The ring a new context's handle points at starts with a 32-byte header
 * of eight 32-bit words: context index, number of event slots, head 0,
 * tail 0, magic 0xa10a10a1, compat features 1, incompat features 0,
 * header length 32. The slot count fills whole pages of 32-byte slots
 * after the header: room for at least 2 * max(nr_events, 4 * possible
 * CPUs) + 2 slots, so (pages * 4096 - 32) / 32. The values checked here
 * hold for any machine with fewer than 16 CPUs.
 */
#include "poly_test.h"
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_io_setup 206
#define SYS_io_destroy 207

static i64 io_setup(u64 nr, void *ctxp) { return syscall2(SYS_io_setup, nr, ctxp); }
static i64 io_destroy(u64 ctx) { return syscall1(SYS_io_destroy, ctx); }

static void slots(const char *name, u64 nr, u32 want) {
    u64 ctx = 0;
    i64 r = io_setup(nr, &ctx);
    if (r != 0) {
        check(name, 0, r);
        return;
    }
    u32 got = ((volatile u32 *)ctx)[1];
    check(name, got == want, got);
    io_destroy(ctx);
}

int test_main(void) {
    u64 ctx = 0;
    i64 r = io_setup(1, &ctx);
    check("setup_ok", r == 0, r);
    if (r != 0) return finish();
    volatile u32 *h = (volatile u32 *)ctx;
    check("index_0", h[0] == 0, h[0]);
    check("nr_one_page", h[1] == 127, h[1]);
    check("head_0", h[2] == 0, h[2]);
    check("tail_0", h[3] == 0, h[3]);
    check("magic", h[4] == 0xa10a10a1u, h[4]);
    check("compat_1", h[5] == 1, h[5]);
    check("incompat_0", h[6] == 0, h[6]);
    check("header_len_32", h[7] == 32, h[7]);
    check("first_slot_zero", h[8] == 0 && h[15] == 0, h[8]);
    check("destroy_ok", io_destroy(ctx) == 0, 0);

    slots("nr_2", 2, 127);
    slots("nr_126", 126, 255);
    slots("nr_127", 127, 383);
    slots("nr_1000", 1000, 2047);
    slots("nr_4096", 4096, 8319);
    return finish();
}
