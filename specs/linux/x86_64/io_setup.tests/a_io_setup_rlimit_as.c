/* The ring is part of the address space: when RLIMIT_AS leaves no room
 * for it, io_setup is ENOMEM and *ctx_idp stays 0; with the limit
 * restored it works again. ENOMEM comes after the nr_events checks
 * (EINVAL, EAGAIN) and before the write of the handle (a read-only 0 is
 * ENOMEM, not EFAULT).
 */
#include "poly_test.h"
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_io_setup 206
#define SYS_io_destroy 207

static i64 io_setup(u64 nr, void *ctxp) { return syscall2(SYS_io_setup, nr, ctxp); }
static i64 io_destroy(u64 ctx) { return syscall1(SYS_io_destroy, ctx); }

static const u64 zero_ro = 0;

int test_main(void) {
    struct a_rlimit old, low;
    i64 r = syscall2(SYS_getrlimit, RLIMIT_AS, &old);
    check("getrlimit_ok", r == 0, r);
    low = old;
    low.rlim_cur = 64 * 1024; /* below what the process already uses */
    r = syscall2(SYS_setrlimit, RLIMIT_AS, &low);
    check("setrlimit_ok", r == 0, r);
    u64 ctx = 0;
    r = io_setup(1, &ctx);
    check("low_as_enomem", r == -ENOMEM, r);
    check("ctx_untouched", ctx == 0, (i64)ctx);
    r = io_setup(0x400001, &ctx);
    check("low_as_einval_first", r == -EINVAL, r);
    r = io_setup(65537, &ctx);
    check("low_as_eagain_first", r == -EAGAIN, r);
    r = io_setup(1, (void *)&zero_ro);
    check("low_as_ro_enomem", r == -ENOMEM, r);
    r = syscall2(SYS_setrlimit, RLIMIT_AS, &old);
    check("restore_ok", r == 0, r);
    r = io_setup(1, &ctx);
    check("restored_ok", r == 0, r);
    if (r == 0) check("destroy_ok", io_destroy(ctx) == 0, 0);
    return finish();
}
