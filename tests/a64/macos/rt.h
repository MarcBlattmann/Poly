/* A tiny freestanding runtime for the arm64 macOS test programs (ADR 0239).
 * No libSystem calls: system calls go straight to the kernel with the arm64
 * Darwin convention (number in x16, `svc #0x80`, carry set on error), so the
 * same binary runs on a real Mac and on Poly's translator. */
#pragma once
typedef unsigned long u64;
typedef long i64;
typedef unsigned int u32;
typedef int i32;
typedef unsigned short u16;
typedef unsigned char u8;
typedef unsigned long size_t;

static inline long sys3(long n, long a, long b, long c) {
    register long x16 __asm__("x16") = n;
    register long x0 __asm__("x0") = a;
    register long x1 __asm__("x1") = b;
    register long x2 __asm__("x2") = c;
    __asm__ volatile("svc #0x80" : "+r"(x0), "+r"(x1) : "r"(x16), "r"(x2) : "memory", "cc");
    return x0;
}
#define SYS_exit 1
#define SYS_write 4

__attribute__((noreturn)) static inline void sys_exit(int code) {
    sys3(SYS_exit, code, 0, 0);
    __builtin_unreachable();
}

/* The compiler may call these for struct copies and loops. */
void *memcpy(void *d, const void *s, size_t n);
void *memset(void *d, int c, size_t n);
void *memmove(void *d, const void *s, size_t n);
void bzero(void *d, size_t n);
int memcmp(const void *a, const void *b, size_t n);

static char out_buf[4096];
static size_t out_len;
static void flush(void) {
    if (out_len) sys3(SYS_write, 1, (long)out_buf, (long)out_len);
    out_len = 0;
}
static void putc_(char c) {
    if (out_len == sizeof out_buf) flush();
    out_buf[out_len++] = c;
}
static void puts_(const char *s) { while (*s) putc_(*s++); }
static void putu(u64 v) {
    char t[24]; int n = 0;
    do { t[n++] = (char)('0' + v % 10); v /= 10; } while (v);
    while (n) putc_(t[--n]);
}
static void puti(i64 v) { if (v < 0) { putc_('-'); putu((u64)0 - (u64)v); } else putu((u64)v); }
static void putx(u64 v) {
    puts_("0x");
    for (int i = 60; i >= 0; i -= 4) putc_("0123456789abcdef"[(v >> i) & 15]);
}
/* Fixed-point print of a double with `digits` decimals (no libm). */
static void putf(double d, int digits) {
    if (d != d) { puts_("nan"); return; }
    if (d < 0) { putc_('-'); d = -d; }
    if (d > 1e18) { puts_("big"); return; }
    u64 ip = (u64)d;
    double frac = d - (double)ip;
    putu(ip);
    putc_('.');
    for (int i = 0; i < digits; i++) { frac *= 10; int dg = (int)frac; putc_((char)('0' + dg)); frac -= dg; }
}
static void nl(void) { putc_('\n'); }

#ifndef RT_NO_MAIN
int run(void);
int main(void) { int r = run(); flush(); return r; }
#endif
