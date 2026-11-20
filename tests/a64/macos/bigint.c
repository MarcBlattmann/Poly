#include "rt.h"
typedef unsigned __int128 u128;
#define N 32
static void mul(u64 *r, const u64 *a, const u64 *b) {
    u64 t[2*N] = {0};
    for (int i = 0; i < N; i++) { u64 c = 0; for (int j = 0; j < N; j++) { u128 p = (u128)a[i] * b[j] + t[i+j] + c; t[i+j] = (u64)p; c = (u64)(p >> 64); } t[i+N] = c; }
    for (int i = 0; i < N; i++) r[i] = t[i];
}
static void add(u64 *r, const u64 *a, const u64 *b) { u64 c = 0; for (int i = 0; i < N; i++) { u64 s = a[i] + c; c = s < c; s += b[i]; c += s < b[i]; r[i] = s; } }
static u64 a[N], b[N], c[N];
static u64 gcd(u64 x, u64 y) { while (y) { u64 t = x % y; x = y; y = t; } return x; }
static i64 sdiv_sum(void) { i64 s = 0; for (i64 i = -1000; i < 1000; i++) if (i) s += 1000000007 / i + (-77777 % i); return s; }
int run(void) {
    for (int i = 0; i < N; i++) { a[i] = 0x9E3779B97F4A7C15ull * (i + 1); b[i] = ~(u64)i * 0xBF58476D1CE4E5B9ull; }
    for (int k = 0; k < 200; k++) { mul(c, a, b); add(a, c, b); }
    u64 x = 0; for (int i = 0; i < N; i++) x ^= a[i] * (i + 1);
    puts_("bigint mix="); putx(x); nl();
    u64 g = 0; for (u64 i = 1; i < 2000; i++) g += gcd(i * 7919, 1000000 + i);
    puts_("gcd sum="); putu(g); nl();
    puts_("signed div sum="); puti(sdiv_sum()); nl();
    u64 pc = 0, lz = 0; for (u64 v = 1; v; v <<= 1) { pc += __builtin_popcountll(v * 0x5555); lz += __builtin_clzll(v) + __builtin_ctzll(v | 0x8000000000000000ull); }
    puts_("bits="); putu(pc); putc_(' '); putu(lz); nl();
    u64 rev = 0; for (u64 i = 0; i < 1000; i++) rev ^= __builtin_bitreverse64(i * 0x1234567) + __builtin_bswap64(i) + __builtin_bswap32((u32)i);
    puts_("rev="); putx(rev); nl();
    return 0;
}
