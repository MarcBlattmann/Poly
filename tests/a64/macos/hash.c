#include "rt.h"
#include <arm_acle.h>
static const u32 K[64] = {
0x428a2f98,0x71374491,0xb5c0fbcf,0xe9b5dba5,0x3956c25b,0x59f111f1,0x923f82a4,0xab1c5ed5,0xd807aa98,0x12835b01,0x243185be,0x550c7dc3,0x72be5d74,0x80deb1fe,0x9bdc06a7,0xc19bf174,
0xe49b69c1,0xefbe4786,0x0fc19dc6,0x240ca1cc,0x2de92c6f,0x4a7484aa,0x5cb0a9dc,0x76f988da,0x983e5152,0xa831c66d,0xb00327c8,0xbf597fc7,0xc6e00bf3,0xd5a79147,0x06ca6351,0x14292967,
0x27b70a85,0x2e1b2138,0x4d2c6dfc,0x53380d13,0x650a7354,0x766a0abb,0x81c2c92e,0x92722c85,0xa2bfe8a1,0xa81a664b,0xc24b8b70,0xc76c51a3,0xd192e819,0xd6990624,0xf40e3585,0x106aa070,
0x19a4c116,0x1e376c08,0x2748774c,0x34b0bcb5,0x391c0cb3,0x4ed8aa4a,0x5b9cca4f,0x682e6ff3,0x748f82ee,0x78a5636f,0x84c87814,0x8cc70208,0x90befffa,0xa4506ceb,0xbef9a3f7,0xc67178f2};
#define ROR(x,n) (((x) >> (n)) | ((x) << (32 - (n))))
static void sha256(const u8 *m, size_t len, u8 out[32]) {
    u32 h[8] = {0x6a09e667,0xbb67ae85,0x3c6ef372,0xa54ff53a,0x510e527f,0x9b05688c,0x1f83d9ab,0x5be0cd19};
    u8 blk[64]; size_t i = 0; int done = 0, padded = 0;
    while (!done) {
        size_t n = len - i < 64 ? len - i : 64;
        if (i <= len) { memcpy(blk, m + i, n); } else n = 0;
        if (n < 64) {
            memset(blk + n, 0, 64 - n);
            if (!padded) { blk[n] = 0x80; padded = 1; }
            if (n < 56) { u64 bits = (u64)len * 8; for (int k = 0; k < 8; k++) blk[63 - k] = (u8)(bits >> (8 * k)); done = 1; }
        }
        u32 w[64];
        for (int t = 0; t < 16; t++) w[t] = (u32)blk[4*t] << 24 | (u32)blk[4*t+1] << 16 | (u32)blk[4*t+2] << 8 | blk[4*t+3];
        for (int t = 16; t < 64; t++) { u32 s0 = ROR(w[t-15],7) ^ ROR(w[t-15],18) ^ (w[t-15] >> 3), s1 = ROR(w[t-2],17) ^ ROR(w[t-2],19) ^ (w[t-2] >> 10); w[t] = w[t-16] + s0 + w[t-7] + s1; }
        u32 a=h[0],b=h[1],c=h[2],d=h[3],e=h[4],f=h[5],g=h[6],hh=h[7];
        for (int t = 0; t < 64; t++) {
            u32 t1 = hh + (ROR(e,6) ^ ROR(e,11) ^ ROR(e,25)) + ((e & f) ^ (~e & g)) + K[t] + w[t];
            u32 t2 = (ROR(a,2) ^ ROR(a,13) ^ ROR(a,22)) + ((a & b) ^ (a & c) ^ (b & c));
            hh = g; g = f; f = e; e = d + t1; d = c; c = b; b = a; a = t1 + t2;
        }
        h[0]+=a;h[1]+=b;h[2]+=c;h[3]+=d;h[4]+=e;h[5]+=f;h[6]+=g;h[7]+=hh;
        i += 64;
    }
    for (int k = 0; k < 8; k++) for (int j = 0; j < 4; j++) out[4*k+j] = (u8)(h[k] >> (24 - 8*j));
}
static u32 crc_soft(const u8 *p, size_t n) {
    u32 c = 0xFFFFFFFF;
    while (n--) { c ^= *p++; for (int k = 0; k < 8; k++) c = (c >> 1) ^ (0xEDB88320 & -(c & 1)); }
    return ~c;
}
static u32 crc_hw(const u8 *p, size_t n) {
    u32 c = 0xFFFFFFFF;
    while (n >= 8) { u64 v; memcpy(&v, p, 8); c = __crc32d(c, v); p += 8; n -= 8; }
    while (n--) c = __crc32b(c, *p++);
    return ~c;
}
static u32 crc32c_hw(const u8 *p, size_t n) {
    u32 c = 0xFFFFFFFF;
    while (n >= 4) { u32 v; memcpy(&v, p, 4); c = __crc32cw(c, v); p += 4; n -= 4; }
    while (n--) c = __crc32cb(c, *p++);
    return ~c;
}
static u64 fnv(const u8 *p, size_t n) { u64 h = 0xcbf29ce484222325ull; while (n--) { h ^= *p++; h *= 0x100000001b3ull; } return h; }
static u8 data[100000];
int run(void) {
    const char *abc = "abc";
    u8 d[32];
    sha256((const u8 *)abc, 3, d);
    puts_("sha256(abc)=");
    for (int i = 0; i < 32; i++) { putc_("0123456789abcdef"[d[i] >> 4]); putc_("0123456789abcdef"[d[i] & 15]); }
    nl();
    for (u32 i = 0; i < sizeof data; i++) data[i] = (u8)(i * 2654435761u >> 13);
    sha256(data, sizeof data, d);
    puts_("sha256(data)=");
    for (int i = 0; i < 32; i++) { putc_("0123456789abcdef"[d[i] >> 4]); putc_("0123456789abcdef"[d[i] & 15]); }
    nl();
    puts_("crc32 soft="); putx(crc_soft(data, sizeof data)); puts_(" hw="); putx(crc_hw(data, sizeof data)); puts_(" crc32c="); putx(crc32c_hw(data, sizeof data)); nl();
    puts_("fnv1a="); putx(fnv(data, sizeof data)); nl();
    return 0;
}
