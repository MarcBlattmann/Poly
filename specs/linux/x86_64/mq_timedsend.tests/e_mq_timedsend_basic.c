/* mq_timedsend(2) queues a message of msg_len bytes (0 allowed, up to
 * mq_msgsize) with priority msg_prio (0..32767). Receivers get the highest
 * priority first and, within a priority, the oldest first. mq_curmsgs
 * counts queued messages. Errors: msg_len > mq_msgsize EMSGSIZE; prio >=
 * 32768 EINVAL; a descriptor not open for writing, closed, or not a queue
 * EBADF; an unreadable message EFAULT.
 */
#include "poly_ipc.h"

/* asm/unistd_64.h */
#define SYS_fstat 5
#define SYS_lseek 8
#define SYS_execve 59
#define SYS_umask 95
#define SYS_mq_open 240
#define SYS_mq_unlink 241
#define SYS_mq_timedsend 242
#define SYS_mq_timedreceive 243
#define SYS_mq_notify 244
#define SYS_mq_getsetattr 245
/* asm-generic/errno-base.h, asm-generic/errno.h */
#define EBADF 9
#define EFAULT 14
#define EINVAL 22
#define EMSGSIZE 90
/* asm-generic/fcntl.h */
#define O_WRONLY 1
#define O_TRUNC 01000
/* asm-generic/siginfo.h: sigev_notify values and si_code SI_MESGQ */
#define SIGEV_SIGNAL 0
#define SIGEV_NONE 1
#define SIGEV_THREAD 2
#define SI_MESGQ (-3)

/* struct mq_attr (linux/mqueue.h): 64 bytes. */
struct k_mq_attr {
    i64 mq_flags, mq_maxmsg, mq_msgsize, mq_curmsgs;
    i64 reserved[4];
};
/* struct sigevent (asm-generic/siginfo.h): 64 bytes. */
struct k_sigevent {
    u64 sigev_value;
    int sigev_signo;
    int sigev_notify;
    int pad[12];
};

static __attribute__((unused)) i64 mq_open(const char *name, i64 oflag, i64 mode, struct k_mq_attr *attr) {
    return syscall4(SYS_mq_open, name, oflag, mode, attr);
}
static __attribute__((unused)) i64 mq_unlink(const char *name) { return syscall1(SYS_mq_unlink, name); }
static __attribute__((unused)) i64 mq_send(int fd, const void *m, u64 len, unsigned prio, struct k_timespec *t) {
    return syscall5(SYS_mq_timedsend, fd, m, len, prio, t);
}
static __attribute__((unused)) i64 mq_recv(int fd, void *m, u64 len, unsigned *prio, struct k_timespec *t) {
    return syscall5(SYS_mq_timedreceive, fd, m, len, prio, t);
}
static __attribute__((unused)) i64 mq_notify(int fd, struct k_sigevent *se) { return syscall2(SYS_mq_notify, fd, se); }
static __attribute__((unused)) i64 mq_getsetattr(int fd, struct k_mq_attr *n, struct k_mq_attr *o) {
    return syscall3(SYS_mq_getsetattr, fd, n, o);
}
/* A small queue that fits any user's RLIMIT_MSGQUEUE many times over. */
static __attribute__((unused)) i64 mq_small(const char *name, i64 oflag, i64 maxmsg, i64 msgsize) {
    struct k_mq_attr a;
    memset(&a, 0, sizeof a);
    a.mq_maxmsg = maxmsg;
    a.mq_msgsize = msgsize;
    return mq_open(name, oflag | O_CREAT, 0600, &a);
}
static __attribute__((unused)) struct k_mq_attr mq_attr_of(int fd) {
    struct k_mq_attr a;
    memset(&a, 0x55, sizeof a);
    mq_getsetattr(fd, 0, &a);
    return a;
}
/* An absolute CLOCK_REALTIME time `ms` from now. */
static __attribute__((unused)) struct k_timespec abs_in_ms(i64 ms) {
    struct k_timespec t;
    syscall2(SYS_clock_gettime, CLOCK_REALTIME, &t);
    t.tv_nsec += (ms % 1000) * 1000000L;
    t.tv_sec += ms / 1000 + t.tv_nsec / 1000000000L;
    t.tv_nsec %= 1000000000L;
    return t;
}
static __attribute__((unused)) i64 fork_sync(void) {
    sig_mask(SIG_BLOCK, SIGBIT(SIGCHLD), 0);
    return syscall5(SYS_clone, 0x4000 /* CLONE_VFORK */ | SIGCHLD, 0, 0, 0, 0);
}

#define Q "e_mq_timedsend_basic"

int test_main(void) {
    mq_unlink(Q);
    i64 fd = mq_small(Q, O_RDWR | O_NONBLOCK, 8, 16);
    check("create", fd == 3, fd);
    check("send_p1_a", mq_send((int)fd, "a", 1, 1, 0) == 0, 0);
    check("send_p5_b", mq_send((int)fd, "bb", 2, 5, 0) == 0, 0);
    check("send_p1_c", mq_send((int)fd, "ccc", 3, 1, 0) == 0, 0);
    check("send_p32767", mq_send((int)fd, "max", 3, 32767, 0) == 0, 0);
    check("send_empty", mq_send((int)fd, "", 0, 0, 0) == 0, 0);
    check("send_full_size", mq_send((int)fd, "0123456789abcdef", 16, 5, 0) == 0, 0);
    check("curmsgs_6", mq_attr_of((int)fd).mq_curmsgs == 6, mq_attr_of((int)fd).mq_curmsgs);

    static const char *want[6] = { "max", "bb", "0123456789abcdef", "a", "ccc", "" };
    static const unsigned wprio[6] = { 32767, 5, 5, 1, 1, 0 };
    int ok = 1;
    for (int i = 0; i < 6; i++) {
        char buf[16];
        unsigned prio = 99;
        i64 n = mq_recv((int)fd, buf, sizeof buf, &prio, 0);
        if (n != (i64)str_len(want[i]) || !mem_eq(buf, want[i], (u64)n) || prio != wprio[i]) ok = 0;
    }
    check("priority_then_fifo", ok, 0);

    i64 r = mq_send((int)fd, "0123456789abcdefX", 17, 0, 0);
    check("too_long_emsgsize", r == -EMSGSIZE, r);
    r = mq_send((int)fd, "x", 1, 32768, 0);
    check("prio_32768_einval", r == -EINVAL, r);
    r = mq_send((int)fd, (const void *)8, 4, 0, 0);
    check("msg_efault", r == -EFAULT, r);
    r = mq_send(99, "x", 1, 0, 0);
    check("closed_ebadf", r == -EBADF, r);
    r = mq_send(1, "x", 1, 0, 0);
    check("not_queue_ebadf", r == -EBADF, r);
    i64 ro = mq_open(Q, O_RDONLY, 0, 0);
    r = mq_send((int)ro, "x", 1, 0, 0);
    check("rdonly_ebadf", r == -EBADF, r);
    r = mq_send(99, "x", 1, 32768, 0);
    check("prio_checked_before_fd", r == -EINVAL, r);
    r = mq_send((int)ro, "0123456789abcdefX", 17, 0, 0);
    check("access_checked_before_length", r == -EBADF, r);
    r = mq_send((int)fd, (const void *)8, 17, 0, 0);
    check("length_checked_before_copy", r == -EMSGSIZE, r);
    check("nothing_queued_by_errors", mq_attr_of((int)fd).mq_curmsgs == 0, 0);
    close((int)ro);
    close((int)fd);
    mq_unlink(Q);
    return finish();
}
