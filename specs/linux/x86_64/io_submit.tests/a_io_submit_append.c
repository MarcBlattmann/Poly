/* Writes to an fd opened O_APPEND go to the end of the file whatever
 * aio_offset says, and so does a write with RWF_APPEND (0x10) in
 * aio_rw_flags on an ordinary fd. RWF_HIPRI (1), RWF_DSYNC (2) and
 * RWF_SYNC (4) are accepted on a regular file and write normally. The
 * fd's own offset never moves.
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
#define D "/tmp/a_io_submit_append"
/* linux/fs.h */
#define RWF_HIPRI 0x1
#define RWF_DSYNC 0x2
#define RWF_SYNC 0x4
#define RWF_APPEND 0x10
int test_main(void) {
    u64 ctx = 0;
    i64 r = io_setup(4, &ctx);
    check("setup_ok", r == 0, r);
    if (!ctx) return finish();
    i64 fd = a_open(D, O_RDWR | O_CREAT | O_TRUNC, 0644);
    expect("fill", a_write(fd, "0123", 4), 4);
    i64 ap = a_open(D, O_WRONLY | O_APPEND, 0);
    struct a_iocb cb;
    set_cb(&cb, IOCB_CMD_PWRITE, ap, "A", 1, 0, 1);
    expect("append_fd", run1(ctx, &cb), 1);
    expect("size_5", file_size(fd), 5);
    set_cb(&cb, IOCB_CMD_PWRITE, ap, "B", 1, 100, 2);
    expect("append_fd_far_offset", run1(ctx, &cb), 1);
    expect("size_6", file_size(fd), 6);
    set_cb(&cb, IOCB_CMD_PWRITE, fd, "C", 1, 1, 3);
    cb.rw_flags = RWF_APPEND;
    expect("rwf_append", run1(ctx, &cb), 1);
    expect("size_7", file_size(fd), 7);
    set_cb(&cb, IOCB_CMD_PWRITE, fd, "h", 1, 1, 4);
    cb.rw_flags = RWF_HIPRI;
    expect("rwf_hipri", run1(ctx, &cb), 1);
    set_cb(&cb, IOCB_CMD_PWRITE, fd, "d", 1, 2, 5);
    cb.rw_flags = RWF_DSYNC;
    expect("rwf_dsync", run1(ctx, &cb), 1);
    set_cb(&cb, IOCB_CMD_PWRITE, fd, "s", 1, 3, 6);
    cb.rw_flags = RWF_SYNC;
    expect("rwf_sync", run1(ctx, &cb), 1);
    char buf[8];
    expect("pread", syscall4(SYS_pread64, fd, buf, 8, 0), 7);
    check("content", mem_eq(buf, "0hdsABC", 7), buf[1]);
    expect("fd_offset", syscall3(SYS_lseek, fd, 0, SEEK_CUR), 4);
    expect("ap_offset", syscall3(SYS_lseek, ap, 0, SEEK_CUR), 0);
    a_close(ap);
    a_close(fd);
    a_unlink(D);
    expect("destroy", io_destroy(ctx), 0);
    return finish();
}
