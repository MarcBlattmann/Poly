/* Completed requests come back as 32-byte io_event records in completion
 * order: data = the iocb's aio_data, obj = the iocb's user address, res =
 * the request's result (bytes written), res2 = 0. At most nr are returned;
 * the rest stay queued for the next call. Each call moves the ring's head
 * word (header word 2) up to the tail word (3); the records are also in
 * the ring right after the 32-byte header.
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
static char data[512];
int test_main(void) {
    u64 ctx = 0;
    i64 r = io_setup(4, &ctx);
    check("setup_ok", r == 0, r);
    if (!ctx) return finish();
    volatile u32 *ring = (volatile u32 *)ctx;
    i64 fd = a_open("/tmp/a_io_getevents_event_fields", O_RDWR | O_CREAT | O_TRUNC, 0600);
    check("open", fd >= 0, fd);
    struct a_iocb cb[3];
    struct a_iocb *v[3];
    for (int i = 0; i < 3; i++) {
        memset(&cb[i], 0, sizeof cb[i]);
        cb[i].data = 0x1111111100000000ULL + (u64)i;
        cb[i].opcode = IOCB_CMD_PWRITE;
        cb[i].fildes = (u32)fd;
        cb[i].buf = (u64)data;
        cb[i].nbytes = (u64)(100 * (i + 1));
        cb[i].offset = 1000 * i;
        v[i] = &cb[i];
    }
    expect("submit", io_submit(ctx, 3, v), 3);
    expect("head_before", ring[2], 0);
    expect("tail_before", ring[3], 3);
    struct a_io_event ev[8];
    memset(ev, 0x5a, sizeof ev);
    struct a_ts zero = {0, 0};
    expect("get_one", io_getevents(ctx, 1, 1, ev, &zero), 1);
    check("ev0_data", ev[0].data == 0x1111111100000000ULL, (i64)ev[0].data);
    check("ev0_obj", ev[0].obj == (u64)&cb[0], (i64)ev[0].obj);
    expect("ev0_res", ev[0].res, 100);
    expect("ev0_res2", ev[0].res2, 0);
    check("ev1_untouched", ev[1].data == 0x5a5a5a5a5a5a5a5aULL, (i64)ev[1].data);
    expect("head_after_one", ring[2], 1);
    expect("tail_after_one", ring[3], 3);
    memset(ev, 0x5a, sizeof ev);
    expect("get_rest", io_getevents(ctx, 0, 8, ev, &zero), 2);
    check("ev1_data", ev[0].data == 0x1111111100000001ULL, (i64)ev[0].data);
    check("ev1_obj", ev[0].obj == (u64)&cb[1], (i64)ev[0].obj);
    expect("ev1_res", ev[0].res, 200);
    check("ev2_data", ev[1].data == 0x1111111100000002ULL, (i64)ev[1].data);
    check("ev2_obj", ev[1].obj == (u64)&cb[2], (i64)ev[1].obj);
    expect("ev2_res", ev[1].res, 300);
    expect("ev2_res2", ev[1].res2, 0);
    check("ev3_untouched", ev[2].data == 0x5a5a5a5a5a5a5a5aULL, (i64)ev[2].data);
    expect("head_after_all", ring[2], 3);
    expect("drained", io_getevents(ctx, 0, 8, ev, &zero), 0);
    volatile struct a_io_event *slots = (volatile struct a_io_event *)(ctx + 32);
    check("slot1_data", slots[1].data == 0x1111111100000001ULL, (i64)slots[1].data);
    check("slot2_obj", slots[2].obj == (u64)&cb[2], (i64)slots[2].obj);
    expect("slot2_res", slots[2].res, 300);
    a_close(fd);
    a_unlink("/tmp/a_io_getevents_event_fields");
    expect("destroy", io_destroy(ctx), 0);
    return finish();
}
