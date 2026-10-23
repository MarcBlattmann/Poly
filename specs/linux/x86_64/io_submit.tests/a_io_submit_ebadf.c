/* EBADF: aio_fildes is not open (99, -1), not open for the direction
 * asked (write on O_RDONLY, read on O_WRONLY), or an O_PATH fd; and
 * IOCB_FLAG_RESFD with an aio_resfd that is not open. Check order inside
 * one iocb: aio_reserved2 (EINVAL) before the fd (EBADF), the fd before
 * the opcode and the offset (EINVAL), and a bad resfd (EBADF) before an
 * invalid priority (EINVAL). A request rejected for aio_reserved2 or its
 * fd keeps its aio_key; one rejected later has aio_key set to 0.
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
#define D "/tmp/a_io_submit_ebadf"
int test_main(void) {
    u64 ctx = 0;
    i64 r = io_setup(4, &ctx);
    check("setup_ok", r == 0, r);
    if (!ctx) return finish();
    i64 fd = a_open(D, O_RDWR | O_CREAT | O_TRUNC, 0644);
    i64 ro = a_open(D, O_RDONLY, 0);
    i64 wo = a_open(D, O_WRONLY, 0);
    i64 pa = a_open(D, O_PATH, 0);
    check("opens", fd >= 0 && ro >= 0 && wo >= 0 && pa >= 0, pa);
    struct a_iocb cb;
    char buf[8];
    set_cb(&cb, IOCB_CMD_PWRITE, 99, "a", 1, 0, 1);
    expect("fd_not_open", submit1(ctx, &cb), -EBADF);
    set_cb(&cb, IOCB_CMD_PWRITE, -1, "a", 1, 0, 1);
    expect("fd_minus_1", submit1(ctx, &cb), -EBADF);
    set_cb(&cb, IOCB_CMD_PWRITE, ro, "a", 1, 0, 1);
    expect("write_on_rdonly", submit1(ctx, &cb), -EBADF);
    set_cb(&cb, IOCB_CMD_PREAD, wo, buf, 1, 0, 1);
    expect("read_on_wronly", submit1(ctx, &cb), -EBADF);
    set_cb(&cb, IOCB_CMD_PREAD, pa, buf, 1, 0, 1);
    expect("read_on_opath", submit1(ctx, &cb), -EBADF);
    set_cb(&cb, IOCB_CMD_PWRITE, fd, "a", 1, 0, 1);
    cb.flags = IOCB_FLAG_RESFD;
    cb.resfd = 99;
    expect("resfd_not_open", submit1(ctx, &cb), -EBADF);
    set_cb(&cb, IOCB_CMD_PWRITE, 99, "a", 1, 0, 1);
    cb.reserved2 = 1;
    cb.key = 5;
    expect("reserved2_before_fd", submit1(ctx, &cb), -EINVAL);
    expect("key_kept_reserved2", cb.key, 5);
    set_cb(&cb, 77, 99, "a", 1, 0, 1);
    cb.key = 5;
    expect("fd_before_opcode", submit1(ctx, &cb), -EBADF);
    expect("key_kept_fd", cb.key, 5);
    set_cb(&cb, IOCB_CMD_PWRITE, 99, "a", 1, -1, 1);
    expect("fd_before_offset", submit1(ctx, &cb), -EBADF);
    set_cb(&cb, IOCB_CMD_PWRITE, fd, "a", 1, 0, 1);
    cb.flags = IOCB_FLAG_RESFD | IOCB_FLAG_IOPRIO;
    cb.resfd = 99;
    cb.reqprio = 0xffff;
    expect("resfd_before_ioprio", submit1(ctx, &cb), -EBADF);
    set_cb(&cb, 77, fd, "a", 1, 0, 1);
    cb.key = 5;
    expect("opcode_bad", submit1(ctx, &cb), -EINVAL);
    expect("key_cleared_opcode", cb.key, 0);
    set_cb(&cb, IOCB_CMD_PWRITE, fd, "a", 1, -1, 1);
    cb.key = 5;
    expect("offset_bad", submit1(ctx, &cb), -EINVAL);
    expect("key_cleared_offset", cb.key, 0);
    struct a_io_event ev[2];
    expect("no_events", ready(ctx, ev, 2), 0);
    expect("size_0", file_size(fd), 0);
    a_close(pa);
    a_close(wo);
    a_close(ro);
    a_close(fd);
    a_unlink(D);
    expect("destroy", io_destroy(ctx), 0);
    return finish();
}
