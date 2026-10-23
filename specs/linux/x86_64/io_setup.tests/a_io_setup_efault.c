/* ctx_idp is read (8 bytes) before anything else is checked: NULL, a low
 * address, a non-canonical or kernel address and a PROT_NONE page are
 * EFAULT, also when nr_events is 0, too large (EINVAL range) or over
 * the system limit (EAGAIN range). Any byte alignment is fine.
 */
#include "poly_test.h"
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_io_setup 206
#define SYS_io_destroy 207

static i64 io_setup(u64 nr, void *ctxp) { return syscall2(SYS_io_setup, nr, ctxp); }
static i64 io_destroy(u64 ctx) { return syscall1(SYS_io_destroy, ctx); }

int test_main(void) {
    i64 r = io_setup(1, 0);
    check("null_efault", r == -EFAULT, r);
    r = io_setup(1, (void *)8);
    check("low_efault", r == -EFAULT, r);
    r = io_setup(1, (void *)0x8000000000000000ULL);
    check("noncanonical_efault", r == -EFAULT, r);
    r = io_setup(1, (void *)0xffffffff81000000ULL);
    check("kernel_efault", r == -EFAULT, r);

    void *none = (void *)syscall6(SYS_mmap, 0, 4096, PROT_NONE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    check("mmap_ok", (i64)none > 0, (i64)none);
    r = io_setup(1, none);
    check("prot_none_efault", r == -EFAULT, r);
    /* the 8 bytes straddle into the PROT_NONE page */
    char *rw = (char *)syscall6(SYS_mmap, 0, 8192, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    syscall3(SYS_mprotect, rw + 4096, 4096, PROT_NONE);
    r = io_setup(1, rw + 4092);
    check("straddle_efault", r == -EFAULT, r);

    r = io_setup(0, 0);
    check("nr0_null_efault", r == -EFAULT, r);
    r = io_setup(0x400001, 0);
    check("einval_range_null_efault", r == -EFAULT, r);
    r = io_setup(65537, 0);
    check("eagain_range_null_efault", r == -EFAULT, r);
    r = io_setup(0x80000000ULL, none);
    check("eagain_wrap_prot_none_efault", r == -EFAULT, r);

    memset(rw, 0x55, 32);
    memset(rw + 3, 0, 8);
    r = io_setup(1, rw + 3);
    check("unaligned_ok", r == 0, r);
    u64 ctx;
    memcpy(&ctx, rw + 3, 8);
    check("unaligned_written", ctx != 0 && (ctx & 4095) == 0, 0);
    check("neighbours_untouched", (u8)rw[2] == 0x55 && (u8)rw[11] == 0x55, rw[11]);
    if (r == 0) check("unaligned_destroy_ok", io_destroy(ctx) == 0, 0);
    return finish();
}
