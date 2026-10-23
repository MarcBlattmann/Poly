/* An unreadable timeout pointer is EFAULT, and it is checked before the
 * context and before min_nr/nr. A readable timeout is never written
 * back, even after a wait that ran to its end.
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
int test_main(void) {
    u64 ctx = 0;
    i64 r = io_setup(4, &ctx);
    check("setup_ok", r == 0, r);
    if (!ctx) return finish();
    struct a_io_event ev[2];
    expect("bad_timeout", io_getevents(ctx, 0, 1, ev, (void *)1), -EFAULT);
    expect("bad_timeout_min0_nr0", io_getevents(ctx, 0, 0, ev, (void *)1), -EFAULT);
    expect("bad_timeout_bad_ctx", io_getevents(0, 0, 1, ev, (void *)1), -EFAULT);
    expect("bad_timeout_min_gt_nr", io_getevents(ctx, 2, 1, ev, (void *)1), -EFAULT);
    expect("bad_timeout_bad_everything", io_getevents(0, -1, -1, (void *)1, (void *)1), -EFAULT);
    char *page = (char *)syscall6(SYS_mmap, 0, 8192, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    expect("unmap_second", syscall2(SYS_munmap, page + 4096, 4096), 0);
    struct a_ts *straddle = (struct a_ts *)(page + 4096 - 8);
    straddle->sec = 0;
    expect("timeout_straddles_hole", io_getevents(ctx, 0, 1, ev, straddle), -EFAULT);
    struct a_ts *last = (struct a_ts *)(page + 4096 - 16);
    last->sec = 0;
    last->nsec = 0;
    expect("timeout_at_page_end", io_getevents(ctx, 0, 1, ev, last), 0);
    struct a_ts t = {0, 20000000};
    expect("timeout_20ms", io_getevents(ctx, 1, 1, ev, &t), 0);
    check("timeout_not_written", t.sec == 0 && t.nsec == 20000000, t.nsec);
    expect("bad_ctx_good_timeout", io_getevents(0, 0, 1, ev, &t), -EINVAL);
    expect("destroy", io_destroy(ctx), 0);
    return finish();
}
