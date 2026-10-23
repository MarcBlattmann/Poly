/* After munmap of part of a multi-page ring (a middle page and the last
 * page), the context is still alive: io_destroy of the handle returns 0
 * and removes every remaining page of the ring.
 */
#include "poly_test.h"
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_io_setup 206
#define SYS_io_destroy 207

static i64 io_setup(u64 nr, void *ctxp) { return syscall2(SYS_io_setup, nr, ctxp); }
static i64 io_destroy(u64 ctx) { return syscall1(SYS_io_destroy, ctx); }

int test_main(void) {
    u64 a = 0;
    unsigned char v[4];
    i64 r = io_setup(1000, &a);
    check("setup_ok", r == 0, r);
    if (!a) return finish();
    r = syscall2(SYS_munmap, a + 5 * 4096, 4096);
    check("munmap_middle_ok", r == 0, r);
    r = syscall2(SYS_munmap, a + 15 * 4096, 4096);
    check("munmap_last_ok", r == 0, r);
    r = syscall3(SYS_mincore, a + 4 * 4096, 4096, v);
    check("page4_mapped", r == 0, r);
    r = io_destroy(a);
    check("destroy_ok", r == 0, r);
    r = syscall3(SYS_mincore, a, 4096, v);
    check("page0_gone", r == -ENOMEM, r);
    r = syscall3(SYS_mincore, a + 4 * 4096, 4096, v);
    check("page4_gone", r == -ENOMEM, r);
    r = syscall3(SYS_mincore, a + 6 * 4096, 4096, v);
    check("page6_gone", r == -ENOMEM, r);
    r = syscall3(SYS_mincore, a + 14 * 4096, 4096, v);
    check("page14_gone", r == -ENOMEM, r);
    r = io_destroy(a);
    check("again_einval", r == -EINVAL, r);
    return finish();
}
