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
#define SYS_munmap 73
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
#define EBADF 9
#define ENOMEM 12
#define EACCES 13
#define EFAULT 14
#define EEXIST 17
#define ENOTDIR 20
#define EISDIR 21
#define EINVAL 22
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

/* <sys/stat.h> */
#define S_IFMT 0170000
#define S_IFCHR 0020000
#define S_IFDIR 0040000
#define S_IFREG 0100000
#define S_IFIFO 0010000

/* <sys/dirent.h> */
#define DT_UNKNOWN 0
#define DT_DIR 4
#define DT_REG 8

/* <sys/sysctl.h> */
#define CTL_MAXNAME 12
#define CTL_KERN 1
#define CTL_HW 6
#define KERN_OSTYPE 1
#define KERN_OSRELEASE 2
#define HW_MACHINE 1
#define HW_NCPU 3
#define HW_PAGESIZE 7

/* <mach/kern_return.h>, <mach/port.h> */
#define KERN_SUCCESS 0
#define MACH_PORT_NULL 0u
#define MACH_PORT_DEAD 0xffffffffu

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

#define bsd0(n) poly_bsd6((n), 0, 0, 0, 0, 0, 0)
#define bsd1(n, a) poly_bsd6((n), (i64)(a), 0, 0, 0, 0, 0)
#define bsd2(n, a, b) poly_bsd6((n), (i64)(a), (i64)(b), 0, 0, 0, 0)
#define bsd3(n, a, b, c) poly_bsd6((n), (i64)(a), (i64)(b), (i64)(c), 0, 0, 0)
#define bsd4(n, a, b, c, d) poly_bsd6((n), (i64)(a), (i64)(b), (i64)(c), (i64)(d), 0, 0)
#define bsd6(n, a, b, c, d, e, f) poly_bsd6((n), (i64)(a), (i64)(b), (i64)(c), (i64)(d), (i64)(e), (i64)(f))
#define mach0(n) poly_mach6((n), 0, 0, 0, 0, 0, 0)
#define mach1(n, a) poly_mach6((n), (i64)(a), 0, 0, 0, 0, 0)

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

int test_main(void);

int main(void) { poly_exit(test_main()); }

#endif
