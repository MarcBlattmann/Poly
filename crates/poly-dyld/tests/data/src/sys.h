/* Raw BSD system calls for the poly-dyld test programs: x86_64 `syscall`
 * with class 2 in the top byte of rax, arm64 `svc #0x80` with the number in
 * x16 (the convention in specs/macos/bsd/write.yaml). Written for Poly; no
 * Apple or third-party source was read. */
#ifndef POLY_DYLD_TEST_SYS_H
#define POLY_DYLD_TEST_SYS_H

#define SYS_exit 1
#define SYS_write 4

static inline long poly_syscall3(long n, long a, long b, long c) {
#if defined(__x86_64__)
    long r;
    __asm__ volatile("syscall"
                     : "=a"(r)
                     : "a"(0x2000000L | n), "D"(a), "S"(b), "d"(c)
                     : "rcx", "r11", "memory", "cc");
    return r;
#elif defined(__aarch64__)
    register long x0 __asm__("x0") = a;
    register long x1 __asm__("x1") = b;
    register long x2 __asm__("x2") = c;
    register long x16 __asm__("x16") = n;
    __asm__ volatile("svc #0x80" : "+r"(x0), "+r"(x1) : "r"(x2), "r"(x16) : "memory", "cc");
    return x0;
#else
#error "x86_64 or arm64 only"
#endif
}

static inline unsigned long poly_strlen(const char *s) {
    unsigned long n = 0;
    while (s[n]) n++;
    return n;
}

#endif
