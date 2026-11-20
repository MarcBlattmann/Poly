/* A stand-in for /usr/lib/libSystem.B.dylib for the arm64 test programs of
 * the translated XNU process (ADR 0257), written for Poly: `exit`,
 * `poly_write`, a small `malloc` family on anonymous `mmap`, and the few
 * string functions the compiler may call. Nothing of Apple's library; the
 * names are the public C library's. Poly's real libSystem is a later
 * milestone (ADR 0157).
 *
 * malloc: blocks of 16 to 4096 bytes come from power-of-two size classes
 * carved out of 1 MiB chunks; larger ones are mappings of their own. Each
 * block has a 16-byte header with its size, so free and realloc find it. */
#include "sys.h"

typedef unsigned long size_t;

#define CHUNK (1UL << 20)
#define CLASSES 9 /* 16, 32, ..., 4096 */
#define HEADER 16UL

static void *free_list[CLASSES];
static char *chunk_next, *chunk_end;
static unsigned long mapped_bytes;
int libsystem_inits;

__attribute__((constructor)) static void libsystem_init(void) { libsystem_inits++; }

void exit(int status) {
    poly_syscall3(SYS_exit, status, 0, 0);
    for (;;) {
    }
}

long poly_write(int fd, const char *buf, long len) { return poly_syscall3(SYS_write, fd, (long)buf, len); }

void *memcpy(void *d, const void *s, size_t n) {
    char *dd = d;
    const char *ss = s;
    while (n--) *dd++ = *ss++;
    return d;
}

void *memmove(void *d, const void *s, size_t n) {
    char *dd = d;
    const char *ss = s;
    if (dd < ss) {
        while (n--) *dd++ = *ss++;
    } else {
        while (n--) dd[n] = ss[n];
    }
    return d;
}

void *memset(void *d, int c, size_t n) {
    char *dd = d;
    while (n--) *dd++ = (char)c;
    return d;
}

void bzero(void *d, size_t n) { memset(d, 0, n); }

int memcmp(const void *a, const void *b, size_t n) {
    const unsigned char *x = a, *y = b;
    for (; n; n--, x++, y++)
        if (*x != *y) return *x - *y;
    return 0;
}

size_t strlen(const char *s) { return poly_strlen(s); }

static void *map(size_t len) {
    struct sysret r = poly_syscall6(SYS_mmap, 0, (long)len, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANON, -1, 0);
    if (r.failed) return 0;
    mapped_bytes += len;
    return (void *)r.value;
}

static int class_of(size_t n) {
    int c = 0;
    size_t s = 16;
    while (s < n) {
        s <<= 1;
        c++;
    }
    return c;
}

void *malloc(size_t n) {
    size_t need = n ? n : 1;
    if (need > (16UL << (CLASSES - 1))) {
        size_t len = (need + HEADER + 16383) & ~16383UL;
        char *p = map(len);
        if (!p) return 0;
        *(size_t *)p = len;
        ((size_t *)p)[1] = 0x6c61726765; /* "large" */
        return p + HEADER;
    }
    int c = class_of(need);
    if (free_list[c]) {
        void *p = free_list[c];
        free_list[c] = *(void **)p;
        return p;
    }
    size_t size = (16UL << c) + HEADER;
    if (!chunk_next || chunk_end - chunk_next < (long)size) {
        chunk_next = map(CHUNK);
        if (!chunk_next) return 0;
        chunk_end = chunk_next + CHUNK;
    }
    char *p = chunk_next;
    chunk_next += size;
    *(size_t *)p = (size_t)c;
    ((size_t *)p)[1] = 0x736d616c6c; /* "small" */
    return p + HEADER;
}

static size_t usable(void *p) {
    size_t *h = (size_t *)((char *)p - HEADER);
    return h[1] == 0x6c61726765 ? h[0] - HEADER : 16UL << h[0];
}

void free(void *p) {
    if (!p) return;
    size_t *h = (size_t *)((char *)p - HEADER);
    if (h[1] == 0x6c61726765) {
        size_t len = h[0];
        poly_syscall6(SYS_munmap, (long)h, (long)len, 0, 0, 0, 0);
        mapped_bytes -= len;
        return;
    }
    *(void **)p = free_list[h[0]];
    free_list[h[0]] = p;
}

void *calloc(size_t n, size_t m) {
    size_t total = n * m;
    if (m && total / m != n) return 0;
    void *p = malloc(total);
    if (p) memset(p, 0, total);
    return p;
}

void *realloc(void *p, size_t n) {
    if (!p) return malloc(n);
    size_t have = usable(p);
    if (n <= have) return p;
    void *q = malloc(n);
    if (!q) return 0;
    memcpy(q, p, have);
    free(p);
    return q;
}

unsigned long poly_mapped_bytes(void) { return mapped_bytes; }

void _tlv_bootstrap(void) {}

void dyld_stub_binder(void) __asm__("dyld_stub_binder");
void dyld_stub_binder(void) {}
