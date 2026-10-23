/* A signal handler interrupts a waiting io_getevents: with no event
 * collected it fails with EINTR, and it is not restarted even with
 * SA_RESTART (also with a timeout left). If an event was already
 * collected (min_nr 2, one ready), it returns that 1 instead.
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
#define SYS_rt_sigreturn 15
#define SA_RESTART 0x10000000
#define ITIMER_REAL 0
extern void ige_restorer(void);
__asm__(".text\nige_restorer:\n    mov $15, %eax\n    syscall\n    hlt\n");
static volatile int hits;
static void on_alarm(int sig) {
    (void)sig;
    hits++;
}
static void arm_100ms(void) {
    struct a_ts it[2] = {{0, 0}, {0, 100000}}; /* interval 0, value 100 ms (timeval) */
    syscall3(SYS_setitimer, ITIMER_REAL, it, 0);
}
int test_main(void) {
    u64 ctx = 0;
    i64 r = io_setup(4, &ctx);
    check("setup_ok", r == 0, r);
    if (!ctx) return finish();
    int p1[2], p2[2];
    expect("pipe1", syscall2(SYS_pipe2, p1, 0), 0);
    expect("pipe2", syscall2(SYS_pipe2, p2, 0), 0);
    struct a_sigaction sa = {(u64)on_alarm, SA_RESTORER, (u64)ige_restorer, 0};
    expect("sigaction", syscall4(SYS_rt_sigaction, SIGALRM, &sa, 0, 8), 0);
    struct a_iocb cb1, cb2;
    memset(&cb1, 0, sizeof cb1);
    cb1.data = 1;
    cb1.opcode = IOCB_CMD_POLL;
    cb1.fildes = (u32)p1[0];
    cb1.buf = POLLIN;
    cb2 = cb1;
    cb2.data = 2;
    cb2.fildes = (u32)p2[0];
    struct a_iocb *v[2] = {&cb1, &cb2};
    expect("submit", io_submit(ctx, 2, v), 2);
    struct a_io_event ev[4];
    arm_100ms();
    i64 t0 = now_ms();
    expect("null_timeout_eintr", io_getevents(ctx, 1, 4, ev, 0), -EINTR);
    check("interrupted_late_enough", now_ms() - t0 >= 80, now_ms() - t0);
    expect("handler_ran", hits, 1);
    sa.flags |= SA_RESTART;
    expect("sigaction_restart", syscall4(SYS_rt_sigaction, SIGALRM, &sa, 0, 8), 0);
    struct a_ts t5 = {5, 0};
    arm_100ms();
    t0 = now_ms();
    expect("sa_restart_still_eintr", io_getevents(ctx, 1, 4, ev, &t5), -EINTR);
    check("not_restarted", now_ms() - t0 < 4000, now_ms() - t0);
    expect("handler_ran_2", hits, 2);
    check("timeout_not_written", t5.sec == 5 && t5.nsec == 0, t5.sec);
    expect("write1", a_write(p1[1], "x", 1), 1);
    arm_100ms();
    expect("partial_returns_count", io_getevents(ctx, 2, 4, ev, 0), 1);
    expect("handler_ran_3", hits, 3);
    expect("partial_data", (i64)ev[0].data, 1);
    struct a_ts zero = {0, 0};
    expect("second_still_pending", io_getevents(ctx, 0, 4, ev, &zero), 0);
    expect("write2", a_write(p2[1], "x", 1), 1);
    expect("second_now", io_getevents(ctx, 1, 4, ev, &zero), 1);
    expect("second_data", (i64)ev[0].data, 2);
    expect("destroy", io_destroy(ctx), 0);
    return finish();
}
