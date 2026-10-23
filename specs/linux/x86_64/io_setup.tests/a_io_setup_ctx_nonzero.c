/* The value at ctx_idp must be 0 before the call: any other value
 * (1, a live handle, all ones) is EINVAL and the value is left as it was.
 * That check comes before the nr_events limits (a non-zero value with an
 * nr_events that would be EAGAIN is still EINVAL).
 */
#include "poly_test.h"
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_io_setup 206
#define SYS_io_destroy 207

static i64 io_setup(u64 nr, void *ctxp) { return syscall2(SYS_io_setup, nr, ctxp); }
static i64 io_destroy(u64 ctx) { return syscall1(SYS_io_destroy, ctx); }

int test_main(void) {
    u64 v = 1;
    i64 r = io_setup(1, &v);
    check("one_einval", r == -EINVAL, r);
    check("one_kept", v == 1, (i64)v);

    v = ~0ULL;
    r = io_setup(1, &v);
    check("all_ones_einval", r == -EINVAL, r);
    check("all_ones_kept", v == ~0ULL, (i64)v);

    v = 1ULL << 32;
    r = io_setup(1, &v);
    check("high_half_einval", r == -EINVAL, r);

    u64 live = 0;
    r = io_setup(1, &live);
    check("setup_ok", r == 0, r);
    u64 again = live;
    r = io_setup(1, &again);
    check("live_handle_einval", r == -EINVAL, r);
    check("live_handle_kept", again == live, 0);

    v = 3;
    r = io_setup(65537, &v);
    check("before_eagain", r == -EINVAL, r);
    r = io_setup(0x80000000ULL, &v);
    check("before_eagain_wrap", r == -EINVAL, r);

    r = io_destroy(live);
    check("destroy_ok", r == 0, r);
    return finish();
}
