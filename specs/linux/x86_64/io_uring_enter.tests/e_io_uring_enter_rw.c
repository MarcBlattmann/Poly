/* IORING_OP_READ / WRITE (addr = buffer, len = bytes, off = file offset)
 * and READV / WRITEV (addr = iovec array, len = count). res is the byte
 * count or -errno. off = -1 (IORING_FEAT_RW_CUR_POS) uses and advances the
 * file position like read(2)/write(2); an explicit offset leaves it alone.
 * A read past EOF gives 0, an unwritable buffer -EFAULT, a write on a
 * read-only fd -EBADF.
 */
#include "poly_ipc.h"

/* asm/unistd_64.h */
#define SYS_io_uring_setup 425
#define SYS_io_uring_enter 426
#define SYS_io_uring_register 427
#define SYS_statx 332
#define SYS_pread64 17
/* asm-generic/errno-base.h, asm-generic/errno.h */
#define EBADF 9
#define ENOMEM 12
#define EFAULT 14
#define EINVAL 22
#define ENXIO 6
#define ENOSYS 38
#define ECANCELED 125
#define ETIME 62
#define EALREADY 114
#define EBADFD 77
#define EOVERFLOW 75
/* asm-generic/mman-common.h */
#define MAP_POPULATE 0x8000

/* linux/io_uring.h */
#define IORING_SETUP_IOPOLL (1U << 0)
#define IORING_SETUP_SQPOLL (1U << 1)
#define IORING_SETUP_SQ_AFF (1U << 2)
#define IORING_SETUP_CQSIZE (1U << 3)
#define IORING_SETUP_CLAMP (1U << 4)
#define IORING_SETUP_ATTACH_WQ (1U << 5)
#define IORING_SETUP_R_DISABLED (1U << 6)
#define IORING_SETUP_SUBMIT_ALL (1U << 7)
#define IORING_SETUP_NO_SQARRAY (1U << 16)
#define IORING_FEAT_SINGLE_MMAP (1U << 0)
#define IORING_FEAT_NODROP (1U << 1)
#define IORING_FEAT_SUBMIT_STABLE (1U << 2)
#define IORING_FEAT_RW_CUR_POS (1U << 3)
#define IORING_FEAT_EXT_ARG (1U << 8)
#define IORING_OFF_SQ_RING 0UL
#define IORING_OFF_CQ_RING 0x8000000UL
#define IORING_OFF_SQES 0x10000000UL
#define IORING_ENTER_GETEVENTS (1U << 0)
#define IORING_ENTER_SQ_WAKEUP (1U << 1)
#define IORING_ENTER_SQ_WAIT (1U << 2)
#define IORING_ENTER_EXT_ARG (1U << 3)
#define IOSQE_FIXED_FILE (1U << 0)
#define IOSQE_IO_DRAIN (1U << 1)
#define IOSQE_IO_LINK (1U << 2)
#define IOSQE_IO_HARDLINK (1U << 3)
#define IOSQE_ASYNC (1U << 4)
#define IOSQE_CQE_SKIP_SUCCESS (1U << 6)
#define IORING_TIMEOUT_ABS (1U << 0)
#define IORING_FSYNC_DATASYNC (1U << 0)
#define IORING_POLL_ADD_MULTI (1U << 0)
#define IORING_CQE_F_MORE (1U << 1)

#define IORING_OP_NOP 0
#define IORING_OP_READV 1
#define IORING_OP_WRITEV 2
#define IORING_OP_FSYNC 3
#define IORING_OP_READ_FIXED 4
#define IORING_OP_WRITE_FIXED 5
#define IORING_OP_POLL_ADD 6
#define IORING_OP_POLL_REMOVE 7
#define IORING_OP_TIMEOUT 11
#define IORING_OP_TIMEOUT_REMOVE 12
#define IORING_OP_ACCEPT 13
#define IORING_OP_ASYNC_CANCEL 14
#define IORING_OP_CONNECT 16
#define IORING_OP_OPENAT 18
#define IORING_OP_CLOSE 19
#define IORING_OP_STATX 21
#define IORING_OP_READ 22
#define IORING_OP_WRITE 23
#define IORING_OP_SEND 26
#define IORING_OP_RECV 27

#define IORING_REGISTER_BUFFERS 0
#define IORING_UNREGISTER_BUFFERS 1
#define IORING_REGISTER_FILES 2
#define IORING_UNREGISTER_FILES 3
#define IORING_REGISTER_EVENTFD 4
#define IORING_UNREGISTER_EVENTFD 5
#define IORING_REGISTER_FILES_UPDATE 6
#define IORING_REGISTER_PROBE 8

/* struct io_sqring_offsets / io_cqring_offsets (40 bytes each). */
struct k_sqring_offsets {
    unsigned head, tail, ring_mask, ring_entries, flags, dropped, array, resv1;
    u64 user_addr;
};
struct k_cqring_offsets {
    unsigned head, tail, ring_mask, ring_entries, overflow, cqes, flags, resv1;
    u64 user_addr;
};
/* struct io_uring_params (120 bytes). */
struct k_uring_params {
    unsigned sq_entries, cq_entries, flags, sq_thread_cpu, sq_thread_idle, features, wq_fd;
    unsigned resv[3];
    struct k_sqring_offsets sq_off;
    struct k_cqring_offsets cq_off;
};
/* struct io_uring_sqe (64 bytes): the fields the tests use. */
struct k_sqe {
    unsigned char opcode;
    unsigned char flags;
    unsigned short ioprio;
    int fd;
    u64 off;   /* or addr2 */
    u64 addr;  /* or splice_off_in */
    unsigned len;
    unsigned op_flags; /* rw_flags, fsync_flags, poll32_events, timeout_flags, accept_flags, open_flags, statx_flags, msg_flags, ... */
    u64 user_data;
    unsigned short buf_index;
    unsigned short personality;
    int file_index; /* or splice_fd_in */
    u64 addr3;
    u64 pad2;
};
/* struct io_uring_cqe (16 bytes). */
struct k_cqe {
    u64 user_data;
    int res;
    unsigned flags;
};

/* A ring mapped the way liburing maps it. */
struct ring {
    int fd;
    struct k_uring_params p;
    unsigned char *sq_ring, *cq_ring;
    struct k_sqe *sqes;
    unsigned *sq_head, *sq_tail, *sq_mask, *sq_entries, *sq_flags, *sq_array;
    unsigned *cq_head, *cq_tail, *cq_mask, *cq_entries, *cq_overflow;
    struct k_cqe *cqes;
    unsigned pending;
};

static __attribute__((unused)) i64 uring_setup(unsigned entries, struct k_uring_params *p) {
    return syscall2(SYS_io_uring_setup, entries, p);
}
static __attribute__((unused)) i64 uring_enter(int fd, unsigned to_submit, unsigned min_complete, unsigned flags,
                                               void *arg, u64 argsz) {
    return syscall6(SYS_io_uring_enter, fd, to_submit, min_complete, flags, (i64)arg, (i64)argsz);
}
static __attribute__((unused)) i64 uring_register(int fd, unsigned op, void *arg, unsigned nr) {
    return syscall4(SYS_io_uring_register, fd, op, arg, nr);
}
static __attribute__((unused)) void *map_ring(int fd, u64 len, u64 off) {
    return (void *)syscall6(SYS_mmap, 0, (i64)len, PROT_READ | PROT_WRITE, MAP_SHARED | MAP_POPULATE, fd, (i64)off);
}
#define BAD_MAP(p) ((u64)(p) > (u64)-4096L)

/* Set up a ring with `entries` and `flags`; 0 on success, -errno. */
static __attribute__((unused)) i64 ring_init(struct ring *r, unsigned entries, unsigned flags) {
    memset(r, 0, sizeof *r);
    r->p.flags = flags;
    i64 fd = uring_setup(entries, &r->p);
    if (fd < 0) return fd;
    r->fd = (int)fd;
    u64 sq_len = r->p.sq_off.array + r->p.sq_entries * 4;
    u64 cq_len = r->p.cq_off.cqes + r->p.cq_entries * sizeof(struct k_cqe);
    if (r->p.features & IORING_FEAT_SINGLE_MMAP) {
        if (cq_len > sq_len) sq_len = cq_len;
        r->sq_ring = map_ring(r->fd, sq_len, IORING_OFF_SQ_RING);
        r->cq_ring = r->sq_ring;
    } else {
        r->sq_ring = map_ring(r->fd, sq_len, IORING_OFF_SQ_RING);
        r->cq_ring = map_ring(r->fd, cq_len, IORING_OFF_CQ_RING);
    }
    if (BAD_MAP(r->sq_ring) || BAD_MAP(r->cq_ring)) return -ENOMEM;
    r->sqes = map_ring(r->fd, r->p.sq_entries * sizeof(struct k_sqe), IORING_OFF_SQES);
    if (BAD_MAP(r->sqes)) return -ENOMEM;
    r->sq_head = (unsigned *)(r->sq_ring + r->p.sq_off.head);
    r->sq_tail = (unsigned *)(r->sq_ring + r->p.sq_off.tail);
    r->sq_mask = (unsigned *)(r->sq_ring + r->p.sq_off.ring_mask);
    r->sq_entries = (unsigned *)(r->sq_ring + r->p.sq_off.ring_entries);
    r->sq_flags = (unsigned *)(r->sq_ring + r->p.sq_off.flags);
    r->sq_array = (unsigned *)(r->sq_ring + r->p.sq_off.array);
    r->cq_head = (unsigned *)(r->cq_ring + r->p.cq_off.head);
    r->cq_tail = (unsigned *)(r->cq_ring + r->p.cq_off.tail);
    r->cq_mask = (unsigned *)(r->cq_ring + r->p.cq_off.ring_mask);
    r->cq_entries = (unsigned *)(r->cq_ring + r->p.cq_off.ring_entries);
    r->cq_overflow = (unsigned *)(r->cq_ring + r->p.cq_off.overflow);
    r->cqes = (struct k_cqe *)(r->cq_ring + r->p.cq_off.cqes);
    return 0;
}

/* The next free SQE (zeroed), published in the SQ array; 0 when full. */
static __attribute__((unused)) struct k_sqe *get_sqe(struct ring *r) {
    unsigned head = __atomic_load_n(r->sq_head, __ATOMIC_ACQUIRE);
    unsigned tail = *r->sq_tail + r->pending;
    if (tail - head >= *r->sq_entries) return 0;
    unsigned idx = tail & *r->sq_mask;
    struct k_sqe *s = &r->sqes[idx];
    memset(s, 0, sizeof *s);
    r->sq_array[idx] = idx;
    r->pending++;
    return s;
}
/* Make the pending SQEs visible to the kernel (advance the SQ tail). */
static __attribute__((unused)) unsigned flush_sq(struct ring *r) {
    unsigned n = r->pending;
    __atomic_store_n(r->sq_tail, *r->sq_tail + n, __ATOMIC_RELEASE);
    r->pending = 0;
    return n;
}
/* Submit everything pending, waiting for `wait` completions. */
static __attribute__((unused)) i64 submit_wait(struct ring *r, unsigned wait) {
    unsigned n = flush_sq(r);
    return uring_enter(r->fd, n, wait, wait ? IORING_ENTER_GETEVENTS : 0, 0, 0);
}
static __attribute__((unused)) unsigned cq_ready(struct ring *r) {
    return __atomic_load_n(r->cq_tail, __ATOMIC_ACQUIRE) - *r->cq_head;
}
/* Pop one CQE into *out; 0 when the CQ ring is empty. */
static __attribute__((unused)) int pop_cqe(struct ring *r, struct k_cqe *out) {
    unsigned head = *r->cq_head;
    if (head == __atomic_load_n(r->cq_tail, __ATOMIC_ACQUIRE)) return 0;
    *out = r->cqes[head & *r->cq_mask];
    __atomic_store_n(r->cq_head, head + 1, __ATOMIC_RELEASE);
    return 1;
}
/* Wait for one CQE (io_uring_enter GETEVENTS) and pop it; -errno or 1. */
static __attribute__((unused)) i64 wait_cqe(struct ring *r, struct k_cqe *out) {
    if (pop_cqe(r, out)) return 1;
    i64 e = uring_enter(r->fd, 0, 1, IORING_ENTER_GETEVENTS, 0, 0);
    if (e < 0) return e;
    return pop_cqe(r, out);
}
static __attribute__((unused)) void prep(struct k_sqe *s, int op, int fd, u64 addr, unsigned len, u64 off, u64 ud) {
    s->opcode = (unsigned char)op;
    s->fd = fd;
    s->addr = addr;
    s->len = len;
    s->off = off;
    s->user_data = ud;
}
static __attribute__((unused)) i64 open3(const char *path, i64 flags, i64 mode) {
    return syscall4(SYS_openat, AT_FDCWD, path, flags, mode);
}
static __attribute__((unused)) i64 unlink_path(const char *path) { return syscall3(SYS_unlinkat, AT_FDCWD, path, 0); }
struct k_iov {
    void *base;
    u64 len;
};

#define F "/tmp/e_io_uring_enter_rw"

int test_main(void) {
    struct ring r;
    check("init", ring_init(&r, 8, 0) == 0, 0);
    struct k_cqe c;
    unlink_path(F);
    int fd = (int)open3(F, O_RDWR | O_CREAT | O_TRUNC, 0600);

    prep(get_sqe(&r), IORING_OP_WRITE, fd, (u64) "hello world", 11, 0, 1);
    check("write_submit", submit_wait(&r, 1) == 1, 0);
    check("write_res", pop_cqe(&r, &c) == 1 && c.res == 11 && c.user_data == 1, c.res);
    check("pos_unchanged", syscall3(SYS_lseek, fd, 0, 1) == 0, syscall3(SYS_lseek, fd, 0, 1));

    char buf[16];
    memset(buf, 0, sizeof buf);
    prep(get_sqe(&r), IORING_OP_READ, fd, (u64)buf, 5, 6, 2);
    submit_wait(&r, 1);
    check("read_res", pop_cqe(&r, &c) == 1 && c.res == 5, c.res);
    check("read_data", mem_eq(buf, "world", 5), buf[0]);

    memset(buf, 0, sizeof buf);
    prep(get_sqe(&r), IORING_OP_READ, fd, (u64)buf, 3, (u64)-1, 3);
    submit_wait(&r, 1);
    check("cur_pos_read", pop_cqe(&r, &c) == 1 && c.res == 3 && mem_eq(buf, "hel", 3), c.res);
    check("cur_pos_advanced", syscall3(SYS_lseek, fd, 0, 1) == 3, syscall3(SYS_lseek, fd, 0, 1));
    prep(get_sqe(&r), IORING_OP_READ, fd, (u64)buf, 2, (u64)-1, 4);
    submit_wait(&r, 1);
    check("cur_pos_again", pop_cqe(&r, &c) == 1 && c.res == 2 && mem_eq(buf, "lo", 2), c.res);

    prep(get_sqe(&r), IORING_OP_READ, fd, (u64)buf, 4, 100, 5);
    submit_wait(&r, 1);
    check("past_eof_zero", pop_cqe(&r, &c) == 1 && c.res == 0, c.res);

    struct k_iov wv[2] = { { "AB", 2 }, { "CDE", 3 } };
    prep(get_sqe(&r), IORING_OP_WRITEV, fd, (u64)wv, 2, 20, 6);
    submit_wait(&r, 1);
    check("writev_res", pop_cqe(&r, &c) == 1 && c.res == 5, c.res);
    char x[3] = { 0 }, y[4] = { 0 };
    struct k_iov rv[2] = { { x, 2 }, { y, 3 } };
    prep(get_sqe(&r), IORING_OP_READV, fd, (u64)rv, 2, 20, 7);
    submit_wait(&r, 1);
    check("readv_res", pop_cqe(&r, &c) == 1 && c.res == 5, c.res);
    check("readv_data", mem_eq(x, "AB", 2) && mem_eq(y, "CDE", 3), x[0]);

    prep(get_sqe(&r), IORING_OP_READ, fd, 8, 4, 0, 8);
    submit_wait(&r, 1);
    check("bad_buffer_efault", pop_cqe(&r, &c) == 1 && c.res == -EFAULT, c.res);

    int ro = (int)open3(F, O_RDONLY, 0);
    prep(get_sqe(&r), IORING_OP_WRITE, ro, (u64) "z", 1, 0, 9);
    submit_wait(&r, 1);
    check("write_rdonly_ebadf", pop_cqe(&r, &c) == 1 && c.res == -EBADF, c.res);

    /* Two linked-free SQEs in one enter: both complete, in order. */
    prep(get_sqe(&r), IORING_OP_WRITE, fd, (u64) "12", 2, 0, 10);
    prep(get_sqe(&r), IORING_OP_WRITE, fd, (u64) "34", 2, 2, 11);
    i64 n = submit_wait(&r, 2);
    check("two_submitted", n == 2, n);
    struct k_cqe c2;
    check("two_cqes", pop_cqe(&r, &c) == 1 && pop_cqe(&r, &c2) == 1 && c.res == 2 && c2.res == 2, 0);
    memset(buf, 0, sizeof buf);
    syscall4(SYS_pread64, fd, buf, 4, 0);
    check("two_writes_landed", mem_eq(buf, "1234", 4), buf[0]);

    close(ro);
    close(fd);
    close(r.fd);
    unlink_path(F);
    return finish();
}
