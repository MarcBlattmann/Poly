/* Contexts belong to the address space: a process sharing it (CLONE_VM)
 * may submit to the parent's context, and the event is reaped by the
 * parent. A fork child (own memory) gets EINVAL for the parent's handle,
 * and nothing it tries reaches the parent's context; a context it makes
 * itself works.
 */
#include "poly_test.h"
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_io_setup 206
#define SYS_io_destroy 207
#define SYS_io_getevents 208
#define SYS_io_submit 209
#define SYS_eventfd2 290
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
static A_UNUSED i64 io_submit(u64 ctx, i64 n, void *v) { return syscall3(SYS_io_submit, ctx, n, v); }
static A_UNUSED void set_cb(struct a_iocb *cb, u16 op, i64 fd, const void *buf, u64 n, i64 off, u64 data) {
    memset(cb, 0, sizeof *cb);
    cb->opcode = op;
    cb->fildes = (u32)fd;
    cb->buf = (u64)buf;
    cb->nbytes = n;
    cb->offset = off;
    cb->data = data;
}
/* io_submit of one iocb */
static A_UNUSED i64 submit1(u64 ctx, struct a_iocb *cb) {
    struct a_iocb *v[1] = {cb};
    return io_submit(ctx, 1, v);
}
/* takes whatever is ready, waiting at most 1 s for at least one */
static A_UNUSED i64 reap(u64 ctx, struct a_io_event *ev, i64 nr) {
    struct a_ts t = {1, 0};
    return io_getevents(ctx, 1, nr, ev, &t);
}
static A_UNUSED i64 ready(u64 ctx, struct a_io_event *ev, i64 nr) {
    struct a_ts z = {0, 0};
    return io_getevents(ctx, 0, nr, ev, &z);
}
/* one request: submit it, reap its event, return the event's res (or the submit error) */
static A_UNUSED i64 run1(u64 ctx, struct a_iocb *cb) {
    i64 r = submit1(ctx, cb);
    if (r != 1) return r == 0 ? -1000 : r;
    struct a_io_event ev[2];
    i64 g = reap(ctx, ev, 2);
    if (g != 1) return -2000 - g;
    return ev[0].res;
}
static A_UNUSED i64 file_size(i64 fd) {
    struct a_stat st;
    if (syscall2(SYS_fstat, fd, &st) != 0) return -1;
    return st.st_size;
}
#define D "/tmp/a_io_submit_shared"
static u64 ctx;
static i64 fd;
static struct a_iocb tcb;
static i64 tres;
static volatile i32 done;
static int thread_fn(void *arg) {
    (void)arg;
    tres = submit1(ctx, &tcb);
    return 0;
}
int test_main(void) {
    i64 r = io_setup(4, &ctx);
    check("setup_ok", r == 0, r);
    if (!ctx) return finish();
    fd = a_open(D, O_RDWR | O_CREAT | O_TRUNC, 0644);
    set_cb(&tcb, IOCB_CMD_PWRITE, fd, "t", 1, 0, 42);
    a_spawn(0, &done, thread_fn, 0);
    while (done) a_relax();
    expect("thread_submit", tres, 1);
    struct a_io_event ev[2];
    expect("parent_reaps", ready(ctx, ev, 2), 1);
    expect("thread_data", (i64)ev[0].data, 42);
    expect("thread_res", ev[0].res, 1);
    volatile i32 *sh = a_shared();
    sh[0] = sh[1] = sh[2] = -1000;
    i64 pid = a_fork_sync();
    if (pid == 0) {
        struct a_iocb cb;
        set_cb(&cb, IOCB_CMD_PWRITE, fd, "c", 1, 1, 7);
        sh[0] = (i32)submit1(ctx, &cb);
        u64 mine = 0;
        sh[1] = (i32)io_setup(1, &mine);
        sh[2] = mine ? (i32)run1(mine, &cb) : -1;
        syscall1(SYS_exit, 0);
    }
    check("fork", pid > 0, pid);
    expect("child_parent_ctx", sh[0], -EINVAL);
    expect("child_setup", sh[1], 0);
    expect("child_own_ctx", sh[2], 1);
    expect("parent_ctx_untouched", ready(ctx, ev, 2), 0);
    expect("size_2", file_size(fd), 2);
    i32 st = 0;
    syscall4(SYS_wait4, pid, &st, 0, 0);
    a_close(fd);
    a_unlink(D);
    expect("destroy", io_destroy(ctx), 0);
    return finish();
}
