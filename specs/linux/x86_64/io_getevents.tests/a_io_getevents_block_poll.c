/* io_getevents blocks until a request completes: a poll request on an
 * empty pipe completes when another process (CLONE_VM) writes to the pipe
 * about 100 ms later. With a NULL timeout, and with a negative timeout
 * ({-1, 0} and {0, -1} do not expire), the call returns 1 then; the
 * event's res is the poll result (POLLIN | POLLRDNORM).
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
static int pfd[2];
static volatile i32 done;
static int writer(void *arg) {
    (void)arg;
    struct a_ts t = {0, 100000000};
    syscall2(SYS_nanosleep, &t, 0);
    return a_write(pfd[1], "x", 1) == 1 ? 0 : 1;
}
static void one_wait(u64 ctx, const char *name, struct a_ts *timeout, u64 tag) {
    struct a_iocb cb;
    memset(&cb, 0, sizeof cb);
    cb.data = tag;
    cb.opcode = IOCB_CMD_POLL;
    cb.fildes = (u32)pfd[0];
    cb.buf = POLLIN;
    struct a_iocb *v[1] = {&cb};
    struct a_ts zero = {0, 0};
    struct a_io_event ev[2];
    expect("submit_poll", io_submit(ctx, 1, v), 1);
    expect("not_ready_yet", io_getevents(ctx, 1, 2, ev, &zero), 0);
    i64 t0 = now_ms();
    a_spawn(0, &done, writer, 0);
    i64 r = io_getevents(ctx, 1, 2, ev, timeout);
    i64 dt = now_ms() - t0;
    check(name, r == 1, r);
    check("waited_for_writer", dt >= 80, dt);
    expect("event_data", (i64)ev[0].data, (i64)tag);
    check("event_obj", ev[0].obj == (u64)&cb, (i64)ev[0].obj);
    expect("event_res", ev[0].res, POLLIN | POLLRDNORM);
    expect("event_res2", ev[0].res2, 0);
    while (done) a_relax();
    char c;
    a_read(pfd[0], &c, 1);
}
int test_main(void) {
    u64 ctx = 0;
    i64 r = io_setup(4, &ctx);
    check("setup_ok", r == 0, r);
    if (!ctx) return finish();
    expect("pipe", syscall2(SYS_pipe2, pfd, 0), 0);
    one_wait(ctx, "null_timeout", 0, 1);
    struct a_ts neg_sec = {-1, 0};
    one_wait(ctx, "negative_sec", &neg_sec, 2);
    struct a_ts neg_ns = {0, -1};
    one_wait(ctx, "negative_nsec", &neg_ns, 3);
    struct a_ts long_ts = {30, 0};
    one_wait(ctx, "long_timeout_returns_early", &long_ts, 4);
    expect("destroy", io_destroy(ctx), 0);
    return finish();
}
