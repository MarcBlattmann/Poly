/* When events are ready and the events buffer cannot take them (bad
 * pointer, read-only page, or the records would run into an unmapped
 * page) io_getevents is EFAULT and consumes nothing: the head word stays
 * and a later call with a good buffer gets every event.
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
    i64 fd = a_open("/tmp/a_io_getevents_events_efault", O_RDWR | O_CREAT | O_TRUNC, 0600);
    check("open", fd >= 0, fd);
    struct a_iocb cb[2];
    struct a_iocb *v[2];
    for (int i = 0; i < 2; i++) {
        memset(&cb[i], 0, sizeof cb[i]);
        cb[i].data = (u64)(40 + i);
        cb[i].opcode = IOCB_CMD_PWRITE;
        cb[i].fildes = (u32)fd;
        cb[i].buf = (u64)data;
        cb[i].nbytes = (u64)(i + 1);
        v[i] = &cb[i];
    }
    expect("submit", io_submit(ctx, 2, v), 2);
    struct a_ts zero = {0, 0};
    expect("bad_pointer", io_getevents(ctx, 0, 2, (void *)8, &zero), -EFAULT);
    expect("null_pointer", io_getevents(ctx, 1, 1, 0, &zero), -EFAULT);
    expect("bad_pointer_null_timeout", io_getevents(ctx, 1, 2, (void *)8, 0), -EFAULT);
    expect("head_unchanged_1", ring[2], 0);
    char *page = (char *)syscall6(SYS_mmap, 0, 8192, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    expect("unmap_second", syscall2(SYS_munmap, page + 4096, 4096), 0);
    struct a_io_event *straddle = (struct a_io_event *)(page + 4096 - 32);
    expect("straddles_hole", io_getevents(ctx, 0, 2, straddle, &zero), -EFAULT);
    expect("head_unchanged_2", ring[2], 0);
    expect("mprotect_read", syscall3(SYS_mprotect, page, 4096, PROT_READ), 0);
    expect("read_only_buffer", io_getevents(ctx, 0, 2, page, &zero), -EFAULT);
    expect("head_unchanged_3", ring[2], 0);
    struct a_io_event ev[4];
    expect("good_buffer", io_getevents(ctx, 0, 4, ev, &zero), 2);
    expect("ev0_data", (i64)ev[0].data, 40);
    expect("ev0_res", ev[0].res, 1);
    expect("ev1_data", (i64)ev[1].data, 41);
    expect("ev1_res", ev[1].res, 2);
    expect("head_after", ring[2], 2);
    expect("empty_bad_pointer", io_getevents(ctx, 0, 2, (void *)8, &zero), 0);
    a_close(fd);
    a_unlink("/tmp/a_io_getevents_events_efault");
    expect("destroy", io_destroy(ctx), 0);
    return finish();
}
