/* io_pgetevents(2) without a signal mask (usig NULL, or usig with a NULL
 * sigmask, whatever sigsetsize says) behaves as io_getevents: same
 * returns, same events, same EINVAL for min_nr > nr or an unknown
 * context.
 */
#include "poly_ipc.h"

/* asm/unistd_64.h */
#define SYS_msync 26
#define SYS_io_setup 206
#define SYS_io_destroy 207
#define SYS_io_getevents 208
#define SYS_io_submit 209
#define SYS_io_cancel 210
#define SYS_io_pgetevents 333
#define SYS_setitimer 38
#define SYS_lseek 8
/* asm-generic/errno-base.h */
#define EBADF 9
#define EFAULT 14
#define EINVAL 22
#define ENOMEM 12
/* asm-generic/mman-common.h */
#define MS_ASYNC 1
/* linux/fs.h: RWF_* */
#define RWF_HIPRI 0x1
#define RWF_DSYNC 0x2
#define RWF_SYNC 0x4
#define RWF_NOWAIT 0x8
#define RWF_APPEND 0x10

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

/* struct iocb (64 bytes, little-endian x86-64 field order). */
struct k_iocb {
    u64 aio_data;
    unsigned aio_key;
    int aio_rw_flags;
    unsigned short aio_lio_opcode;
    short aio_reqprio;
    unsigned aio_fildes;
    u64 aio_buf;
    u64 aio_nbytes;
    i64 aio_offset;
    u64 aio_reserved2;
    unsigned aio_flags;
    unsigned aio_resfd;
};

/* struct io_event (32 bytes). */
struct k_io_event {
    u64 data;
    u64 obj;
    i64 res;
    i64 res2;
};

/* The completion ring a context id points at (the header libaio reads). */
struct k_aio_ring {
    unsigned id;
    unsigned nr;
    unsigned head;
    unsigned tail;
    unsigned magic;
    unsigned compat_features;
    unsigned incompat_features;
    unsigned header_length;
};
#define AIO_RING_MAGIC 0xa10a10a1u

struct k_iovec2 {
    void *base;
    u64 len;
};

static __attribute__((unused)) i64 io_setup(i64 nr, u64 *ctx) { return syscall2(SYS_io_setup, nr, ctx); }
static __attribute__((unused)) i64 io_destroy(u64 ctx) { return syscall1(SYS_io_destroy, ctx); }
static __attribute__((unused)) i64 io_submit(u64 ctx, i64 nr, struct k_iocb **v) {
    return syscall3(SYS_io_submit, ctx, nr, v);
}
static __attribute__((unused)) i64 io_getevents(u64 ctx, i64 min, i64 nr, struct k_io_event *ev,
                                                struct k_timespec *ts) {
    return syscall5(SYS_io_getevents, ctx, min, nr, ev, ts);
}
/* struct __aio_sigset (linux/aio_abi.h). */
struct k_aio_sigset {
    const u64 *sigmask;
    u64 sigsetsize;
};
static __attribute__((unused)) i64 io_pgetevents(u64 ctx, i64 min, i64 nr, struct k_io_event *ev,
                                                 struct k_timespec *ts, struct k_aio_sigset *us) {
    return syscall6(SYS_io_pgetevents, (i64)ctx, min, nr, (i64)ev, (i64)ts, (i64)us);
}
static __attribute__((unused)) i64 io_cancel(u64 ctx, struct k_iocb *cb, struct k_io_event *res) {
    return syscall3(SYS_io_cancel, ctx, cb, res);
}
/* Submit one iocb. */
static __attribute__((unused)) i64 submit1(u64 ctx, struct k_iocb *cb) {
    struct k_iocb *v[1] = { cb };
    return io_submit(ctx, 1, v);
}
/* Reap without waiting. */
static __attribute__((unused)) i64 reap(u64 ctx, struct k_io_event *ev, i64 nr) {
    struct k_timespec z = { 0, 0 };
    return io_getevents(ctx, 0, nr, ev, &z);
}
/* Wait up to `ms` for at least one event. */
static __attribute__((unused)) i64 reap_wait(u64 ctx, struct k_io_event *ev, i64 nr, i64 ms) {
    struct k_timespec t = { ms / 1000, (ms % 1000) * 1000000L };
    return io_getevents(ctx, 1, nr, ev, &t);
}
static __attribute__((unused)) void iocb_rw(struct k_iocb *cb, int op, int fd, void *buf, u64 n, i64 off, u64 data) {
    memset(cb, 0, sizeof *cb);
    cb->aio_lio_opcode = (unsigned short)op;
    cb->aio_fildes = (unsigned)fd;
    cb->aio_buf = (u64)buf;
    cb->aio_nbytes = n;
    cb->aio_offset = off;
    cb->aio_data = data;
}
static __attribute__((unused)) i64 open3(const char *path, i64 flags, i64 mode) {
    return syscall4(SYS_openat, AT_FDCWD, path, flags, mode);
}
static __attribute__((unused)) i64 unlink_path(const char *path) { return syscall3(SYS_unlinkat, AT_FDCWD, path, 0); }
/* Is the page at `p` mapped? (msync says ENOMEM for an unmapped range.) */
static __attribute__((unused)) int page_mapped(u64 p) { return syscall3(SYS_msync, p, 4096, MS_ASYNC) == 0; }

#define F "/tmp/e_io_pgetevents_basic"

int test_main(void) {
    u64 ctx = 0;
    check("setup", io_setup(4, &ctx) == 0, 0);
    struct k_io_event ev[4];
    struct k_timespec z = { 0, 0 };
    i64 r = io_pgetevents(ctx, 0, 4, ev, &z, 0);
    check("empty_usig_null", r == 0, r);
    struct k_aio_sigset us = { 0, 4 };
    r = io_pgetevents(ctx, 0, 4, ev, &z, &us);
    check("null_mask_any_size", r == 0, r);
    r = io_pgetevents(ctx, 3, 1, ev, &z, 0);
    check("min_gt_nr_einval", r == -EINVAL, r);
    r = io_pgetevents(1, 0, 1, ev, &z, 0);
    check("bad_ctx_einval", r == -EINVAL, r);

    unlink_path(F);
    int fd = (int)open3(F, O_RDWR | O_CREAT | O_TRUNC, 0600);
    write(fd, "xy", 2);
    char b1[2], b2[2];
    struct k_iocb c1, c2;
    iocb_rw(&c1, IOCB_CMD_PREAD, fd, b1, 1, 0, 1);
    iocb_rw(&c2, IOCB_CMD_PREAD, fd, b2, 1, 1, 2);
    struct k_iocb *v[2] = { &c1, &c2 };
    check("submit", io_submit(ctx, 2, v) == 2, 0);
    struct k_timespec t = { 1, 0 };
    r = io_pgetevents(ctx, 1, 1, ev, &t, 0);
    check("one_of_two", r == 1 && ev[0].data == 1 && ev[0].res == 1, r);
    us.sigsetsize = 8;
    r = io_pgetevents(ctx, 1, 4, ev, 0, &us);
    check("second_null_timeout", r == 1 && ev[0].data == 2 && b2[0] == 'y', r);
    t.tv_sec = 0;
    t.tv_nsec = 40000000;
    i64 t0 = now_ns();
    r = io_pgetevents(ctx, 1, 4, ev, &t, 0);
    check("timeout_zero_events", r == 0 && now_ns() - t0 >= 30000000, r);

    check("destroy", io_destroy(ctx) == 0, 0);
    close(fd);
    unlink_path(F);
    return finish();
}
