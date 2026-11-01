/* mq_open(2) errors: a name with a '/' (also a leading one, as the raw
 * call sees it), "." or ".." is EACCES; an empty name ENOENT; a name over
 * 255 bytes ENAMETOOLONG (255 is fine); an unreadable name or attr EFAULT;
 * no queue and no O_CREAT ENOENT; O_CREAT|O_EXCL on an existing queue
 * EEXIST; access mode 3 EINVAL; with O_CREAT of a new queue, mq_maxmsg or
 * mq_msgsize <= 0 EINVAL, and above msg_max (10) / msgsize_max (8192)
 * EINVAL for a caller without CAP_SYS_RESOURCE. An existing queue opened
 * with O_CREAT ignores attr entirely. Failures use no descriptor.
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

#define Q "e_mq_open_errors"

int test_main(void) {
    mq_unlink(Q);
    struct k_mq_attr a;
    memset(&a, 0, sizeof a);
    a.mq_maxmsg = 4;
    a.mq_msgsize = 64;
    i64 r;
    r = mq_open("/e_mq_open_errors", O_CREAT | O_RDWR, 0600, &a);
    check("leading_slash_eacces", r == -EACCES, r);
    r = mq_open("e_mq/x", O_CREAT | O_RDWR, 0600, &a);
    check("inner_slash_eacces", r == -EACCES, r);
    r = mq_open(".", O_CREAT | O_RDWR, 0600, &a);
    check("dot_eacces", r == -EACCES, r);
    r = mq_open("..", O_CREAT | O_RDWR, 0600, &a);
    check("dotdot_eacces", r == -EACCES, r);
    r = mq_open("", O_CREAT | O_RDWR, 0600, &a);
    check("empty_enoent", r == -ENOENT, r);
    static char longname[300];
    memset(longname, 'q', 256);
    r = mq_open(longname, O_CREAT | O_RDWR, 0600, &a);
    check("256_enametoolong", r == -ENAMETOOLONG, r);
    longname[255] = 0;
    r = mq_open(longname, O_CREAT | O_RDWR, 0600, &a);
    check("255_ok", r == 3, r);
    close(3);
    check("255_unlink", mq_unlink(longname) == 0, 0);
    r = mq_open((const char *)8, O_CREAT | O_RDWR, 0600, &a);
    check("name_efault", r == -EFAULT, r);
    r = mq_open(Q, O_CREAT | O_RDWR, 0600, (struct k_mq_attr *)8);
    check("attr_efault", r == -EFAULT, r);
    r = mq_open(Q, O_RDWR, 0600, 0);
    check("missing_enoent", r == -ENOENT, r);

    struct k_mq_attr b = a;
    b.mq_maxmsg = 0;
    r = mq_open(Q, O_CREAT | O_RDWR, 0600, &b);
    check("maxmsg_0_einval", r == -EINVAL, r);
    b = a;
    b.mq_msgsize = 0;
    r = mq_open(Q, O_CREAT | O_RDWR, 0600, &b);
    check("msgsize_0_einval", r == -EINVAL, r);
    b = a;
    b.mq_msgsize = -1;
    r = mq_open(Q, O_CREAT | O_RDWR, 0600, &b);
    check("msgsize_negative_einval", r == -EINVAL, r);
    b = a;
    b.mq_maxmsg = 65537;
    r = mq_open(Q, O_CREAT | O_RDWR, 0600, &b);
    check("maxmsg_above_hard_max_einval", r == -EINVAL, r);

    i64 fd = mq_open(Q, O_CREAT | O_RDWR, 0600, &a);
    check("create", fd == 3, fd);
    r = mq_open(Q, O_CREAT | O_EXCL | O_RDWR, 0600, &a);
    check("excl_eexist", r == -EEXIST, r);
    r = mq_open(Q, 3, 0, 0);
    check("accmode_3_einval", r == -EINVAL, r);
    b = a;
    b.mq_maxmsg = 0;
    r = mq_open(Q, O_CREAT | O_RDWR, 0600, &b);
    check("existing_ignores_attr", r == 4, r);
    close(4);
    r = mq_open(Q, O_CREAT | O_RDWR, 0600, (struct k_mq_attr *)8);
    check("existing_still_reads_attr_efault", r == -EFAULT, r);
    r = syscall4(SYS_openat, AT_FDCWD, "/", O_RDONLY, 0);
    check("no_fd_leaked", r == 4, r);
    close(4);
    close((int)fd);
    check("unlink", mq_unlink(Q) == 0, 0);
    return finish();
}
