/* min_nr is how many events to wait for. Events are taken as they come:
 * with min_nr 2, one event ready and a 50 ms timeout, the call returns 1
 * when the timeout runs out (and that event is consumed). With a NULL
 * timeout it waits until a second request completes and returns both, in
 * completion order. With one ready and a zero timeout it returns 1.
 * poly-diff: syscalls=off (two processes)
 */
#include "poly_test.h"
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_io_setup 206
#define SYS_io_destroy 207
#define SYS_io_getevents 208
#define SYS_io_submit 209
/* linux/aio_abi.h */
#define IOCB_CMD_PWRITE 1
#define IOCB_CMD_POLL 5
/* asm-generic/poll.h */
#define POLLIN 0x0001
#define POLLRDNORM 0x0040

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
static A_UNUSED i64 io_submit(u64 ctx, i64 n, struct a_iocb **v) { return syscall3(SYS_io_submit, ctx, n, v); }
static A_UNUSED i64 now_ms(void) {
    struct a_ts t;
    syscall2(SYS_clock_gettime, CLOCK_MONOTONIC, &t);
    return t.sec * 1000 + t.nsec / 1000000;
}
static int p1[2], p2[2];
static volatile i32 done;
static int writer(void *arg) {
    (void)arg;
    struct a_ts t = {0, 100000000};
    syscall2(SYS_nanosleep, &t, 0);
    return a_write(p2[1], "y", 1) == 1 ? 0 : 1;
}
static i64 submit_poll(u64 ctx, struct a_iocb *cb, int fd, u64 tag) {
    memset(cb, 0, sizeof *cb);
    cb->data = tag;
    cb->opcode = IOCB_CMD_POLL;
    cb->fildes = (u32)fd;
    cb->buf = POLLIN;
    struct a_iocb *v[1] = {cb};
    return io_submit(ctx, 1, v);
}
int test_main(void) {
    u64 ctx = 0;
    i64 r = io_setup(4, &ctx);
    check("setup_ok", r == 0, r);
    if (!ctx) return finish();
    expect("pipe1", syscall2(SYS_pipe2, p1, 0), 0);
    expect("pipe2", syscall2(SYS_pipe2, p2, 0), 0);
    struct a_iocb cb1, cb2;
    struct a_io_event ev[4];
    struct a_ts zero = {0, 0};
    char c;
    expect("submit1", submit_poll(ctx, &cb1, p1[0], 11), 1);
    expect("write1", a_write(p1[1], "x", 1), 1);
    struct a_ts t50 = {0, 50000000};
    i64 t0 = now_ms();
    expect("min2_one_ready_timeout", io_getevents(ctx, 2, 4, ev, &t50), 1);
    i64 dt = now_ms() - t0;
    check("waited_whole_timeout", dt >= 50, dt);
    expect("got_11", (i64)ev[0].data, 11);
    expect("consumed", io_getevents(ctx, 0, 4, ev, &zero), 0);
    a_read(p1[0], &c, 1);
    expect("submit1b", submit_poll(ctx, &cb1, p1[0], 21), 1);
    expect("write1b", a_write(p1[1], "x", 1), 1);
    expect("min2_one_ready_zero_timeout", io_getevents(ctx, 2, 4, ev, &zero), 1);
    expect("got_21", (i64)ev[0].data, 21);
    a_read(p1[0], &c, 1);
    expect("submit1c", submit_poll(ctx, &cb1, p1[0], 31), 1);
    expect("submit2", submit_poll(ctx, &cb2, p2[0], 32), 1);
    expect("write1c", a_write(p1[1], "x", 1), 1);
    t0 = now_ms();
    a_spawn(0, &done, writer, 0);
    expect("min2_null_timeout", io_getevents(ctx, 2, 4, ev, 0), 2);
    dt = now_ms() - t0;
    check("waited_for_second", dt >= 80, dt);
    expect("first_is_31", (i64)ev[0].data, 31);
    expect("second_is_32", (i64)ev[1].data, 32);
    while (done) a_relax();
    expect("destroy", io_destroy(ctx), 0);
    return finish();
}
