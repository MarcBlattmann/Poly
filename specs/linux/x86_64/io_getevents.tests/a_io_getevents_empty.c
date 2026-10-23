/* io_getevents on a context with no completed requests: with a zero
 * timeout it returns 0 at once whatever min_nr is; with min_nr 0 it
 * returns 0 at once even without a timeout; nr 0 returns 0. The events
 * buffer is never touched when there is nothing to copy (an invalid
 * pointer still gives 0).
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
int test_main(void) {
    u64 ctx = 0;
    i64 r = io_setup(4, &ctx);
    check("setup_ok", r == 0, r);
    if (!ctx) return finish();
    struct a_io_event ev[4];
    memset(ev, 0x5a, sizeof ev);
    struct a_ts zero = {0, 0};
    expect("min0_zero_timeout", io_getevents(ctx, 0, 4, ev, &zero), 0);
    expect("min1_zero_timeout", io_getevents(ctx, 1, 4, ev, &zero), 0);
    expect("min4_zero_timeout", io_getevents(ctx, 4, 4, ev, &zero), 0);
    expect("min0_null_timeout", io_getevents(ctx, 0, 4, ev, 0), 0);
    expect("nr0_min0", io_getevents(ctx, 0, 0, ev, &zero), 0);
    expect("nr0_null_timeout", io_getevents(ctx, 0, 0, ev, 0), 0);
    expect("nr0_null_events", io_getevents(ctx, 0, 0, 0, &zero), 0);
    expect("bad_events_nothing_to_copy", io_getevents(ctx, 0, 4, (void *)8, &zero), 0);
    expect("null_events_nothing_to_copy", io_getevents(ctx, 1, 4, 0, &zero), 0);
    check("events_untouched", ev[0].data == 0x5a5a5a5a5a5a5a5aULL && ev[3].res2 == 0x5a5a5a5a5a5a5a5aLL, ev[0].res);
    struct a_ts one_ns = {0, 1};
    expect("one_ns_timeout", io_getevents(ctx, 1, 1, ev, &one_ns), 0);
    volatile u32 *ring = (volatile u32 *)ctx;
    expect("ring_head_still_0", ring[2], 0);
    expect("ring_tail_still_0", ring[3], 0);
    expect("destroy", io_destroy(ctx), 0);
    return finish();
}
