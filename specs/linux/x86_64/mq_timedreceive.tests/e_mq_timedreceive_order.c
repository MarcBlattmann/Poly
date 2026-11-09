/* Message order across many priorities: mq_timedreceive(2) always returns
 * the highest priority present, FIFO within a priority, also when sends
 * and receives interleave. Message contents are kept byte for byte,
 * including NUL bytes; message boundaries are kept.
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

#define Q "e_mq_timedreceive_order"

int test_main(void) {
    mq_unlink(Q);
    i64 fd = mq_small(Q, O_RDWR | O_NONBLOCK, 10, 8);
    check("create", fd == 3, fd);
    static const unsigned pr[8] = { 3, 0, 3, 7, 0, 7, 1, 3 };
    for (int i = 0; i < 8; i++) {
        char m[2] = { (char)('a' + i), 0 };
        mq_send((int)fd, m, 2, pr[i], 0);
    }
    /* Expected: prio 7: d f; prio 3: a c h; prio 1: g; prio 0: b e. */
    static const char exp1[3] = { 'd', 'f', 'a' };
    int ok = 1;
    char buf[8];
    unsigned prio;
    for (int i = 0; i < 3; i++) {
        i64 n = mq_recv((int)fd, buf, sizeof buf, &prio, 0);
        if (n != 2 || buf[0] != exp1[i] || buf[1] != 0) ok = 0;
    }
    check("first_three", ok, 0);
    mq_send((int)fd, "X", 1, 3, 0);
    mq_send((int)fd, "Y", 1, 5, 0);
    static const char exp2[6] = { 'Y', 'c', 'h', 'X', 'g', 'b' };
    static const unsigned pr2[6] = { 5, 3, 3, 3, 1, 0 };
    ok = 1;
    for (int i = 0; i < 6; i++) {
        i64 n = mq_recv((int)fd, buf, sizeof buf, &prio, 0);
        if (n < 1 || buf[0] != exp2[i] || prio != pr2[i]) ok = 0;
    }
    check("interleaved_order", ok, 0);
    i64 n = mq_recv((int)fd, buf, sizeof buf, &prio, 0);
    check("last_e", n == 2 && buf[0] == 'e' && prio == 0, n);
    char bin[8] = { 0, 1, 0, 2, 0, 0, 3, 0 };
    mq_send((int)fd, bin, 8, 0, 0);
    mq_send((int)fd, bin, 3, 0, 0);
    char got[8];
    n = mq_recv((int)fd, got, sizeof got, &prio, 0);
    check("binary_kept", n == 8 && mem_eq(got, bin, 8), n);
    n = mq_recv((int)fd, got, sizeof got, &prio, 0);
    check("boundary_kept", n == 3, n);
    close((int)fd);
    mq_unlink(Q);
    return finish();
}
