/*
 * Support for macOS test programs (ADR 0182): the BSD system calls and Mach
 * traps a test asks for are made directly, never through libSystem, so the
 * behaviour observed is the kernel's. Built by `cargo xtask pipeline mac
 * build` for x86_64 (the convention Poly's XNU personality speaks) and arm64.
 *
 * Written for Poly from public sources only: the system call conventions
 * are the ones open-source XNU documents (osfmk/mach/i386/syscall_sw.h:
 * class 2 in the top byte of rax for BSD calls, class 1 for Mach traps;
 * arm64: the number in x16, negative for Mach traps, `svc #0x80`); BSD
 * calls report an error with the carry flag set and the errno as the
 * result. Numbers and values come from the macOS SDK headers named beside
 * them. Behaviour only; no code is copied from anywhere.
 *
 * A test is one C file that includes this header and defines
 * `int test_main(void)`. The program is linked against libSystem (macOS
 * starts no program without dyld), but a test calls nothing in it: `mac
 * build` refuses an executable that imports a symbol. Output goes to fd 1
 * with the raw write call, and the program ends with the raw exit call.
 *
 * Output protocol (as the Linux corpus, tests/linux/include/poly_test.h):
 *
 *     ok <check>
 *     not ok <check> (got <value>)
 *     value <name> <value>
 *     summary <passed> passed, <failed> failed
 *
 * The exit status is 0 when every check passed, 1 otherwise. Never print
 * addresses, times, pids, port names or anything else that differs between
 * runs: every test runs several times and must print the same bytes. A
 * value may differ between x86_64 and arm64 (the page size does); each
 * architecture's output is recorded on its own.
 *
 * Each test runs in a fresh, empty scratch directory, which is its current
 * directory: create files with relative names.
 */
#ifndef POLY_MTEST_H
#define POLY_MTEST_H

typedef unsigned long u64;
typedef long i64;
typedef unsigned int u32;
typedef int i32;
typedef unsigned short u16;
typedef short i16;
typedef unsigned char u8;
typedef unsigned long size_t;

/* BSD system call numbers, <sys/syscall.h>. */
#define SYS_exit 1
#define SYS_read 3
#define SYS_write 4
#define SYS_open 5
#define SYS_close 6
#define SYS_unlink 10
#define SYS_getpid 20
#define SYS_getuid 24
#define SYS_getppid 39
#define SYS_getgid 47
#define SYS_umask 60
#define SYS_fork 2
#define SYS_wait4 7
#define SYS_chdir 12
#define SYS_fchdir 13
#define SYS_chmod 15
#define SYS_kill 37
#define SYS_pipe 42
#define SYS_sigaction 46
#define SYS_sigprocmask 48
#define SYS_sigpending 52
#define SYS_ioctl 54
#define SYS_symlink 57
#define SYS_readlink 58
#define SYS_execve 59
#define SYS_vfork 66
#define SYS_dup2 90
#define SYS_fcntl 92
#define SYS_sigsuspend 111
#define SYS_sigreturn 184
#define SYS_csops 169
#define SYS_getattrlist 220
#define SYS_posix_spawn 244
#define SYS_shared_region_check_np 294
#define SYS_issetugid 327
#define SYS_stat64 338
#define SYS_lstat64 340
#define SYS_bsdthread_register 366
#define SYS_kqueue 362
#define SYS_kevent 363
#define SYS_proc_info 336
#define SYS_fstatat64 470
#define SYS_getentropy 500
#define SYS_munmap 73
#define SYS_mknod 14
#define SYS_msync 65
#define SYS_madvise 75
#define SYS_mincore 78
#define SYS_mlock 203
#define SYS_munlock 204
#define SYS_minherit 250
#define SYS_psynch_mutexwait 301
#define SYS_psynch_mutexdrop 302
#define SYS_psynch_cvbroad 303
#define SYS_psynch_cvsignal 304
#define SYS_psynch_cvwait 305
#define SYS_bsdthread_create 360
#define SYS_bsdthread_terminate 361
#define SYS_ulock_wait 515
#define SYS_ulock_wake 516
#define SYS_ulock_wait2 544
#define SYS_mprotect 74
#define SYS_mkdir 136
#define SYS_rmdir 137
#define SYS_mmap 197
#define SYS_lseek 199
#define SYS_sysctl 202
#define SYS_sysctlbyname 274
#define SYS_fstat64 339
#define SYS_getdirentries64 344
#define SYS_thread_selfid 372

/* Mach trap numbers, the trap table of open-source XNU
 * (osfmk/kern/syscall_sw.c; the names in <mach/mach_traps.h>). */
#define MACH_mach_reply_port 26
#define MACH_thread_self_trap 27
#define MACH_task_self_trap 28
#define MACH_host_self_trap 29
#define MACH_mach_timebase_info_trap 89

/* errno values, <sys/errno.h>. */
#define EPERM 1
#define ENOENT 2
#define ESRCH 3
#define EINTR 4
#define EIO 5
#define ENXIO 6
#define ENOEXEC 8
#define EBADF 9
#define ECHILD 10
#define ENOMEM 12
#define EACCES 13
#define EFAULT 14
#define EEXIST 17
#define ENOTDIR 20
#define EISDIR 21
#define EINVAL 22
#define ENOTTY 25
#define EAGAIN 35
#define ENOTSUP 45
#define ETIMEDOUT 60
#define EOWNERDEAD 105
#define EPIPE 32
#define ELOOP 62
#define ERANGE 34
#define ENODEV 19
#define ESPIPE 29
#define ENAMETOOLONG 63
#define ENOTEMPTY 66
#define ENOSYS 78

/* <sys/unistd.h> */
#define SEEK_SET 0
#define SEEK_CUR 1
#define SEEK_END 2

/* <sys/fcntl.h> */
#define O_RDONLY 0x0000
#define O_WRONLY 0x0001
#define O_RDWR 0x0002
#define O_NONBLOCK 0x0004
#define O_APPEND 0x0008
#define O_NOFOLLOW 0x0100
#define O_CREAT 0x0200
#define O_TRUNC 0x0400
#define O_EXCL 0x0800
#define O_DIRECTORY 0x100000
#define O_CLOEXEC 0x1000000

/* <sys/mman.h> */
#define PROT_NONE 0x00
#define PROT_READ 0x01
#define PROT_WRITE 0x02
#define PROT_EXEC 0x04
#define MAP_SHARED 0x0001
#define MAP_PRIVATE 0x0002
#define MAP_FIXED 0x0010
#define MAP_ANON 0x1000

/* <sys/mman.h>, <mach/vm_inherit.h> */
#define MS_ASYNC 0x0001
#define MS_INVALIDATE 0x0002
#define MS_SYNC 0x0010
#define MADV_NORMAL 0
#define MADV_RANDOM 1
#define MADV_SEQUENTIAL 2
#define MADV_WILLNEED 3
#define MADV_DONTNEED 4
#define MADV_FREE 5
#define MADV_ZERO_WIRED_PAGES 6
#define MADV_FREE_REUSABLE 7
#define MADV_FREE_REUSE 8
#define MADV_CAN_REUSE 9
#define MADV_ZERO 11
#define MINCORE_INCORE 0x1
#define MINCORE_REFERENCED 0x2
#define MINCORE_MODIFIED 0x4
#define VM_INHERIT_SHARE 0
#define VM_INHERIT_COPY 1
#define VM_INHERIT_NONE 2
#define VM_INHERIT_DONATE_COPY 3

/* <sys/stat.h> */
#define S_IFMT 0170000
#define S_IFCHR 0020000
#define S_IFDIR 0040000
#define S_IFREG 0100000
#define S_IFIFO 0010000
#define S_IFLNK 0120000

/* <sys/signal.h> */
#define SIGHUP 1
#define SIGINT 2
#define SIGQUIT 3
#define SIGABRT 6
#define SIGKILL 9
#define SIGSEGV 11
#define SIGSYS 12
#define SIGPIPE 13
#define SIGALRM 14
#define SIGTERM 15
#define SIGSTOP 17
#define SIGTSTP 18
#define SIGCONT 19
#define SIGCHLD 20
#define SIGINFO 29
#define SIGUSR1 30
#define SIGUSR2 31
#define SA_RESTART 0x0002
#define SA_RESETHAND 0x0004
#define SA_NODEFER 0x0010
#define SA_SIGINFO 0x0040
#define SIG_DFL ((void *)0)
#define SIG_IGN ((void *)1)
#define SIG_BLOCK 1
#define SIG_UNBLOCK 2
#define SIG_SETMASK 3
#define SIGBIT(sig) (1u << ((sig)-1))

/* <sys/wait.h> */
#define WNOHANG 1
#define WUNTRACED 2
#define W_EXITED(s) (((s) & 0177) == 0)
#define W_EXITSTATUS(s) (((s) >> 8) & 0xff)
#define W_TERMSIG(s) ((s) & 0177)
#define W_STOPPED(s) (((s) & 0xff) == 0177)
#define W_STOPSIG(s) (((s) >> 8) & 0xff)

/* <sys/fcntl.h> commands */
#define F_DUPFD 0
#define F_GETFD 1
#define F_SETFD 2
#define F_GETFL 3
#define F_SETFL 4
#define F_DUPFD_CLOEXEC 67
#define F_GETPATH 50
#define FD_CLOEXEC 1
#define AT_FDCWD (-2)
#define AT_SYMLINK_NOFOLLOW 0x0020

/* <sys/ioctl.h> */
#define TIOCGWINSZ 0x40087468
#define FIONREAD 0x4004667f
#define FIONBIO 0x8004667e
#define FIOCLEX 0x20006601
#define FIONCLEX 0x20006602

/* <sys/event.h> */
#define EVFILT_READ (-1)
#define EVFILT_WRITE (-2)
#define EVFILT_VNODE (-4)
#define EVFILT_PROC (-5)
#define EVFILT_SIGNAL (-6)
#define EVFILT_TIMER (-7)
#define EV_ADD 0x0001
#define EV_DELETE 0x0002
#define EV_ENABLE 0x0004
#define EV_DISABLE 0x0008
#define EV_ONESHOT 0x0010
#define EV_CLEAR 0x0020
#define EV_RECEIPT 0x0040
#define EV_EOF 0x8000
#define EV_ERROR 0x4000
#define NOTE_EXIT 0x80000000u
struct poly_kevent {
    u64 ident;
    i16 filter;
    u16 flags;
    u32 fflags;
    i64 data;
    void *udata;
};
_Static_assert(sizeof(struct poly_kevent) == 32, "struct kevent is 32 bytes");

/* <sys/proc_info.h> */
#define PROC_INFO_CALL_PIDINFO 2
#define PROC_PIDLISTFDS 1
#define PROC_PIDTBSDINFO 3
#define PROX_FDTYPE_VNODE 1
#define PROX_FDTYPE_KQUEUE 5
#define PROX_FDTYPE_PIPE 6

/* <sys/dirent.h> */
#define DT_UNKNOWN 0
#define DT_DIR 4
#define DT_FIFO 1
#define DT_CHR 2
#define DT_REG 8
#define DT_LNK 10

/* <sys/sysctl.h> */
#define CTL_MAXNAME 12
#define CTL_KERN 1
#define CTL_HW 6
#define KERN_OSTYPE 1
#define KERN_OSRELEASE 2
#define HW_MACHINE 1
#define HW_NCPU 3
#define HW_PAGESIZE 7
#define KERN_VERSION 4
#define KERN_MAXPROC 6
#define KERN_MAXFILES 7
#define KERN_ARGMAX 8
#define KERN_HOSTNAME 10
#define KERN_PROC 14
#define KERN_BOOTTIME 21
#define KERN_PROCARGS2 49
#define KERN_OSVERSION 65
#define KERN_PROC_PID 1
#define KERN_OSREV 3
#define HW_BYTEORDER 4
#define HW_MEMSIZE 24
/* XNU bsd/sys/sysctl.h (not in the SDK): the name-to-number call. */
#define CTL_SYSCTL 0
#define CTL_SYSCTL_NAME2OID 3
/* struct kinfo_proc of <sys/sysctl.h> (648 bytes): where the fields are. */
#define KINFO_PROC_SIZE 648
#define KP_P_STAT 36
#define KP_P_PID 40
#define KP_P_COMM 243
#define KP_E_UID 420
#define KP_E_PPID 560

/* <mach/kern_return.h>, <mach/port.h> */
#define KERN_SUCCESS 0
#define MACH_PORT_NULL 0u
#define MACH_PORT_DEAD 0xffffffffu
#define KERN_INVALID_ADDRESS 1
#define KERN_PROTECTION_FAILURE 2
#define KERN_NO_SPACE 3
#define KERN_INVALID_ARGUMENT 4
#define KERN_NOT_RECEIVER 7
#define KERN_MEMORY_ERROR 10
#define KERN_INVALID_NAME 15
#define KERN_INVALID_TASK 16
#define KERN_INVALID_RIGHT 17
#define KERN_INVALID_VALUE 18
#define KERN_UREFS_OVERFLOW 19
#define KERN_INVALID_CAPABILITY 20
/* <mach/message.h>: what a trap returns when the task port is not valid */
#define MACH_SEND_INVALID_DEST 0x10000003
#define MACH_MSG_TYPE_MAKE_SEND 20
/* <mach/port.h> */
#define MACH_PORT_RIGHT_SEND 0
#define MACH_PORT_RIGHT_RECEIVE 1
#define MACH_PORT_RIGHT_SEND_ONCE 2
#define MACH_PORT_RIGHT_PORT_SET 3
#define MACH_PORT_RIGHT_DEAD_NAME 4
#define MACH_PORT_TYPE_SEND 0x10000
#define MACH_PORT_TYPE_RECEIVE 0x20000
#define MACH_PORT_TYPE_SEND_ONCE 0x40000
#define MACH_PORT_TYPE_PORT_SET 0x80000
#define MACH_PORT_TYPE_DEAD_NAME 0x100000
/* <mach/vm_statistics.h> */
#define VM_FLAGS_ANYWHERE 0x00000001
/* Mach trap numbers: XNU's trap table (osfmk/kern/syscall_sw.c) */
#define MACH__kernelrpc_mach_vm_allocate_trap 10
#define MACH__kernelrpc_mach_vm_deallocate_trap 12
#define MACH__kernelrpc_mach_vm_protect_trap 14
#define MACH__kernelrpc_mach_port_allocate_trap 16
#define MACH__kernelrpc_mach_port_deallocate_trap 18
#define MACH__kernelrpc_mach_port_mod_refs_trap 19
#define MACH__kernelrpc_mach_port_insert_right_trap 21
#define MACH__kernelrpc_mach_port_type_trap 76
#define MACH_mach_wait_until_trap 90
/* XNU bsd/sys/ulock.h (not installed in the SDK) */
#define UL_COMPARE_AND_WAIT 1
#define UL_UNFAIR_LOCK 2
#define UL_COMPARE_AND_WAIT_SHARED 3
#define UL_COMPARE_AND_WAIT64 5
#define ULF_WAKE_ALL 0x100
#define ULF_NO_ERRNO 0x01000000

/* struct stat64 (struct stat with 64-bit inodes, the SDK's default),
 * <sys/stat.h>: 144 bytes. */
struct poly_timespec {
    i64 tv_sec;
    i64 tv_nsec;
};
struct poly_stat64 {
    i32 st_dev;
    u16 st_mode;
    u16 st_nlink;
    u64 st_ino;
    u32 st_uid;
    u32 st_gid;
    i32 st_rdev;
    struct poly_timespec st_atimespec;
    struct poly_timespec st_mtimespec;
    struct poly_timespec st_ctimespec;
    struct poly_timespec st_birthtimespec;
    i64 st_size;
    i64 st_blocks;
    i32 st_blksize;
    u32 st_flags;
    u32 st_gen;
    i32 st_lspare;
    i64 st_qspare[2];
};
_Static_assert(sizeof(struct poly_stat64) == 144, "struct stat64 is 144 bytes");

/* struct dirent with 64-bit inodes, <sys/dirent.h>: what getdirentries64
 * returns, one record of d_reclen bytes after another. */
struct poly_dirent64 {
    u64 d_ino;
    u64 d_seekoff;
    u16 d_reclen;
    u16 d_namlen;
    u8 d_type;
    char d_name[1024];
};

/* struct mach_timebase_info, <mach/mach_time.h>. */
struct poly_timebase {
    u32 numer;
    u32 denom;
};

/* What a BSD call returned: `err` is 0, or the errno when the carry flag
 * was set; `ret` and `ret2` are the two result registers. */
typedef struct {
    i64 ret;
    i64 ret2;
    i64 err;
} sysret;

#if defined(__x86_64__)
#define POLY_ARCH "x86_64"
static inline sysret poly_bsd6(i64 n, i64 a, i64 b, i64 c, i64 d, i64 e, i64 f) {
    i64 rax = 0x2000000 | n, rdx = c;
    u8 carry;
    register i64 r10 __asm__("r10") = d;
    register i64 r8 __asm__("r8") = e;
    register i64 r9 __asm__("r9") = f;
    __asm__ volatile("syscall\n\tsetc %2"
                     : "+a"(rax), "+d"(rdx), "=q"(carry), "+r"(r10), "+r"(r8), "+r"(r9)
                     : "D"(a), "S"(b)
                     : "rcx", "r11", "memory", "cc");
    sysret r = {rax, rdx, 0};
    if (carry) r.err = rax;
    return r;
}
static inline i64 poly_mach6(i64 n, i64 a, i64 b, i64 c, i64 d, i64 e, i64 f) {
    i64 rax = 0x1000000 | n;
    register i64 r10 __asm__("r10") = d;
    register i64 r8 __asm__("r8") = e;
    register i64 r9 __asm__("r9") = f;
    __asm__ volatile("syscall"
                     : "+a"(rax), "+d"(c), "+r"(r10), "+r"(r8), "+r"(r9)
                     : "D"(a), "S"(b)
                     : "rcx", "r11", "memory", "cc");
    return rax;
}
#elif defined(__arm64__) || defined(__aarch64__)
#define POLY_ARCH "arm64"
static inline sysret poly_bsd6(i64 n, i64 a, i64 b, i64 c, i64 d, i64 e, i64 f) {
    register i64 x0 __asm__("x0") = a;
    register i64 x1 __asm__("x1") = b;
    register i64 x2 __asm__("x2") = c;
    register i64 x3 __asm__("x3") = d;
    register i64 x4 __asm__("x4") = e;
    register i64 x5 __asm__("x5") = f;
    register i64 x16 __asm__("x16") = n;
    i64 carry;
    __asm__ volatile("svc #0x80\n\tcset %[c], cs"
                     : "+r"(x0), "+r"(x1), "+r"(x2), "+r"(x3), "+r"(x4), "+r"(x5), "+r"(x16), [c] "=r"(carry)
                     :
                     : "x6", "x7", "x8", "x9", "x10", "x11", "x12", "x13", "x14", "x15", "x17", "memory", "cc");
    sysret r = {x0, x1, 0};
    if (carry) r.err = x0;
    return r;
}
static inline i64 poly_mach6(i64 n, i64 a, i64 b, i64 c, i64 d, i64 e, i64 f) {
    register i64 x0 __asm__("x0") = a;
    register i64 x1 __asm__("x1") = b;
    register i64 x2 __asm__("x2") = c;
    register i64 x3 __asm__("x3") = d;
    register i64 x4 __asm__("x4") = e;
    register i64 x5 __asm__("x5") = f;
    register i64 x16 __asm__("x16") = -n;
    __asm__ volatile("svc #0x80"
                     : "+r"(x0), "+r"(x1), "+r"(x2), "+r"(x3), "+r"(x4), "+r"(x5), "+r"(x16)
                     :
                     : "x6", "x7", "x8", "x9", "x10", "x11", "x12", "x13", "x14", "x15", "x17", "memory", "cc");
    return x0;
}
#else
#error "macOS tests are built for x86_64 and arm64"
#endif

#if defined(__arm64__) || defined(__aarch64__)
#define POLY_ARM64 1
/* Calls with seven or eight arguments (psynch): x0..x7 on arm64. x86_64
 * passes them on the stack and is not done yet, so those tests are
 * arm64-only for now. */
static inline sysret poly_bsd8(i64 n, i64 a, i64 b, i64 c, i64 d, i64 e, i64 f, i64 g, i64 h) {
    register i64 x0 __asm__("x0") = a;
    register i64 x1 __asm__("x1") = b;
    register i64 x2 __asm__("x2") = c;
    register i64 x3 __asm__("x3") = d;
    register i64 x4 __asm__("x4") = e;
    register i64 x5 __asm__("x5") = f;
    register i64 x6 __asm__("x6") = g;
    register i64 x7 __asm__("x7") = h;
    register i64 x16 __asm__("x16") = n;
    i64 carry;
    __asm__ volatile("svc #0x80\n\tcset %[c], cs"
                     : "+r"(x0), "+r"(x1), "+r"(x2), "+r"(x3), "+r"(x4), "+r"(x5), "+r"(x6), "+r"(x7), "+r"(x16), [c] "=r"(carry)
                     :
                     : "x8", "x9", "x10", "x11", "x12", "x13", "x14", "x15", "x17", "memory", "cc");
    sysret r = {x0, x1, 0};
    if (carry) r.err = x0;
    return r;
}
/* mach_absolute_time() on arm64 is the virtual counter (24 MHz here:
 * mach_timebase_info 125/3). */
static inline u64 poly_mach_now(void) {
    u64 t;
    __asm__ volatile("isb\n\tmrs %0, cntvct_el0" : "=r"(t));
    return t;
}
#endif

#define bsd0(n) poly_bsd6((n), 0, 0, 0, 0, 0, 0)
#define bsd1(n, a) poly_bsd6((n), (i64)(a), 0, 0, 0, 0, 0)
#define bsd2(n, a, b) poly_bsd6((n), (i64)(a), (i64)(b), 0, 0, 0, 0)
#define bsd3(n, a, b, c) poly_bsd6((n), (i64)(a), (i64)(b), (i64)(c), 0, 0, 0)
#define bsd4(n, a, b, c, d) poly_bsd6((n), (i64)(a), (i64)(b), (i64)(c), (i64)(d), 0, 0)
#define bsd6(n, a, b, c, d, e, f) poly_bsd6((n), (i64)(a), (i64)(b), (i64)(c), (i64)(d), (i64)(e), (i64)(f))
#define mach0(n) poly_mach6((n), 0, 0, 0, 0, 0, 0)
#define mach1(n, a) poly_mach6((n), (i64)(a), 0, 0, 0, 0, 0)

/* What the program was started with: a test that starts a child runs
 * itself again (execve of poly_argv[0]) with an argument that selects its
 * child part. The path is the one the runner gave, absolute. */
static int poly_argc;
static char **poly_argv;
static char **poly_envp;

/* The compiler may emit calls to these; defined here so that a test
 * imports nothing from libSystem. */
void *memset(void *dst, int c, size_t n) {
    unsigned char *d = dst;
    while (n--) *d++ = (unsigned char)c;
    return dst;
}
void *memcpy(void *dst, const void *src, size_t n) {
    unsigned char *d = dst;
    const unsigned char *s = src;
    while (n--) *d++ = *s++;
    return dst;
}

static __attribute__((unused)) size_t str_len(const char *s) {
    size_t n = 0;
    while (s[n]) n++;
    return n;
}
static __attribute__((unused)) int str_eq(const char *a, const char *b) {
    while (*a && *a == *b) a++, b++;
    return *a == *b;
}
static __attribute__((unused)) int mem_eq(const void *a, const void *b, size_t n) {
    const unsigned char *x = a, *y = b;
    for (size_t i = 0; i < n; i++)
        if (x[i] != y[i]) return 0;
    return 1;
}
static __attribute__((unused)) void put_fd(int fd, const char *s) { bsd3(SYS_write, fd, s, str_len(s)); }
static __attribute__((unused)) void put(const char *s) { put_fd(1, s); }
static __attribute__((unused)) void put_num(i64 v) {
    char buf[24];
    int i = sizeof buf;
    u64 u = v < 0 ? -(u64)v : (u64)v;
    do buf[--i] = (char)('0' + u % 10);
    while (u /= 10);
    if (v < 0) buf[--i] = '-';
    bsd3(SYS_write, 1, buf + i, sizeof buf - i);
}

static int passed, failed;

/* Report one check; `got` is printed when it fails. */
static __attribute__((unused)) void check(const char *name, int ok, i64 got) {
    if (ok) {
        passed++;
        put("ok ");
        put(name);
        put("\n");
    } else {
        failed++;
        put("not ok ");
        put(name);
        put(" (got ");
        put_num(got);
        put(")\n");
    }
}

/* A call that must succeed: ok when err is 0, else the errno is shown. */
static __attribute__((unused)) void check_ok(const char *name, sysret r) { check(name, r.err == 0, r.err); }

/* A call that must fail with `want`: shows the result if it succeeded
 * (as -1000000 - ret, so a success is never mistaken for an errno). */
static __attribute__((unused)) void check_err(const char *name, sysret r, i64 want) {
    check(name, r.err == want, r.err ? r.err : -1000000 - r.ret);
}

/* A deterministic value worth recording (it may differ per architecture). */
static __attribute__((unused)) void value(const char *name, i64 v) {
    put("value ");
    put(name);
    put(" ");
    put_num(v);
    put("\n");
}
static __attribute__((unused)) void value_str(const char *name, const char *s) {
    put("value ");
    put(name);
    put(" ");
    put(s);
    put("\n");
}

static __attribute__((unused)) int finish(void) {
    put("summary ");
    put_num(passed);
    put(" passed, ");
    put_num(failed);
    put(" failed\n");
    return failed ? 1 : 0;
}

static __attribute__((noreturn, unused)) void poly_exit(int code) {
    for (;;) bsd1(SYS_exit, code);
}

/* Signal actions the way the kernel takes them: struct __sigaction with a
 * user-space trampoline (<sys/signal.h>; the kernel calls it with the
 * handler, the info style, the signal, the siginfo and the context, and the
 * trampoline ends with sigreturn). */
struct poly_ksigaction {
    void *handler;
    void *tramp;
    u32 mask;
    i32 flags;
};
struct poly_usigaction {
    void *handler;
    u32 mask;
    i32 flags;
};
static __attribute__((unused)) void poly_tramp(void (*h)(int, void *, void *), int style, int sig, void *info,
                                               void *uctx) {
    h(sig, info, uctx);
    bsd2(SYS_sigreturn, uctx, style);
    for (;;) {}
}
/* sigaction(sig, {handler, mask, flags}, old): handler is a function
 * (int sig, void *siginfo, void *ucontext), SIG_IGN or SIG_DFL. */
static __attribute__((unused)) sysret poly_sigaction(int sig, void *handler, u32 mask, i32 flags,
                                                     struct poly_usigaction *old) {
    struct poly_ksigaction n = {handler, (void *)poly_tramp, mask, flags};
    return bsd3(SYS_sigaction, sig, &n, old);
}
static __attribute__((unused)) u32 poly_sigmask(void) {
    u32 m = 0;
    bsd3(SYS_sigprocmask, SIG_BLOCK, 0, &m);
    return m;
}

/* Children: poly_start_self runs this program again with argv[1] = mode and
 * argv[2] = arg (both may be 0) and returns the child's pid; the child part
 * of a test is `if (is_child("mode"))`. */
static __attribute__((unused)) int is_child(const char *mode) { return poly_argc > 1 && str_eq(poly_argv[1], mode); }
static __attribute__((unused)) i64 poly_start_self(const char *mode, const char *arg, char **envp) {
    char *argv[4] = {poly_argv[0], (char *)mode, (char *)arg, 0};
    char *noenv[1] = {0};
    sysret f = bsd0(SYS_fork);
    if (f.ret2 == 1) {
        bsd3(SYS_execve, poly_argv[0], argv, envp ? envp : noenv);
        poly_exit(126);
    }
    return f.ret;
}
/* wait4 for one child: its wait status, or -1. */
static __attribute__((unused)) int poly_wait(i64 pid) {
    int st = -1;
    sysret r = bsd4(SYS_wait4, pid, &st, 0, 0);
    return r.err ? -1 : st;
}
static __attribute__((unused)) i64 poly_atoi(const char *s) {
    i64 v = 0;
    for (; *s >= '0' && *s <= '9'; s++) v = v * 10 + (*s - '0');
    return v;
}

/* What a re-executed (or spawned) child can see of what it inherited, one
 * line each: its arguments after argv[0], its environment, whether fds 5
 * and 6 are open, the SIGUSR1 and SIGUSR2 handlers (0 default, 1 ignored,
 * 2 a handler) and its signal mask. */
static __attribute__((unused)) void poly_report_child(void) {
    put("child args:");
    for (int i = 1; i < poly_argc; i++) {
        put(" [");
        put(poly_argv[i]);
        put("]");
    }
    put("\nchild env:");
    for (char **e = poly_envp; *e; e++) {
        put(" ");
        put(*e);
    }
    put("\nchild fd5=");
    put(bsd3(SYS_fcntl, 5, F_GETFD, 0).err ? "closed" : "open");
    put(" fd6=");
    put(bsd3(SYS_fcntl, 6, F_GETFD, 0).err ? "closed" : "open");
    struct poly_usigaction o;
    bsd3(SYS_sigaction, SIGUSR1, 0, &o);
    put(" usr1=");
    put_num(o.handler == SIG_DFL ? 0 : o.handler == SIG_IGN ? 1 : 2);
    bsd3(SYS_sigaction, SIGUSR2, 0, &o);
    put(" usr2=");
    put_num(o.handler == SIG_DFL ? 0 : o.handler == SIG_IGN ? 1 : 2);
    put(" mask=");
    put_num(poly_sigmask());
    put("\n");
}

int test_main(void);

int main(int argc, char **argv, char **envp) {
    poly_argc = argc;
    poly_argv = argv;
    poly_envp = envp;
    poly_exit(test_main());
}

#endif
