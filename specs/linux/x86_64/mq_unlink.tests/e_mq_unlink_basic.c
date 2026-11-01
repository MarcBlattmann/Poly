/* mq_unlink(2) removes a queue's name at once: a new mq_open without
 * O_CREAT then fails with ENOENT and O_CREAT|O_EXCL makes a new, empty
 * queue. Descriptors already open keep working on the old queue (its
 * messages stay readable) until closed. Errors: a missing name ENOENT,
 * an empty name ENOENT, a name with '/' EACCES, over 255 bytes
 * ENAMETOOLONG, an unreadable name EFAULT.
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

#define Q "e_mq_unlink_basic"

int test_main(void) {
    mq_unlink(Q);
    i64 fd = mq_small(Q, O_RDWR | O_NONBLOCK, 4, 32);
    check("create", fd == 3, fd);
    mq_send((int)fd, "old", 3, 0, 0);
    i64 r = mq_unlink(Q);
    check("unlink_ok", r == 0, r);
    r = mq_unlink(Q);
    check("unlink_again_enoent", r == -ENOENT, r);
    r = mq_open(Q, O_RDWR, 0, 0);
    check("open_after_unlink_enoent", r == -ENOENT, r);

    i64 fd2 = mq_small(Q, O_RDWR | O_EXCL | O_NONBLOCK, 4, 32);
    check("recreate_excl", fd2 == 4, fd2);
    check("new_queue_empty", mq_attr_of((int)fd2).mq_curmsgs == 0, 0);
    char buf[32];
    unsigned prio;
    i64 n = mq_recv((int)fd, buf, sizeof buf, &prio, 0);
    check("old_fd_old_message", n == 3 && mem_eq(buf, "old", 3), n);
    check("old_fd_still_sends", mq_send((int)fd, "x", 1, 0, 0) == 0, 0);
    check("new_queue_unaffected", mq_attr_of((int)fd2).mq_curmsgs == 0, 0);
    close((int)fd);
    close((int)fd2);

    r = mq_unlink("");
    check("empty_enoent", r == -ENOENT, r);
    r = mq_unlink("/e_mq_unlink_basic");
    check("slash_eacces", r == -EACCES, r);
    static char longname[300];
    memset(longname, 'u', 256);
    r = mq_unlink(longname);
    check("long_enametoolong", r == -ENAMETOOLONG, r);
    r = mq_unlink((const char *)8);
    check("efault", r == -EFAULT, r);
    check("final_unlink", mq_unlink(Q) == 0, 0);
    return finish();
}
