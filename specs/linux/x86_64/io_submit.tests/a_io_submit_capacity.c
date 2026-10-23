/* A context takes only so many requests whose events have not been
 * reaped (at least the nr given to io_setup). A call asking for more
 * submits what fits and returns that count; once the context is full,
 * io_submit returns EAGAIN and does nothing. After the events are reaped
 * with io_getevents, requests are accepted again. Pending requests
 * (poll on an empty pipe) and completed but unreaped ones (writes) count
 * the same. The limit itself depends on the CPU count, so the returned
 * counts differ between machines.
 * poly-diff: syscalls=off (the capacity, and so the number of calls, is machine-dependent)
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
#define D "/tmp/a_io_submit_capacity"
#define MAXN 1200
static struct a_iocb cbs[MAXN];
static struct a_iocb *v[MAXN];
static struct a_io_event ev[MAXN];
int test_main(void) {
    u64 ctx = 0;
    i64 r = io_setup(1, &ctx);
    check("setup_ok", r == 0, r);
    if (!ctx) return finish();
    i64 fd = a_open(D, O_RDWR | O_CREAT | O_TRUNC, 0644);
    for (int i = 0; i < MAXN; i++) {
        set_cb(&cbs[i], IOCB_CMD_PWRITE, fd, "w", 1, i, (u64)i);
        v[i] = &cbs[i];
    }
    i64 k = io_submit(ctx, MAXN, v);
    check("took_some", k >= 1 && k < MAXN, k);
    if (k < 1 || k >= MAXN) return finish();
    expect("full_eagain", io_submit(ctx, 1, v + k), -EAGAIN);
    expect("full_eagain_batch", io_submit(ctx, 3, v + k), -EAGAIN);
    expect("size_k", file_size(fd), k);
    i64 got = ready(ctx, ev, MAXN);
    expect("reaped_all", got, k);
    int inorder = 1;
    for (i64 i = 0; i < got; i++)
        if (ev[i].data != (u64)i) inorder = 0;
    check("events_in_order", inorder, 0);
    expect("accepts_again", io_submit(ctx, 1, v + k), 1);
    expect("its_event", ready(ctx, ev, MAXN), 1);
    i64 k1 = 0;
    for (;;) {
        i64 s = io_submit(ctx, 1, v + k1);
        if (s != 1) {
            expect("one_by_one_ends_eagain", s, -EAGAIN);
            break;
        }
        if (++k1 >= MAXN) break;
    }
    expect("one_by_one_same_limit", k1, k);
    expect("reap_again", ready(ctx, ev, MAXN), k1);
    int p[2];
    expect("pipe", syscall2(SYS_pipe2, p, 0), 0);
    for (int i = 0; i < MAXN; i++) set_cb(&cbs[i], IOCB_CMD_POLL, p[0], (void *)1, 0, 0, (u64)i);
    i64 kp = io_submit(ctx, MAXN, v);
    expect("pending_same_limit", kp, k);
    expect("pending_full", io_submit(ctx, 1, v + kp), -EAGAIN);
    expect("none_ready", ready(ctx, ev, MAXN), 0);
    expect("wake_all", a_write(p[1], "x", 1), 1);
    expect("all_complete", reap(ctx, ev, MAXN), kp);
    a_close(p[0]);
    a_close(p[1]);
    a_close(fd);
    a_unlink(D);
    expect("destroy", io_destroy(ctx), 0);
    return finish();
}
