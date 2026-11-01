/* mq_timedsend(2) on a full queue (mq_curmsgs == mq_maxmsg): with
 * O_NONBLOCK it fails with EAGAIN at once; otherwise it blocks until room
 * appears or the absolute CLOCK_REALTIME abs_timeout passes (ETIMEDOUT; a
 * time already past times out at once, but is fine when there is room).
 * A given timeout is checked first, whether or not the call has to wait:
 * an invalid one (tv_nsec >= 1e9 or < 0, negative tv_sec) is EINVAL, an
 * unreadable one EFAULT, and nothing is queued.
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

#define Q "e_mq_timedsend_full"

int test_main(void) {
    mq_unlink(Q);
    i64 fd = mq_small(Q, O_RDWR | O_NONBLOCK, 2, 8);
    check("create", fd == 3, fd);
    struct k_timespec bad = { 0, 1000000000 };
    i64 r = mq_send((int)fd, "1", 1, 0, &bad);
    check("bad_ts_einval_with_room", r == -EINVAL, r);
    r = mq_send((int)fd, "1", 1, 0, (struct k_timespec *)8);
    check("ts_efault_with_room", r == -EFAULT, r);
    check("nothing_queued", mq_attr_of((int)fd).mq_curmsgs == 0, 0);
    struct k_timespec past0 = { 1, 0 };
    check("send_1_past_ts_room_ok", mq_send((int)fd, "1", 1, 0, &past0) == 0, 0);
    check("send_2", mq_send((int)fd, "2", 1, 0, 0) == 0, 0);
    r = mq_send((int)fd, "3", 1, 0, 0);
    check("full_nonblock_eagain", r == -EAGAIN, r);

    struct k_mq_attr blocking;
    memset(&blocking, 0, sizeof blocking);
    mq_getsetattr((int)fd, &blocking, 0);
    struct k_timespec t = abs_in_ms(60);
    i64 t0 = now_ns();
    r = mq_send((int)fd, "3", 1, 0, &t);
    i64 el = now_ns() - t0;
    check("full_blocking_etimedout", r == -ETIMEDOUT, r);
    check("waited", el >= 40000000, el / 1000000);
    struct k_timespec past = { 1, 0 };
    r = mq_send((int)fd, "3", 1, 0, &past);
    check("past_etimedout", r == -ETIMEDOUT, r);
    r = mq_send((int)fd, "3", 1, 0, &bad);
    check("bad_nsec_einval", r == -EINVAL, r);
    struct k_timespec neg = { -1, 0 };
    r = mq_send((int)fd, "3", 1, 0, &neg);
    check("negative_sec_einval", r == -EINVAL, r);
    r = mq_send((int)fd, "3", 1, 0, (struct k_timespec *)8);
    check("ts_efault", r == -EFAULT, r);
    check("still_two", mq_attr_of((int)fd).mq_curmsgs == 2, mq_attr_of((int)fd).mq_curmsgs);

    char buf[8];
    unsigned prio;
    mq_recv((int)fd, buf, sizeof buf, &prio, 0);
    t = abs_in_ms(1000);
    r = mq_send((int)fd, "3", 1, 0, &t);
    check("room_again", r == 0, r);
    close((int)fd);
    mq_unlink(Q);
    return finish();
}
