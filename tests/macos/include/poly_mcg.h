/*
 * Support for macOS test programs about CoreGraphics (ADR 0182, the
 * CoreGraphics batch). A test that includes this header says so in its header
 * comment with the line `poly-mac: coregraphics` (`mac build` then links it
 * with `-framework CoreGraphics -framework CoreFoundation` and lets it import
 * CoreGraphics's and CoreFoundation's own symbols; all other tests import
 * nothing, see poly_mtest.h), and defines `int test_main(void)`. Types and
 * constants come from the public SDK headers (<CoreGraphics/CoreGraphics.h>).
 *
 * The tests draw into a CGBitmapContext the test makes itself and report the
 * bytes of chosen pixels, or a hash of the whole buffer (FNV-1a of the bytes,
 * defined below, so it is the same on both sides), through the raw `write`
 * system call of poly_mcf.h (`check`, `value`, `finish`), never stdio.
 * The recorded output is therefore pixel exact: Poly's CoreGraphics has to give
 * the same bytes.
 *
 * Output protocol (as poly_mtest.h and the Linux corpus):
 *
 *     ok <check>
 *     not ok <check> (got <value>)
 *     value <name> <value>
 *     summary <passed> passed, <failed> failed
 *
 * Never print addresses or anything else that differs between runs. A test
 * starts in a fresh, empty scratch directory with an empty environment.
 *
 * Written for Poly from public sources and from observation only. No Apple or
 * third-party source (no Quartz, Cairo or Skia internals) was read.
 */
#ifndef POLY_MCG_H
#define POLY_MCG_H

#include "poly_mcf.h"
#include <CoreGraphics/CoreGraphics.h>
#include <stdint.h>

/* A width x height bitmap context over our own buffer, as 8 bit RGBA, premultiplied
 * alpha, last byte order (kCGImageAlphaPremultipliedLast), sRGB, zeroed. */
typedef struct {
    CGContextRef ctx;
    unsigned char *data;
    size_t w, h, stride;
    CGColorSpaceRef cs;
} pxbuf;

/* Pixel buffers come from a static pool (a test imports nothing but CoreGraphics' and
 * CoreFoundation's symbols, so no calloc); the pool is zero at start and handed out once. */
#define PX_POOL_BYTES (4u << 20)
static unsigned char px_pool[PX_POOL_BYTES];
static size_t px_pool_used;
static inline unsigned char *px_alloc(size_t n) {
    n = (n + 63) & ~(size_t)63;
    if (px_pool_used + n > PX_POOL_BYTES) return NULL;
    unsigned char *p = px_pool + px_pool_used;
    px_pool_used += n;
    return p;
}

static inline pxbuf px_make(size_t w, size_t h, CGBitmapInfo info, CGColorSpaceRef cs, size_t bytes_per_pixel, size_t bits_per_comp) {
    pxbuf b;
    b.w = w;
    b.h = h;
    b.stride = w * bytes_per_pixel;
    b.data = px_alloc(h * b.stride);
    b.cs = cs ? cs : CGColorSpaceCreateWithName(kCGColorSpaceSRGB);
    b.ctx = CGBitmapContextCreate(b.data, w, h, bits_per_comp, b.stride, b.cs, info);
    return b;
}
/* The usual: 8 bit RGBA, premultiplied last. */
static inline pxbuf px_rgba(size_t w, size_t h) {
    return px_make(w, h, (CGBitmapInfo)kCGImageAlphaPremultipliedLast, NULL, 4, 8);
}
/* Row 0 of the buffer is the TOP of the picture; CoreGraphics' own y axis points up. */
static inline const unsigned char *px_at(const pxbuf *b, size_t x, size_t row) { return b->data + row * b->stride + x * 4; }
static inline uint32_t px_u32(const pxbuf *b, size_t x, size_t row) {
    const unsigned char *p = px_at(b, x, row);
    return (uint32_t)p[0] << 24 | (uint32_t)p[1] << 16 | (uint32_t)p[2] << 8 | p[3];
}
static inline uint64_t px_hash(const pxbuf *b) {
    uint64_t h = 1469598103934665603ULL;
    for (size_t i = 0; i < b->h * b->stride; i++) h = (h ^ b->data[i]) * 1099511628211ULL;
    return h;
}
static inline void px_clear(pxbuf *b) { memset(b->data, 0, b->h * b->stride); }

/* Print the bytes of one pixel as "RRGGBBAA" hex, as a value line. */
static inline void px_value(const char *name, const pxbuf *b, size_t x, size_t row) {
    static const char hex[] = "0123456789abcdef";
    char s[9];
    const unsigned char *p = px_at(b, x, row);
    for (int i = 0; i < 4; i++) {
        s[i * 2] = hex[p[i] >> 4];
        s[i * 2 + 1] = hex[p[i] & 15];
    }
    s[8] = 0;
    value_str(name, s);
}
/* Print the whole buffer's hash as a value line (hex). */
static inline void px_hash_value(const char *name, const pxbuf *b) {
    static const char hex[] = "0123456789abcdef";
    char s[17];
    uint64_t h = px_hash(b);
    for (int i = 0; i < 16; i++) s[i] = hex[(h >> (60 - 4 * i)) & 15];
    s[16] = 0;
    value_str(name, s);
}
/* Print a grid of one channel (alpha by default) as rows of hex bytes, for a
 * small buffer: the pixel exact record of an antialiased edge. */
static inline void px_dump(const char *name, const pxbuf *b, int channel) {
    static const char hex[] = "0123456789abcdef";
    char line[512];
    for (size_t y = 0; y < b->h; y++) {
        size_t n = 0;
        for (size_t x = 0; x < b->w && n + 3 < sizeof line; x++) {
            unsigned char v = px_at(b, x, y)[channel];
            line[n++] = hex[v >> 4];
            line[n++] = hex[v & 15];
            line[n++] = x + 1 < b->w ? ' ' : 0;
        }
        line[n] = 0;
        char label[80];
        size_t k = 0;
        for (const char *p = name; *p && k < 60; p++) label[k++] = *p;
        label[k++] = ' ';
        label[k++] = (char)('0' + (y / 10) % 10);
        label[k++] = (char)('0' + y % 10);
        label[k] = 0;
        value_str(label, line);
    }
}

/* Print every pixel of a small buffer as RRGGBBAA hex, one row per value line. */
static inline void px_dump_rgba(const char *name, const pxbuf *b) {
    static const char hex[] = "0123456789abcdef";
    for (size_t y = 0; y < b->h; y++) {
        char line[1024];
        size_t n = 0;
        for (size_t x = 0; x < b->w && n + 10 < sizeof line; x++) {
            const unsigned char *p = px_at(b, x, y);
            for (int i = 0; i < 4; i++) {
                line[n++] = hex[p[i] >> 4];
                line[n++] = hex[p[i] & 15];
            }
            line[n++] = x + 1 < b->w ? ' ' : 0;
        }
        line[n] = 0;
        char label[80];
        size_t k = 0;
        for (const char *q = name; *q && k < 60; q++) label[k++] = *q;
        label[k++] = ' ';
        label[k++] = (char)('0' + (y / 10) % 10);
        label[k++] = (char)('0' + y % 10);
        label[k] = 0;
        value_str(label, line);
    }
}

/* A double as the hex of its bits (exact, the same on both architectures). */
static inline void value_dbl(const char *name, double d) {
    static const char hex[] = "0123456789abcdef";
    uint64_t u;
    memcpy(&u, &d, 8);
    char s[17];
    for (int i = 0; i < 16; i++) s[i] = hex[(u >> (60 - 4 * i)) & 15];
    s[16] = 0;
    value_str(name, s);
}

/* Fill a whole buffer's context with an opaque colour first (a plain backdrop). */
static inline void px_backdrop(pxbuf *b, double r, double g, double bl, double a) {
    CGContextSaveGState(b->ctx);
    CGContextSetBlendMode(b->ctx, kCGBlendModeCopy);
    CGContextSetRGBFillColor(b->ctx, (CGFloat)r, (CGFloat)g, (CGFloat)bl, (CGFloat)a);
    CGContextFillRect(b->ctx, CGRectMake(0, 0, (CGFloat)b->w, (CGFloat)b->h));
    CGContextRestoreGState(b->ctx);
}


/* A CGImage over a static pixel array (no copy: the provider points at the array).
 * `data` must stay alive; premultiplied last RGBA, 8 bits, sRGB. */
static inline CGImageRef px_image(const unsigned char *data, size_t w, size_t h, size_t row_bytes, CGBitmapInfo info, bool interpolate) {
    CGDataProviderRef prov = CGDataProviderCreateWithData(NULL, data, h * row_bytes, NULL);
    CGColorSpaceRef cs = CGColorSpaceCreateWithName(kCGColorSpaceSRGB);
    CGImageRef img = CGImageCreate(w, h, 8, 32, row_bytes, cs, info, prov, NULL, interpolate, kCGRenderingIntentDefault);
    CGDataProviderRelease(prov);
    return img;
}

#endif
