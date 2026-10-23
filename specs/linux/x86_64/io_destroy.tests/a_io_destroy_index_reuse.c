/* io_destroy frees the context's index (header word 0): the next
 * io_setup takes the lowest free index. Contexts 0..3 are made; 2 and 0
 * are destroyed; the next two contexts get 0 and then 2, the one after
 * that 4.
 */
#include "poly_test.h"
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_io_setup 206
#define SYS_io_destroy 207

static i64 io_setup(u64 nr, void *ctxp) { return syscall2(SYS_io_setup, nr, ctxp); }
static i64 io_destroy(u64 ctx) { return syscall1(SYS_io_destroy, ctx); }

static u32 idx(u64 c) { return ((volatile u32 *)c)[0]; }

int test_main(void) {
    u64 c[4] = {0}, n0 = 0, n1 = 0, n2 = 0;
    for (int i = 0; i < 4; i++) {
        i64 r = io_setup(1, &c[i]);
        check("setup_ok", r == 0, r);
        if (r != 0) return finish();
        check("index_in_order", idx(c[i]) == (u32)i, idx(c[i]));
    }
    i64 r = io_destroy(c[2]);
    check("destroy_2_ok", r == 0, r);
    r = io_destroy(c[0]);
    check("destroy_0_ok", r == 0, r);
    r = io_setup(1, &n0);
    check("new0_ok", r == 0, r);
    check("new0_index_0", n0 && idx(n0) == 0, n0 ? idx(n0) : 99);
    r = io_setup(1, &n1);
    check("new1_ok", r == 0, r);
    check("new1_index_2", n1 && idx(n1) == 2, n1 ? idx(n1) : 99);
    r = io_setup(1, &n2);
    check("new2_ok", r == 0, r);
    check("new2_index_4", n2 && idx(n2) == 4, n2 ? idx(n2) : 99);
    u64 all[6] = {c[1], c[3], n0, n1, n2, 0};
    for (int i = 0; i < 5; i++) {
        r = io_destroy(all[i]);
        check("destroy_rest_ok", r == 0, r);
    }
    return finish();
}
