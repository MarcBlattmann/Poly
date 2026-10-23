/* io_getevents with a handle that is not a live context of this process
 * is EINVAL: 0, 1, handle+32, the handle with bit 32 flipped, an
 * ordinary page, a ring whose header word 0 no longer holds the
 * context's index, a ring made PROT_NONE (EINVAL, not EFAULT), and a
 * destroyed handle. A read-only ring still works: the kernel moves the
 * head word itself.
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
    i64 r = io_setup(8, &ctx);
    check("setup_ok", r == 0, r);
    if (!ctx) return finish();
    struct a_io_event ev[2];
    struct a_ts zero = {0, 0};
    volatile u32 *ring = (volatile u32 *)ctx;
    expect("zero", io_getevents(0, 0, 1, ev, &zero), -EINVAL);
    expect("one", io_getevents(1, 0, 1, ev, &zero), -EINVAL);
    expect("plus32", io_getevents(ctx + 32, 0, 1, ev, &zero), -EINVAL);
    expect("bit32_flipped", io_getevents(ctx ^ (1ULL << 32), 0, 1, ev, &zero), -EINVAL);
    u32 *plain = (u32 *)syscall6(SYS_mmap, 0, 4096, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    for (int i = 0; i < 8; i++) plain[i] = ring[i];
    expect("copy_of_header", io_getevents((u64)plain, 0, 1, ev, &zero), -EINVAL);
    expect("bad_ctx_min_gt_nr", io_getevents(0, 2, 1, ev, &zero), -EINVAL);
    u32 index = ring[0];
    ring[0] = index + 5;
    expect("index_word_changed", io_getevents(ctx, 0, 1, ev, &zero), -EINVAL);
    ring[0] = index;
    expect("index_word_restored", io_getevents(ctx, 0, 1, ev, &zero), 0);
    expect("mprotect_none", syscall3(SYS_mprotect, ctx, 4096, PROT_NONE), 0);
    expect("ring_prot_none", io_getevents(ctx, 0, 1, ev, &zero), -EINVAL);
    expect("mprotect_read", syscall3(SYS_mprotect, ctx, 4096, PROT_READ), 0);
    expect("ring_read_only_empty", io_getevents(ctx, 0, 1, ev, &zero), 0);
    i64 fd = a_open("/tmp/a_io_getevents_bad_ctx", O_RDWR | O_CREAT | O_TRUNC, 0600);
    struct a_iocb cb;
    memset(&cb, 0, sizeof cb);
    cb.data = 9;
    cb.opcode = IOCB_CMD_PWRITE;
    cb.fildes = (u32)fd;
    cb.buf = (u64)data;
    cb.nbytes = 3;
    struct a_iocb *v[1] = {&cb};
    expect("submit", io_submit(ctx, 1, v), 1);
    expect("ring_read_only_event", io_getevents(ctx, 1, 2, ev, &zero), 1);
    expect("event_data", (i64)ev[0].data, 9);
    expect("head_moved_on_read_only_ring", ring[2], 1);
    expect("mprotect_rw", syscall3(SYS_mprotect, ctx, 4096, PROT_READ | PROT_WRITE), 0);
    expect("destroy", io_destroy(ctx), 0);
    expect("destroyed", io_getevents(ctx, 0, 1, ev, &zero), -EINVAL);
    a_close(fd);
    a_unlink("/tmp/a_io_getevents_bad_ctx");
    return finish();
}
