/* io_getevents min_nr / nr checks: both are signed 64-bit longs; min_nr
 * < 0, nr < 0 or min_nr > nr are EINVAL (nothing is consumed); values
 * above 32 bits are used whole (2^32 is fine, 2^32+1 > 1 is EINVAL); a
 * huge nr is fine.
 */
#include "poly_test.h"
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_io_setup 206
#define SYS_io_destroy 207
#define SYS_io_getevents 208
#define SYS_io_submit 209
/* linux/aio_abi.h */
#define IOCB_CMD_PWRITE 1
#define IOCB_CMD_POLL 5
/* asm-generic/poll.h */
#define POLLIN 0x0001
#define POLLRDNORM 0x0040

/* linux/aio_abi.h: struct iocb and struct io_event (x86-64 layout) */
struct a_iocb {
    u64 data;
    u32 key;
    u32 rw_flags;
    u16 opcode;
    u16 reqprio;
    u32 fildes;
    u64 buf;
    u64 nbytes;
    i64 offset;
    u64 reserved2;
    u32 flags;
    u32 resfd;
};
struct a_io_event {
    u64 data;
    u64 obj;
    i64 res;
    i64 res2;
};
struct a_ts {
    i64 sec;
    i64 nsec;
};

static A_UNUSED i64 io_setup(u64 nr, u64 *ctxp) { return syscall2(SYS_io_setup, nr, ctxp); }
static A_UNUSED i64 io_destroy(u64 ctx) { return syscall1(SYS_io_destroy, ctx); }
static A_UNUSED i64 io_getevents(u64 ctx, i64 min_nr, i64 nr, void *events, void *timeout) {
    return syscall5(SYS_io_getevents, ctx, min_nr, nr, events, timeout);
}
static A_UNUSED i64 io_submit(u64 ctx, i64 n, struct a_iocb **v) { return syscall3(SYS_io_submit, ctx, n, v); }
static A_UNUSED i64 now_ms(void) {
    struct a_ts t;
    syscall2(SYS_clock_gettime, CLOCK_MONOTONIC, &t);
    return t.sec * 1000 + t.nsec / 1000000;
}
static char data[16];
int test_main(void) {
    u64 ctx = 0;
    i64 r = io_setup(4, &ctx);
    check("setup_ok", r == 0, r);
    if (!ctx) return finish();
    i64 fd = a_open("/tmp/a_io_getevents_nr_einval", O_RDWR | O_CREAT | O_TRUNC, 0600);
    check("open", fd >= 0, fd);
    struct a_iocb cb;
    memset(&cb, 0, sizeof cb);
    cb.data = 77;
    cb.opcode = IOCB_CMD_PWRITE;
    cb.fildes = (u32)fd;
    cb.buf = (u64)data;
    cb.nbytes = 5;
    struct a_iocb *v[1] = {&cb};
    expect("submit", io_submit(ctx, 1, v), 1);
    struct a_io_event ev[2];
    struct a_ts zero = {0, 0};
    expect("min_gt_nr", io_getevents(ctx, 2, 1, ev, &zero), -EINVAL);
    expect("min_neg", io_getevents(ctx, -1, 1, ev, &zero), -EINVAL);
    expect("nr_neg", io_getevents(ctx, 0, -1, ev, &zero), -EINVAL);
    expect("both_neg", io_getevents(ctx, -2, -1, ev, &zero), -EINVAL);
    expect("min_1_nr_0", io_getevents(ctx, 1, 0, ev, &zero), -EINVAL);
    expect("min_2p32p1_nr_1", io_getevents(ctx, 0x100000001LL, 1, ev, &zero), -EINVAL);
    expect("min_neg_null_timeout", io_getevents(ctx, -1, 1, ev, 0), -EINVAL);
    volatile u32 *ring = (volatile u32 *)ctx;
    expect("nothing_consumed", ring[2], 0);
    expect("nr0_leaves_event", io_getevents(ctx, 0, 0, ev, &zero), 0);
    expect("min_2p32_nr_2p32", io_getevents(ctx, 0x100000000LL, 0x100000000LL, ev, &zero), 1);
    expect("event_data", (i64)ev[0].data, 77);
    expect("event_res", ev[0].res, 5);
    expect("nr_max_empty", io_getevents(ctx, 0, 0x7fffffffffffffffLL, ev, &zero), 0);
    expect("min_max_nr_max_zero_timeout", io_getevents(ctx, 0x7fffffffffffffffLL, 0x7fffffffffffffffLL, ev, &zero), 0);
    a_close(fd);
    a_unlink("/tmp/a_io_getevents_nr_einval");
    expect("destroy", io_destroy(ctx), 0);
    return finish();
}
