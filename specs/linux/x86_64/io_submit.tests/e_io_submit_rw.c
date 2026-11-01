/* io_submit(2) of IOCB_CMD_PREAD and IOCB_CMD_PWRITE on a regular file:
 * positioned I/O at aio_offset that leaves the file offset alone. The
 * event carries aio_data, the iocb's address (obj), res = bytes moved
 * (or -errno) and res2 = 0. Reads past the end give 0, a write past the
 * end extends the file, a too-large count is cut at the end of file, and
 * an unreadable buffer completes with res = -EFAULT (the submit itself
 * succeeds). A count is first capped at MAX_RW_COUNT (0x7ffff000) and the
 * range [buf, buf + that) must lie inside the user address space (below
 * 0x7ffffffff000), or the submit fails with EFAULT: so a huge count works
 * with a buffer low in memory and fails with one near the top.
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
/* fork that suspends the parent until the child exits (CLONE_VFORK
 * without CLONE_VM), with SIGCHLD blocked so its arrival does not land at
 * a varying point of the trace. */
static __attribute__((unused)) i64 fork_sync(void) {
    sig_mask(SIG_BLOCK, SIGBIT(SIGCHLD), 0);
    return syscall5(SYS_clone, 0x4000 /* CLONE_VFORK */ | SIGCHLD, 0, 0, 0, 0);
}
/* Is the page at `p` mapped? (msync says ENOMEM for an unmapped range.) */
static __attribute__((unused)) int page_mapped(u64 p) { return syscall3(SYS_msync, p, 4096, MS_ASYNC) == 0; }

#define F "/tmp/e_io_submit_rw"

int test_main(void) {
    u64 ctx = 0;
    check("setup", io_setup(8, &ctx) == 0, 0);
    unlink_path(F);
    int fd = (int)open3(F, O_RDWR | O_CREAT | O_TRUNC, 0600);
    write(fd, "hello world", 11);

    /* Static, not on the stack: the huge-count check below needs the
     * buffer far from the top of the address space (see the header). */
    static char buf[32];
    memset(buf, 0, sizeof buf);
    struct k_iocb cb;
    struct k_io_event ev[4];
    iocb_rw(&cb, IOCB_CMD_PREAD, fd, buf, 5, 6, 0x1234);
    check("pread_submit", submit1(ctx, &cb) == 1, 0);
    memset(ev, 0, sizeof ev);
    i64 n = reap(ctx, ev, 4);
    check("pread_one_event", n == 1, n);
    check("pread_data", ev[0].data == 0x1234, (i64)ev[0].data);
    check("pread_obj", ev[0].obj == (u64)&cb, 0);
    check("pread_res", ev[0].res == 5, ev[0].res);
    check("pread_res2", ev[0].res2 == 0, ev[0].res2);
    check("pread_bytes", mem_eq(buf, "world", 5), buf[0]);
    check("file_offset_unchanged", syscall3(SYS_lseek, fd, 0, 1) == 11, syscall3(SYS_lseek, fd, 0, 1));

    iocb_rw(&cb, IOCB_CMD_PWRITE, fd, "XY", 2, 20, 2);
    check("pwrite_submit", submit1(ctx, &cb) == 1, 0);
    n = reap(ctx, ev, 4);
    check("pwrite_res", n == 1 && ev[0].res == 2 && ev[0].data == 2, ev[0].res);
    check("pwrite_extends", syscall3(SYS_lseek, fd, 0, 2) == 22, syscall3(SYS_lseek, fd, 0, 2));

    iocb_rw(&cb, IOCB_CMD_PREAD, fd, buf, 4, 100, 3);
    submit1(ctx, &cb);
    n = reap(ctx, ev, 4);
    check("pread_past_eof_zero", n == 1 && ev[0].res == 0, ev[0].res);

    iocb_rw(&cb, IOCB_CMD_PREAD, fd, buf, 0x7fffffffffffffffUL, 0, 4);
    i64 s = submit1(ctx, &cb);
    check("huge_count_submit", s == 1, s);
    n = reap(ctx, ev, 4);
    check("huge_count_cut_at_eof", n == 1 && ev[0].res == 22, ev[0].res);

    /* Near the top of the address space the capped range leaves it. */
    iocb_rw(&cb, IOCB_CMD_PREAD, fd, (void *)0x7ffff0000000UL, 0x7fffffffffffffffUL, 0, 7);
    s = submit1(ctx, &cb);
    check("huge_count_high_buffer_efault", s == -EFAULT, s);
    check("nothing_queued", reap(ctx, ev, 4) == 0, 0);

    iocb_rw(&cb, IOCB_CMD_PREAD, fd, (void *)8, 4, 0, 5);
    check("bad_buffer_submit_ok", submit1(ctx, &cb) == 1, 0);
    n = reap(ctx, ev, 4);
    check("bad_buffer_res_efault", n == 1 && ev[0].res == -EFAULT, ev[0].res);

    iocb_rw(&cb, IOCB_CMD_PWRITE, fd, "abc", 0, 0, 6);
    submit1(ctx, &cb);
    n = reap(ctx, ev, 4);
    check("zero_length_write", n == 1 && ev[0].res == 0, ev[0].res);

    check("destroy", io_destroy(ctx) == 0, 0);
    close(fd);
    unlink_path(F);
    return finish();
}
