/* IOCB_CMD_POLL: aio_buf holds the poll events asked for. If the fd is
 * already ready the request completes at once with res = the ready events
 * among those asked (POLLIN on a pipe with data, POLLOUT on a pipe's
 * write end, POLLIN on a regular file). If not, it stays pending and
 * completes when the fd becomes ready. An empty mask never completes.
 * aio_nbytes, aio_offset, aio_rw_flags and the upper 32 bits of aio_buf
 * must be 0 (EINVAL).
 */
#include "poly_test.h"
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_io_setup 206
#define SYS_io_destroy 207
#define SYS_io_getevents 208
#define SYS_io_submit 209
#define SYS_eventfd2 290
/* linux/aio_abi.h */
#define IOCB_CMD_PREAD 0
#define IOCB_CMD_PWRITE 1
#define IOCB_CMD_FSYNC 2
#define IOCB_CMD_FDSYNC 3
#define IOCB_CMD_POLL 5
#define IOCB_CMD_NOOP 6
#define IOCB_CMD_PREADV 7
#define IOCB_CMD_PWRITEV 8
#define IOCB_FLAG_RESFD 1
#define IOCB_FLAG_IOPRIO 2

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
static A_UNUSED void set_cb(struct a_iocb *cb, u16 op, i64 fd, const void *buf, u64 n, i64 off, u64 data) {
    memset(cb, 0, sizeof *cb);
    cb->opcode = op;
    cb->fildes = (u32)fd;
    cb->buf = (u64)buf;
    cb->nbytes = n;
    cb->offset = off;
    cb->data = data;
}
/* io_submit of one iocb */
static A_UNUSED i64 submit1(u64 ctx, struct a_iocb *cb) {
    struct a_iocb *v[1] = {cb};
    return io_submit(ctx, 1, v);
}
/* takes whatever is ready, waiting at most 1 s for at least one */
static A_UNUSED i64 reap(u64 ctx, struct a_io_event *ev, i64 nr) {
    struct a_ts t = {1, 0};
    return io_getevents(ctx, 1, nr, ev, &t);
}
static A_UNUSED i64 ready(u64 ctx, struct a_io_event *ev, i64 nr) {
    struct a_ts z = {0, 0};
    return io_getevents(ctx, 0, nr, ev, &z);
}
/* one request: submit it, reap its event, return the event's res (or the submit error) */
static A_UNUSED i64 run1(u64 ctx, struct a_iocb *cb) {
    i64 r = submit1(ctx, cb);
    if (r != 1) return r == 0 ? -1000 : r;
    struct a_io_event ev[2];
    i64 g = reap(ctx, ev, 2);
    if (g != 1) return -2000 - g;
    return ev[0].res;
}
static A_UNUSED i64 file_size(i64 fd) {
    struct a_stat st;
    if (syscall2(SYS_fstat, fd, &st) != 0) return -1;
    return st.st_size;
}
#define D "/tmp/a_io_submit_poll"
/* asm-generic/poll.h */
#define POLLIN 0x0001
#define POLLOUT 0x0004
int test_main(void) {
    u64 ctx = 0;
    i64 r = io_setup(8, &ctx);
    check("setup_ok", r == 0, r);
    if (!ctx) return finish();
    int p[2];
    expect("pipe", syscall2(SYS_pipe2, p, 0), 0);
    i64 fd = a_open(D, O_RDWR | O_CREAT | O_TRUNC, 0644);
    struct a_iocb cb, pend, never;
    struct a_io_event ev[4];
    set_cb(&cb, IOCB_CMD_POLL, p[1], (void *)POLLOUT, 0, 0, 1);
    expect("pollout_ready", run1(ctx, &cb), POLLOUT);
    set_cb(&cb, IOCB_CMD_POLL, fd, (void *)POLLIN, 0, 0, 2);
    expect("file_pollin", run1(ctx, &cb), POLLIN);
    set_cb(&pend, IOCB_CMD_POLL, p[0], (void *)POLLIN, 0, 0, 3);
    expect("pending_submit", submit1(ctx, &pend), 1);
    set_cb(&never, IOCB_CMD_POLL, p[0], (void *)0, 0, 0, 4);
    expect("empty_mask_submit", submit1(ctx, &never), 1);
    expect("nothing_ready", ready(ctx, ev, 4), 0);
    expect("write", a_write(p[1], "x", 1), 1);
    expect("pending_done", reap(ctx, ev, 4), 1);
    expect("pending_data", (i64)ev[0].data, 3);
    check("pending_has_pollin", (ev[0].res & POLLIN) != 0, ev[0].res);
    set_cb(&cb, IOCB_CMD_POLL, p[0], (void *)POLLIN, 0, 0, 5);
    expect("pollin_ready_now", run1(ctx, &cb), POLLIN);
    expect("empty_mask_still_pending", ready(ctx, ev, 4), 0);
    set_cb(&cb, IOCB_CMD_POLL, p[0], (void *)POLLIN, 1, 0, 6);
    expect("nbytes_set", submit1(ctx, &cb), -EINVAL);
    set_cb(&cb, IOCB_CMD_POLL, p[0], (void *)POLLIN, 0, 1, 7);
    expect("offset_set", submit1(ctx, &cb), -EINVAL);
    set_cb(&cb, IOCB_CMD_POLL, p[0], (void *)0x100000001ull, 0, 0, 8);
    expect("buf_high_bits", submit1(ctx, &cb), -EINVAL);
    set_cb(&cb, IOCB_CMD_POLL, p[0], (void *)POLLIN, 0, 0, 9);
    cb.rw_flags = 1;
    expect("rw_flags_set", submit1(ctx, &cb), -EINVAL);
    set_cb(&cb, IOCB_CMD_POLL, 99, (void *)POLLIN, 0, 0, 10);
    expect("bad_fd", submit1(ctx, &cb), -EBADF);
    expect("no_more", ready(ctx, ev, 4), 0);
    a_close(fd);
    a_unlink(D);
    expect("destroy", io_destroy(ctx), 0);
    a_close(p[0]);
    a_close(p[1]);
    return finish();
}
