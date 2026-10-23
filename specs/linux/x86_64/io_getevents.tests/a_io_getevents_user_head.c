/* The ring's head word (header word 2) is shared with the caller:
 * io_getevents starts reading at whatever the caller left there. Moving
 * head forward skips events (a caller reaping them itself), setting it
 * to the tail word leaves nothing, and moving it back returns the
 * records still in those slots again.
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
static char data[16];
int test_main(void) {
    u64 ctx = 0;
    i64 r = io_setup(4, &ctx);
    check("setup_ok", r == 0, r);
    if (!ctx) return finish();
    volatile u32 *ring = (volatile u32 *)ctx;
    i64 fd = a_open("/tmp/a_io_getevents_user_head", O_RDWR | O_CREAT | O_TRUNC, 0600);
    check("open", fd >= 0, fd);
    struct a_iocb cb[4];
    struct a_iocb *v[4];
    for (int i = 0; i < 4; i++) {
        memset(&cb[i], 0, sizeof cb[i]);
        cb[i].data = (u64)(100 + i);
        cb[i].opcode = IOCB_CMD_PWRITE;
        cb[i].fildes = (u32)fd;
        cb[i].buf = (u64)data;
        cb[i].nbytes = 1;
        v[i] = &cb[i];
    }
    expect("submit", io_submit(ctx, 4, v), 4);
    expect("tail", ring[3], 4);
    struct a_io_event ev[8];
    struct a_ts zero = {0, 0};
    ring[2] = 2;
    expect("after_skip", io_getevents(ctx, 0, 8, ev, &zero), 2);
    expect("first_after_skip", (i64)ev[0].data, 102);
    expect("second_after_skip", (i64)ev[1].data, 103);
    expect("head_at_tail", ring[2], 4);
    ring[2] = 1;
    expect("replay", io_getevents(ctx, 0, 8, ev, &zero), 3);
    expect("replay_0", (i64)ev[0].data, 101);
    expect("replay_2", (i64)ev[2].data, 103);
    expect("submit_more", io_submit(ctx, 2, v), 2);
    expect("tail_6", ring[3], 6);
    ring[2] = ring[3];
    expect("reaped_by_caller", io_getevents(ctx, 0, 8, ev, &zero), 0);
    expect("head_kept", ring[2], 6);
    a_close(fd);
    a_unlink("/tmp/a_io_getevents_user_head");
    expect("destroy", io_destroy(ctx), 0);
    return finish();
}
