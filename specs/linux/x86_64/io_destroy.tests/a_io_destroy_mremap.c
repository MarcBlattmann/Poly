/* A ring moved with mremap (MREMAP_MAYMOVE|MREMAP_FIXED, whole ring)
 * moves the handle: io_destroy of the old address is EINVAL, of the new
 * one 0, and the ring is then gone from the new address.
 */
#include "poly_test.h"
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_io_setup 206
#define SYS_io_destroy 207

static i64 io_setup(u64 nr, void *ctxp) { return syscall2(SYS_io_setup, nr, ctxp); }
static i64 io_destroy(u64 ctx) { return syscall1(SYS_io_destroy, ctx); }

#define MREMAP_MAYMOVE 1
#define MREMAP_FIXED 2

int test_main(void) {
    u64 a = 0;
    unsigned char v[4];
    i64 r = io_setup(1, &a);
    check("setup_ok", r == 0, r);
    if (!a) return finish();
    /* reserve a target range, then move the ring onto it */
    i64 t = syscall6(SYS_mmap, 0, 3 * 4096, PROT_NONE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    check("reserve_ok", t > 0, t);
    if (t <= 0) return finish();
    u64 dst = (u64)t + 4096;
    r = syscall5(SYS_mremap, a, 4096, 4096, MREMAP_MAYMOVE | MREMAP_FIXED, dst);
    check("mremap_ok", r == (i64)dst, r);
    if (r != (i64)dst) return finish();
    check("header_moved", ((volatile u32 *)dst)[4] == 0xa10a10a1u, ((volatile u32 *)dst)[4]);
    r = io_destroy(a);
    check("old_address_einval", r == -EINVAL, r);
    r = io_destroy(dst);
    check("new_address_ok", r == 0, r);
    r = syscall3(SYS_mincore, dst, 4096, v);
    check("moved_ring_gone", r == -ENOMEM, r);
    r = io_destroy(dst);
    check("new_address_again_einval", r == -EINVAL, r);
    syscall2(SYS_munmap, t, 3 * 4096);
    return finish();
}
