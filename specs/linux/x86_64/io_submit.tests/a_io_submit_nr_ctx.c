/* The call-level checks: nr < 0 is EINVAL (before the context is looked
 * up), an unknown context is EINVAL (0, 1, handle + 32, a destroyed
 * handle) and is checked before the iocb array is read; nr 0 with a live
 * context returns 0 without reading the array (even a bad pointer), but
 * nr 0 with an unknown context is still EINVAL. A huge nr is not an
 * error: it is cut down, and the call returns how many it took.
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
#define D "/tmp/a_io_submit_nr_ctx"
int test_main(void) {
    u64 ctx = 0;
    i64 r = io_setup(4, &ctx);
    check("setup_ok", r == 0, r);
    if (!ctx) return finish();
    i64 fd = a_open(D, O_RDWR | O_CREAT | O_TRUNC, 0644);
    struct a_iocb cb;
    set_cb(&cb, IOCB_CMD_PWRITE, fd, "a", 1, 0, 1);
    struct a_iocb *v[1] = {&cb};
    expect("nr0", io_submit(ctx, 0, v), 0);
    expect("nr0_bad_array", io_submit(ctx, 0, (void *)1), 0);
    expect("nr0_null_array", io_submit(ctx, 0, 0), 0);
    expect("nr0_bad_ctx", io_submit(0, 0, v), -EINVAL);
    expect("nr_neg", io_submit(ctx, -1, v), -EINVAL);
    expect("nr_min", io_submit(ctx, (i64)0x8000000000000000ull, v), -EINVAL);
    expect("nr_neg_bad_ctx", io_submit(0, -1, v), -EINVAL);
    expect("ctx_0", io_submit(0, 1, v), -EINVAL);
    expect("ctx_1", io_submit(1, 1, v), -EINVAL);
    expect("ctx_plus_32", io_submit(ctx + 32, 1, v), -EINVAL);
    expect("ctx_bad_array_bad", io_submit(0, 1, (void *)1), -EINVAL);
    u64 dead = 0;
    expect("setup2", io_setup(1, &dead), 0);
    expect("destroy2", io_destroy(dead), 0);
    expect("destroyed_ctx", io_submit(dead, 1, v), -EINVAL);
    struct a_io_event ev[2];
    expect("no_events_yet", ready(ctx, ev, 2), 0);
    expect("size_0", file_size(fd), 0);
    expect("huge_nr", io_submit(ctx, 0x4000000000000000ll, v), 1);
    expect("its_event", ready(ctx, ev, 2), 1);
    expect("size_1", file_size(fd), 1);
    a_close(fd);
    a_unlink(D);
    expect("destroy", io_destroy(ctx), 0);
    return finish();
}
