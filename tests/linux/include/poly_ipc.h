/*
 * More freestanding support for the Linux test corpus, for the tests of
 * signals, futexes, pipes, readiness (poll, select, epoll), event files,
 * sockets, credentials, terminals and mounts. Include after poly_test.h.
 *
 * Written for Poly from the public x86-64 psABI and the Linux UAPI headers:
 * syscall numbers from asm/unistd_64.h; errno values from
 * asm-generic/errno-base.h and asm-generic/errno.h; signal numbers, sigaction
 * flags and stack_t from asm/signal.h and asm-generic/signal-defs.h;
 * siginfo_t from asm-generic/siginfo.h; struct sigcontext and ucontext from
 * asm/sigcontext.h and asm/ucontext.h; clone flags from linux/sched.h;
 * futex operations from linux/futex.h; socket constants and structures
 * from linux/socket.h, linux/in.h, linux/un.h and asm-generic/socket.h;
 * ioctl numbers from asm-generic/ioctls.h. Behaviour only; no code is
 * copied from anywhere.
 *
 * Tests that run more than one task (fork, threads) or whose syscalls
 * depend on timing carry the directive `poly-diff: syscalls=off` in their
 * source: the diff runner then compares only their exit status and output.
 * In those tests only the first task prints.
 */
#ifndef POLY_IPC_H
#define POLY_IPC_H

#include "poly_test.h"

/* asm/unistd_64.h */
#define SYS_open 2
#define SYS_close 3
#define SYS_poll 7
#define SYS_lseek 8
#define SYS_rt_sigaction 13
#define SYS_rt_sigprocmask 14
#define SYS_rt_sigreturn 15
#define SYS_ioctl 16
#define SYS_pipe 22
#define SYS_select 23
#define SYS_sched_yield 24
#define SYS_dup 32
#define SYS_dup2 33
#define SYS_pause 34
#define SYS_nanosleep 35
#define SYS_getitimer 36
#define SYS_alarm 37
#define SYS_setitimer 38
#define SYS_getpid 39
#define SYS_socket 41
#define SYS_connect 42
#define SYS_accept 43
#define SYS_sendto 44
#define SYS_recvfrom 45
#define SYS_sendmsg 46
#define SYS_recvmsg 47
#define SYS_shutdown 48
#define SYS_bind 49
#define SYS_listen 50
#define SYS_getsockname 51
#define SYS_getpeername 52
#define SYS_socketpair 53
#define SYS_setsockopt 54
#define SYS_getsockopt 55
#define SYS_clone 56
#define SYS_fork 57
#define SYS_exit 60
#define SYS_wait4 61
#define SYS_kill 62
#define SYS_fcntl 72
#define SYS_mkdir 83
#define SYS_rmdir 84
#define SYS_unlink 87
#define SYS_getuid 102
#define SYS_getgid 104
#define SYS_setuid 105
#define SYS_setgid 106
#define SYS_geteuid 107
#define SYS_getegid 108
#define SYS_setpgid 109
#define SYS_getppid 110
#define SYS_getpgrp 111
#define SYS_setsid 112
#define SYS_setreuid 113
#define SYS_setregid 114
#define SYS_getgroups 115
#define SYS_setgroups 116
#define SYS_setresuid 117
#define SYS_getresuid 118
#define SYS_setresgid 119
#define SYS_getresgid 120
#define SYS_getpgid 121
#define SYS_setfsuid 122
#define SYS_setfsgid 123
#define SYS_getsid 124
#define SYS_capget 125
#define SYS_capset 126
#define SYS_rt_sigpending 127
#define SYS_rt_sigtimedwait 128
#define SYS_rt_sigqueueinfo 129
#define SYS_rt_sigsuspend 130
#define SYS_sigaltstack 131
#define SYS_prctl 157
#define SYS_mount 165
#define SYS_umount2 166
#define SYS_gettid 186
#define SYS_tkill 200
#define SYS_futex 202
#define SYS_epoll_create 213
#define SYS_set_tid_address 218
#define SYS_epoll_wait 232
#define SYS_epoll_ctl 233
#define SYS_tgkill 234
#define SYS_openat 257
#define SYS_pselect6 270
#define SYS_ppoll 271
#define SYS_set_robust_list 273
#define SYS_get_robust_list 274
#define SYS_epoll_pwait 281
#define SYS_signalfd 282
#define SYS_timerfd_create 283
#define SYS_eventfd 284
#define SYS_timerfd_settime 286
#define SYS_timerfd_gettime 287
#define SYS_accept4 288
#define SYS_signalfd4 289
#define SYS_eventfd2 290
#define SYS_epoll_create1 291
#define SYS_pipe2 293
#define SYS_rt_tgsigqueueinfo 297

/* asm-generic/errno-base.h, asm-generic/errno.h */
#define EPERM 1
#define ENOENT 2
#define ESRCH 3
#define EINTR 4
#define ENXIO 6
#define ECHILD 10
#define EAGAIN 11
#define EACCES 13
#define EBUSY 16
#define EEXIST 17
#define ENODEV 19
#define ENOTDIR 20
#define ENFILE 23
#define EMFILE 24
#define ENOTTY 25
#define ESPIPE 29
#define EPIPE 32
#define EDEADLK 35
#define ENAMETOOLONG 36
#define ENOTSOCK 88
#define EDESTADDRREQ 89
#define EMSGSIZE 90
#define EPROTOTYPE 91
#define ENOPROTOOPT 92
#define EPROTONOSUPPORT 93
#define ESOCKTNOSUPPORT 94
#define EOPNOTSUPP 95
#define EAFNOSUPPORT 97
#define EADDRINUSE 98
#define EADDRNOTAVAIL 99
#define ECONNABORTED 103
#define ECONNRESET 104
#define EISCONN 106
#define ENOTCONN 107
#define ETIMEDOUT 110
#define ECONNREFUSED 111
#define EALREADY 114
#define EINPROGRESS 115
#define EOWNERDEAD 130

#define syscall4(n, a, b, c, d) syscall6((n), (i64)(a), (i64)(b), (i64)(c), (i64)(d), 0, 0)
#define syscall5(n, a, b, c, d, e) syscall6((n), (i64)(a), (i64)(b), (i64)(c), (i64)(d), (i64)(e), 0)

/* asm/signal.h */
#define SIGHUP 1
#define SIGINT 2
#define SIGQUIT 3
#define SIGILL 4
#define SIGTRAP 5
#define SIGABRT 6
#define SIGBUS 7
#define SIGFPE 8
#define SIGKILL 9
#define SIGUSR1 10
#define SIGSEGV 11
#define SIGUSR2 12
#define SIGPIPE 13
#define SIGALRM 14
#define SIGTERM 15
#define SIGCHLD 17
#define SIGCONT 18
#define SIGSTOP 19
#define SIGTSTP 20
#define SIGURG 23
#define SIGWINCH 28
#define SIGSYS 31
#define SIGRTMIN 32
#define SIGRTMAX 64

#define SA_NOCLDSTOP 0x00000001UL
#define SA_NOCLDWAIT 0x00000002UL
#define SA_SIGINFO 0x00000004UL
#define SA_RESTORER 0x04000000UL
#define SA_ONSTACK 0x08000000UL
#define SA_RESTART 0x10000000UL
#define SA_NODEFER 0x40000000UL
#define SA_RESETHAND 0x80000000UL

#define SIG_DFL 0UL
#define SIG_IGN 1UL
#define SIG_BLOCK 0
#define SIG_UNBLOCK 1
#define SIG_SETMASK 2

#define SS_ONSTACK 1
#define SS_DISABLE 2
#define MINSIGSTKSZ 2048

/* asm-generic/siginfo.h: si_code values */
#define SI_USER 0
#define SI_KERNEL 0x80
#define SI_QUEUE (-1)
#define SI_TIMER (-2)
#define SI_TKILL (-6)
#define CLD_EXITED 1
#define CLD_KILLED 2

/* The kernel's sigaction on x86-64: handler, flags, restorer, mask. */
struct k_sigaction {
    u64 handler;
    u64 flags;
    u64 restorer;
    u64 mask;
};

/* siginfo_t, 128 bytes; the fields the tests read. */
struct k_siginfo {
    int si_signo;
    int si_errno;
    int si_code;
    int pad0;
    union {
        struct {
            int pid;
            unsigned uid;
            u64 value; /* sigqueue: sigval; SIGCHLD: the int status here */
        } kill;
        u64 addr; /* faults */
        u64 raw[14];
    } u;
};

/* stack_t */
struct k_stack {
    u64 ss_sp;
    int ss_flags;
    int pad0;
    u64 ss_size;
};

/* The ucontext the kernel builds for an SA_SIGINFO handler: uc_mcontext is
 * struct sigcontext (asm/sigcontext.h), general registers in its order. */
#define UC_R8 0
#define UC_R12 4
#define UC_RDI 8
#define UC_RBX 11
#define UC_RAX 13
#define UC_RSP 15
#define UC_RIP 16
struct k_ucontext {
    u64 uc_flags;
    u64 uc_link;
    struct k_stack uc_stack;
    u64 gregs[18]; /* r8-r15, rdi, rsi, rbp, rbx, rdx, rax, rcx, rsp, rip, eflags */
    u64 segs;      /* cs, gs, fs, ss */
    u64 err, trapno, oldmask, cr2;
    u64 fpstate;
    u64 reserved1[8];
    u64 uc_sigmask;
};

#define SIGBIT(s) (1UL << ((s) - 1))

/* The signal return trampoline every handler returns through. */
extern void poly_sigreturn(void);
__asm__(".text\n"
        ".global poly_sigreturn\n"
        "poly_sigreturn:\n"
        "    mov $15, %eax\n"
        "    syscall\n"
        "    hlt\n");

static __attribute__((unused)) i64 sig_action(int sig, u64 handler, u64 flags, u64 mask) {
    struct k_sigaction sa = { handler, flags | SA_RESTORER, (u64)poly_sigreturn, mask };
    return syscall4(SYS_rt_sigaction, sig, &sa, 0, 8);
}

static __attribute__((unused)) i64 sig_handler(int sig, void (*fn)(int), u64 flags) {
    return sig_action(sig, (u64)fn, flags, 0);
}

static __attribute__((unused)) i64 sig_mask(int how, u64 set, u64 *old) {
    return syscall4(SYS_rt_sigprocmask, how, &set, old, 8);
}

static __attribute__((unused)) u64 sig_current_mask(void) {
    u64 old = 0;
    syscall4(SYS_rt_sigprocmask, SIG_BLOCK, 0, &old, 8);
    return old;
}

static __attribute__((unused)) u64 sig_pending(void) {
    u64 set = 0;
    syscall2(SYS_rt_sigpending, &set, 8);
    return set;
}

static __attribute__((unused)) i64 getpid(void) { return syscall0(SYS_getpid); }
static __attribute__((unused)) i64 gettid(void) { return syscall0(SYS_gettid); }
static __attribute__((unused)) i64 raise_sig(int sig) { return syscall2(SYS_kill, getpid(), sig); }
static __attribute__((unused)) i64 close(int fd) { return syscall1(SYS_close, fd); }
static __attribute__((unused)) i64 read(int fd, void *buf, u64 n) { return syscall3(SYS_read, fd, buf, n); }
static __attribute__((unused)) i64 write(int fd, const void *buf, u64 n) { return syscall3(SYS_write, fd, buf, n); }

static __attribute__((unused)) int mem_eq(const void *a, const void *b, u64 n) {
    const unsigned char *x = a, *y = b;
    for (u64 i = 0; i < n; i++)
        if (x[i] != y[i]) return 0;
    return 1;
}

/* asm-generic/fcntl.h */
#define O_RDONLY 0
#define O_WRONLY 1
#define O_RDWR 2
#define O_CREAT 0100
#define O_EXCL 0200
#define O_NOCTTY 0400
#define O_TRUNC 01000
#define O_NONBLOCK 04000
#define O_DIRECTORY 0200000
#define O_CLOEXEC 02000000
#define AT_FDCWD (-100)
#define F_DUPFD 0
#define F_GETFD 1
#define F_SETFD 2
#define F_GETFL 3
#define F_SETFL 4
#define F_SETPIPE_SZ 1031
#define F_GETPIPE_SZ 1032
#define FD_CLOEXEC 1

/* asm-generic/ioctls.h */
#define TCGETS 0x5401
#define TCSETS 0x5402
#define TCSETSW 0x5403
#define TCSETSF 0x5404
#define TIOCGPGRP 0x540F
#define TIOCSPGRP 0x5410
#define TIOCOUTQ 0x5411
#define TIOCGWINSZ 0x5413
#define TIOCSWINSZ 0x5414
#define FIONREAD 0x541B
#define FIONBIO 0x5421
#define TIOCGPTN 0x80045430
#define TIOCSPTLCK 0x40045431
#define TIOCGPTLCK 0x80045439

/* Time. */
struct k_timespec {
    i64 tv_sec;
    i64 tv_nsec;
};

static __attribute__((unused)) i64 now_ns(void) {
    struct k_timespec t;
    syscall2(SYS_clock_gettime, CLOCK_MONOTONIC, &t);
    return t.tv_sec * 1000000000L + t.tv_nsec;
}

static __attribute__((unused)) void sleep_ms(i64 ms) {
    struct k_timespec t = { ms / 1000, (ms % 1000) * 1000000L };
    syscall2(SYS_nanosleep, &t, 0);
}

/* Processes. Wait status bits, as wait(2) describes them. */
#define WNOHANG 1
#define WALL 0x40000000
#define W_EXITED(s) (((s) & 0x7f) == 0)
#define W_EXITCODE(s) (((s) >> 8) & 0xff)
#define W_SIGNALED(s) (((s) & 0x7f) != 0 && ((s) & 0x7f) != 0x7f)
#define W_TERMSIG(s) ((s) & 0x7f)

static __attribute__((unused)) i64 fork(void) { return syscall0(SYS_fork); }

/* Wait for `pid`: the raw wait status, or -1 when the wait failed. */
static __attribute__((unused)) int wait_status(i64 pid) {
    int status = 0;
    i64 r = syscall4(SYS_wait4, pid, &status, WALL, 0);
    return r == pid ? status : -1;
}

/* Exit code of `pid`; -1 when it was killed or the wait failed. */
static __attribute__((unused)) int wait_exit(i64 pid) {
    int s = wait_status(pid);
    return s != -1 && W_EXITED(s) ? W_EXITCODE(s) : -1;
}

/* linux/sched.h */
#define CLONE_VM 0x00000100
#define CLONE_FS 0x00000200
#define CLONE_FILES 0x00000400
#define CLONE_SIGHAND 0x00000800
#define CLONE_THREAD 0x00010000
#define CLONE_SYSVSEM 0x00040000
#define CLONE_PARENT_SETTID 0x00100000
#define CLONE_CHILD_CLEARTID 0x00200000
#define CLONE_CHILD_SETTID 0x01000000
#define THREAD_FLAGS                                                                                  \
    (CLONE_VM | CLONE_FS | CLONE_FILES | CLONE_SIGHAND | CLONE_THREAD | CLONE_SYSVSEM | CLONE_PARENT_SETTID | \
     CLONE_CHILD_CLEARTID)

/*
 * Start a task running fn(arg) on the stack that ends at `stack_top` (16
 * byte aligned); it exits (the thread only) with fn's result. Returns the
 * new task's id or a negative errno. On x86-64 the arguments of clone are
 * (flags, newsp, parent_tid, child_tid, tls).
 */
i64 poly_clone(u64 flags, void *stack_top, int *ptid, int *ctid, int (*fn)(void *), void *arg);
__asm__(".text\n"
        ".global poly_clone\n"
        "poly_clone:\n"
        "    sub $16, %rsi\n"
        "    mov %r8, 0(%rsi)\n"
        "    mov %r9, 8(%rsi)\n"
        "    mov %rcx, %r10\n"
        "    xor %r8d, %r8d\n"
        "    mov $56, %eax\n"
        "    syscall\n"
        "    test %rax, %rax\n"
        "    jnz 1f\n"
        "    xor %ebp, %ebp\n"
        "    pop %rax\n"
        "    pop %rdi\n"
        "    call *%rax\n"
        "    mov %eax, %edi\n"
        "    mov $60, %eax\n"
        "    syscall\n"
        "    hlt\n"
        "1:  ret\n");

/* linux/futex.h */
#define FUTEX_WAIT 0
#define FUTEX_WAKE 1
#define FUTEX_FD 2
#define FUTEX_REQUEUE 3
#define FUTEX_CMP_REQUEUE 4
#define FUTEX_WAKE_OP 5
#define FUTEX_LOCK_PI 6
#define FUTEX_UNLOCK_PI 7
#define FUTEX_TRYLOCK_PI 8
#define FUTEX_WAIT_BITSET 9
#define FUTEX_WAKE_BITSET 10
#define FUTEX_PRIVATE_FLAG 128
#define FUTEX_CLOCK_REALTIME 256
#define FUTEX_BITSET_MATCH_ANY 0xffffffffU
#define FUTEX_WAITERS 0x80000000U
#define FUTEX_OWNER_DIED 0x40000000U
#define FUTEX_TID_MASK 0x3fffffffU
#define FUTEX_OP_SET 0
#define FUTEX_OP_ADD 1
#define FUTEX_OP_OR 2
#define FUTEX_OP_ANDN 3
#define FUTEX_OP_XOR 4
#define FUTEX_OP_OPARG_SHIFT 8
#define FUTEX_OP_CMP_EQ 0
#define FUTEX_OP_CMP_NE 1
#define FUTEX_OP_CMP_LT 2
#define FUTEX_OP_CMP_LE 3
#define FUTEX_OP_CMP_GT 4
#define FUTEX_OP_CMP_GE 5
#define FUTEX_OP(op, oparg, cmp, cmparg) \
    ((((op) & 0xfU) << 28) | (((cmp) & 0xfU) << 24) | (((oparg) & 0xfffU) << 12) | ((cmparg) & 0xfffU))

static __attribute__((unused)) i64 futex(volatile unsigned *uaddr, int op, unsigned val, const void *timeout_or_val2,
                                         volatile unsigned *uaddr2, unsigned val3) {
    return syscall6(SYS_futex, (i64)uaddr, op, val, (i64)timeout_or_val2, (i64)uaddr2, val3);
}

/* Anonymous memory both processes keep after fork. */
static __attribute__((unused)) void *shared_page(u64 len) {
    i64 r = syscall6(SYS_mmap, 0, len, PROT_READ | PROT_WRITE, MAP_SHARED | MAP_ANONYMOUS, -1, 0);
    return r < 0 ? (void *)0 : (void *)r;
}

/*
 * A one-word gate between tasks that share memory: `gate_open` sets it
 * and wakes every waiter; `gate_wait` returns once it is set. Waiting
 * retries until the word changes, so the number of calls is not fixed.
 */
static __attribute__((unused)) void gate_open(volatile unsigned *g) {
    __atomic_store_n(g, 1, __ATOMIC_SEQ_CST);
    futex(g, FUTEX_WAKE, 0x7fffffff, 0, 0, 0);
}

static __attribute__((unused)) void gate_wait(volatile unsigned *g) {
    while (__atomic_load_n(g, __ATOMIC_SEQ_CST) == 0) futex(g, FUTEX_WAIT, 0, 0, 0, 0);
}

/* Sockets: linux/socket.h, linux/in.h, linux/un.h, asm-generic/socket.h. */
#define AF_UNSPEC 0
#define AF_UNIX 1
#define AF_INET 2
#define AF_INET6 10
#define SOCK_STREAM 1
#define SOCK_DGRAM 2
#define SOCK_RAW 3
#define SOCK_SEQPACKET 5
#define SOCK_NONBLOCK O_NONBLOCK
#define SOCK_CLOEXEC O_CLOEXEC
#define IPPROTO_TCP 6
#define IPPROTO_UDP 17
#define SOL_SOCKET 1
#define SO_REUSEADDR 2
#define SO_TYPE 3
#define SO_ERROR 4
#define SO_BROADCAST 6
#define SO_SNDBUF 7
#define SO_RCVBUF 8
#define SO_KEEPALIVE 9
#define SO_LINGER 13
#define SO_REUSEPORT 15
#define SO_PASSCRED 16
#define SO_PEERCRED 17
#define SO_RCVLOWAT 18
#define SO_RCVTIMEO 20
#define SO_SNDTIMEO 21
#define SO_ACCEPTCONN 30
#define SO_PROTOCOL 38
#define SO_DOMAIN 39
#define TCP_NODELAY 1
#define SCM_RIGHTS 1
#define SCM_CREDENTIALS 2
#define MSG_OOB 1
#define MSG_PEEK 2
#define MSG_CTRUNC 0x08
#define MSG_TRUNC 0x20
#define MSG_DONTWAIT 0x40
#define MSG_WAITALL 0x100
#define MSG_NOSIGNAL 0x4000
#define MSG_CMSG_CLOEXEC 0x40000000
#define SHUT_RD 0
#define SHUT_WR 1
#define SHUT_RDWR 2
#define INADDR_LOOPBACK 0x7f000001U

struct k_sockaddr_un {
    unsigned short sun_family;
    char sun_path[108];
};

struct k_sockaddr_in {
    unsigned short sin_family;
    unsigned short sin_port; /* network byte order */
    unsigned sin_addr;       /* network byte order */
    unsigned char zero[8];
};

struct k_iovec {
    void *base;
    u64 len;
};

struct k_msghdr {
    void *name;
    int namelen;
    int pad0;
    struct k_iovec *iov;
    u64 iovlen;
    void *control;
    u64 controllen;
    int flags;
    int pad1;
};

struct k_cmsghdr {
    u64 len;
    int level;
    int type;
};

struct k_ucred {
    int pid;
    unsigned uid;
    unsigned gid;
};

static __attribute__((unused)) unsigned short be16(unsigned short v) { return (unsigned short)((v << 8) | (v >> 8)); }
static __attribute__((unused)) unsigned be32(unsigned v) {
    return (v >> 24) | ((v >> 8) & 0xff00) | ((v << 8) & 0xff0000) | (v << 24);
}

/* The IPv4 loopback address with `port` (host order; 0: any port). */
static __attribute__((unused)) struct k_sockaddr_in in_loopback(unsigned short port) {
    struct k_sockaddr_in a;
    memset(&a, 0, sizeof a);
    a.sin_family = AF_INET;
    a.sin_port = be16(port);
    a.sin_addr = be32(INADDR_LOOPBACK);
    return a;
}

/* Append the decimal digits of `v` to `s` (for unique names; never printed). */
static __attribute__((unused)) char *append_num(char *s, u64 v) {
    char tmp[24];
    int n = 0;
    do tmp[n++] = (char)('0' + v % 10);
    while (v /= 10);
    while (n) *s++ = tmp[--n];
    *s = 0;
    return s;
}

static __attribute__((unused)) char *append_str(char *s, const char *t) {
    while (*t) *s++ = *t++;
    *s = 0;
    return s;
}

#endif
