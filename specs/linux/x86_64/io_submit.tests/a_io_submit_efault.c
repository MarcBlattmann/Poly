/* EFAULT from io_submit: the pointer array is unreadable, an entry is
 * NULL or points to unreadable memory, the iocb runs into an unmapped
 * page, or the iocb is read-only (its key field cannot be written). In
 * all these cases nothing is done. When the array itself runs into an
 * unmapped page after its first entry, the first request is submitted and
 * the call returns 1.
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
#define D "/tmp/a_io_submit_efault"
int test_main(void) {
    u64 ctx = 0;
    i64 r = io_setup(4, &ctx);
    check("setup_ok", r == 0, r);
    if (!ctx) return finish();
    i64 fd = a_open(D, O_RDWR | O_CREAT | O_TRUNC, 0644);
    struct a_iocb cb;
    set_cb(&cb, IOCB_CMD_PWRITE, fd, "a", 1, 0, 1);
    struct a_iocb *v[2] = {0, 0};
    expect("array_bad", io_submit(ctx, 1, (void *)1), -EFAULT);
    expect("entry_null", io_submit(ctx, 1, v), -EFAULT);
    v[0] = (struct a_iocb *)8;
    expect("entry_bad", io_submit(ctx, 1, v), -EFAULT);
    char *pg = (char *)syscall6(SYS_mmap, 0, 2 * A_PAGE, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    check("mmap", (i64)pg > 0, (i64)pg);
    if ((i64)pg <= 0) return finish();
    struct a_iocb *rocb = (struct a_iocb *)pg;
    set_cb(rocb, IOCB_CMD_PWRITE, fd, "b", 1, 0, 2);
    expect("mprotect_ro", syscall3(SYS_mprotect, pg, A_PAGE, PROT_READ), 0);
    v[0] = rocb;
    expect("iocb_read_only", io_submit(ctx, 1, v), -EFAULT);
    expect("mprotect_rw", syscall3(SYS_mprotect, pg, A_PAGE, PROT_READ | PROT_WRITE), 0);
    expect("unmap_second", syscall2(SYS_munmap, pg + A_PAGE, A_PAGE), 0);
    struct a_iocb *edge = (struct a_iocb *)(pg + A_PAGE - 32);
    memset(edge, 0, 32);
    edge->opcode = IOCB_CMD_PWRITE;
    edge->fildes = (u32)fd;
    v[0] = edge;
    expect("iocb_straddles", io_submit(ctx, 1, v), -EFAULT);
    struct a_io_event ev[2];
    expect("no_events", ready(ctx, ev, 2), 0);
    expect("size_0", file_size(fd), 0);
    struct a_iocb **tail = (struct a_iocb **)(pg + A_PAGE - 8);
    tail[0] = &cb;
    expect("array_straddles", io_submit(ctx, 2, tail), 1);
    expect("first_ran", ready(ctx, ev, 2), 1);
    check("first_obj", ev[0].obj == (u64)&cb, (i64)ev[0].obj);
    expect("size_1", file_size(fd), 1);
    v[0] = &cb;
    v[1] = 0;
    expect("second_null", io_submit(ctx, 2, v), 1);
    expect("first_ran_again", ready(ctx, ev, 2), 1);
    a_close(fd);
    a_unlink(D);
    expect("destroy", io_destroy(ctx), 0);
    return finish();
}
