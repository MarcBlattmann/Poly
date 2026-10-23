/* Several pending POLL requests on one context: cancelling one leaves the
 * others pending (one still completes normally when its fd becomes
 * ready); each cancelled request yields exactly one res 0 event, in any
 * order relative to each other. A context with only cancelled requests
 * reaped is empty again.
 */
#include "poly_test.h"
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_io_setup 206
#define SYS_io_destroy 207
#define SYS_io_getevents 208
#define SYS_io_submit 209
#define SYS_io_cancel 210
#define SYS_eventfd2 290
/* linux/aio_abi.h */
#define IOCB_CMD_PWRITE 1
#define IOCB_CMD_POLL 5
#define IOCB_FLAG_RESFD 1
/* asm-generic/errno.h */
#ifndef EINPROGRESS
#define EINPROGRESS 115
#endif
/* asm-generic/poll.h */
#define POLLIN 0x0001
#define POLLOUT 0x0004

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
static A_UNUSED i64 io_submit(u64 ctx, i64 n, void *v) { return syscall3(SYS_io_submit, ctx, n, v); }
static A_UNUSED i64 io_cancel(u64 ctx, const void *iocb, void *result) {
    return syscall3(SYS_io_cancel, ctx, iocb, result);
}
/* io_submit of one iocb */
static A_UNUSED i64 submit1(u64 ctx, struct a_iocb *cb) {
    struct a_iocb *v[1] = {cb};
    return io_submit(ctx, 1, v);
}
/* submits a POLL request for `events` on fd */
static A_UNUSED i64 poll_on(u64 ctx, struct a_iocb *cb, i64 fd, u64 events, u64 data) {
    memset(cb, 0, sizeof *cb);
    cb->opcode = IOCB_CMD_POLL;
    cb->fildes = (u32)fd;
    cb->buf = events;
    cb->data = data;
    return submit1(ctx, cb);
}
/* waits at most 1 s for at least one event (a cancelled request's event
 * arrives shortly after io_cancel returns, not necessarily before) */
static A_UNUSED i64 reap(u64 ctx, struct a_io_event *ev, i64 nr) {
    struct a_ts t = {1, 0};
    return io_getevents(ctx, 1, nr, ev, &t);
}
/* events ready now, without waiting */
static A_UNUSED i64 ready(u64 ctx, struct a_io_event *ev, i64 nr) {
    struct a_ts z = {0, 0};
    return io_getevents(ctx, 0, nr, ev, &z);
}
/* waits up to 1 s until n events have arrived; returns how many came */
static A_UNUSED i64 reap_n(u64 ctx, struct a_io_event *ev, i64 n) {
    struct a_ts t = {1, 0};
    i64 got = 0;
    for (int i = 0; i < 4 && got < n; i++) {
        i64 r = io_getevents(ctx, n - got, n - got, ev + got, &t);
        if (r <= 0) break;
        got += r;
    }
    return got;
}
int test_main(void) {
    u64 ctx = 0;
    i64 r = io_setup(8, &ctx);
    check("setup_ok", r == 0, r);
    if (!ctx) return finish();
    int p[4][2];
    for (int i = 0; i < 4; i++) expect("pipe", syscall2(SYS_pipe2, p[i], 0), 0);
    struct a_iocb cb[4];
    struct a_io_event ev[8];
    for (int i = 0; i < 4; i++) expect("submit", poll_on(ctx, &cb[i], p[i][0], POLLIN, 80 + i), 1);
    expect("cancel_1", io_cancel(ctx, &cb[1], 0), -EINPROGRESS);
    expect("event_1", reap(ctx, ev, 8), 1);
    expect("event_1_data", (i64)ev[0].data, 81);
    expect("event_1_res", ev[0].res, 0);
    expect("others_pending", ready(ctx, ev, 8), 0);
    expect("write_2", a_write(p[2][1], "x", 1), 1);
    expect("event_2", reap(ctx, ev, 8), 1);
    expect("event_2_data", (i64)ev[0].data, 82);
    check("event_2_pollin", (ev[0].res & POLLIN) != 0, ev[0].res);
    expect("cancel_2_done", io_cancel(ctx, &cb[2], 0), -EINVAL);
    expect("cancel_1_again", io_cancel(ctx, &cb[1], 0), -EINVAL);
    expect("cancel_3", io_cancel(ctx, &cb[3], 0), -EINPROGRESS);
    expect("cancel_0", io_cancel(ctx, &cb[0], 0), -EINPROGRESS);
    expect("two_events", reap_n(ctx, ev, 2), 2);
    int seen0 = 0, seen3 = 0, res_ok = 1;
    for (int i = 0; i < 2; i++) {
        if (ev[i].data == 80 && ev[i].obj == (u64)&cb[0]) seen0++;
        if (ev[i].data == 83 && ev[i].obj == (u64)&cb[3]) seen3++;
        if (ev[i].res != 0 || ev[i].res2 != 0) res_ok = 0;
    }
    expect("seen_0", seen0, 1);
    expect("seen_3", seen3, 1);
    expect("res_zero", res_ok, 1);
    for (int i = 0; i < 4; i++) a_write(p[i][1], "y", 1);
    expect("nothing_left", ready(ctx, ev, 8), 0);
    expect("destroy", io_destroy(ctx), 0);
    for (int i = 0; i < 4; i++) {
        a_close(p[i][0]);
        a_close(p[i][1]);
    }
    return finish();
}
