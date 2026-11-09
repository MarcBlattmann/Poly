/* Reading a message-queue descriptor with read(2) gives a 60-byte status
 * line "QSIZE:<total bytes of queued messages, left-aligned in 10>
 * NOTIFY:<notify type, 5> SIGNO:<signal, 5> NOTIFY_PID:<pid, 6>\n"; write(2)
 * is EINVAL. Several descriptors from separate mq_open calls name the same
 * queue; a descriptor survives fork (the child uses the same queue).
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

#define Q "e_mq_open_readfd"

int test_main(void) {
    mq_unlink(Q);
    i64 fd = mq_small(Q, O_RDWR, 4, 64);
    check("create", fd == 3, fd);
    char buf[128];
    memset(buf, 0, sizeof buf);
    i64 n = read((int)fd, buf, sizeof buf);
    check("status_len", n == 60, n);
    check("status_empty", mem_eq(buf, "QSIZE:0          NOTIFY:0     SIGNO:0     NOTIFY_PID:0     \n", 60), buf[6]);
    mq_send((int)fd, "hello", 5, 0, 0);
    mq_send((int)fd, "abc", 3, 2, 0);
    syscall3(SYS_lseek, fd, 0, 0);
    memset(buf, 0, sizeof buf);
    n = read((int)fd, buf, sizeof buf);
    check("status_qsize_8", n == 60 && mem_eq(buf, "QSIZE:8          NOTIFY:0", 25), buf[6]);
    n = write((int)fd, "x", 1);
    check("write_einval", n == -EINVAL, n);

    i64 pid = fork_sync();
    if (pid == 0) {
        char m[64];
        unsigned prio = 9;
        i64 got = mq_recv((int)fd, m, sizeof m, &prio, 0);
        exit_group(got == 3 && prio == 2 && mem_eq(m, "abc", 3) ? 0 : 1);
    }
    int st = wait_exit(pid);
    check("child_received", st == 0, st);
    i64 fd2 = mq_open(Q, O_RDONLY, 0, 0);
    check("second_open", fd2 == 4, fd2);
    check("one_left", mq_attr_of((int)fd2).mq_curmsgs == 1, mq_attr_of((int)fd2).mq_curmsgs);
    close((int)fd2);
    close((int)fd);
    mq_unlink(Q);
    return finish();
}
