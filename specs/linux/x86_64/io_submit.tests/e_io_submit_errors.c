/* io_submit(2) errors. Whole-call errors: nr < 0 or an unknown context
 * is EINVAL, an unreadable iocb array or iocb EFAULT; nr == 0 returns 0.
 * Per-iocb errors (reported by the call when the first iocb fails):
 * aio_reserved2 != 0 EINVAL; unknown opcodes (4, NOOP 6, 99) EINVAL; a
 * bad fd or an O_PATH fd EBADF; a write on a read-only fd EBADF; a
 * negative offset EINVAL; a directory EINVAL; unknown aio_rw_flags
 * EOPNOTSUPP. aio_key and unknown aio_flags bits are ignored.
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

#define F "/tmp/e_io_submit_errors"

int test_main(void) {
    u64 ctx = 0;
    check("setup", io_setup(8, &ctx) == 0, 0);
    unlink_path(F);
    int fd = (int)open3(F, O_RDWR | O_CREAT | O_TRUNC, 0600);
    write(fd, "data", 4);
    char buf[8];
    struct k_iocb cb;
    struct k_iocb *v[1] = { &cb };
    struct k_io_event ev[8];
    iocb_rw(&cb, IOCB_CMD_PREAD, fd, buf, 2, 0, 1);

    check("nr_zero", io_submit(ctx, 0, v) == 0, io_submit(ctx, 0, v));
    check("nr_negative_einval", io_submit(ctx, -1, v) == -EINVAL, io_submit(ctx, -1, v));
    check("bad_ctx_einval", io_submit(12345, 1, v) == -EINVAL, io_submit(12345, 1, v));
    check("ctx_zero_einval", io_submit(0, 1, v) == -EINVAL, io_submit(0, 1, v));
    check("array_efault", io_submit(ctx, 1, (struct k_iocb **)8) == -EFAULT, 0);
    struct k_iocb *badv[1] = { (struct k_iocb *)8 };
    check("iocb_efault", io_submit(ctx, 1, badv) == -EFAULT, io_submit(ctx, 1, badv));

    cb.aio_reserved2 = 1;
    check("reserved2_einval", submit1(ctx, &cb) == -EINVAL, submit1(ctx, &cb));
    cb.aio_reserved2 = 0;
    cb.aio_fildes = 77;
    check("bad_fd_ebadf", submit1(ctx, &cb) == -EBADF, submit1(ctx, &cb));
    cb.aio_fildes = (unsigned)fd;
    cb.aio_lio_opcode = 4;
    check("opcode_4_einval", submit1(ctx, &cb) == -EINVAL, submit1(ctx, &cb));
    cb.aio_lio_opcode = IOCB_CMD_NOOP;
    check("opcode_noop_einval", submit1(ctx, &cb) == -EINVAL, submit1(ctx, &cb));
    cb.aio_lio_opcode = 99;
    check("opcode_99_einval", submit1(ctx, &cb) == -EINVAL, submit1(ctx, &cb));
    cb.aio_lio_opcode = IOCB_CMD_PREAD;
    cb.aio_rw_flags = 0x80;
    check("rw_flags_unknown_eopnotsupp", submit1(ctx, &cb) == -EOPNOTSUPP, submit1(ctx, &cb));
    cb.aio_rw_flags = 0;
    cb.aio_offset = -1;
    check("negative_offset_einval", submit1(ctx, &cb) == -EINVAL, submit1(ctx, &cb));
    cb.aio_offset = 0;

    int ro = (int)open3(F, O_RDONLY, 0);
    iocb_rw(&cb, IOCB_CMD_PWRITE, ro, "zz", 2, 0, 1);
    check("write_on_rdonly_ebadf", submit1(ctx, &cb) == -EBADF, submit1(ctx, &cb));
    int wo = (int)open3(F, O_WRONLY, 0);
    iocb_rw(&cb, IOCB_CMD_PREAD, wo, buf, 2, 0, 1);
    check("read_on_wronly_ebadf", submit1(ctx, &cb) == -EBADF, submit1(ctx, &cb));
    int pf = (int)open3(F, 010000000 /* O_PATH */, 0);
    iocb_rw(&cb, IOCB_CMD_PREAD, pf, buf, 2, 0, 1);
    check("o_path_ebadf", submit1(ctx, &cb) == -EBADF, submit1(ctx, &cb));
    int dfd = (int)open3("/tmp", O_RDONLY | O_DIRECTORY, 0);
    iocb_rw(&cb, IOCB_CMD_PREAD, dfd, buf, 2, 0, 1);
    check("directory_einval", submit1(ctx, &cb) == -EINVAL, submit1(ctx, &cb));
    check("no_events_from_failures", reap(ctx, ev, 8) == 0, reap(ctx, ev, 8));

    /* Ignored fields. */
    iocb_rw(&cb, IOCB_CMD_PREAD, fd, buf, 2, 0, 9);
    cb.aio_key = 1;
    cb.aio_flags = 4;
    cb.aio_resfd = 12345;
    check("key_and_unknown_flags_ignored", submit1(ctx, &cb) == 1, 0);
    i64 n = reap(ctx, ev, 8);
    check("ignored_fields_event", n == 1 && ev[0].res == 2 && ev[0].data == 9, ev[0].res);

    check("destroy", io_destroy(ctx) == 0, 0);
    close(fd);
    close(ro);
    close(wo);
    close(pf);
    close(dfd);
    unlink_path(F);
    return finish();
}
