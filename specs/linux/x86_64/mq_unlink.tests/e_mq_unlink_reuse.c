/* After mq_unlink(2) the old queue lives on for its open descriptors and
 * a queue made under the same name is a different one: messages sent to
 * one are not seen by the other, and the old queue disappears when its
 * last descriptor closes (its name is not resurrected).
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

#define Q "e_mq_unlink_reuse"

int test_main(void) {
    mq_unlink(Q);
    i64 a = mq_small(Q, O_RDWR | O_NONBLOCK, 2, 16);
    check("first", a == 3, a);
    check("unlink", mq_unlink(Q) == 0, 0);
    i64 b = mq_small(Q, O_RDWR | O_NONBLOCK, 3, 24);
    check("second", b == 4, b);
    check("different_attr", mq_attr_of((int)a).mq_maxmsg == 2 && mq_attr_of((int)b).mq_maxmsg == 3, 0);
    mq_send((int)a, "to-a", 4, 0, 0);
    mq_send((int)b, "to-b", 4, 0, 0);
    char buf[32];
    unsigned prio;
    i64 n = mq_recv((int)b, buf, sizeof buf, &prio, 0);
    check("b_gets_b", n == 4 && mem_eq(buf, "to-b", 4), n);
    n = mq_recv((int)b, buf, sizeof buf, &prio, 0);
    check("b_empty", n == -EAGAIN, n);
    n = mq_recv((int)a, buf, sizeof buf, &prio, 0);
    check("a_gets_a", n == 4 && mem_eq(buf, "to-a", 4), n);
    close((int)a);
    i64 c = mq_open(Q, O_RDONLY, 0, 0);
    check("name_is_second", c == 3 && mq_attr_of((int)c).mq_maxmsg == 3, c);
    close((int)c);
    close((int)b);
    check("unlink_second", mq_unlink(Q) == 0, 0);
    check("gone", mq_open(Q, O_RDONLY, 0, 0) == -ENOENT, 0);
    return finish();
}
