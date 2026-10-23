/* Reads and writes on a pipe: aio_offset is ignored (any value works).
 * On an O_NONBLOCK empty pipe a read is accepted and its event has res
 * -EAGAIN. On a blocking empty pipe the read does not wait in the
 * background: io_submit itself blocks until another process (CLONE_VM)
 * writes about 100 ms later, and the event is ready when it returns.
 * poly-diff: syscalls=off (two processes)
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
static int pfd[2];
static volatile i32 done;
static i64 now_ms(void) {
    struct a_ts t;
    syscall2(SYS_clock_gettime, CLOCK_MONOTONIC, &t);
    return t.sec * 1000 + t.nsec / 1000000;
}
static int writer(void *arg) {
    (void)arg;
    struct a_ts t = {0, 100000000};
    syscall2(SYS_nanosleep, &t, 0);
    return a_write(pfd[1], "w", 1) == 1 ? 0 : 1;
}
int test_main(void) {
    u64 ctx = 0;
    i64 r = io_setup(4, &ctx);
    check("setup_ok", r == 0, r);
    if (!ctx) return finish();
    int q[2];
    expect("pipe_nb", syscall2(SYS_pipe2, q, O_NONBLOCK), 0);
    struct a_iocb cb;
    char c = 0;
    set_cb(&cb, IOCB_CMD_PREAD, q[0], &c, 1, 0, 1);
    expect("nb_empty_read", run1(ctx, &cb), -EAGAIN);
    set_cb(&cb, IOCB_CMD_PWRITE, q[1], "ab", 2, 12345, 2);
    expect("write_any_offset", run1(ctx, &cb), 2);
    char buf[4];
    set_cb(&cb, IOCB_CMD_PREAD, q[0], buf, 4, 999, 3);
    expect("read_any_offset", run1(ctx, &cb), 2);
    check("read_ab", buf[0] == 'a' && buf[1] == 'b', buf[0]);
    expect("pipe", syscall2(SYS_pipe2, pfd, 0), 0);
    set_cb(&cb, IOCB_CMD_PREAD, pfd[0], &c, 1, 0, 4);
    a_spawn(0, &done, writer, 0);
    i64 t0 = now_ms();
    r = submit1(ctx, &cb);
    i64 dt = now_ms() - t0;
    expect("blocking_submit", r, 1);
    check("blocked_until_write", dt >= 80, dt);
    check("not_too_long", dt < 5000, dt);
    struct a_io_event ev[2];
    expect("event_ready", ready(ctx, ev, 2), 1);
    expect("event_res", ev[0].res, 1);
    expect("byte", c, 'w');
    while (done) a_relax();
    expect("destroy", io_destroy(ctx), 0);
    return finish();
}
