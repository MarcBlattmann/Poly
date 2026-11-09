/* io_setup(2) success: the context id is the address of a readable,
 * page-aligned completion ring mapped into the caller, whose header
 * libaio reads: magic 0xa10a10a1, compat_features 1, incompat_features 0,
 * header_length 32, head == tail == 0 and nr at least the number asked
 * for. Every context gets its own ring (distinct ids). The ring's tail
 * moves when an operation completes and its head when io_getevents reaps.
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

#define F "/tmp/e_io_setup_ring"

int test_main(void) {
    u64 a = 0, b = 0;
    i64 r = io_setup(1, &a);
    check("setup_a", r == 0, r);
    r = io_setup(128, &b);
    check("setup_b", r == 0, r);
    check("ids_nonzero", a != 0 && b != 0, 0);
    check("ids_distinct", a != b, 0);
    check("id_page_aligned", (a & 4095) == 0 && (b & 4095) == 0, (i64)(a & 4095));
    check("ring_mapped", page_mapped(a) && page_mapped(b), 0);

    struct k_aio_ring *ra = (struct k_aio_ring *)a, *rb = (struct k_aio_ring *)b;
    check("magic", ra->magic == AIO_RING_MAGIC && rb->magic == AIO_RING_MAGIC, ra->magic);
    check("compat_features", ra->compat_features == 1, ra->compat_features);
    check("incompat_features", ra->incompat_features == 0, ra->incompat_features);
    check("header_length", ra->header_length == 32, ra->header_length);
    check("head_tail_zero", ra->head == 0 && ra->tail == 0, ra->tail);
    check("nr_at_least_asked", ra->nr >= 1 && rb->nr >= 128, rb->nr);

    /* A completed read moves the tail; reaping it moves the head. */
    unlink_path(F);
    int fd = (int)open3(F, O_RDWR | O_CREAT | O_TRUNC, 0600);
    write(fd, "abcdef", 6);
    char buf[8] = { 0 };
    struct k_iocb cb;
    iocb_rw(&cb, IOCB_CMD_PREAD, fd, buf, 3, 1, 42);
    check("submit", submit1(a, &cb) == 1, 0);
    check("tail_moved", ra->tail == 1 && ra->head == 0, ra->tail);
    struct k_io_event *slot = (struct k_io_event *)(a + ra->header_length);
    check("ring_event_data", slot[0].data == 42, (i64)slot[0].data);
    check("ring_event_res", slot[0].res == 3, slot[0].res);
    check("ring_event_obj", slot[0].obj == (u64)&cb, 0);
    struct k_io_event ev[2];
    check("reap", reap(a, ev, 2) == 1, 0);
    check("head_moved", ra->head == 1 && ra->tail == 1, ra->head);
    check("read_data", mem_eq(buf, "bcd", 3), buf[0]);

    check("destroy_a", io_destroy(a) == 0, 0);
    check("destroy_b", io_destroy(b) == 0, 0);
    close(fd);
    unlink_path(F);
    return finish();
}
