/* The ring is ordinary shared memory to the caller: it can write the
 * header (even the slot count and magic) and io_destroy still succeeds;
 * mprotect to read-only is allowed, but asking for PROT_EXEC is EACCES.
 */
#include "poly_test.h"
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_io_setup 206
#define SYS_io_destroy 207
/* asm-generic/mman-common.h */
#define PROT_EXEC 0x4

static i64 io_setup(u64 nr, void *ctxp) { return syscall2(SYS_io_setup, nr, ctxp); }
static i64 io_destroy(u64 ctx) { return syscall1(SYS_io_destroy, ctx); }

int test_main(void) {
    u64 a = 0;
    i64 r = io_setup(1, &a);
    check("setup_ok", r == 0, r);
    if (r != 0) return finish();
    volatile u32 *h = (volatile u32 *)a;
    h[1] = 0;
    h[4] = 0;
    h[100] = 0x1234;
    check("write_visible", h[100] == 0x1234 && h[4] == 0, h[100]);
    r = io_destroy(a);
    check("destroy_after_scribble_ok", r == 0, r);

    u64 b = 0;
    r = io_setup(1, &b);
    check("setup_b_ok", r == 0, r);
    r = syscall3(SYS_mprotect, b, 4096, PROT_READ);
    check("mprotect_ro_ok", r == 0, r);
    check("still_readable", ((volatile u32 *)b)[4] == 0xa10a10a1u, ((volatile u32 *)b)[4]);
    r = syscall3(SYS_mprotect, b, 4096, PROT_READ | PROT_EXEC);
    check("mprotect_exec_eacces", r == -EACCES, r);
    r = syscall3(SYS_mprotect, b, 4096, PROT_READ | PROT_WRITE);
    check("mprotect_rw_ok", r == 0, r);
    r = io_destroy(b);
    check("destroy_b_ok", r == 0, r);
    return finish();
}
