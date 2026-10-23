/* io_destroy gives the context's nr_events back to the system-wide
 * fs.aio-max-nr budget (65536 by default): with one context holding the
 * whole budget another io_setup is EAGAIN; after io_destroy the full
 * budget can be taken again, repeatedly.
 * Assumes fs.aio-max-nr is 65536 and no other AIO contexts are live.
 */
#include "poly_test.h"
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_io_setup 206
#define SYS_io_destroy 207

static i64 io_setup(u64 nr, void *ctxp) { return syscall2(SYS_io_setup, nr, ctxp); }
static i64 io_destroy(u64 ctx) { return syscall1(SYS_io_destroy, ctx); }

#define EAGAIN 11

int test_main(void) {
    for (int round = 0; round < 3; round++) {
        u64 big = 0, small = 0;
        i64 r = io_setup(65536, &big);
        check("full_budget_ok", r == 0, r);
        if (r != 0) return finish();
        r = io_setup(1, &small);
        check("over_budget_eagain", r == -EAGAIN, r);
        if (r == 0) io_destroy(small);
        r = io_destroy(big);
        check("destroy_ok", r == 0, r);
    }
    u64 c = 0;
    i64 r = io_setup(32768, &c);
    check("half_ok", r == 0, r);
    u64 d = 0;
    r = io_setup(32768, &d);
    check("other_half_ok", r == 0, r);
    r = io_destroy(c);
    check("destroy_half_ok", r == 0, r);
    u64 e = 0;
    r = io_setup(32768, &e);
    check("half_again_ok", r == 0, r);
    io_destroy(d);
    io_destroy(e);
    return finish();
}
