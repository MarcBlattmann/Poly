/* fs.aio-max-nr (default 65536) bounds the sum of the nr_events values
 * (as asked, not the ring's slot count) of all live contexts: with one
 * context of 40000 alive, 30000 more is EAGAIN and 25000 more is fine;
 * io_destroy gives its share back. The limit is system-wide: this
 * assumes no other process holds contexts while it runs, so it is the
 * only test of this entry that comes near the limit.
 */
#include "poly_test.h"
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_io_setup 206
#define SYS_io_destroy 207

static i64 io_setup(u64 nr, void *ctxp) { return syscall2(SYS_io_setup, nr, ctxp); }
static i64 io_destroy(u64 ctx) { return syscall1(SYS_io_destroy, ctx); }

int test_main(void) {
    u64 a = 0, b = 0, c = 0;
    i64 r = io_setup(40000, &a);
    check("first_40000_ok", r == 0, r);
    r = io_setup(30000, &b);
    check("then_30000_eagain", r == -EAGAIN, r);
    check("eagain_ctx_untouched", b == 0, (i64)b);
    r = io_setup(25536, &b);
    check("then_25536_ok", r == 0, r);
    r = io_setup(1, &c);
    check("full_then_1_eagain", r == -EAGAIN, r);

    check("destroy_25536_ok", io_destroy(b) == 0, 0);
    r = io_setup(1, &c);
    check("after_destroy_1_ok", r == 0, r);
    check("destroy_1_ok", io_destroy(c) == 0, 0);
    check("destroy_40000_ok", io_destroy(a) == 0, 0);

    /* small requests are counted as asked, not as their ring size
     * (a 1-event ring has 127 slots) */
    u64 small[4] = { 0, 0, 0, 0 };
    for (int i = 0; i < 4; i++) io_setup(1, &small[i]);
    a = 0;
    r = io_setup(65532, &a);
    check("four_small_plus_65532_ok", r == 0, r);
    c = 0;
    r = io_setup(1, &c);
    check("one_more_eagain", r == -EAGAIN, r);
    if (a) io_destroy(a);
    for (int i = 0; i < 4; i++) io_destroy(small[i]);
    return finish();
}
