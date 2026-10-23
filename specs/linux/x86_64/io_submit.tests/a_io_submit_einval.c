/* Per-request EINVAL: aio_reserved2 not 0, an unknown opcode (4, NOOP 6,
 * 9, 100), a negative offset, a length of 2^63 or more, an invalid
 * aio_reqprio with IOCB_FLAG_IOPRIO, IOCB_FLAG_RESFD naming an fd that is
 * not an eventfd, and a read of a directory. Unknown aio_flags bits are
 * ignored, and aio_reqprio is ignored without IOCB_FLAG_IOPRIO. An
 * unknown aio_rw_flags bit is EOPNOTSUPP. No failing request makes an
 * event or touches the file.
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
#define D "/tmp/a_io_submit_einval"
static u64 ctx;
static i64 fd;
static struct a_iocb cb;
static void good(u64 data) { set_cb(&cb, IOCB_CMD_PWRITE, fd, "abcd", 4, 0, data); }
int test_main(void) {
    i64 r = io_setup(4, &ctx);
    check("setup_ok", r == 0, r);
    if (!ctx) return finish();
    fd = a_open(D, O_RDWR | O_CREAT | O_TRUNC, 0644);
    good(1);
    cb.reserved2 = 1;
    expect("reserved2", submit1(ctx, &cb), -EINVAL);
    good(2);
    cb.reserved2 = 0x8000000000000000ull;
    expect("reserved2_high", submit1(ctx, &cb), -EINVAL);
    static const u16 ops[] = {4, IOCB_CMD_NOOP, 9, 100, 0xffff};
    for (int i = 0; i < 5; i++) {
        good(3);
        cb.opcode = ops[i];
        check("bad_opcode", submit1(ctx, &cb) == -EINVAL, ops[i]);
    }
    good(4);
    cb.offset = -1;
    expect("write_offset_neg", submit1(ctx, &cb), -EINVAL);
    char buf[8];
    set_cb(&cb, IOCB_CMD_PREAD, fd, buf, 8, -4096, 5);
    expect("read_offset_neg", submit1(ctx, &cb), -EINVAL);
    set_cb(&cb, IOCB_CMD_PREAD, fd, buf, 0x8000000000000000ull, 0, 6);
    expect("length_2p63", submit1(ctx, &cb), -EINVAL);
    set_cb(&cb, IOCB_CMD_PREAD, fd, buf, ~0ull, 0, 7);
    expect("length_all_ones", submit1(ctx, &cb), -EINVAL);
    good(8);
    cb.flags = IOCB_FLAG_IOPRIO;
    cb.reqprio = 0xffff;
    expect("ioprio_invalid", submit1(ctx, &cb), -EINVAL);
    good(9);
    cb.flags = IOCB_FLAG_IOPRIO;
    cb.reqprio = 4 << 13;
    expect("ioprio_class_4", submit1(ctx, &cb), -EINVAL);
    good(10);
    cb.flags = IOCB_FLAG_RESFD;
    cb.resfd = (u32)fd;
    expect("resfd_not_eventfd", submit1(ctx, &cb), -EINVAL);
    good(11);
    cb.rw_flags = 0x80000000;
    expect("rw_flags_unknown", submit1(ctx, &cb), -EOPNOTSUPP);
    i64 dir = a_open("/tmp", O_RDONLY | O_DIRECTORY, 0);
    set_cb(&cb, IOCB_CMD_PREAD, dir, buf, 8, 0, 12);
    expect("read_directory", submit1(ctx, &cb), -EINVAL);
    a_close(dir);
    struct a_io_event ev[4];
    expect("no_events", ready(ctx, ev, 4), 0);
    expect("size_0", file_size(fd), 0);
    good(13);
    cb.flags = 0x80 | 4;
    expect("unknown_flags_ignored", run1(ctx, &cb), 4);
    good(14);
    cb.reqprio = 0xffff;
    expect("reqprio_without_flag", run1(ctx, &cb), 4);
    good(15);
    cb.flags = IOCB_FLAG_IOPRIO;
    cb.reqprio = (2 << 13) | 7;
    expect("ioprio_be_7", run1(ctx, &cb), 4);
    a_close(fd);
    a_unlink(D);
    expect("destroy", io_destroy(ctx), 0);
    return finish();
}
