/* 40 live contexts can be destroyed in any order (odd ones first, then
 * even ones backwards): each io_destroy returns 0 once and EINVAL for a
 * repeat, and the not yet destroyed ones stay valid throughout.
 */
#include "poly_test.h"
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_io_setup 206
#define SYS_io_destroy 207

static i64 io_setup(u64 nr, void *ctxp) { return syscall2(SYS_io_setup, nr, ctxp); }
static i64 io_destroy(u64 ctx) { return syscall1(SYS_io_destroy, ctx); }

#define N 40

int test_main(void) {
    static u64 c[N];
    int made = 0;
    for (int i = 0; i < N; i++) {
        i64 r = io_setup(2, &c[i]);
        if (r != 0) {
            check("setup_ok", 0, r);
            break;
        }
        made++;
    }
    check("all_made", made == N, made);
    int ok = 1, again = 1;
    for (int i = 1; i < made; i += 2) {
        if (io_destroy(c[i]) != 0) ok = 0;
        if (io_destroy(c[i]) != -EINVAL) again = 0;
    }
    check("odd_destroyed", ok, 0);
    check("odd_repeat_einval", again, 0);
    int intact = 1;
    for (int i = 0; i < made; i += 2)
        if (((volatile u32 *)c[i])[4] != 0xa10a10a1u) intact = 0;
    check("even_rings_intact", intact, 0);
    ok = 1;
    again = 1;
    for (int i = (made - 1) & ~1; i >= 0; i -= 2) {
        if (io_destroy(c[i]) != 0) ok = 0;
        if (io_destroy(c[i]) != -EINVAL) again = 0;
    }
    check("even_destroyed", ok, 0);
    check("even_repeat_einval", again, 0);
    return finish();
}
