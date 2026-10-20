/*
 * Definitions and helpers shared by the file, memory, process and time
 * tests of the Linux corpus (the `a_*` programs). Include after
 * poly_test.h.
 *
 * Constants and struct layouts are ABI facts from the public Linux UAPI
 * headers, cited per block; syscall numbers from asm/unistd_64.h. The
 * helpers are written for Poly; no code is copied from anywhere.
 */
#ifndef A_SYS_H
#define A_SYS_H

#include "poly_test.h"

typedef unsigned int u32;
typedef int i32;
typedef unsigned short u16;
typedef unsigned char u8;

/* asm/unistd_64.h */
#define SYS_close 3
#define SYS_stat 4
#define SYS_fstat 5
#define SYS_lstat 6
#define SYS_lseek 8
#define SYS_rt_sigprocmask 14
#define SYS_pread64 17
#define SYS_pwrite64 18
#define SYS_readv 19
#define SYS_writev 20
#define SYS_access 21
#define SYS_sched_yield 24
#define SYS_mremap 25
#define SYS_madvise 28
#define SYS_dup 32
#define SYS_dup2 33
#define SYS_nanosleep 35
#define SYS_getitimer 36
#define SYS_alarm 37
#define SYS_setitimer 38
#define SYS_getpid 39
#define SYS_clone 56
#define SYS_fork 57
#define SYS_vfork 58
#define SYS_execve 59
#define SYS_exit 60
#define SYS_wait4 61
#define SYS_kill 62
#define SYS_fcntl 72
#define SYS_fsync 74
#define SYS_fdatasync 75
#define SYS_truncate 76
#define SYS_ftruncate 77
#define SYS_getcwd 79
#define SYS_chdir 80
#define SYS_fchdir 81
#define SYS_rename 82
#define SYS_mkdir 83
#define SYS_rmdir 84
#define SYS_link 86
#define SYS_unlink 87
#define SYS_symlink 88
#define SYS_readlink 89
#define SYS_chmod 90
#define SYS_fchmod 91
#define SYS_chown 92
#define SYS_fchown 93
#define SYS_umask 95
#define SYS_gettimeofday 96
#define SYS_getrlimit 97
#define SYS_getrusage 98
#define SYS_sysinfo 99
#define SYS_times 100
#define SYS_getuid 102
#define SYS_getgid 104
#define SYS_setpgid 109
#define SYS_getppid 110
#define SYS_getpgrp 111
#define SYS_setsid 112
#define SYS_getpgid 121
#define SYS_getsid 124
#define SYS_statfs 137
#define SYS_fstatfs 138
#define SYS_setrlimit 160
#define SYS_gettid 186
#define SYS_time 201
#define SYS_getdents64 217
#define SYS_set_tid_address 218
#define SYS_timer_create 222
#define SYS_timer_settime 223
#define SYS_timer_gettime 224
#define SYS_timer_getoverrun 225
#define SYS_timer_delete 226
#define SYS_clock_settime 227
#define SYS_clock_getres 229
#define SYS_clock_nanosleep 230
#define SYS_exit_group 231
#define SYS_tgkill 234
#define SYS_utimes 235
#define SYS_waitid 247
#define SYS_openat 257
#define SYS_mkdirat 258
#define SYS_fchownat 260
#define SYS_newfstatat 262
#define SYS_unlinkat 263
#define SYS_renameat 264
#define SYS_linkat 265
#define SYS_symlinkat 266
#define SYS_readlinkat 267
#define SYS_fchmodat 268
#define SYS_faccessat 269
#define SYS_utimensat 280
#define SYS_dup3 292
#define SYS_pipe2 293
#define SYS_preadv 295
#define SYS_pwritev 296
#define SYS_prlimit64 302
#define SYS_renameat2 316
#define SYS_statx 332
#define SYS_faccessat2 439

/* asm-generic/errno-base.h, asm-generic/errno.h */
#define EPERM 1
#define ENOENT 2
#define ESRCH 3
#define EINTR 4
#define EIO 5
#define ENXIO 6
#define E2BIG 7
#define ENOEXEC 8
#define ECHILD 10
#define EAGAIN 11
#define EACCES 13
#define EBUSY 16
#define EEXIST 17
#define EXDEV 18
#define ENODEV 19
#define ENOTDIR 20
#define EISDIR 21
#define ENFILE 23
#define EMFILE 24
#define ENOTTY 25
#define ETXTBSY 26
#define EFBIG 27
#define ENOSPC 28
#define ESPIPE 29
#define EROFS 30
#define EMLINK 31
#define EPIPE 32
#define ERANGE 34
#define ENAMETOOLONG 36
#define ENOTEMPTY 39
#define ELOOP 40
#define EOVERFLOW 75
#define EOPNOTSUPP 95

/* asm-generic/fcntl.h (octal, as there) */
#define O_RDONLY 00
#define O_WRONLY 01
#define O_RDWR 02
#define O_ACCMODE 03
#define O_CREAT 0100
#define O_EXCL 0200
#define O_NOCTTY 0400
#define O_TRUNC 01000
#define O_APPEND 02000
#define O_NONBLOCK 04000
#define O_DSYNC 010000
#define O_DIRECT 040000
#define O_LARGEFILE 0100000
#define O_DIRECTORY 0200000
#define O_NOFOLLOW 0400000
#define O_NOATIME 01000000
#define O_CLOEXEC 02000000
#define O_PATH 010000000
#define O_TMPFILE (020000000 | O_DIRECTORY)

#define F_DUPFD 0
#define F_GETFD 1
#define F_SETFD 2
#define F_GETFL 3
#define F_SETFL 4
#define F_DUPFD_CLOEXEC 1030 /* linux/fcntl.h: F_LINUX_SPECIFIC_BASE + 6 */
#define FD_CLOEXEC 1

/* linux/fcntl.h */
#define AT_FDCWD (-100)
#define AT_SYMLINK_NOFOLLOW 0x100
#define AT_EACCESS 0x200
#define AT_REMOVEDIR 0x200
#define AT_SYMLINK_FOLLOW 0x400
#define AT_NO_AUTOMOUNT 0x800
#define AT_EMPTY_PATH 0x1000

/* linux/fs.h */
#define SEEK_SET 0
#define SEEK_CUR 1
#define SEEK_END 2
#define SEEK_DATA 3
#define SEEK_HOLE 4
#define RENAME_NOREPLACE 1
#define RENAME_EXCHANGE 2
#define RENAME_WHITEOUT 4

/* unistd.h (access modes) */
#define F_OK 0
#define X_OK 1
#define W_OK 2
#define R_OK 4

/* linux/stat.h */
#define S_IFMT 0170000
#define S_IFSOCK 0140000
#define S_IFLNK 0120000
#define S_IFREG 0100000
#define S_IFBLK 060000
#define S_IFDIR 040000
#define S_IFCHR 020000
#define S_IFIFO 010000
#define S_ISUID 04000
#define S_ISGID 02000
#define S_ISVTX 01000

/* asm/stat.h (x86-64): struct stat, 144 bytes */
struct a_stat {
    u64 st_dev;
    u64 st_ino;
    u64 st_nlink;
    u32 st_mode;
    u32 st_uid;
    u32 st_gid;
    u32 pad0;
    u64 st_rdev;
    i64 st_size;
    i64 st_blksize;
    i64 st_blocks;
    u64 st_atime_sec, st_atime_nsec;
    u64 st_mtime_sec, st_mtime_nsec;
    u64 st_ctime_sec, st_ctime_nsec;
    i64 unused[3];
};

/* linux/stat.h: struct statx, 256 bytes */
struct a_statx_timestamp {
    i64 tv_sec;
    u32 tv_nsec;
    i32 reserved;
};
struct a_statx {
    u32 stx_mask;
    u32 stx_blksize;
    u64 stx_attributes;
    u32 stx_nlink;
    u32 stx_uid;
    u32 stx_gid;
    u16 stx_mode;
    u16 spare0;
    u64 stx_ino;
    u64 stx_size;
    u64 stx_blocks;
    u64 stx_attributes_mask;
    struct a_statx_timestamp stx_atime, stx_btime, stx_ctime, stx_mtime;
    u32 stx_rdev_major, stx_rdev_minor;
    u32 stx_dev_major, stx_dev_minor;
    u64 spare[14];
};
#define STATX_TYPE 0x1
#define STATX_MODE 0x2
#define STATX_NLINK 0x4
#define STATX_UID 0x8
#define STATX_GID 0x10
#define STATX_ATIME 0x20
#define STATX_MTIME 0x40
#define STATX_CTIME 0x80
#define STATX_INO 0x100
#define STATX_SIZE 0x200
#define STATX_BLOCKS 0x400
#define STATX_BASIC_STATS 0x7ff
#define STATX_BTIME 0x800
#define STATX__RESERVED 0x80000000u
#define AT_STATX_SYNC_AS_STAT 0x0000
#define AT_STATX_FORCE_SYNC 0x2000
#define AT_STATX_DONT_SYNC 0x4000

/* linux/dirent.h: struct linux_dirent64 (variable length) */
struct a_dirent64 {
    u64 d_ino;
    i64 d_off;
    u16 d_reclen;
    u8 d_type;
    char d_name[];
};
#define DT_UNKNOWN 0
#define DT_FIFO 1
#define DT_CHR 2
#define DT_DIR 4
#define DT_BLK 6
#define DT_REG 8
#define DT_LNK 10
#define DT_SOCK 12

/* asm-generic/statfs.h (x86-64, __statfs_word = long): 120 bytes */
struct a_statfs {
    i64 f_type;
    i64 f_bsize;
    u64 f_blocks;
    u64 f_bfree;
    u64 f_bavail;
    u64 f_files;
    u64 f_ffree;
    i32 f_fsid[2];
    i64 f_namelen;
    i64 f_frsize;
    i64 f_flags;
    i64 f_spare[4];
};

/* linux/uio.h */
struct a_iovec {
    void *iov_base;
    size_t iov_len;
};

/* linux/time_types.h, linux/time.h */
struct a_timespec {
    i64 tv_sec, tv_nsec;
};
struct a_timeval {
    i64 tv_sec, tv_usec;
};
struct a_itimerspec {
    struct a_timespec it_interval, it_value;
};
struct a_itimerval {
    struct a_timeval it_interval, it_value;
};
struct a_timezone {
    i32 tz_minuteswest, tz_dsttime;
};
#define CLOCK_PROCESS_CPUTIME_ID 2
#define CLOCK_THREAD_CPUTIME_ID 3
#define CLOCK_MONOTONIC_RAW 4
#define CLOCK_REALTIME_COARSE 5
#define CLOCK_MONOTONIC_COARSE 6
#define CLOCK_BOOTTIME 7
#define CLOCK_REALTIME_ALARM 8
#define CLOCK_BOOTTIME_ALARM 9
#define CLOCK_TAI 11
#define TIMER_ABSTIME 1
#define ITIMER_REAL 0
#define ITIMER_VIRTUAL 1
#define ITIMER_PROF 2

/* asm-generic/siginfo.h: struct sigevent, 64 bytes */
struct a_sigevent {
    u64 sigev_value;
    i32 sigev_signo;
    i32 sigev_notify;
    i32 sigev_tid;
    i32 pad[11];
};
#define SIGEV_SIGNAL 0
#define SIGEV_NONE 1
#define SIGEV_THREAD_ID 4

/* linux/times.h */
struct a_tms {
    i64 tms_utime, tms_stime, tms_cutime, tms_cstime;
};

/* linux/resource.h: struct rusage, 144 bytes */
struct a_rusage {
    struct a_timeval ru_utime, ru_stime;
    i64 ru_maxrss, ru_ixrss, ru_idrss, ru_isrss, ru_minflt, ru_majflt, ru_nswap, ru_inblock, ru_oublock, ru_msgsnd,
        ru_msgrcv, ru_nsignals, ru_nvcsw, ru_nivcsw;
};
#define RUSAGE_SELF 0
#define RUSAGE_CHILDREN (-1)
#define RUSAGE_THREAD 1

/* linux/resource.h, asm-generic/resource.h */
struct a_rlimit {
    u64 rlim_cur, rlim_max;
};
#define RLIM_INFINITY (~0UL)
#define RLIMIT_CPU 0
#define RLIMIT_FSIZE 1
#define RLIMIT_DATA 2
#define RLIMIT_STACK 3
#define RLIMIT_CORE 4
#define RLIMIT_RSS 5
#define RLIMIT_NPROC 6
#define RLIMIT_NOFILE 7
#define RLIMIT_MEMLOCK 8
#define RLIMIT_AS 9
#define RLIMIT_LOCKS 10
#define RLIMIT_SIGPENDING 11
#define RLIMIT_MSGQUEUE 12
#define RLIMIT_NICE 13
#define RLIMIT_RTPRIO 14
#define RLIMIT_RTTIME 15
#define RLIM_NLIMITS 16

/* linux/sysinfo.h: struct sysinfo, 112 bytes on x86-64 */
struct a_sysinfo {
    i64 uptime;
    u64 loads[3];
    u64 totalram, freeram, sharedram, bufferram, totalswap, freeswap;
    u16 procs;
    u16 pad;
    u32 pad2;
    u64 totalhigh, freehigh;
    u32 mem_unit;
    char f[4];
};

/* linux/utsname.h: struct new_utsname */
struct a_utsname {
    char sysname[65], nodename[65], release[65], version[65], machine[65], domainname[65];
};

/* linux/wait.h */
#define WNOHANG 0x1
#define WUNTRACED 0x2
#define WSTOPPED 0x2
#define WEXITED 0x4
#define WCONTINUED 0x8
#define WNOWAIT 0x01000000
#define __WNOTHREAD 0x20000000
#define __WALL 0x40000000
#define __WCLONE 0x80000000
#define P_ALL 0
#define P_PID 1
#define P_PGID 2
/* asm-generic/siginfo.h: SIGCHLD codes */
#define CLD_EXITED 1
#define CLD_KILLED 2
#define CLD_DUMPED 3

/* asm-generic/siginfo.h: siginfo_t is 128 bytes; the SIGCHLD fields */
struct a_siginfo {
    i32 si_signo;
    i32 si_errno;
    i32 si_code;
    i32 pad;
    i32 si_pid;
    i32 si_uid;
    i32 si_status;
    i32 rest[25];
};

/* linux/sched.h */
#define CSIGNAL 0xff
#define CLONE_VM 0x100
#define CLONE_FS 0x200
#define CLONE_FILES 0x400
#define CLONE_SIGHAND 0x800
#define CLONE_VFORK 0x4000
#define CLONE_PARENT 0x8000
#define CLONE_THREAD 0x10000
#define CLONE_SYSVSEM 0x40000
#define CLONE_SETTLS 0x80000
#define CLONE_PARENT_SETTID 0x100000
#define CLONE_CHILD_CLEARTID 0x200000
#define CLONE_CHILD_SETTID 0x01000000

/* asm-generic/signal.h (x86-64 numbers) */
#define SIGHUP 1
#define SIGKILL 9
#define SIGUSR1 10
#define SIGSEGV 11
#define SIGALRM 14
#define SIGTERM 15
#define SIGCHLD 17
#define SIG_BLOCK 0
#define SIG_UNBLOCK 1
#define SIG_SETMASK 2

/* asm-generic/mman-common.h, asm/mman.h */
#define PROT_EXEC 0x4
#define PROT_GROWSDOWN 0x01000000
#define PROT_GROWSUP 0x02000000
#define MAP_TYPE 0x0f
#define MAP_SHARED_VALIDATE 0x03
#define MAP_32BIT 0x40
#define MAP_GROWSDOWN 0x0100
#define MAP_NORESERVE 0x4000
#define MAP_POPULATE 0x8000
#define MAP_FIXED_NOREPLACE 0x100000
#define MREMAP_MAYMOVE 1
#define MREMAP_FIXED 2
#define MADV_NORMAL 0
#define MADV_RANDOM 1
#define MADV_SEQUENTIAL 2
#define MADV_WILLNEED 3
#define MADV_DONTNEED 4
#define MADV_FREE 8
#define MADV_REMOVE 9
#define MADV_DONTFORK 10
#define MADV_DOFORK 11
#define MADV_MERGEABLE 12
#define MADV_UNMERGEABLE 13
#define MADV_HUGEPAGE 14
#define MADV_NOHUGEPAGE 15
#define MADV_DONTDUMP 16
#define MADV_DODUMP 17
#define MADV_WIPEONFORK 18
#define MADV_KEEPONFORK 19
#define MADV_COLD 20
#define MADV_PAGEOUT 21

/* linux/stat.h: utimensat special nanosecond values */
#define UTIME_NOW ((1L << 30) - 1L)
#define UTIME_OMIT ((1L << 30) - 2L)

/* linux/random.h */
#define GRND_RANDOM 0x2
#define GRND_INSECURE 0x4

/* auxv tags, linux/auxvec.h */
#define AT_NULL 0
#define AT_PAGESZ 6
#define AT_EXECFN 31

#define syscall4(n, a, b, c, d) syscall6((n), (i64)(a), (i64)(b), (i64)(c), (i64)(d), 0, 0)
#define syscall5(n, a, b, c, d, e) syscall6((n), (i64)(a), (i64)(b), (i64)(c), (i64)(d), (i64)(e), 0)

#define A_UNUSED __attribute__((unused))

/* One check whose result must equal `want`; prints the result if not. */
static A_UNUSED void expect(const char *name, i64 got, i64 want) { check(name, got == want, got); }

static A_UNUSED int mem_eq(const void *a, const void *b, size_t n) {
    const unsigned char *x = a, *y = b;
    for (size_t i = 0; i < n; i++)
        if (x[i] != y[i]) return 0;
    return 1;
}

/* Copy NUL-terminated `src` to `dst`, return the end of `dst`. */
static A_UNUSED char *str_cpy(char *dst, const char *src) {
    while ((*dst = *src++)) dst++;
    return dst;
}

/* `a` followed by `b` into `out`. */
static A_UNUSED char *str_cat2(char *out, const char *a, const char *b) {
    str_cpy(str_cpy(out, a), b);
    return out;
}

static A_UNUSED i64 a_open(const char *path, i64 flags, i64 mode) {
    return syscall4(SYS_openat, AT_FDCWD, path, flags, mode);
}
static A_UNUSED i64 a_close(i64 fd) { return syscall1(SYS_close, fd); }
static A_UNUSED i64 a_write(i64 fd, const void *buf, size_t n) { return syscall3(SYS_write, fd, buf, n); }
static A_UNUSED i64 a_read(i64 fd, void *buf, size_t n) { return syscall3(SYS_read, fd, buf, n); }
static A_UNUSED i64 a_mkdir(const char *path, i64 mode) { return syscall3(SYS_mkdirat, AT_FDCWD, path, mode); }
static A_UNUSED i64 a_unlink(const char *path) { return syscall3(SYS_unlinkat, AT_FDCWD, path, 0); }
static A_UNUSED i64 a_rmdir(const char *path) { return syscall3(SYS_unlinkat, AT_FDCWD, path, AT_REMOVEDIR); }
static A_UNUSED i64 a_stat_path(const char *path, struct a_stat *st) {
    return syscall4(SYS_newfstatat, AT_FDCWD, path, st, 0);
}
static A_UNUSED i64 a_lstat_path(const char *path, struct a_stat *st) {
    return syscall4(SYS_newfstatat, AT_FDCWD, path, st, AT_SYMLINK_NOFOLLOW);
}

/* Create `path` with `text` as its contents; returns 0 or -errno. */
static A_UNUSED i64 a_make_file(const char *path, const char *text, i64 mode) {
    i64 fd = a_open(path, O_WRONLY | O_CREAT | O_TRUNC, mode);
    if (fd < 0) return fd;
    i64 n = a_write(fd, text, str_len(text));
    a_close(fd);
    return n < 0 ? n : 0;
}

/* The page size the tests assume (x86-64). */
#define A_PAGE 4096

/* ---- Time without system calls ------------------------------------- */

static A_UNUSED u64 a_rdtsc(void) {
    u32 lo, hi;
    __asm__ volatile("rdtsc" : "=a"(lo), "=d"(hi));
    return ((u64)hi << 32) | lo;
}

static A_UNUSED void a_relax(void) { __asm__ volatile("pause" ::: "memory"); }

/* Spin, without system calls, until *p == want or about `ticks` TSC ticks
 * passed. Returns 1 if the value was seen. */
static A_UNUSED int a_spin_until(volatile i32 *p, i32 want, u64 ticks) {
    u64 start = a_rdtsc();
    while (*p != want) {
        if (a_rdtsc() - start > ticks) return *p == want;
        a_relax();
    }
    return 1;
}

/* About 10 s of TSC ticks at 3 GHz (at least 3 s on any x86-64 TSC up to
 * 10 GHz): the most a test waits for another task before giving up. */
#define A_SPIN_LIMIT 30000000000UL

static A_UNUSED void a_spin_ticks(u64 ticks) {
    u64 start = a_rdtsc();
    while (a_rdtsc() - start < ticks) a_relax();
}

/* ---- Children and threads ------------------------------------------ */

/*
 * a_clone(flags, stack_top, ptid, ctid, tls, fn, arg): the raw clone
 * system call (x86-64 argument order: flags, stack, parent_tid,
 * child_tid, tls). In the child, `fn(arg)` runs on `stack_top` and its
 * return value is passed to exit (60). Returns the child's id or -errno in
 * the parent.
 */
long a_clone(u64 flags, void *stack_top, i32 *ptid, i32 *ctid, u64 tls, int (*fn)(void *), void *arg);
__asm__(".text\n"
        ".global a_clone\n"
        "a_clone:\n"
        "    mov 8(%rsp), %rax\n"   /* arg (7th argument, on the stack) */
        "    and $-16, %rsi\n"
        "    sub $16, %rsi\n"
        "    mov %r9, 0(%rsi)\n"    /* fn */
        "    mov %rax, 8(%rsi)\n"   /* arg */
        "    mov %rcx, %r10\n"      /* child_tid */
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

/* Stacks for children and threads. */
#define A_STACK_SIZE 65536
static A_UNUSED char a_stacks[4][A_STACK_SIZE] __attribute__((aligned(16)));

/*
 * A child process that shares this one's memory (CLONE_VM) and sends
 * SIGCHLD at its end. The kernel clears `*done` when the child has ended
 * (CLONE_CHILD_CLEARTID), so the parent can wait for that by watching
 * memory, without a system call: the order of the two processes' system
 * calls is then the same on every run. Returns the child's pid.
 */
static A_UNUSED long a_spawn(int slot, volatile i32 *done, int (*fn)(void *), void *arg) {
    *done = -1;
    return a_clone(CLONE_VM | CLONE_CHILD_CLEARTID | SIGCHLD, a_stacks[slot] + A_STACK_SIZE, 0, (i32 *)done, 0, fn,
                   arg);
}

/* Block SIGCHLD, so a child's end does not interrupt this process at a
 * point that varies from run to run (a tracer would report it there). */
static A_UNUSED i64 a_block_sigchld(void) {
    u64 set = 1UL << (SIGCHLD - 1);
    return syscall4(SYS_rt_sigprocmask, SIG_BLOCK, &set, 0, 8);
}

/* wait4 status decoding (sys/wait.h macros, as documented in wait(2)). */
#define A_WIFEXITED(s) (((s) & 0x7f) == 0)
#define A_WEXITSTATUS(s) (((s) >> 8) & 0xff)
#define A_WIFSIGNALED(s) (((s) & 0x7f) != 0 && ((s) & 0x7f) != 0x7f)
#define A_WTERMSIG(s) ((s) & 0x7f)

/* ---- The initial stack ---------------------------------------------- */

static A_UNUSED i64 a_argc(void) { return (i64)poly_initial_sp[0]; }
static A_UNUSED char **a_argv(void) { return (char **)(poly_initial_sp + 1); }
static A_UNUSED char **a_envp(void) { return a_argv() + a_argc() + 1; }

/* Value of auxiliary vector entry `tag`, 0 if there is none. */
static A_UNUSED u64 a_auxv(u64 tag) {
    char **e = a_envp();
    while (*e) e++;
    for (u64 *a = (u64 *)(e + 1); a[0] != AT_NULL; a += 2)
        if (a[0] == tag) return a[1];
    return 0;
}

/* The path this program was started from: AT_EXECFN, else argv[0]. */
static A_UNUSED const char *a_self_path(void) {
    const char *p = (const char *)a_auxv(AT_EXECFN);
    return p ? p : a_argv()[0];
}

#endif
