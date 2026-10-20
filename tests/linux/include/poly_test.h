/*
 * Freestanding support for the Linux test corpus: no libc, raw system calls
 * only, so every syscall a test makes is one it asks for.
 *
 * Written for Poly from the public x86-64 psABI and the Linux UAPI headers
 * (syscall numbers: asm/unistd_64.h; errno values: asm-generic/errno-base.h
 * and asm-generic/errno.h). Behaviour only; no code is copied from anywhere.
 *
 * Output protocol (machine readable, one line per check, deterministic so
 * the diff runner can compare it byte for byte):
 *
 *     ok <check>
 *     not ok <check> (got <value>)
 *     summary <passed> passed, <failed> failed
 *
 * The exit status is 0 when every check passed, 1 otherwise. Never print
 * addresses, times, pids or random bytes: they differ between runs.
 */
#ifndef POLY_TEST_H
#define POLY_TEST_H

typedef unsigned long u64;
typedef long i64;
typedef unsigned long size_t;

/* System call numbers, asm/unistd_64.h. */
#define SYS_read 0
#define SYS_write 1
#define SYS_mmap 9
#define SYS_mprotect 10
#define SYS_munmap 11
#define SYS_brk 12
#define SYS_uname 63
#define SYS_arch_prctl 158
#define SYS_clock_gettime 228
#define SYS_exit_group 231
#define SYS_getrandom 318

/* errno values, asm-generic/errno-base.h and asm-generic/errno.h. */
#define EBADF 9
#define ENOMEM 12
#define EFAULT 14
#define EINVAL 22
#define ENOSYS 38

/* asm-generic/mman-common.h, asm-generic/mman.h */
#define PROT_NONE 0x0
#define PROT_READ 0x1
#define PROT_WRITE 0x2
#define MAP_SHARED 0x01
#define MAP_PRIVATE 0x02
#define MAP_FIXED 0x10
#define MAP_ANONYMOUS 0x20

/* asm/prctl.h */
#define ARCH_SET_FS 0x1002
#define ARCH_GET_FS 0x1003

/* linux/time.h */
#define CLOCK_REALTIME 0
#define CLOCK_MONOTONIC 1

/* linux/random.h */
#define GRND_NONBLOCK 0x1

static inline i64 syscall6(i64 n, i64 a, i64 b, i64 c, i64 d, i64 e, i64 f) {
    i64 ret;
    register i64 r10 __asm__("r10") = d;
    register i64 r8 __asm__("r8") = e;
    register i64 r9 __asm__("r9") = f;
    __asm__ volatile("syscall"
                     : "=a"(ret)
                     : "a"(n), "D"(a), "S"(b), "d"(c), "r"(r10), "r"(r8), "r"(r9)
                     : "rcx", "r11", "memory");
    return ret;
}

#define syscall0(n) syscall6((n), 0, 0, 0, 0, 0, 0)
#define syscall1(n, a) syscall6((n), (i64)(a), 0, 0, 0, 0, 0)
#define syscall2(n, a, b) syscall6((n), (i64)(a), (i64)(b), 0, 0, 0, 0)
#define syscall3(n, a, b, c) syscall6((n), (i64)(a), (i64)(b), (i64)(c), 0, 0, 0)

/* The compiler may emit calls to these even in freestanding code. */
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

static __attribute__((unused)) void put_fd(int fd, const char *s) { syscall3(SYS_write, fd, s, str_len(s)); }
static __attribute__((unused)) void put(const char *s) { put_fd(1, s); }

static __attribute__((unused)) void put_num(i64 v) {
    char buf[24];
    int i = sizeof buf;
    u64 u = v < 0 ? -(u64)v : (u64)v;
    do buf[--i] = (char)('0' + u % 10);
    while (u /= 10);
    if (v < 0) buf[--i] = '-';
    syscall3(SYS_write, 1, buf + i, sizeof buf - i);
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

static __attribute__((unused)) int finish(void) {
    put("summary ");
    put_num(passed);
    put(" passed, ");
    put_num(failed);
    put(" failed\n");
    return failed ? 1 : 0;
}

static __attribute__((noreturn)) void exit_group(int code) {
    for (;;) syscall1(SYS_exit_group, code);
}

int test_main(void);

__attribute__((used)) static void start_c(void) { exit_group(test_main()); }

/* Entry point: clear the frame pointer, align the stack, run the test. */
__asm__(".text\n"
        ".global _start\n"
        "_start:\n"
        "    xor %ebp, %ebp\n"
        "    and $-16, %rsp\n"
        "    call start_c\n"
        "    hlt\n");

#endif
