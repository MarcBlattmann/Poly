/* Raw BSD system calls for the arm64 test programs of the translated XNU
 * process (ADR 0257): `svc #0x80` with the call number in x16, arguments in
 * x0..x5, the carry flag set on error with the errno in x0 (ADR 0239,
 * specs/macos/bsd/write.yaml). Call numbers are those of the SDK's
 * <sys/syscall.h>, flags those of <sys/mman.h>. Written for Poly; no Apple
 * or third-party source was read. */
#ifndef POLY_A64_XNU_SYS_H
#define POLY_A64_XNU_SYS_H

#define SYS_exit 1
#define SYS_write 4
#define SYS_munmap 73
#define SYS_mmap 197

#define PROT_READ 1
#define PROT_WRITE 2
#define MAP_PRIVATE 2
#define MAP_ANON 0x1000

/* The result and whether the carry flag was set. */
struct sysret {
    long value;
    int failed;
};

static inline struct sysret poly_syscall6(long n, long a, long b, long c, long d, long e, long f) {
    register long x0 __asm__("x0") = a;
    register long x1 __asm__("x1") = b;
    register long x2 __asm__("x2") = c;
    register long x3 __asm__("x3") = d;
    register long x4 __asm__("x4") = e;
    register long x5 __asm__("x5") = f;
    register long x16 __asm__("x16") = n;
    long carry;
    __asm__ volatile("svc #0x80\n\tcset %[c], cs"
                     : "+r"(x0), "+r"(x1), [c] "=r"(carry)
                     : "r"(x2), "r"(x3), "r"(x4), "r"(x5), "r"(x16)
                     : "memory", "cc");
    struct sysret r = {x0, (int)carry};
    return r;
}

static inline long poly_syscall3(long n, long a, long b, long c) {
    return poly_syscall6(n, a, b, c, 0, 0, 0).value;
}

static inline unsigned long poly_strlen(const char *s) {
    unsigned long n = 0;
    while (s[n]) n++;
    return n;
}

#endif
