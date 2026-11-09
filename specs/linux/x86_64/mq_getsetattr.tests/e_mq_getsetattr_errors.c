/* mq_getsetattr(2) errors: a closed fd or a descriptor that is not a
 * message queue (stdout, a pipe, a regular file) is EBADF; an unreadable
 * newattr or unwritable oldattr EFAULT. fcntl(F_SETFL, O_NONBLOCK) on a
 * queue descriptor changes the mq_flags it reports.
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

#define Q "e_mq_getsetattr_errors"

int test_main(void) {
    mq_unlink(Q);
    i64 fd = mq_small(Q, O_RDWR, 3, 16);
    check("create", fd == 3, fd);
    struct k_mq_attr o;
    i64 r = mq_getsetattr(99, 0, &o);
    check("closed_ebadf", r == -EBADF, r);
    r = mq_getsetattr(1, 0, &o);
    check("stdout_ebadf", r == -EBADF, r);
    int p[2];
    syscall2(SYS_pipe2, p, 0);
    r = mq_getsetattr(p[0], 0, &o);
    check("pipe_ebadf", r == -EBADF, r);
    r = mq_getsetattr((int)fd, (struct k_mq_attr *)8, &o);
    check("newattr_efault", r == -EFAULT, r);
    r = mq_getsetattr((int)fd, 0, (struct k_mq_attr *)8);
    check("oldattr_efault", r == -EFAULT, r);
    struct k_mq_attr junk;
    memset(&junk, 0, sizeof junk);
    junk.mq_flags = 1;
    r = mq_getsetattr(99, &junk, &o);
    check("flags_checked_before_fd", r == -EINVAL, r);
    r = mq_getsetattr(99, (struct k_mq_attr *)8, &o);
    check("newattr_copied_before_fd", r == -EFAULT, r);
    r = mq_getsetattr(99, 0, (struct k_mq_attr *)8);
    check("fd_checked_before_oldattr", r == -EBADF, r);

    check("fcntl_set_nonblock", syscall3(SYS_fcntl, fd, F_SETFL, O_NONBLOCK) == 0, 0);
    struct k_mq_attr a = mq_attr_of((int)fd);
    check("flags_follow_fcntl", a.mq_flags == O_NONBLOCK, a.mq_flags);
    char buf[16];
    unsigned pr;
    check("fcntl_nonblock_effective", mq_recv((int)fd, buf, sizeof buf, &pr, 0) == -EAGAIN, 0);
    close(p[0]);
    close(p[1]);
    close((int)fd);
    mq_unlink(Q);
    return finish();
}
