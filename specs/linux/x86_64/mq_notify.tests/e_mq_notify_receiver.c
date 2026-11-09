/* No notification is sent while another process is blocked in
 * mq_timedreceive on the queue: the message goes to the receiver and the
 * registration stays. Closing the descriptor through which the process
 * registered removes the registration, so another one can register.
 * poly-diff: syscalls=off (two processes)
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

#define Q "e_mq_notify_receiver"

static volatile int hits;
static void on_usr2(int s) {
    (void)s;
    hits++;
}

int test_main(void) {
    mq_unlink(Q);
    i64 fd = mq_small(Q, O_RDWR, 4, 8);
    check("create", fd == 3, fd);
    sig_handler(SIGUSR2, on_usr2, 0);
    struct k_sigevent se;
    memset(&se, 0, sizeof se);
    se.sigev_notify = SIGEV_SIGNAL;
    se.sigev_signo = SIGUSR2;
    check("register", mq_notify((int)fd, &se) == 0, 0);

    i64 pid = fork();
    if (pid == 0) {
        char b[8];
        unsigned p;
        struct k_timespec t = abs_in_ms(5000);
        exit_group(mq_recv((int)fd, b, sizeof b, &p, &t) == 1 && b[0] == 'r' ? 0 : 1);
    }
    sleep_ms(80);
    check("send", mq_send((int)fd, "r", 1, 0, 0) == 0, 0);
    check("child_got_it", wait_exit(pid) == 0, 0);
    check("no_signal", hits == 0, hits);
    check("still_registered", mq_notify((int)fd, &se) == -EBUSY, 0);
    mq_send((int)fd, "s", 1, 0, 0);
    check("now_signalled", hits == 1, hits);

    check("register_again", mq_notify((int)fd, &se) == 0, 0);
    i64 fd2 = mq_open(Q, O_RDWR, 0, 0);
    close((int)fd);
    pid = fork_sync();
    if (pid == 0) {
        struct k_sigevent s2;
        memset(&s2, 0, sizeof s2);
        s2.sigev_notify = SIGEV_NONE;
        exit_group(mq_notify((int)fd2, &s2) == 0 ? 0 : 1);
    }
    check("close_drops_registration", wait_exit(pid) == 0, 0);
    close((int)fd2);
    mq_unlink(Q);
    return finish();
}
