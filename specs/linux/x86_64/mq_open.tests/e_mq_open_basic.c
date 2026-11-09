/* mq_open(2) creates or opens a POSIX message queue. The raw call takes
 * the name without the leading slash. It returns a descriptor (lowest
 * free, always close-on-exec) whose file status flags hold the access
 * mode and O_NONBLOCK. Without an attr the queue gets the defaults
 * (mq_maxmsg 10 = fs.mqueue.msg_default, mq_msgsize 8192 =
 * msgsize_default); with one, its mq_maxmsg and mq_msgsize (mq_flags and
 * mq_curmsgs are ignored). The mode is masked by the umask (022). The
 * queue persists after its last descriptor is closed until mq_unlink.
 * O_TRUNC and O_DIRECTORY are accepted and ignored.
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

#define Q "e_mq_open_basic"

int test_main(void) {
    mq_unlink(Q);
    i64 fd = mq_open(Q, O_CREAT | O_RDWR | O_NONBLOCK, 0666, 0);
    check("create_fd", fd == 3, fd);
    check("cloexec_always", syscall2(SYS_fcntl, fd, F_GETFD) == FD_CLOEXEC, syscall2(SYS_fcntl, fd, F_GETFD));
    check("status_flags", syscall2(SYS_fcntl, fd, F_GETFL) == (O_RDWR | O_NONBLOCK), syscall2(SYS_fcntl, fd, F_GETFL));
    struct k_mq_attr a = mq_attr_of((int)fd);
    check("default_maxmsg", a.mq_maxmsg == 10, a.mq_maxmsg);
    check("default_msgsize", a.mq_msgsize == 8192, a.mq_msgsize);
    check("flags_nonblock", a.mq_flags == O_NONBLOCK, a.mq_flags);
    check("curmsgs_0", a.mq_curmsgs == 0, a.mq_curmsgs);
    struct {
        u64 dev, ino, nlink;
        unsigned mode, uid, gid, pad;
        u64 rdev;
        i64 size;
        char rest[80];
    } st;
    memset(&st, 0, sizeof st);
    check("fstat", syscall2(SYS_fstat, fd, &st) == 0, 0);
    check("mode_regular_umask", st.mode == (0100000 | 0644), st.mode);
    check("owner_is_caller", st.uid == (unsigned)syscall0(SYS_geteuid), st.uid);

    check("send", mq_send((int)fd, "kept", 4, 1, 0) == 0, 0);
    close((int)fd);
    fd = mq_open(Q, O_RDONLY, 0, 0);
    check("reopen_existing", fd == 3, fd);
    char buf[8192];
    unsigned prio = 0;
    i64 n = mq_recv((int)fd, buf, sizeof buf, &prio, 0);
    check("message_survived_close", n == 4 && mem_eq(buf, "kept", 4) && prio == 1, n);
    check("rdonly_flags", syscall2(SYS_fcntl, fd, F_GETFL) == O_RDONLY, syscall2(SYS_fcntl, fd, F_GETFL));
    close((int)fd);

    struct k_mq_attr want;
    memset(&want, 0, sizeof want);
    want.mq_flags = 12345;
    want.mq_maxmsg = 3;
    want.mq_msgsize = 100;
    want.mq_curmsgs = 7;
    mq_unlink(Q);
    fd = mq_open(Q, O_CREAT | O_EXCL | O_WRONLY, 0600, &want);
    check("create_with_attr", fd == 3, fd);
    a = mq_attr_of((int)fd);
    check("attr_maxmsg", a.mq_maxmsg == 3, a.mq_maxmsg);
    check("attr_msgsize", a.mq_msgsize == 100, a.mq_msgsize);
    check("attr_flags_ignored", a.mq_flags == 0 && a.mq_curmsgs == 0, a.mq_flags);
    i64 fd2 = mq_open(Q, O_RDWR | O_TRUNC | O_DIRECTORY, 0, 0);
    check("trunc_directory_ignored", fd2 == 4, fd2);
    check("second_fd_same_queue", mq_send((int)fd2, "x", 1, 0, 0) == 0 && mq_attr_of((int)fd).mq_curmsgs == 1, 0);
    close((int)fd2);
    close((int)fd);
    check("unlink", mq_unlink(Q) == 0, 0);
    return finish();
}
