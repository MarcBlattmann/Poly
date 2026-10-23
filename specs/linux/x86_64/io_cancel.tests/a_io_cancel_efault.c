/* io_cancel reads only the 32-bit aio_key field (byte offset 8) of the
 * iocb. If that cannot be read (NULL, address 8, PROT_NONE page, key in an
 * unmapped page, kernel or non-canonical address) the result is EFAULT,
 * and this is checked before the context: EFAULT also with ctx_id 0. An
 * iocb whose key is readable but whose rest lies in an unmapped page is
 * not EFAULT (here EINVAL: no such request). A read-only iocb can be
 * cancelled.
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
    int p[2];
    expect("pipe", syscall2(SYS_pipe2, p, 0), 0);
    struct a_io_event ev[4];
    expect("null", io_cancel(ctx, 0, 0), -EFAULT);
    expect("null_ctx0", io_cancel(0, 0, 0), -EFAULT);
    expect("addr8", io_cancel(ctx, (void *)8, 0), -EFAULT);
    expect("addr8_ctx0", io_cancel(0, (void *)8, 0), -EFAULT);
    expect("kernel_addr", io_cancel(ctx, (void *)0xffff800000000000ull, 0), -EFAULT);
    expect("noncanonical", io_cancel(ctx, (void *)0x0000800000000000ull, 0), -EFAULT);
    char *m = (char *)syscall6(SYS_mmap, 0, 3 * 4096, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    check("mmap", (i64)m > 0, (i64)m);
    if ((i64)m <= 0) return finish();
    /* page 0: iocb, page 1: unmapped, page 2: unused */
    expect("munmap_mid", syscall2(SYS_munmap, m + 4096, 4096), 0);
    expect("key_readable_rest_unmapped", io_cancel(ctx, m + 4096 - 12, 0), -EINVAL);
    expect("key_unmapped", io_cancel(ctx, m + 4096 - 8, 0), -EFAULT);
    expect("key_unmapped_ctx0", io_cancel(0, m + 4096 - 8, 0), -EFAULT);
    struct a_iocb *cb = (struct a_iocb *)m;
    expect("submit", poll_on(ctx, cb, p[0], POLLIN, 9), 1);
    expect("prot_none", syscall3(SYS_mprotect, m, 4096, PROT_NONE), 0);
    expect("cancel_prot_none", io_cancel(ctx, cb, 0), -EFAULT);
    expect("still_pending", ready(ctx, ev, 4), 0);
    expect("prot_read", syscall3(SYS_mprotect, m, 4096, PROT_READ), 0);
    expect("cancel_read_only", io_cancel(ctx, cb, 0), -EINPROGRESS);
    expect("event", reap(ctx, ev, 4), 1);
    expect("event_data", (i64)ev[0].data, 9);
    check("event_obj", ev[0].obj == (u64)cb, (i64)ev[0].obj);
    expect("event_res", ev[0].res, 0);
    expect("cancel_read_only_again", io_cancel(ctx, cb, 0), -EINVAL);
    expect("destroy", io_destroy(ctx), 0);
    syscall2(SYS_munmap, m, 4096);
    syscall2(SYS_munmap, m + 8192, 4096);
    a_close(p[0]);
    a_close(p[1]);
    return finish();
}
